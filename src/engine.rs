use std::sync::{Arc, Mutex};
use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::Duration;
use std::fs::{self, File};
use std::process::Command;
use std::io::{Read, Write, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::env;
use std::fmt;
use std::ffi::{CString, CStr};
use chrono::{DateTime, Local, Datelike, Timelike};
use std::os::raw::c_char;

use std::net::{TcpListener, TcpStream, Shutdown};
use std::io::{BufReader, BufRead};

use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use async_recursion::async_recursion;

use regex::Regex;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use tiny_http::{Server, Response, Header, Method};
#[cfg(not(any(target_arch = "wasm32")))]
use libloading::{Library, Symbol};
use lazy_static::lazy_static;

use std::cell::RefCell;
use tokio::task_local;

#[cfg(target_arch = "wasm32")]
use web_sys;

#[cfg(not(any(target_arch = "wasm32")))]
use libffi::middle::{Cif, Type, CodePtr, Arg};
#[cfg(not(any(target_arch = "wasm32")))]
use std::ffi::c_void;

lazy_static! {
    // static ref GC_STATE: Mutex<GcState> = Mutex::new(GcState::new());
    static ref FFI_SIGNATURES: Mutex<HashMap<String, (String, Vec<String>)>> = Mutex::new(HashMap::new());
static ref FFI_STRUCTS: Mutex<HashMap<String, FfiStructDef>> = Mutex::new(HashMap::new());

static ref BUILTIN_MODULE_CACHE: Mutex<HashMap<String, DgnValue>> =
        Mutex::new(HashMap::new());

    static ref TOKEN_REGEXES: Vec<(&'static str, Regex)> = vec![
        ("INC", Regex::new(r"^\+\+").unwrap()), 
        ("DEC", Regex::new(r"^--").unwrap()), 
        ("COMMENT_ML", Regex::new(r"(?s)^/\*.*?\*/").unwrap()), 
        ("COMMENT_SL", Regex::new(r"^//[^\n]*").unwrap()), 
        ("HEX", Regex::new(r"^0x[0-9a-fA-F]+").unwrap()),
        ("NUMBER", Regex::new(r"^[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?").unwrap()),
        ("STRING", Regex::new(r#"^"([^"\\]|\\.)*"|^'([^'\\]|\\.)+'"#).unwrap()), 
        ("NAME", Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*").unwrap()),
        ("OP", Regex::new(r"^(==|!=|<=|>=|=>|<<|>>|\?\?|[!+\-*/%<>=?:;,&|^~(){}\[\].])").unwrap()),
        ("NEWLINE", Regex::new(r"^\n").unwrap()),
        ("SKIP", Regex::new(r"^[ \t\r\u{FEFF}\x00]+").unwrap()), 
        ("MISMATCH", Regex::new(r"^.").unwrap()),
    ];
}

task_local! {
    pub static GC_STATE: RefCell<GcState>;
}

struct FfiStructDef {
    size: usize,
    fields: Vec<(String, usize, Type)>, // Name, Offset, LibFFI Type
    libffi_type: Type, // The constructed Type::structure
}
// We need Send/Sync to store Type in a global Mutex, forcing unsafe impl if necessary
// (libffi Types are generally thread-safe as they are read-only descriptors)
#[cfg(not(any(target_arch = "wasm32")))]
unsafe impl Send for FfiStructDef {}
#[cfg(not(any(target_arch = "wasm32")))]
unsafe impl Sync for FfiStructDef {}

pub const BUNDLE_SIGNATURE: &[u8; 16] = b"DRAGON_BUNDLE_v1";


#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    Inc, Dec, CommentMl, CommentSl, Number, String, Name, Op, Newline, Eof,
    Keyword(String)
}

#[derive(Debug, Clone)]
pub struct Token {
    pub typ: TokenType,
    pub value: String,
    pub line: usize,
    pub col: usize,
    pub num_val: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub node: ExprNode,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub node: StmtNode,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub enum ExprNode {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    This,
    Var(String),
    List(Vec<Expr>),
    Obj(Vec<(String, Expr)>),
    Unary(String, Box<Expr>),
    Binary(String, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    GetAttr(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    PreOp(String, Box<Expr>),
    PostOp(String, Box<Expr>),
    FuncExpr(Vec<String>, usize, Box<Stmt>),
    New(Box<Expr>, Vec<Expr>),
    Super(String, Vec<Expr>),
    Await(Box<Expr>), // ADD THIS
}

#[derive(Debug, Clone)]
pub enum StmtNode {
    Block(Vec<Stmt>),
    Expr(Expr),
    Return(Option<Expr>),
    Break,
    Continue,
    Raise(Expr),
    IfChain(Vec<(Option<Expr>, Stmt)>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(String, Expr, Box<Stmt>),
    CFor(Option<Expr>, Option<Expr>, Option<Expr>, Box<Stmt>),
    Switch(Expr, Vec<(Expr, Stmt)>, Option<Box<Stmt>>),
    Try(Box<Stmt>, Option<String>, Option<Box<Stmt>>, Option<Box<Stmt>>),
    FuncDef(String, Vec<String>, usize, Box<Stmt>, bool),
    ClassDef(String, Option<String>, Box<Stmt>),
    Import(String, Option<String>),
    ConstDecl(String, Expr),
}

#[derive(Clone, Debug)]
pub enum PromiseState {
    Pending(Arc<AsyncMutex<Option<JoinHandle<Result<DgnValue, DgnError>>>>>),
    Resolved(Result<DgnValue, DgnError>),
}

#[derive(Clone, Debug)]
pub enum DgnValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    List(Arc<Mutex<Vec<DgnValue>>>),
    Map(Arc<Mutex<HashMap<String, DgnValue>>>),
    ByteArray(Arc<Mutex<Vec<u8>>>),
    Func(DgnFunc),
    Class(Arc<Mutex<Env>>),
    Instance(Arc<Mutex<HashMap<String, DgnValue>>>),
    #[cfg(not(any(target_arch = "wasm32")))]
    NativeLib(Arc<Mutex<Library>>),
    Regex(Arc<Regex>),
    Thread(Arc<thread::JoinHandle<()>>), 
   #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    Socket(Arc<Mutex<TcpStream>>),
    Promise(Arc<AsyncMutex<PromiseState>>),
}


impl DgnValue {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            DgnValue::Number(n) => Some(*n),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct DgnFunc {
    pub name: Option<String>,
    pub params: Vec<String>,
    pub req_params: usize,
    pub body: Option<Stmt>,
    pub closure: Arc<Mutex<Env>>,
    pub is_native: bool,
    pub is_async: bool,
    pub native_fn: Option<Arc<dyn Fn(Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync>>,
    /// Compiled bytecode for this function. When Some, the VM executes this
    /// instead of tree-walking `body`.
    pub compiled_chunk: Option<Arc<Chunk>>,
}

struct GcState {
    objects: Vec<WeakContainer>,
    allocations: usize,
    threshold: usize,
}

enum WeakContainer {
    List(std::sync::Weak<Mutex<Vec<DgnValue>>>),
    Map(std::sync::Weak<Mutex<HashMap<String, DgnValue>>>),
}

impl GcState {
    fn new() -> Self {
        GcState { objects: Vec::new(), allocations: 0, threshold: 1000 }
    }
}

pub fn gc_register(val: &DgnValue) {
    // try_with safely checks if we are running inside an agent's Tokio task
    let _ = GC_STATE.try_with(|state_cell| {
        let mut state = state_cell.borrow_mut(); // No Mutex lock! Fast!
        match val {
            DgnValue::List(l) => {
                state.objects.push(WeakContainer::List(Arc::downgrade(l)));
                state.allocations += 1;
            },
            DgnValue::Map(m) | DgnValue::Instance(m) => {
                state.objects.push(WeakContainer::Map(Arc::downgrade(m)));
                state.allocations += 1;
            },
            _ => {}
        }
    });
}

pub fn gc_check_auto(env: Arc<Mutex<Env>>) {
    let should_collect = GC_STATE.try_with(|state_cell| {
        let state = state_cell.borrow();
        state.allocations >= state.threshold
    }).unwrap_or(false);
    
    if should_collect {
        gc_collect(env);
    }
}

pub fn gc_collect(root_env: Arc<Mutex<Env>>) -> usize {
    let mut visited_objs = HashSet::new(); 
    let mut visited_envs = HashSet::new(); 
    
    mark_env(root_env, &mut visited_objs, &mut visited_envs);
    
    GC_STATE.try_with(|state_cell| {
        let mut state = state_cell.borrow_mut();
        let mut reclaimed = 0;
        
        state.objects.retain(|weak| {
            match weak {
                WeakContainer::List(w) => {
                    if let Some(arc) = w.upgrade() {
                        let ptr = Arc::as_ptr(&arc) as usize;
                        if !visited_objs.contains(&ptr) {
                            arc.lock().unwrap().clear();
                            reclaimed += 1;
                            return false; 
                        }
                        true 
                    } else { false }
                },
                WeakContainer::Map(w) => {
                    if let Some(arc) = w.upgrade() {
                        let ptr = Arc::as_ptr(&arc) as usize;
                        if !visited_objs.contains(&ptr) {
                            arc.lock().unwrap().clear();
                            reclaimed += 1;
                            return false; 
                        }
                        true
                    } else { false }
                }
            }
        });
        
        state.allocations = 0;
        reclaimed
    }).unwrap_or(0)
}

fn mark_env(env: Arc<Mutex<Env>>, visited_objs: &mut HashSet<usize>, visited_envs: &mut HashSet<usize>) {
    let mut curr = Some(env);
    
    while let Some(e_arc) = curr {
        let ptr = Arc::as_ptr(&e_arc) as usize;
        
        if !visited_envs.insert(ptr) {
            break;
        }

        let e = e_arc.lock().unwrap();
        for val in e.store.values() {
            mark_val(val, visited_objs, visited_envs);
        }
        
        curr = e.parent.clone();
    }
}

fn mark_val(val: &DgnValue, visited_objs: &mut HashSet<usize>, visited_envs: &mut HashSet<usize>) {
    match val {
        DgnValue::List(l) => {
            let ptr = Arc::as_ptr(l) as usize;
            if visited_objs.insert(ptr) {
                let vec = l.lock().unwrap();
                for v in vec.iter() { mark_val(v, visited_objs, visited_envs); }
            }
        },
        DgnValue::Map(m) | DgnValue::Instance(m) => {
            let ptr = Arc::as_ptr(m) as usize;
            if visited_objs.insert(ptr) {
                let map = m.lock().unwrap();
                for v in map.values() { mark_val(v, visited_objs, visited_envs); }
            }
        },
        DgnValue::Func(f) => {
            mark_env(f.closure.clone(), visited_objs, visited_envs);
        },
        DgnValue::Class(env) => {
            mark_env(env.clone(), visited_objs, visited_envs);
        },
        _ => {}
    }
}

impl fmt::Debug for DgnFunc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DgnFunc")
         .field("name", &self.name)
         .field("params", &self.params)
         .field("is_native", &self.is_native)
         .finish()
    }
}

#[derive(Debug, Clone)]
pub struct DgnError {
    pub msg: String,
    pub line: usize,
}

impl DgnError {
    pub fn new(msg: &str, line: usize) -> Self {
        DgnError { msg: msg.to_string(), line }
    }
}

#[derive(Clone)]
pub enum EvalResult {
    Val(DgnValue),
    Return(DgnValue),
    Break,
    Continue,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or, NullCoalesce,
    BitAnd, BitOr, BitXor, Shl, Shr,
}

// ─── BYTECODE COMPILER ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Instruction {
    // Stack manipulation
    LoadConst(DgnValue),       // Push a constant
    LoadVar(String),           // Push variable value
    StoreVar(String),          // Pop and assign to variable
    DefineConst(String),       // Pop and define constant
    Pop,                       // Discard top of stack

    // Arithmetic / binary ops
    BinaryOp(BinOp),         // e.g. "+", "-", "==", "and", "or"
    UnaryOp(String),           // e.g. "-", "not"

    // Collections
    MakeList(usize),           // Pop N items, push List
    MakeMap(usize),            // Pop N key-value pairs, push Map

    // Control flow (index = absolute instruction index)
    Jump(usize),               // Unconditional jump
    JumpIfFalse(usize),        // Pop, jump if falsy
    JumpIfTrue(usize),         // Pop, jump if truthy (for `or` short-circuit)

    // Functions
    // MakeFunc removed — replaced by MakeClosure
    Call(usize),               // Pop func + N args, call
    Return,                    // Return top of stack

    // Object access
    GetAttr(String),           // obj.name
    SetAttr(String),           // obj.name = val
    GetIndex,                  // obj[idx]
    SetIndex,                  // obj[idx] = val

    // Loops
    ForIter(String, usize),    // var_name, exit_ip
    Break,                     // break out of loop
    Continue,                  // continue to loop start
    DoWhileStart,              // marks do-while body start ip (patched after)

    // Functions / closures
    MakeClosure(Option<String>, Vec<String>, usize, bool, Box<Vec<Instruction>>, Box<Vec<usize>>),// params, is_async, instrs, lines

    // Classes / instances
    MakeClass(String, Option<String>, Box<Vec<Instruction>>, Box<Vec<usize>>),
    NewObject(usize),          // pops class + N args, creates instance
    SuperCall(String, usize),  // super.method(N args)

    // Exceptions
    PushTry(usize),            // push try frame, arg = catch ip
    PopTry,                    // pop try frame on success
    Raise,                     // pop & throw as error
    EnterCatch(String),        // bind caught error string to named var

    // Async / await
    Await,                     // pop Promise, await it

    // Import
    Import(String, Option<String>), // path, alias

    // Inc/Dec — simple variable forms
    IncVar(String),             // ++var  (pre,  pushes new value)
    DecVar(String),             // --var  (pre,  pushes new value)
    PostIncVar(String),         // var++  (post, pushes old value)
    PostDecVar(String),         // var--  (post, pushes old value)
    // Inc/Dec — attr/index forms: compiler emits obj (and idx) expressions
    // onto the stack BEFORE these instructions, so they just pop.
    // Stack before RMWAttr:  [..., obj]
    // Stack before RMWIndex: [..., obj, idx]
    ReadModWriteAttr(String, bool, bool), // attr, is_pre, is_dec  — pops obj, pushes result
    ReadModWriteIndex(bool, bool),        // is_pre, is_dec        — pops obj+idx, pushes result

    // Misc
    Dup,                       // Duplicate top of stack
    This,                      // Push `this`
    Nop,
    Halt,
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub instructions: Vec<Instruction>,
    pub source_lines: Vec<usize>,
}

impl Chunk {
    pub fn new() -> Self {
        Chunk { instructions: vec![], source_lines: vec![] }
    }

    pub fn emit(&mut self, instr: Instruction, line: usize) -> usize {
        let idx = self.instructions.len();
        self.instructions.push(instr);
        self.source_lines.push(line);
        idx
    }

    // Patch a Jump/JumpIfFalse target after you know the destination
    pub fn patch_jump(&mut self, idx: usize, target: usize) {
        match &mut self.instructions[idx] {
            Instruction::Jump(t) | Instruction::JumpIfFalse(t) | Instruction::JumpIfTrue(t) => {
                *t = target;
            }
            _ => {}
        }
    }
}

pub struct Compiler;

impl Compiler {
    /// Helper: compile a body into its own Chunk and return (instructions, lines)
    fn compile_body(stmt: &Stmt) -> (Vec<Instruction>, Vec<usize>) {
        let mut c = Chunk::new();
        Self::compile_stmt(&mut c, stmt);
        c.emit(Instruction::LoadConst(DgnValue::Null), 0);
        c.emit(Instruction::Return, 0);
        (c.instructions, c.source_lines)
    }

    /// Like compile_stmt but for REPL use: expression statements leave their
    /// value on the stack (no trailing Pop) so the REPL can print it.
    pub fn compile_repl_stmt(chunk: &mut Chunk, stmt: &Stmt) {
        let line = stmt.line;
        match &stmt.node {
            StmtNode::Expr(e) => {
                // No Pop — leave value on stack for the REPL to print
                Self::compile_expr(chunk, e);
            }
            // Everything else compiles normally
            _ => Self::compile_stmt(chunk, stmt),
        }
    }

    pub fn compile_stmt(chunk: &mut Chunk, stmt: &Stmt) {
        let line = stmt.line;
        match &stmt.node {

            StmtNode::Block(stmts) => {
                // PASS 1: Hoist all function definitions to the top
                for s in stmts {
                    if let StmtNode::FuncDef(..) | StmtNode::ClassDef(..) = &s.node {
                        Self::compile_stmt(chunk, s);
                    }
                }
                
                // PASS 2: Compile everything else
                for s in stmts {
                    if !matches!(s.node, StmtNode::FuncDef(..) | StmtNode::ClassDef(..)) {
                        Self::compile_stmt(chunk, s);
                    }
                }
            }

            StmtNode::Expr(e) => {
                Self::compile_expr(chunk, e);
                chunk.emit(Instruction::Pop, line);
            }

            StmtNode::Return(opt_e) => {
                match opt_e {
                    Some(e) => Self::compile_expr(chunk, e),
                    None    => { chunk.emit(Instruction::LoadConst(DgnValue::Null), line); }
                };
                chunk.emit(Instruction::Return, line);
            }

            StmtNode::Break    => { chunk.emit(Instruction::Break, line); }
            StmtNode::Continue => { chunk.emit(Instruction::Continue, line); }

            StmtNode::Raise(e) => {
                Self::compile_expr(chunk, e);
                chunk.emit(Instruction::Raise, line);
            }

            StmtNode::ConstDecl(name, e) => {
                if let ExprNode::FuncExpr(params, req_params, body) = &e.node {
                    let (instrs, lines) = Self::compile_body(body);
                    chunk.emit(Instruction::MakeClosure(
                        Some(name.clone()), params.clone(), *req_params, false,
                        Box::new(instrs), Box::new(lines),
                    ), e.line);
                } else {
                    Self::compile_expr(chunk, e);
                }
                chunk.emit(Instruction::DefineConst(name.clone()), line);
            }

            // ── func definition ──────────────────────────────────────────────
            StmtNode::FuncDef(name, params, req_params, body, is_async) => {
                let (instrs, lines) = Self::compile_body(body);
                let opt_name = if name.is_empty() { None } else { Some(name.clone()) };
                let instr = Instruction::MakeClosure(
                    opt_name, params.clone(), *req_params, *is_async,
                    Box::new(instrs), Box::new(lines),
                );
                chunk.emit(instr, line);
                if !name.is_empty() {
                    chunk.emit(Instruction::StoreVar(name.clone()), line);
                }
            }

            // ── if / elif / else ──────────────────────────────────────────────
            StmtNode::IfChain(chain) => {
                let mut end_jumps = vec![];
                for (cond_opt, body) in chain {
                    if let Some(cond) = cond_opt {
                        Self::compile_expr(chunk, cond);
                        let false_jump = chunk.emit(Instruction::JumpIfFalse(0), line);
                        Self::compile_stmt(chunk, body);
                        let end_jump = chunk.emit(Instruction::Jump(0), line);
                        end_jumps.push(end_jump);
                        chunk.patch_jump(false_jump, chunk.instructions.len());
                    } else {
                        Self::compile_stmt(chunk, body);
                        let end_jump = chunk.emit(Instruction::Jump(0), line);
                        end_jumps.push(end_jump);
                    }
                }
                let end = chunk.instructions.len();
                for j in end_jumps { chunk.patch_jump(j, end); }
            }

            // ── while ─────────────────────────────────────────────────────────
            StmtNode::While(cond, body) => {
                let loop_start = chunk.instructions.len();
                Self::compile_expr(chunk, cond);
                let exit_jump = chunk.emit(Instruction::JumpIfFalse(0), line);
                // compile body; break/continue are patched below
                let body_start = chunk.instructions.len();
                Self::compile_stmt(chunk, body);
                chunk.emit(Instruction::Jump(loop_start), line);
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(exit_jump, loop_end);
                // patch break/continue inside this loop body
                Self::patch_loop_jumps(chunk, body_start, loop_end, loop_start);
            }

            // ── do-while ──────────────────────────────────────────────────────
            StmtNode::DoWhile(body, cond) => {
                let body_start = chunk.instructions.len();
                Self::compile_stmt(chunk, body);
                let cond_start = chunk.instructions.len();
                Self::compile_expr(chunk, cond);
                chunk.emit(Instruction::JumpIfFalse(0), line); // patched below
                let back_jump = chunk.instructions.len() - 1;
                chunk.emit(Instruction::Jump(body_start), line);
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(back_jump, loop_end);
                Self::patch_loop_jumps(chunk, body_start, loop_end, cond_start);
            }

            // ── for-in ────────────────────────────────────────────────────────
            StmtNode::For(var_name, iter_expr, body) => {
                Self::compile_expr(chunk, iter_expr);
                // ForIter pops the iterable, iterates: on each step pushes item
                // and sets var, jumps to exit when exhausted
                let for_iter_ip = chunk.emit(Instruction::ForIter(var_name.clone(), 0), line);
                let body_start = chunk.instructions.len();
                Self::compile_stmt(chunk, body);
                chunk.emit(Instruction::Jump(for_iter_ip), line); // next iteration
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(for_iter_ip, loop_end);
                Self::patch_loop_jumps(chunk, body_start, loop_end, for_iter_ip);
            }

            // ── C-style for ───────────────────────────────────────────────────
            StmtNode::CFor(init, cond, step, body) => {
                if let Some(i) = init {
                    Self::compile_expr(chunk, i);
                    chunk.emit(Instruction::Pop, line);
                }
                let loop_start = chunk.instructions.len();
                let exit_jump = if let Some(c) = cond {
                    Self::compile_expr(chunk, c);
                    Some(chunk.emit(Instruction::JumpIfFalse(0), line))
                } else { None };
                let body_start = chunk.instructions.len();
                Self::compile_stmt(chunk, body);
                let step_ip = chunk.instructions.len();
                if let Some(s) = step {
                    Self::compile_expr(chunk, s);
                    chunk.emit(Instruction::Pop, line);
                }
                chunk.emit(Instruction::Jump(loop_start), line);
                let loop_end = chunk.instructions.len();
                if let Some(j) = exit_jump { chunk.patch_jump(j, loop_end); }
                Self::patch_loop_jumps(chunk, body_start, loop_end, step_ip);
            }

            // ── switch ────────────────────────────────────────────────────────
            StmtNode::Switch(val_expr, cases, default) => {
                Self::compile_expr(chunk, val_expr);
                // Stack: [switch_val]  (stays on stack throughout)
                let mut end_jumps = vec![];
                for (case_expr, block) in cases {
                    chunk.emit(Instruction::Dup, line);
                    Self::compile_expr(chunk, case_expr);
                    chunk.emit(Instruction::BinaryOp(BinOp::Eq), line);
                    let skip = chunk.emit(Instruction::JumpIfFalse(0), line);
                    chunk.emit(Instruction::Pop, line); // pop switch_val on match
                    Self::compile_stmt(chunk, block);
                    end_jumps.push(chunk.emit(Instruction::Jump(0), line));
                    chunk.patch_jump(skip, chunk.instructions.len());
                }
                if let Some(def) = default {
                    chunk.emit(Instruction::Pop, line); // pop switch_val
                    Self::compile_stmt(chunk, def);
                } else {
                    chunk.emit(Instruction::Pop, line); // pop switch_val
                }
                let end = chunk.instructions.len();
                for j in end_jumps { chunk.patch_jump(j, end); }
            }

            // ── try / catch / finally ─────────────────────────────────────────
            StmtNode::Try(try_b, catch_var, catch_b, fin_b) => {
                let push_try_ip = chunk.emit(Instruction::PushTry(0), line);
                Self::compile_stmt(chunk, try_b);
                chunk.emit(Instruction::PopTry, line);
                let skip_catch = chunk.emit(Instruction::Jump(0), line);

                // catch block
                let catch_ip = chunk.instructions.len();
                chunk.patch_jump(push_try_ip, catch_ip);
                if let (Some(var), Some(cb)) = (catch_var, catch_b) {
                    chunk.emit(Instruction::EnterCatch(var.clone()), line);
                    Self::compile_stmt(chunk, cb);
                } else {
                    chunk.emit(Instruction::Pop, line); // discard error
                }
                chunk.patch_jump(skip_catch, chunk.instructions.len());

                // finally block runs regardless
                if let Some(fb) = fin_b {
                    Self::compile_stmt(chunk, fb);
                }
            }

            // ── class ─────────────────────────────────────────────────────────
            StmtNode::ClassDef(name, parent, body) => {
                let (instrs, lines) = Self::compile_body(body);
                chunk.emit(Instruction::MakeClass(
                    name.clone(), parent.clone(),
                    Box::new(instrs), Box::new(lines),
                ), line);
                chunk.emit(Instruction::StoreVar(name.clone()), line);
            }

            // ── import ────────────────────────────────────────────────────────
            StmtNode::Import(path, alias) => {
                chunk.emit(Instruction::Import(path.clone(), alias.clone()), line);
            }
        }
    }

    // ── patch break/continue targets after compiling a loop body ─────────────
    // Scans [start..end) and replaces Break/Continue placeholders.
    // break  → Jump(loop_end)
    // continue → Jump(continue_target)
    fn patch_loop_jumps(chunk: &mut Chunk, start: usize, loop_end: usize, continue_target: usize) {
        for i in start..loop_end {
            match &chunk.instructions[i] {
                Instruction::Break    => { chunk.instructions[i] = Instruction::Jump(loop_end); }
                Instruction::Continue => { chunk.instructions[i] = Instruction::Jump(continue_target); }
                _ => {}
            }
        }
    }

    pub fn compile_expr(chunk: &mut Chunk, expr: &Expr) {
        let line = expr.line;
        match &expr.node {

            ExprNode::Number(n) => { chunk.emit(Instruction::LoadConst(DgnValue::Number(*n)), line); }
            ExprNode::String(s) => { chunk.emit(Instruction::LoadConst(DgnValue::String(s.clone())), line); }
            ExprNode::Bool(b)   => { chunk.emit(Instruction::LoadConst(DgnValue::Bool(*b)), line); }
            ExprNode::Null      => { chunk.emit(Instruction::LoadConst(DgnValue::Null), line); }
            ExprNode::This      => { chunk.emit(Instruction::This, line); }
            ExprNode::Var(n)    => { chunk.emit(Instruction::LoadVar(n.clone()), line); }

            // ── assignment ───────────────────────────────────────────────────
            ExprNode::Assign(target, val) => {
                let mut inferred_name = None;
                if let ExprNode::Var(n) = &target.node {
                    inferred_name = Some(n.clone());
                } else if let ExprNode::GetAttr(_, n) = &target.node {
                    inferred_name = Some(n.clone()); // Handles obj.myFunc = () => {}
                }

                // If the RHS is a lambda, compile it directly with the inferred name
                if let ExprNode::FuncExpr(params, req_params, body) = &val.node {
                    let (instrs, lines) = Self::compile_body(body);
                    chunk.emit(Instruction::MakeClosure(
                        inferred_name, params.clone(), *req_params, false,
                        Box::new(instrs), Box::new(lines),
                    ), val.line);
                } else {
                    Self::compile_expr(chunk, val); // Normal compilation
                }
                match &target.node {
                    ExprNode::Var(n) => {
                        chunk.emit(Instruction::Dup, line);
                        chunk.emit(Instruction::StoreVar(n.clone()), line);
                    }
                    ExprNode::GetAttr(obj, n) => {
                        // stack before SetAttr: [val, obj]  — we need obj on top
                        // Dup val, then push obj, then SetAttr
                        chunk.emit(Instruction::Dup, line);       // [val, val]
                        Self::compile_expr(chunk, obj);            // [val, val, obj]
                        chunk.emit(Instruction::SetAttr(n.clone()), line); // pops obj+val, leaves val
                    }
                    ExprNode::Index(obj, idx) => {
                        chunk.emit(Instruction::Dup, line);
                        Self::compile_expr(chunk, obj);
                        Self::compile_expr(chunk, idx);
                        chunk.emit(Instruction::SetIndex, line);
                    }
                    _ => {}
                }
            }

            // ── unary ────────────────────────────────────────────────────────
            ExprNode::Unary(op, e) => {
                Self::compile_expr(chunk, e);
                chunk.emit(Instruction::UnaryOp(op.clone()), line);
            }

            // ── binary ───────────────────────────────────────────────────────
            ExprNode::Binary(op, l, r) => {
                if op == "and" {
                    Self::compile_expr(chunk, l);
                    chunk.emit(Instruction::Dup, line);
                    let fj = chunk.emit(Instruction::JumpIfFalse(0), line);
                    chunk.emit(Instruction::Pop, line);
                    Self::compile_expr(chunk, r);
                    chunk.patch_jump(fj, chunk.instructions.len());
                } else if op == "or" {
                    Self::compile_expr(chunk, l);
                    chunk.emit(Instruction::Dup, line);
                    let tj = chunk.emit(Instruction::JumpIfTrue(0), line);
                    chunk.emit(Instruction::Pop, line);
                    Self::compile_expr(chunk, r);
                    chunk.patch_jump(tj, chunk.instructions.len());
                } else if op == "??" {
                    Self::compile_expr(chunk, l);
                    chunk.emit(Instruction::Dup, line);
                    // if not null, keep; else pop and eval rhs
                    let not_null = chunk.emit(Instruction::JumpIfTrue(0), line);
                    chunk.emit(Instruction::Pop, line);
                    Self::compile_expr(chunk, r);
                    chunk.patch_jump(not_null, chunk.instructions.len());
                } else {
                    let binop = match op.as_str() {
                        "+"  => BinOp::Add,  "-"  => BinOp::Sub,
                        "*"  => BinOp::Mul,  "/"  => BinOp::Div,  "%" => BinOp::Mod,
                        "==" => BinOp::Eq,   "!=" => BinOp::Ne,
                        "<"  => BinOp::Lt,   ">"  => BinOp::Gt,
                        "<=" => BinOp::Le,   ">=" => BinOp::Ge,
                        "&"  => BinOp::BitAnd, "|" => BinOp::BitOr, "^" => BinOp::BitXor,
                        "<<" => BinOp::Shl,  ">>" => BinOp::Shr,
                        "??" => BinOp::NullCoalesce,
                        _    => BinOp::Add,
                    };
                    Self::compile_expr(chunk, l);
                    Self::compile_expr(chunk, r);
                    chunk.emit(Instruction::BinaryOp(binop), line);
                }
            }

            // ── ternary ──────────────────────────────────────────────────────
            ExprNode::Ternary(cond, t, f) => {
                Self::compile_expr(chunk, cond);
                let fj = chunk.emit(Instruction::JumpIfFalse(0), line);
                Self::compile_expr(chunk, t);
                let ej = chunk.emit(Instruction::Jump(0), line);
                chunk.patch_jump(fj, chunk.instructions.len());
                Self::compile_expr(chunk, f);
                chunk.patch_jump(ej, chunk.instructions.len());
            }

            // ── call ─────────────────────────────────────────────────────────
            ExprNode::Call(func_expr, arg_exprs) => {
                Self::compile_expr(chunk, func_expr);
                for a in arg_exprs { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::Call(arg_exprs.len()), line);
            }

            // ── attribute / index ─────────────────────────────────────────────
            ExprNode::GetAttr(obj, name) => {
                Self::compile_expr(chunk, obj);
                chunk.emit(Instruction::GetAttr(name.clone()), line);
            }
            ExprNode::Index(obj, idx) => {
                Self::compile_expr(chunk, obj);
                Self::compile_expr(chunk, idx);
                chunk.emit(Instruction::GetIndex, line);
            }

            // ── collections ──────────────────────────────────────────────────
            ExprNode::List(items) => {
                for item in items { Self::compile_expr(chunk, item); }
                chunk.emit(Instruction::MakeList(items.len()), line);
            }
            ExprNode::Obj(pairs) => {
                for (k, v) in pairs {
                    chunk.emit(Instruction::LoadConst(DgnValue::String(k.clone())), line);
                    if let ExprNode::FuncExpr(params, req_params, body) = &v.node {
                        let (instrs, lines) = Self::compile_body(body);
                        chunk.emit(Instruction::MakeClosure(
                            Some(k.clone()), params.clone(), *req_params, false,
                            Box::new(instrs), Box::new(lines),
                        ), v.line);
                    } else {
                        Self::compile_expr(chunk, v);
                    }
                }
                chunk.emit(Instruction::MakeMap(pairs.len()), line);
            }

            // ── lambda / func expression ──────────────────────────────────────
            ExprNode::FuncExpr(params, req_params, body) => {
                let (instrs, lines) = Self::compile_body(body);
                chunk.emit(Instruction::MakeClosure(
                    None, params.clone(), *req_params, false,
                    Box::new(instrs), Box::new(lines),
                ), line);
            }

            // ── new ───────────────────────────────────────────────────────────
            ExprNode::New(cls_expr, args) => {
                Self::compile_expr(chunk, cls_expr);
                for a in args { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::NewObject(args.len()), line);
            }

            // ── super.method(...) ─────────────────────────────────────────────
            ExprNode::Super(method, args) => {
                for a in args { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::SuperCall(method.clone(), args.len()), line);
            }

            // ── await ─────────────────────────────────────────────────────────
            ExprNode::Await(inner) => {
                Self::compile_expr(chunk, inner);
                chunk.emit(Instruction::Await, line);
            }

            // ── pre/post inc-dec ──────────────────────────────────────────────
            ExprNode::PreOp(op, target) => {
                let is_dec = op == "DEC";
                match &target.node {
                    ExprNode::Var(n) => {
                        chunk.emit(if is_dec { Instruction::DecVar(n.clone()) } else { Instruction::IncVar(n.clone()) }, line);
                    }
                    ExprNode::GetAttr(obj, attr) => {
                        // Emit obj expression onto stack, then RMWAttr pops it
                        Self::compile_expr(chunk, obj);
                        chunk.emit(Instruction::ReadModWriteAttr(attr.clone(), true, is_dec), line);
                    }
                    ExprNode::Index(obj, idx) => {
                        // Emit obj then idx onto stack, then RMWIndex pops both
                        Self::compile_expr(chunk, obj);
                        Self::compile_expr(chunk, idx);
                        chunk.emit(Instruction::ReadModWriteIndex(true, is_dec), line);
                    }
                    _ => {}
                }
            }
            ExprNode::PostOp(op, target) => {
                let is_dec = op == "DEC";
                match &target.node {
                    ExprNode::Var(n) => {
                        chunk.emit(if is_dec { Instruction::PostDecVar(n.clone()) } else { Instruction::PostIncVar(n.clone()) }, line);
                    }
                    ExprNode::GetAttr(obj, attr) => {
                        Self::compile_expr(chunk, obj);
                        chunk.emit(Instruction::ReadModWriteAttr(attr.clone(), false, is_dec), line);
                    }
                    ExprNode::Index(obj, idx) => {
                        Self::compile_expr(chunk, obj);
                        Self::compile_expr(chunk, idx);
                        chunk.emit(Instruction::ReadModWriteIndex(false, is_dec), line);
                    }
                    _ => {}
                }
            }

            _ => { chunk.emit(Instruction::Nop, line); }
        }
    }
}

pub struct Vm {
    pub stack: Vec<DgnValue>,
    try_frames: Vec<(usize, usize)>,
    for_iters: HashMap<usize, (Vec<DgnValue>, usize)>,
}

impl Vm {
    pub fn new() -> Self {
        Vm { stack: vec![], try_frames: vec![], for_iters: HashMap::new() }
    }

    fn pop(&mut self) -> DgnValue { self.stack.pop().unwrap_or(DgnValue::Null) }

    #[async_recursion]
    pub async fn run(
        &mut self,
        chunk: &Chunk,
        env: Arc<Mutex<Env>>,
    ) -> Result<DgnValue, DgnError> {
        let mut ip = 0;

        while ip < chunk.instructions.len() {
            let line = chunk.source_lines[ip];
            let instr = chunk.instructions[ip].clone();

            match instr {
                Instruction::Halt => break,
                Instruction::Nop  => {}

                // ── stack basics ──────────────────────────────────────────────
                Instruction::LoadConst(v) => self.stack.push(v),
                Instruction::LoadVar(ref n) => {
                    let v = env.lock().unwrap().get(n)
                        .ok_or_else(|| DgnError::new(&format!("Var '{}' not defined", n), line))?;
                    self.stack.push(v);
                }
                Instruction::StoreVar(ref n) => {
                    let v = self.pop();
                    env.lock().unwrap().set(n.clone(), v)
                        .map_err(|e| DgnError::new(&e, line))?;
                }
                Instruction::DefineConst(ref n) => {
                    let v = self.pop();
                    env.lock().unwrap().define_const(n.clone(), v)
                        .map_err(|e| DgnError::new(&e, line))?;
                }
                Instruction::Pop => { self.pop(); }
                Instruction::Dup => {
                    let v = self.stack.last().cloned().unwrap_or(DgnValue::Null);
                    self.stack.push(v);
                }
                Instruction::This => {
                    let v = env.lock().unwrap().get("this")
                        .ok_or(DgnError::new("'this' not defined", line))?;
                    self.stack.push(v);
                }

                // ── jumps ─────────────────────────────────────────────────────
                Instruction::Jump(t)        => { ip = t; continue; }
                Instruction::JumpIfFalse(t) => { let v = self.pop(); if !is_truthy(&v) { ip = t; continue; } }
                Instruction::JumpIfTrue(t)  => { let v = self.pop(); if  is_truthy(&v) { ip = t; continue; } }

                // ── operators ─────────────────────────────────────────────────
                Instruction::UnaryOp(ref op) => {
                    let v = self.pop();
                    let r = match (op.as_str(), v) {
                        ("-",   DgnValue::Number(n)) => DgnValue::Number(-n),
                        ("+",   DgnValue::Number(n)) => DgnValue::Number(n),
                        ("not", x)                   => DgnValue::Bool(!is_truthy(&x)),
                        ("~",   DgnValue::Number(n)) => DgnValue::Number(!(n as i64) as f64),
                        _ => return Err(DgnError::new("Invalid unary op", line)),
                    };
                    self.stack.push(r);
                }
                Instruction::BinaryOp(ref op) => {
                    let r = self.pop();
                    let l = self.pop();
                    self.stack.push(vm_binary_op(op, l, r, line)?);
                }

                // ── collections ───────────────────────────────────────────────
                Instruction::MakeList(n) => {
                    let start = self.stack.len().saturating_sub(n);
                    let items: Vec<DgnValue> = self.stack.drain(start..).collect();
                    let val = DgnValue::List(Arc::new(Mutex::new(items)));
                    gc_register(&val);
                    self.stack.push(val);
                }
                Instruction::MakeMap(n) => {
                    let start = self.stack.len().saturating_sub(n * 2);
                    let pairs: Vec<DgnValue> = self.stack.drain(start..).collect();
                    let mut map = HashMap::new();
                    let mut i = 0;
                    while i + 1 < pairs.len() {
                        if let DgnValue::String(k) = &pairs[i] {
                            map.insert(k.clone(), pairs[i + 1].clone());
                        }
                        i += 2;
                    }
                    let val = DgnValue::Map(Arc::new(Mutex::new(map)));
                    gc_register(&val);
                    self.stack.push(val);
                }

                // ── attribute / index access ──────────────────────────────────
                Instruction::GetAttr(ref name) => {
                    let obj = self.pop();
                    self.stack.push(get_attr(obj, name).map_err(|mut e| { e.line = line; e })?);
                }
                Instruction::SetAttr(ref name) => {
                    // compiler emits: [Dup val] -> [compile obj] -> SetAttr
                    // stack top = obj, below = val (Dup'd), val stays on stack as result
                    let obj = self.pop();
                    let val = self.stack.last().cloned().unwrap_or(DgnValue::Null);
                    if let DgnValue::Map(m) | DgnValue::Instance(m) = obj {
                        m.lock().unwrap().insert(name.clone(), val);
                    }
                }
                Instruction::GetIndex => {
                    let idx = self.pop();
                    let obj = self.pop();
                    let r = match (obj, idx) {
                        (DgnValue::List(l),   DgnValue::Number(n)) =>
                            l.lock().unwrap().get(n as usize).cloned().unwrap_or(DgnValue::Null),
                        (DgnValue::ByteArray(b), DgnValue::Number(n)) =>
                            b.lock().unwrap().get(n as usize).map(|x| DgnValue::Number(*x as f64)).unwrap_or(DgnValue::Null),
                        (DgnValue::Map(m) | DgnValue::Instance(m), DgnValue::String(k)) =>
                            m.lock().unwrap().get(&k).cloned().unwrap_or(DgnValue::Null),
                        (DgnValue::String(s), DgnValue::Number(n)) =>
                            s.chars().nth(n as usize).map(|c| DgnValue::String(c.to_string())).unwrap_or(DgnValue::Null),
                        _ => DgnValue::Null,
                    };
                    self.stack.push(r);
                }
                Instruction::SetIndex => {
                    let idx = self.pop();
                    let obj = self.pop();
                    // val was Dup'd by compiler so it stays on stack as expression result
                    let val = self.stack.last().cloned().unwrap_or(DgnValue::Null);
                    match (obj, idx) {
                        (DgnValue::List(l), DgnValue::Number(n)) => {
                            let mut v = l.lock().unwrap();
                            let i = n as usize;
                            if i < v.len() { v[i] = val; }
                        }
                        (DgnValue::Map(m) | DgnValue::Instance(m), DgnValue::String(k)) => {
                            m.lock().unwrap().insert(k, val);
                        }
                        (DgnValue::ByteArray(b), DgnValue::Number(n)) => {
                            let mut v = b.lock().unwrap();
                            let i = n as usize;
                            if i < v.len() {
                                if let DgnValue::Number(bv) = val { v[i] = bv as u8; }
                            }
                        }
                        _ => {}
                    }
                }

                // ── function call ─────────────────────────────────────────────
                Instruction::Call(argc) => {
                    let start = self.stack.len().saturating_sub(argc);
                    let args: Vec<DgnValue> = self.stack.drain(start..).collect();
                    let func = self.pop();

                    // Async function → spawn task, return Promise
                    if let DgnValue::Func(ref f) = func {
                        if f.is_async && !f.is_native {
                            let f_clone = f.clone();
                            let h = tokio::spawn(async move {
                                call_dgn_func_impl(f_clone, args, None).await
                            });
                            let ha = Arc::new(AsyncMutex::new(Some(h)));
                            self.stack.push(DgnValue::Promise(Arc::new(AsyncMutex::new(
                                PromiseState::Pending(ha)
                            ))));
                            ip += 1;
                            continue;
                        }
                    }

                    let r = call_dgn_func(func, args).await
                        .map_err(|mut e| { if e.line == 0 { e.line = line; } e })?;
                    self.stack.push(r);
                }

                Instruction::Return => { return Ok(self.pop()); }

                // ── closures ──────────────────────────────────────────────────
                Instruction::MakeClosure(ref opt_name, ref params, req_params, is_async, ref body_instrs, ref body_lines) => {
                    let chunk_arc = Arc::new(Chunk {
                        instructions: *body_instrs.clone(),
                        source_lines:  *body_lines.clone(),
                    });
                    let func = DgnFunc {
                        name: opt_name.clone(),
                        params: params.clone(),
                        req_params, // NEW
                        body: None,
                        closure: env.clone(),   // capture env = true lexical closure
                        is_native: false,
                        is_async,
                        native_fn: None,
                        compiled_chunk: Some(chunk_arc),
                    };
                    self.stack.push(DgnValue::Func(func));
                }

                // ── classes ───────────────────────────────────────────────────
                Instruction::MakeClass(ref name, ref parent_name, ref body_instrs, ref body_lines) => {
                    let mut parent_env: Option<Arc<Mutex<Env>>> = None;
                    if let Some(ref pname) = parent_name {
                        match env.lock().unwrap().get(pname) {
                            Some(DgnValue::Class(penv)) => parent_env = Some(penv),
                            _ => return Err(DgnError::new("Parent class not found", line)),
                        }
                    }

                    let class_env = Arc::new(Mutex::new(Env::new(Some(env.clone()), None)));
                    if let Some(pe) = parent_env {
                        class_env.lock().unwrap().store.insert(
                            "__parent_class__".into(), DgnValue::Class(pe)
                        );
                    }

                    // Execute class body to register methods into class_env
                    let body_chunk = Chunk {
                        instructions: *body_instrs.clone(),
                        source_lines:  *body_lines.clone(),
                    };
                    let mut class_vm = Vm::new();
                    class_vm.run(&body_chunk, class_env.clone()).await?;

                    let class_val = DgnValue::Class(class_env);
                    // StoreVar follows in the chunk (emitted by compiler), so just push
                    self.stack.push(class_val);
                }

                Instruction::NewObject(argc) => {
                    let start = self.stack.len().saturating_sub(argc);
                    let args: Vec<DgnValue> = self.stack.drain(start..).collect();
                    let cls_val = self.pop();

                    if let DgnValue::Class(cls_env) = cls_val {
                        let inst_map = Arc::new(Mutex::new(HashMap::new()));
                        inst_map.lock().unwrap().insert(
                            "__class__".into(), DgnValue::Class(cls_env)
                        );
                        let instance = DgnValue::Instance(inst_map);
                        gc_register(&instance);

                        // Call init if it exists
                        if let Ok(init) = get_attr(instance.clone(), "init") {
                            if let DgnValue::Func(_) = init {
                                call_dgn_func(init, args).await
                                    .map_err(|mut e| { e.line = line; e })?;
                            }
                        }
                        self.stack.push(instance);
                    } else {
                        return Err(DgnError::new("Operand is not a class", line));
                    }
                }

                Instruction::SuperCall(ref method_name, argc) => {
                    let start = self.stack.len().saturating_sub(argc);
                    let args: Vec<DgnValue> = self.stack.drain(start..).collect();

                    let this_val = env.lock().unwrap().get("this")
                        .ok_or(DgnError::new("super used outside class method", line))?;

                    let result = if let DgnValue::Instance(ref inst) = this_val {
                        let cls = inst.lock().unwrap().get("__class__").cloned()
                            .ok_or(DgnError::new("Instance missing class metadata", line))?;
                        if let DgnValue::Class(cls_env) = cls {
                            let parent = cls_env.lock().unwrap().get("__parent_class__")
                                .ok_or(DgnError::new("No parent class for super", line))?;
                            if let DgnValue::Class(penv) = parent {
                                let method = penv.lock().unwrap().get(method_name)
                                    .ok_or(DgnError::new(&format!("Method '{}' not in parent", method_name), line))?;
                                if let DgnValue::Func(f) = method {
                                    call_dgn_func_impl(f, args, Some(this_val)).await?
                                } else { DgnValue::Null }
                            } else { return Err(DgnError::new("Parent is not a class", line)); }
                        } else { return Err(DgnError::new("Invalid class metadata", line)); }
                    } else { return Err(DgnError::new("super requires instance context", line)); };

                    self.stack.push(result);
                }

                // ── for-in iterator ───────────────────────────────────────────
                Instruction::ForIter(ref var_name, exit_ip) => {
                    // First visit: pop iterable and store state
                    if !self.for_iters.contains_key(&ip) {
                        let iterable = self.pop();
                        let items: Vec<DgnValue> = match iterable {
                            DgnValue::List(l)      => l.lock().unwrap().clone(),
                            DgnValue::String(s)    => s.chars().map(|c| DgnValue::String(c.to_string())).collect(),
                            DgnValue::Number(n)    => (0..n as i64).map(|i| DgnValue::Number(i as f64)).collect(),
                            DgnValue::ByteArray(b) => b.lock().unwrap().iter().map(|b| DgnValue::Number(*b as f64)).collect(),
                            _ => return Err(DgnError::new("Not iterable", line)),
                        };
                        self.for_iters.insert(ip, (items, 0));
                    }

                    let entry = self.for_iters.get_mut(&ip).unwrap();
                    if entry.1 >= entry.0.len() {
                        // Exhausted — clean up and jump past loop
                        self.for_iters.remove(&ip);
                        ip = exit_ip;
                        continue;
                    }
                    let item = entry.0[entry.1].clone();
                    entry.1 += 1;
                    env.lock().unwrap().set(var_name.clone(), item)
                        .map_err(|e| DgnError::new(&e, line))?;
                }

                // These are patched to Jump by the compiler; should not appear at runtime
                Instruction::Break | Instruction::Continue | Instruction::DoWhileStart => {}

                // ── exceptions ────────────────────────────────────────────────
                Instruction::PushTry(catch_ip) => {
                    self.try_frames.push((catch_ip, self.stack.len()));
                }
                Instruction::PopTry => { self.try_frames.pop(); }
                Instruction::Raise => {
                    let msg = val_to_string(&self.pop());
                    if let Some((catch_ip, depth)) = self.try_frames.pop() {
                        self.stack.truncate(depth);
                        self.stack.push(DgnValue::String(msg));
                        ip = catch_ip;
                        continue;
                    }
                    return Err(DgnError::new(&msg, line));
                }
                Instruction::EnterCatch(ref var_name) => {
                    let err = self.pop();
                    env.lock().unwrap().set(var_name.clone(), err)
                        .map_err(|e| DgnError::new(&e, line))?;
                }

                // ── async / await ─────────────────────────────────────────────
                Instruction::Await => {
                    let val = self.pop();
                    if let DgnValue::Promise(ref sa) = val {
                        let mut state = sa.lock().await;
                        let extracted = match &mut *state {
                            PromiseState::Resolved(r) => {
                                let r = r.clone();
                                drop(state);
                                self.stack.push(r?);
                                ip += 1;
                                continue;
                            }
                            PromiseState::Pending(ha) => ha.lock().await.take(),
                        };
                        if let Some(h) = extracted {
                            let res = h.await.map_err(|_| DgnError::new("Promise panicked", line))?;
                            *state = PromiseState::Resolved(res.clone());
                            self.stack.push(res?);
                        } else {
                            return Err(DgnError::new("Promise already consumed", line));
                        }
                    } else {
                        self.stack.push(val); // await non-promise is a passthrough
                    }
                }

                // ── import ────────────────────────────────────────────────────
                Instruction::Import(ref path, ref alias) => {
                    vm_do_import(path, alias, env.clone(), line).await?;
                }

                // ── inc / dec ─────────────────────────────────────────────────
                Instruction::IncVar(ref n) => {
                    let cur = env.lock().unwrap().get(n).unwrap_or(DgnValue::Number(0.0));
                    if let DgnValue::Number(v) = cur {
                        let nv = v + 1.0;
                        env.lock().unwrap().set(n.clone(), DgnValue::Number(nv))
                            .map_err(|e| DgnError::new(&e, line))?;
                        self.stack.push(DgnValue::Number(nv));
                    } else { return Err(DgnError::new("Cannot ++ non-number", line)); }
                }
                Instruction::DecVar(ref n) => {
                    let cur = env.lock().unwrap().get(n).unwrap_or(DgnValue::Number(0.0));
                    if let DgnValue::Number(v) = cur {
                        let nv = v - 1.0;
                        env.lock().unwrap().set(n.clone(), DgnValue::Number(nv))
                            .map_err(|e| DgnError::new(&e, line))?;
                        self.stack.push(DgnValue::Number(nv));
                    } else { return Err(DgnError::new("Cannot -- non-number", line)); }
                }
                Instruction::PostIncVar(ref n) => {
                    let cur = env.lock().unwrap().get(n).unwrap_or(DgnValue::Number(0.0));
                    if let DgnValue::Number(v) = cur {
                        env.lock().unwrap().set(n.clone(), DgnValue::Number(v + 1.0))
                            .map_err(|e| DgnError::new(&e, line))?;
                        self.stack.push(DgnValue::Number(v)); // old value
                    } else { return Err(DgnError::new("Cannot ++ non-number", line)); }
                }
                Instruction::PostDecVar(ref n) => {
                    let cur = env.lock().unwrap().get(n).unwrap_or(DgnValue::Number(0.0));
                    if let DgnValue::Number(v) = cur {
                        env.lock().unwrap().set(n.clone(), DgnValue::Number(v - 1.0))
                            .map_err(|e| DgnError::new(&e, line))?;
                        self.stack.push(DgnValue::Number(v)); // old value
                    } else { return Err(DgnError::new("Cannot -- non-number", line)); }
                }
                Instruction::ReadModWriteAttr(ref attr, is_pre, is_dec) => {
                    // Stack: [..., obj] — compiler emitted obj expression before this op
                    let obj = self.pop();
                    if let Ok(DgnValue::Number(n)) = get_attr(obj.clone(), attr) {
                        let nv = if is_dec { n - 1.0 } else { n + 1.0 };
                        if let DgnValue::Map(m) | DgnValue::Instance(m) = &obj {
                            m.lock().unwrap().insert(attr.clone(), DgnValue::Number(nv));
                        }
                        self.stack.push(DgnValue::Number(if is_pre { nv } else { n }));
                    } else { return Err(DgnError::new("Cannot ++/-- non-number attr", line)); }
                }
                Instruction::ReadModWriteIndex(is_pre, is_dec) => {
                    // Stack: [..., obj, idx] — compiler emitted both expressions before this op
                    let idx = self.pop();
                    let obj = self.pop();
                    let cur = match (&obj, &idx) {
                        (DgnValue::List(l), DgnValue::Number(i)) =>
                            l.lock().unwrap().get(*i as usize).cloned().unwrap_or(DgnValue::Null),
                        (DgnValue::Map(m) | DgnValue::Instance(m), DgnValue::String(k)) =>
                            m.lock().unwrap().get(k).cloned().unwrap_or(DgnValue::Null),
                        _ => DgnValue::Null,
                    };
                    if let DgnValue::Number(n) = cur {
                        let nv = if is_dec { n - 1.0 } else { n + 1.0 };
                        match (&obj, &idx) {
                            (DgnValue::List(l), DgnValue::Number(i)) => {
                                let mut v = l.lock().unwrap();
                                let i = *i as usize;
                                if i < v.len() { v[i] = DgnValue::Number(nv); }
                            }
                            (DgnValue::Map(m) | DgnValue::Instance(m), DgnValue::String(k)) => {
                                m.lock().unwrap().insert(k.clone(), DgnValue::Number(nv));
                            }
                            _ => {}
                        }
                        self.stack.push(DgnValue::Number(if is_pre { nv } else { n }));
                    } else { return Err(DgnError::new("Cannot ++/-- non-number index", line)); }
                }
            }
            ip += 1;
        }

        Ok(self.stack.pop().unwrap_or(DgnValue::Null))
    }
}

/// Extracted from eval_expr's Binary arm — reusable by both VM and tree-walk
/// Fully bytecode-native import — no eval_stmt, no tree-walk.
pub async fn vm_do_import(
    path: &str,
    alias: &Option<String>,
    env: Arc<Mutex<Env>>,
    line: usize,
) -> Result<(), DgnError> {
    let bind = alias.clone().unwrap_or_else(|| {
        std::path::Path::new(path).file_stem().unwrap().to_str().unwrap().to_string()
    });

    // 1. Built-in modules
    if let Some(module_val) = get_builtin_module(path) {
        env.lock().unwrap().set(bind, module_val).map_err(|e| DgnError::new(&e, line))?;
        return Ok(());
    }

    let root = Env::get_root(env.clone());

    // 2. Bundled sources
    let bundled_src = {
        let r = root.lock().unwrap();
        if let Some(DgnValue::Map(map)) = r.store.get("__bundled_sources__") {
            let m = map.lock().unwrap();
            let key2 = format!("{}.dgn", path);
            if let Some(DgnValue::String(s)) = m.get(path).or_else(|| m.get(&key2)) {
                Some(s.clone())
            } else { None }
        } else { None }
    };

    let (code, new_env_path) = if let Some(src) = bundled_src {
        (src, PathBuf::from(format!("bundle:///{}", path)))
    } else {
        // 3. File system search
        let current_dir = env.lock().unwrap().current_path
            .parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut search_dirs = vec![current_dir.clone(), current_dir.join("dgn_modules")];
        if let Ok(dp) = std::env::var("DRAGON_PATH") {
            for p in std::env::split_paths(&dp) { search_dirs.push(p); }
        }

        let path_obj = Path::new(path);
        let mut target_files = Vec::new();
        if path_obj.extension().is_some() {
            target_files.push(path_obj.to_path_buf());
        } else {
            target_files.push(PathBuf::from(format!("{}.dgn", path)));
            if let Some(name) = path_obj.file_name() {
                let n = name.to_string_lossy();
                target_files.push(PathBuf::from(path).join(format!("{}.dgn", n)));
                target_files.push(PathBuf::from(path).join("index.dgn"));
            }
        }

        let mut found_path = None;
        'outer: for dir in &search_dirs {
            for target in &target_files {
                let candidate = dir.join(target);
                if candidate.exists() && candidate.is_file() {
                    found_path = Some(candidate.canonicalize().unwrap_or(candidate));
                    break 'outer;
                }
            }
        }

        let canon = found_path
            .ok_or_else(|| DgnError::new(&format!("Import not found: '{}'", path), line))?;
        let c = fs::read_to_string(&canon)
            .map_err(|e| DgnError::new(&e.to_string(), line))?;
        (c, canon)
    };

    let cache_key = new_env_path.to_string_lossy().to_string();

    // 4. Module cache check
    let cached_mod = {
        let r = root.lock().unwrap();
        if let Some(DgnValue::Map(modules)) = r.store.get("__modules__") {
            modules.lock().unwrap().get(&cache_key).cloned()
        } else { None }
    };

    let mod_val = if let Some(m) = cached_mod {
        if let DgnValue::Class(mod_env) = &m {
            if mod_env.lock().unwrap().store.contains_key("__loading__") {
                return Err(DgnError::new(
                    &format!("Cyclic import detected: '{}' is already loading.", path), line
                ));
            }
        }
        m
    } else {
        // 5. Parse and compile module source
        let tokens = lex(&code);
        let mut parser = Parser::new(tokens);
        let module_body = parser.parse().map_err(|e| DgnError::new(&e, line))?;

        let mod_env = Arc::new(Mutex::new(Env::new(None, Some(new_env_path))));
        populate_builtins(mod_env.clone());
        mod_env.lock().unwrap().store.insert("__loading__".to_string(), DgnValue::Bool(true));

        let placeholder = DgnValue::Class(mod_env.clone());
        {
            let r = root.lock().unwrap();
            if let Some(DgnValue::Map(modules)) = r.store.get("__modules__") {
                modules.lock().unwrap().insert(cache_key, placeholder.clone());
            }
        }

        // 6. Execute module via VM (pure bytecode — no eval_stmt)
        let mut mod_chunk = Chunk::new();
        Compiler::compile_stmt(&mut mod_chunk, &module_body);
        mod_chunk.emit(Instruction::Halt, 0);

        let mut mod_vm = Vm::new();
        mod_vm.run(&mod_chunk, mod_env.clone()).await?;

        mod_env.lock().unwrap().store.remove("__loading__");
        placeholder
    };

    // 7. Bind module into caller's env
    let final_bind = alias.clone().unwrap_or_else(|| {
        std::path::Path::new(path).file_stem().unwrap().to_str().unwrap().to_string()
    });
    env.lock().unwrap().set(final_bind, mod_val).map_err(|e| DgnError::new(&e, line))?;
    Ok(())
}

pub fn vm_binary_op(op: &BinOp, l: DgnValue, r: DgnValue, line: usize) -> Result<DgnValue, DgnError> {
    match op {
        BinOp::Add => match (l, r) {
            (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Number(a + b)),
            (DgnValue::String(a), b) => Ok(DgnValue::String(a + &val_to_string(&b))),
            (a, DgnValue::String(b)) => Ok(DgnValue::String(val_to_string(&a) + &b)),
            _ => Ok(DgnValue::Null),
        },
        BinOp::Sub => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(a-b)) } else { Ok(DgnValue::Null) },
        BinOp::Mul => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(a*b)) } else { Ok(DgnValue::Null) },
        BinOp::Div => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(a/b)) } else { Ok(DgnValue::Null) },
        BinOp::Mod => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(a%b)) } else { Ok(DgnValue::Null) },
        BinOp::Eq  => match (l,r) {
            (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Bool((a-b).abs() < f64::EPSILON)),
            (DgnValue::String(a), DgnValue::String(b)) => Ok(DgnValue::Bool(a==b)),
            (DgnValue::Bool(a),   DgnValue::Bool(b))   => Ok(DgnValue::Bool(a==b)),
            (DgnValue::Null,      DgnValue::Null)       => Ok(DgnValue::Bool(true)),
            _ => Ok(DgnValue::Bool(false)),
        },
        BinOp::Ne  => match (l,r) {
            (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Bool((a-b).abs() >= f64::EPSILON)),
            (DgnValue::String(a), DgnValue::String(b)) => Ok(DgnValue::Bool(a!=b)),
            _ => Ok(DgnValue::Bool(true)),
        },
        BinOp::Lt  => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Bool(a<b))  } else { Ok(DgnValue::Bool(false)) },
        BinOp::Gt  => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Bool(a>b))  } else { Ok(DgnValue::Bool(false)) },
        BinOp::Le  => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Bool(a<=b)) } else { Ok(DgnValue::Bool(false)) },
        BinOp::Ge  => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Bool(a>=b)) } else { Ok(DgnValue::Bool(false)) },
        BinOp::BitAnd => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number((a as i64 & b as i64) as f64)) } else { Ok(DgnValue::Null) },
        BinOp::BitOr  => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number((a as i64 | b as i64) as f64)) } else { Ok(DgnValue::Null) },
        BinOp::BitXor => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number((a as i64 ^ b as i64) as f64)) } else { Ok(DgnValue::Null) },
        BinOp::Shl   => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(((a as i64) << (b as i64)) as f64)) } else { Ok(DgnValue::Null) },
        BinOp::Shr   => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(((a as i64) >> (b as i64)) as f64)) } else { Ok(DgnValue::Null) },
        BinOp::NullCoalesce => if let DgnValue::Null = l { Ok(r) } else { Ok(l) },
        BinOp::And | BinOp::Or => Ok(DgnValue::Null), // handled by JumpIfFalse/True
    }
}

#[derive(Debug)]
pub struct Env {
    pub store: HashMap<String, DgnValue>,
    pub constants: HashSet<String>,
    pub parent: Option<Arc<Mutex<Env>>>,
    pub current_path: PathBuf, 
}

impl Env {
    

    pub fn new(parent: Option<Arc<Mutex<Env>>>, path: Option<PathBuf>) -> Self {
        let p = path.or_else(|| {
            parent.as_ref().map(|par| par.lock().unwrap().current_path.clone())
        }).unwrap_or_else(|| PathBuf::from("."));

        Env { store: HashMap::new(), constants: HashSet::new(), parent, current_path: p }
    }

    pub fn get(&self, name: &str) -> Option<DgnValue> {
        if let Some(v) = self.store.get(name) {
            return Some(v.clone());
        }
        if let Some(parent) = &self.parent {
            return parent.lock().unwrap().get(name);
        }
        None
    }

    pub fn define(&mut self, name: String, val: DgnValue) {
        self.store.insert(name, val);
    }

    pub fn set(&mut self, name: impl Into<String>, val: DgnValue) -> Result<(), String> {
        let name = name.into();

        if self.constants.contains(&name) {
            return Err(format!("Cannot reassign constant '{}'", name));
        }
        
         if self.update_existing(&name, &val)? {
            return Ok(());
        }
        self.store.insert(name, val);
        Ok(())
    }

    pub fn define_const(&mut self, name: String, val: DgnValue) -> Result<(), String> {
        if self.store.contains_key(&name) {
            return Err(format!("Variable '{}' already declared", name));
        }
        self.store.insert(name.clone(), val);
        self.constants.insert(name);
        Ok(())
    }
    
    pub fn update_existing(&mut self, name: &str, val: &DgnValue) -> Result<bool, String> {
        if self.store.contains_key(name) {
            if self.constants.contains(name) {
                return Err(format!("Cannot reassign constant '{}'", name));
            }
            self.store.insert(name.to_string(), val.clone());
            return Ok(true);
        }
        if let Some(parent) = &self.parent {
            return parent.lock().unwrap().update_existing(name, val);
        }
        Ok(false)
    }

    pub fn get_root(env: Arc<Mutex<Env>>) -> Arc<Mutex<Env>> {
        let mut curr = env;
        loop {
            let parent = curr.lock().unwrap().parent.clone();
            match parent {
                Some(p) => curr = p,
                None => return curr,
            }
        }
    }
}


pub const KEYWORDS: &[&str] = &[
    "func","class","if","elif","else","while","return","True","False","null",
    "and","or","not","for","in","switch","case","default","break","continue",
    "import","as","try","catch","finally","raise","do","new","extends","this",
    "super", "const", "var", "async", "await"
];

pub fn lex(src: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let bytes = src.as_bytes();
    let len = src.len();
    let mut pos = 0;
    let mut line = 1usize;
    let mut col = 0usize;

    while pos < len {
        let start = pos;
        let col_start = col + 1;
        let b = bytes[pos];

        // Skip whitespace fast
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\xEF' || b == 0 {
            pos += 1; col += 1;
            continue;
        }

        if b == b'\n' {
            line += 1; col = 0; pos += 1;
            continue;
        }

        // Comments
        if b == b'/' && pos + 1 < len {
            if bytes[pos+1] == b'/' {
                while pos < len && bytes[pos] != b'\n' { pos += 1; }
                continue;
            }
            if bytes[pos+1] == b'*' {
                pos += 2;
                while pos + 1 < len && !(bytes[pos] == b'*' && bytes[pos+1] == b'/') {
                    if bytes[pos] == b'\n' { line += 1; col = 0; }
                    pos += 1;
                }
                pos += 2;
                continue;
            }
        }

        // Strings
        if b == b'"' || b == b'\'' {
            pos += 1;
            let mut s = String::new();
            while pos < len && bytes[pos] != b {
                if bytes[pos] == b'\\' && pos + 1 < len {
                    pos += 1;
                    match bytes[pos] {
                        b'n'  => s.push('\n'),
                        b't'  => s.push('\t'),
                        b'"'  => s.push('"'),
                        b'\'' => s.push('\''),
                        b'\\' => s.push('\\'),
                        c     => { s.push('\\'); s.push(c as char); }
                    }
                } else {
                    s.push(bytes[pos] as char);
                }
                pos += 1;
            }
            pos += 1; // closing quote
            col += pos - start;
            tokens.push(Token { typ: TokenType::String, value: s, line, col: col_start, num_val: None });
            continue;
        }

        // Numbers (including hex)
        if b.is_ascii_digit() || (b == b'0' && pos+1 < len && bytes[pos+1] == b'x') {
            if b == b'0' && pos+1 < len && bytes[pos+1] == b'x' {
                pos += 2;
                let hex_start = pos;
                while pos < len && bytes[pos].is_ascii_hexdigit() { pos += 1; }
                let hex_str = &src[hex_start..pos];
                let n = u64::from_str_radix(hex_str, 16).unwrap_or(0) as f64;
                col += pos - start;
                tokens.push(Token { typ: TokenType::Number, value: src[start..pos].to_string(), line, col: col_start, num_val: Some(n) });
            } else {
                // 1. Parse whole number part
                while pos < len && bytes[pos].is_ascii_digit() { pos += 1; }
                
                // 2. Parse fractional part (e.g., .4028)
                if pos < len && bytes[pos] == b'.' && pos+1 < len && bytes[pos+1].is_ascii_digit() {
                    pos += 1;
                    while pos < len && bytes[pos].is_ascii_digit() { pos += 1; }
                }
                
                // 3. Parse scientific notation (e.g., e38, e-300, E+10)
                if pos < len && (bytes[pos] == b'e' || bytes[pos] == b'E') {
                    let mut e_pos = pos + 1;
                    
                    // Allow optional + or - after the E
                    if e_pos < len && (bytes[e_pos] == b'+' || bytes[e_pos] == b'-') {
                        e_pos += 1;
                    }
                    
                    // Only consume if the exponent actually contains digits
                    if e_pos < len && bytes[e_pos].is_ascii_digit() {
                        pos = e_pos;
                        while pos < len && bytes[pos].is_ascii_digit() { pos += 1; }
                    }
                }

                let n: f64 = src[start..pos].parse().unwrap_or(0.0);
                col += pos - start;
                tokens.push(Token { typ: TokenType::Number, value: src[start..pos].to_string(), line, col: col_start, num_val: Some(n) });
            }
            continue;
        }

        // Identifiers and keywords
        if b.is_ascii_alphabetic() || b == b'_' {
            while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
            let word = &src[start..pos];
            col += pos - start;
            if KEYWORDS.contains(&word) {
                tokens.push(Token { typ: TokenType::Keyword(word.to_string()), value: word.to_string(), line, col: col_start, num_val: None });
            } else {
                tokens.push(Token { typ: TokenType::Name, value: word.to_string(), line, col: col_start, num_val: None });
            }
            continue;
        }

        // Two-char operators
        if pos + 1 < len {
            let two = &src[pos..pos+2];
            let tok = match two {
                "++" => Some(TokenType::Inc),
                "--" => Some(TokenType::Dec),
                "==" | "!=" | "<=" | ">=" | "=>" | "<<" | ">>" | "??" => Some(TokenType::Op),
                "//" | "/*" => None, // handled above
                _ => None,
            };
            if let Some(t) = tok {
                tokens.push(Token { typ: t, value: two.to_string(), line, col: col_start, num_val: None });
                pos += 2; col += 2;
                continue;
            }
        }

        // Single-char operators
        let op_chars = b"!+-*/%<>=?:;,&|^~(){}[].";
        if op_chars.contains(&b) {
            tokens.push(Token { typ: TokenType::Op, value: (b as char).to_string(), line, col: col_start, num_val: None });
            pos += 1; col += 1;
            continue;
        }

        // Unrecognized
        panic!("Unexpected character '{}' (Code: {}) at {}:{}", b as char, b, line, col);
    }

    tokens.push(Token { typ: TokenType::Eof, value: String::new(), line, col, num_val: None });
    tokens
}

pub struct Parser {
    pub tokens: Vec<Token>,
    pub pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self, offset: usize) -> &Token {
        if self.pos + offset < self.tokens.len() {
            &self.tokens[self.pos + offset]
        } else {
            &self.tokens[self.tokens.len() - 1]
        }
    }

    fn is_lambda_start(&self) -> bool {
        if self.current().value != "(" { return false; }
        let mut i = 1;
        loop {
            let t = self.peek(i);
            // Stop looking if we hit EOF, a newline, or a block start
            if t.typ == TokenType::Eof || t.typ == TokenType::Newline || t.value == "{" { return false; }
            if t.value == ")" {
                return self.peek(i + 1).value == "=>";
            }
            i += 1;
        }
    }

    fn current(&self) -> &Token {
        self.peek(0)
    }

    fn advance(&mut self) -> Token {
        let t = self.current().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn match_token(&mut self, s: &str) -> bool {
        if self.current().value == s {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> Result<Token, String> {
        if self.current().value == s || (s == "NAME" && matches!(self.current().typ, TokenType::Name)) {
            Ok(self.advance())
        } else {
            Err(format!("Expected '{}', got '{}' at line {}", s, self.current().value, self.current().line))
        }
    }

    pub fn parse(&mut self) -> Result<Stmt, String> {
        let mut stmts = Vec::new();
        let start_line = self.current().line;
        while self.current().typ != TokenType::Eof {
            stmts.push(self.parse_stmt()?);
        }
        Ok(Stmt { node: StmtNode::Block(stmts), line: start_line })
    }

    fn parse_block(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        if self.match_token("{") {
            let mut stmts = Vec::new();
            while self.current().value != "}" && self.current().typ != TokenType::Eof {
                stmts.push(self.parse_stmt()?);
            }
            self.expect("}")?;
            Ok(Stmt { node: StmtNode::Block(stmts), line })
        } else {
             let stmt = self.parse_stmt()?;
             Ok(Stmt { node: StmtNode::Block(vec![stmt]), line })
        }
    }

    pub fn parse_stmt(&mut self) -> Result<Stmt, String> {
        while self.current().typ == TokenType::Newline {
            self.advance();
        }
        if self.current().typ == TokenType::Eof {
            return Ok(Stmt { node: StmtNode::Block(vec![]), line: 0 });
        }

        let line = self.current().line;
        let cur = self.current().value.clone();

        if cur == "const" { return self.parse_const(); }
        if cur == "var" { self.advance(); } 
        if cur == "async" { 
            self.advance(); 
            return self.parse_func(true); // ADD THIS BLOCK
        }
        if cur == "func" { return self.parse_func(false); }
        if cur == "class" { return self.parse_class(); }
        if cur == "return" {
            self.advance();
            let mut val = None;
            if self.current().value != ";" && self.current().value != "}" && self.current().typ != TokenType::Newline {
                val = Some(self.parse_expr()?);
            }
            self.match_end_of_stmt()?;
            return Ok(Stmt { node: StmtNode::Return(val), line });
        }
        if cur == "if" { return self.parse_if(); }
        if cur == "while" { return self.parse_while(); }
        if cur == "for" { return self.parse_for(); }
        if cur == "try" { return self.parse_try(); }
        if cur == "switch" { return self.parse_switch(); }
        if cur == "break" { self.advance(); self.match_end_of_stmt()?; return Ok(Stmt { node: StmtNode::Break, line }); }
        if cur == "continue" { self.advance(); self.match_end_of_stmt()?; return Ok(Stmt { node: StmtNode::Continue, line }); }
        if cur == "import" { return self.parse_import(); }
        if cur == "do" { return self.parse_dowhile(); }
        if cur == "raise" { 
            self.advance(); 
            let e = self.parse_expr()?; 
            self.match_end_of_stmt()?; 
            return Ok(Stmt { node: StmtNode::Raise(e), line }); 
        }
        
        if cur == "{" {
            if self.is_object_literal() {
                let e = self.parse_expr()?;
                self.match_end_of_stmt()?;
                return Ok(Stmt { node: StmtNode::Expr(e), line });
            }
            return self.parse_block();
        }

        let expr = self.parse_expr()?;
        if self.current().value == "=" {
             self.advance(); 
             let rhs = self.parse_expr()?;
             self.match_end_of_stmt()?;
             return Ok(Stmt { node: StmtNode::Expr(Expr { node: ExprNode::Assign(Box::new(expr), Box::new(rhs)), line }), line });
        }

        self.match_end_of_stmt()?;
        Ok(Stmt { node: StmtNode::Expr(expr), line })
    }

   fn match_end_of_stmt(&mut self) -> Result<(), String> {
        if self.match_token(";") {
            return Ok(());
        }
        
        if self.current().typ == TokenType::Newline || self.current().typ == TokenType::Eof || self.current().value == "}" {
            return Ok(());
        }
        Ok(())
    }

    fn is_object_literal(&self) -> bool {
        let mut i = 1;
        let mut depth = 0;
        loop {
            let t = self.peek(i);
            if t.typ == TokenType::Eof { return false; }
            if t.value == "{" { depth += 1; }
            else if t.value == "}" {
                if depth == 0 { return false; }
                depth -= 1;
            }
            else if t.value == ":" && depth == 0 { return true; }
            i += 1;
            if i > 50 { return false; } 
        }
    }

     fn parse_const(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("const")?;
        let name = self.expect("NAME")?.value;
        self.expect("=")?;
        let val = self.parse_expr()?;
        self.match_end_of_stmt()?;
        Ok(Stmt { node: StmtNode::ConstDecl(name, val), line })
    }

    fn parse_func(&mut self, is_async: bool) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("func")?;
        let mut name = String::new();
        if let TokenType::Name = self.current().typ {
            name = self.advance().value;
        }
        self.expect("(")?;
        let mut params = Vec::new();
        let mut default_inits = Vec::new();
        let mut req_params = 0;
        
        if self.current().value != ")" {
            loop {
                let p_name = self.expect("NAME")?.value;
                params.push(p_name.clone());
                
                if self.match_token("=") {
                    let def_expr = self.parse_expr()?;
                    let p_var = Expr { node: ExprNode::Var(p_name.clone()), line };
                    let coalesce = Expr { node: ExprNode::Binary("??".to_string(), Box::new(p_var.clone()), Box::new(def_expr)), line };
                    let assign_expr = Expr { node: ExprNode::Assign(Box::new(p_var), Box::new(coalesce)), line };
                    default_inits.push(Stmt { node: StmtNode::Expr(assign_expr), line });
                } else {
                    if !default_inits.is_empty() {
                        return Err(format!("Required parameter '{}' cannot follow optional parameters at line {}", p_name, line));
                    }
                    req_params += 1;
                }
                
                if !self.match_token(",") { break; }
            }
        }
        self.expect(")")?;
        let mut body = self.parse_block()?;

        if !default_inits.is_empty() {
            if let StmtNode::Block(ref mut stmts) = body.node {
                for (i, stmt) in default_inits.into_iter().enumerate() {
                    stmts.insert(i, stmt);
                }
            }
        }

        // FIX: Added req_params here!
        Ok(Stmt { node: StmtNode::FuncDef(name, params, req_params, Box::new(body), is_async), line })
    }

    fn parse_bitwise(&mut self) -> Result<Expr, String> {
    let mut node = self.parse_add()?; 
    while self.current().typ == TokenType::Op {
        let op = self.current().value.clone();
        if ["<<", ">>", "&", "|", "^"].contains(&op.as_str()) {
            self.advance();
            let rhs = self.parse_add()?;
            node = Expr { 
                node: ExprNode::Binary(op, Box::new(node.clone()), Box::new(rhs)), 
                line: node.line 
            };
            continue;
        }
        break;
    }
    Ok(node)
}

    fn parse_class(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("class")?;
        let name = self.expect("NAME")?.value;
        let mut parent = None;
        if self.match_token("extends") {
            parent = Some(self.expect("NAME")?.value);
        }
        let body = self.parse_block()?;
        Ok(Stmt { node: StmtNode::ClassDef(name, parent, Box::new(body)), line })
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("if")?;
        let has_paren = self.match_token("(");
        let cond = self.parse_expr()?;
        if has_paren { self.expect(")")?; }
        let body = self.parse_block()?;
        
        let mut chain = vec![(Some(cond), body)];
        

        while self.current().typ == TokenType::Newline {
            self.advance();
        }
        
        while self.match_token("elif") {
            let p = self.match_token("(");
            let c = self.parse_expr()?;
            if p { self.expect(")")?; }
            let b = self.parse_block()?;
            chain.push((Some(c), b));
            while self.current().typ == TokenType::Newline {
                self.advance();
            }
        }

        if self.match_token("else") {
            let b = self.parse_block()?;
            chain.push((None, b));
        }
        Ok(Stmt { node: StmtNode::IfChain(chain), line })
    }
    fn parse_while(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("while")?;
        let p = self.match_token("(");
        let cond = self.parse_expr()?;
        if p { self.expect(")")?; }
        let body = self.parse_block()?;
        Ok(Stmt { node: StmtNode::While(cond, Box::new(body)), line })
    }

    fn parse_dowhile(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("do")?;
        let body = self.parse_block()?;
        self.expect("while")?;
        let p = self.match_token("(");
        let cond = self.parse_expr()?;
        if p { self.expect(")")?; }
        self.match_end_of_stmt()?;
        Ok(Stmt { node: StmtNode::DoWhile(Box::new(body), cond), line })
    }

    fn parse_for(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("for")?;
        if self.match_token("(") {
            self.match_token("var");
            if matches!(self.current().typ, TokenType::Name) && self.peek(1).value == "in" {
                let name = self.advance().value;
                self.expect("in")?;
                let iter = self.parse_expr()?;
                self.expect(")")?;
                let body = self.parse_block()?;
                return Ok(Stmt { node: StmtNode::For(name, iter, Box::new(body)), line });
            }
            let mut init = None;
            if self.current().value != ";" {
                let ex = self.parse_expr()?;
                if self.current().value == "=" {
                    self.advance();
                    let rhs = self.parse_expr()?;
                    init = Some(Expr { node: ExprNode::Assign(Box::new(ex.clone()), Box::new(rhs)), line: ex.line });
                } else {
                    init = Some(ex);
                }
            }
            self.expect(";")?;
            let mut cond = None;
            if self.current().value != ";" { cond = Some(self.parse_expr()?); }
            self.expect(";")?;
            let mut step = None;
            if self.current().value != ")" {
                 let ex = self.parse_expr()?;
                 if self.current().value == "=" {
                    self.advance();
                    let rhs = self.parse_expr()?;
                    step = Some(Expr { node: ExprNode::Assign(Box::new(ex.clone()), Box::new(rhs)), line: ex.line });
                 } else {
                    step = Some(ex);
                 }
            }
            self.expect(")")?;
            let body = self.parse_block()?;
            return Ok(Stmt { node: StmtNode::CFor(init, cond, step, Box::new(body)), line });
        }
        self.match_token("var");

        let name = self.expect("NAME")?.value;
        self.expect("in")?;
        let iter = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt { node: StmtNode::For(name, iter, Box::new(body)), line })
    }

    fn parse_import(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("import")?;
        let path = if self.current().typ == TokenType::String {
            self.advance().value 
        } else {
            self.expect("NAME")?.value
        };
        let mut as_name = None;
        if self.match_token("as") {
            as_name = Some(self.expect("NAME")?.value);
        }
        self.match_end_of_stmt()?;
        Ok(Stmt { node: StmtNode::Import(path, as_name), line })
    }

    fn parse_try(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("try")?;
        let try_blk = self.parse_block()?;
        let mut catch_var = None;
        let mut catch_blk = None;
        if self.match_token("catch") {
            self.expect("(")?;
            catch_var = Some(self.expect("NAME")?.value);
            self.expect(")")?;
            catch_blk = Some(self.parse_block()?);
        }
        let mut fin_blk = None;
        if self.match_token("finally") {
            fin_blk = Some(self.parse_block()?);
        }
        Ok(Stmt { node: StmtNode::Try(Box::new(try_blk), catch_var, catch_blk.map(Box::new), fin_blk.map(Box::new)), line })
    }

    fn parse_switch(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("switch")?;
        let p = self.match_token("(");
        let val = self.parse_expr()?;
        if p { self.expect(")")?; }
        self.expect("{")?;
        let mut cases = Vec::new();
        let mut default = None;
        while self.current().value != "}" && self.current().typ != TokenType::Eof {
            if self.match_token("case") {
                let v = self.parse_expr()?;
                self.expect(":")?;
                let blk = self.parse_block()?;
                cases.push((v, blk));
            } else if self.match_token("default") {
                self.expect(":")?;
                default = Some(Box::new(self.parse_block()?));
            } else {
                 return Err(format!("Unexpected token in switch at line {}", self.current().line));
            }
        }
        self.expect("}")?;
        Ok(Stmt { node: StmtNode::Switch(val, cases, default), line })
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<Expr, String> {
        // MUST be changed from parse_or() to parse_null_coalesce()
        let node = self.parse_null_coalesce()?; 
        if self.match_token("?") {
            let t = self.parse_expr()?;
            self.expect(":")?;
            let f = self.parse_expr()?;
            return Ok(Expr { node: ExprNode::Ternary(Box::new(node.clone()), Box::new(t), Box::new(f)), line: node.line });
        }
        Ok(node)
    }

    fn parse_null_coalesce(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_or()?;
        while self.match_token("??") {
            let rhs = self.parse_or()?;
            node = Expr { 
                node: ExprNode::Binary("??".to_string(), Box::new(node.clone()), Box::new(rhs)), 
                line: node.line 
            };
        }
        Ok(node)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_and()?;
        while self.match_token("or") {
            let rhs = self.parse_and()?;
            node = Expr { node: ExprNode::Binary("or".to_string(), Box::new(node.clone()), Box::new(rhs)), line: node.line };
        }
        Ok(node)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_not()?;
        while self.match_token("and") {
            let rhs = self.parse_not()?;
            node = Expr { node: ExprNode::Binary("and".to_string(), Box::new(node.clone()), Box::new(rhs)), line: node.line };
        }
        Ok(node)
    }

    fn parse_not(&mut self) -> Result<Expr, String> {
        let line = self.current().line;
        
        if self.match_token("not") {
            let e = self.parse_not()?;
            return Ok(Expr { node: ExprNode::Unary("not".to_string(), Box::new(e)), line });
        }
        if self.current().typ == TokenType::Op && self.current().value == "!" {
            self.advance();
            let e = self.parse_not()?;
            return Ok(Expr { node: ExprNode::Unary("not".to_string(), Box::new(e)), line });
        }

        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_add()?;
        loop {
            if self.current().typ == TokenType::Op {
                let op = self.current().value.clone();
                if ["==","!=","<",">","<=",">=","in"].contains(&op.as_str()) {
                    self.advance();
                    let rhs = self.parse_add()?;
                    node = Expr { node: ExprNode::Binary(op, Box::new(node.clone()), Box::new(rhs)), line: node.line };
                    continue;
                }
            }
            break;
        }
        Ok(node)
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_mul()?;
        loop {
            if self.current().typ == TokenType::Op {
                let op = self.current().value.clone();
                if ["+", "-", "<<", ">>", "&", "|", "^"].contains(&op.as_str()) {
                self.advance();
                let rhs = self.parse_mul()?;
                node = Expr { node: ExprNode::Binary(op, Box::new(node.clone()), Box::new(rhs)), line: node.line };
                continue;
            }
            }
            break;
        }
        Ok(node)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_unary()?;
        loop {
            if self.current().typ == TokenType::Op {
                let op = self.current().value.clone();
                if ["*","/","%"].contains(&op.as_str()) {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    node = Expr { node: ExprNode::Binary(op, Box::new(node.clone()), Box::new(rhs)), line: node.line };
                    continue;
                }
            }
            break;
        }
        Ok(node)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.match_token("await") {
            let line = self.current().line;
            let expr = self.parse_unary()?;
            return Ok(Expr { node: ExprNode::Await(Box::new(expr)), line });
        }

        if self.current().typ == TokenType::Op && ["+", "-"].contains(&self.current().value.as_str()) {
            let op = self.advance().value;
            let v = self.parse_unary()?;
            Ok(Expr { node: ExprNode::Unary(op, Box::new(v)), line: 0 })
        } else if self.match_token("new") {
            let mut cls_expr = self.parse_primary()?;
            
            while self.match_token(".") {
                let t = self.advance();
                let attr = match t.typ {
                    TokenType::Name | TokenType::Keyword(_) => t.value,
                    _ => return Err(format!("Expected property name after '.', got '{}' at line {}", t.value, t.line))
                };
                let line = cls_expr.line;
                cls_expr = Expr { 
                    node: ExprNode::GetAttr(Box::new(cls_expr), attr), 
                    line 
                };
            }

            let mut args = Vec::new();
            if self.match_token("(") && self.current().value != ")" {
                loop {
                    args.push(self.parse_expr()?);
                    if !self.match_token(",") { break; }
                }
                self.expect(")")?;
            } else if self.current().value == ")" {
                self.advance();
            }
            
            return Ok(Expr { node: ExprNode::New(Box::new(cls_expr), args), line: 0 });
        } else {
            self.parse_postfix()
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_primary()?;
        loop {
            if self.match_token("(") {
                let mut args = Vec::new();
                let is_end = self.current().typ == TokenType::Op && self.current().value == ")";
                if !is_end {
                    loop {
                        args.push(self.parse_expr()?);
                        if !self.match_token(",") { break; }
                    }
                }
                self.expect(")")?;
                node = Expr { node: ExprNode::Call(Box::new(node.clone()), args), line: node.line };
            } else if self.match_token(".") {
                let t = self.advance();
                let attr = match t.typ {
                    TokenType::Name | TokenType::Keyword(_) => t.value,
                    _ => return Err(format!("Expected property name after '.', got '{}' at line {}", t.value, t.line))
                };
                
                node = Expr { node: ExprNode::GetAttr(Box::new(node.clone()), attr), line: node.line };
            } else if self.match_token("[") {
                let idx = self.parse_expr()?;
                self.expect("]")?;
                node = Expr { node: ExprNode::Index(Box::new(node.clone()), Box::new(idx)), line: node.line };
            } else if matches!(self.current().typ, TokenType::Inc | TokenType::Dec) {
                let op = self.advance().value;
                node = Expr { node: ExprNode::PostOp(if op=="++"{"INC".into()}else{"DEC".into()}, Box::new(node.clone())), line: node.line };
            } else {
                break;
            }
        }
        Ok(node)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let t = self.current().clone();
        let line = t.line;
        match t.typ {
            TokenType::Number => { self.advance(); Ok(Expr { node: ExprNode::Number(t.num_val.unwrap()), line }) },
            TokenType::String => { self.advance(); Ok(Expr { node: ExprNode::String(t.value), line }) },
            TokenType::Keyword(ref k) => match k.as_str() {
                "True" => { self.advance(); Ok(Expr { node: ExprNode::Bool(true), line }) },
                "False" => { self.advance(); Ok(Expr { node: ExprNode::Bool(false), line }) },
                "null" => { self.advance(); Ok(Expr { node: ExprNode::Null, line }) },
                "this" => { self.advance(); Ok(Expr { node: ExprNode::This, line }) },
                "super" => {
                    self.advance();
                    self.expect(".")?; 
                    let method = self.expect("NAME")?.value;
                    self.expect("(")?;
                    let mut args = Vec::new();
                    if self.current().value != ")" {
                        loop {
                            args.push(self.parse_expr()?);
                            if !self.match_token(",") { break; }
                        }
                    }
                    self.expect(")")?;
                    Ok(Expr { node:ExprNode::Super(method, args), line})
                },
                "func" => {
                    self.advance();
                    self.expect("(")?;
                    let mut params = Vec::new();
                    let mut default_inits = Vec::new();
                    let mut req_params = 0; // FIX: Added
                    
                    if self.current().value != ")" {
                        loop {
                            let p_name = self.expect("NAME")?.value;
                            params.push(p_name.clone());
                            
                            if self.match_token("=") {
                                let def_expr = self.parse_expr()?;
                                let p_var = Expr { node: ExprNode::Var(p_name.clone()), line };
                                let coalesce = Expr { node: ExprNode::Binary("??".to_string(), Box::new(p_var.clone()), Box::new(def_expr)), line };
                                let assign_expr = Expr { node: ExprNode::Assign(Box::new(p_var), Box::new(coalesce)), line };
                                default_inits.push(Stmt { node: StmtNode::Expr(assign_expr), line });
                            } else {
                                if !default_inits.is_empty() {
                                    return Err(format!("Required parameter '{}' cannot follow optional parameters at line {}", p_name, line));
                                }
                                req_params += 1;
                            }
                            if !self.match_token(",") { break; }
                        }
                    }
                    self.expect(")")?;
                    let mut body = self.parse_block()?;
                    if !default_inits.is_empty() {
                        if let StmtNode::Block(ref mut stmts) = body.node {
                            for (i, stmt) in default_inits.into_iter().enumerate() {
                                stmts.insert(i, stmt);
                            }
                        }
                    }
                    // FIX: Added req_params here!
                    Ok(Expr { node: ExprNode::FuncExpr(params, req_params, Box::new(body)), line })
                },
                _ => { self.advance(); Ok(Expr { node: ExprNode::Var(t.value), line }) }
            },
            TokenType::Name => { self.advance(); Ok(Expr { node: ExprNode::Var(t.value), line }) },
            TokenType::Op => {
                if t.value == "(" {

                    if self.is_lambda_start() {
                        self.advance(); // consume '('
                        let mut params = Vec::new();
                        let mut default_inits = Vec::new();
                        let mut req_params = 0; // FIX: Added
                        
                        if self.current().value != ")" {
                            loop {
                                let p_name = self.expect("NAME")?.value;
                                params.push(p_name.clone());
                                
                                if self.match_token("=") {
                                    let def_expr = self.parse_expr()?;
                                    let p_var = Expr { node: ExprNode::Var(p_name.clone()), line };
                                    let coalesce = Expr { node: ExprNode::Binary("??".to_string(), Box::new(p_var.clone()), Box::new(def_expr)), line };
                                    let assign_expr = Expr { node: ExprNode::Assign(Box::new(p_var), Box::new(coalesce)), line };
                                    default_inits.push(Stmt { node: StmtNode::Expr(assign_expr), line });
                                } else {
                                    if !default_inits.is_empty() {
                                        return Err(format!("Required parameter '{}' cannot follow optional parameters at line {}", p_name, line));
                                    }
                                    req_params += 1;
                                }
                                if !self.match_token(",") { break; }
                            }
                        }
                        self.expect(")")?;
                        self.expect("=>")?;

                        let mut body = if self.current().value == "{" {
                            self.parse_block()?
                        } else {
                            let expr = self.parse_expr()?;
                            Stmt { node: StmtNode::Return(Some(expr)), line: self.current().line }
                        };

                        if !default_inits.is_empty() {
                            if !matches!(body.node, StmtNode::Block(_)) {
                                body = Stmt { node: StmtNode::Block(vec![body]), line: line };
                            }
                            if let StmtNode::Block(ref mut stmts) = body.node {
                                for (i, stmt) in default_inits.into_iter().enumerate() {
                                    stmts.insert(i, stmt);
                                }
                            }
                        }
                        // FIX: Added req_params here!
                        return Ok(Expr { node: ExprNode::FuncExpr(params, req_params, Box::new(body)), line });
                    }

                    self.advance();
                    let e = self.parse_expr()?;
                    self.expect(")")?;
                    Ok(e)
                } else if t.value == "[" {
                    self.advance();
                    let mut list = Vec::new();
                    let is_end = self.current().typ == TokenType::Op && self.current().value == "]";
                    if !is_end {
                        loop {
                            list.push(self.parse_expr()?);
                            if !self.match_token(",") { break; }
                        }
                    }
                    self.expect("]")?;
                    Ok(Expr { node: ExprNode::List(list), line })
                } else if t.value == "{" {
                    self.advance();
                    let mut obj = Vec::new();
                    let is_end = self.current().typ == TokenType::Op && self.current().value == "}";
                    if !is_end {
                        loop {
                            let k = if self.current().typ == TokenType::String {
                                self.advance().value
                            } else {
                                self.expect("NAME")?.value
                            };
                            self.expect(":")?;
                            let v = self.parse_expr()?;
                            obj.push((k, v));
                            if !self.match_token(",") { break; }
                        }
                    }
                    self.expect("}")?;
                    Ok(Expr { node: ExprNode::Obj(obj), line })
                } else {
                    Err(format!("Unexpected token {} at line {}", t.value, line))
                }
            },
            _ => Err(format!("Unexpected token {:?} at line {}", t, line)),
        }
    }
}

pub fn is_truthy(v: &DgnValue) -> bool {
    match v {
        DgnValue::Null => false,
        DgnValue::Bool(b) => *b,
        DgnValue::Number(n) => *n != 0.0,
        DgnValue::String(s) => !s.is_empty(),
        DgnValue::List(l) => !l.lock().unwrap().is_empty(),
        DgnValue::Map(m) => !m.lock().unwrap().is_empty(),
        DgnValue::ByteArray(b) => !b.lock().unwrap().is_empty(),
        _ => true,
    }
}

pub fn val_to_string(v: &DgnValue) -> String {
    match v {
        DgnValue::Null => "null".to_string(),
        DgnValue::Bool(b) => if *b { "True".to_string() } else { "False".to_string() },
        DgnValue::Number(n) => if n.fract() == 0.0 { format!("{}", *n as i64) } else { format!("{}", n) },
        DgnValue::String(s) => s.clone(),
        DgnValue::List(l) => {
            let l = l.lock().unwrap();
            let parts: Vec<String> = l.iter().map(val_to_string).collect();
            format!("[{}]", parts.join(", "))
        },
        DgnValue::Map(m) => {
            let m = m.lock().unwrap();
            let mut parts: Vec<String> = m.iter().map(|(k,v)| format!("{}: {}", k, val_to_string(v))).collect();
            parts.sort(); 
            format!("{{ {} }}", parts.join(", "))
        },
        DgnValue::Func(_) => "<function>".to_string(),
        DgnValue::Class(_) => "<class>".to_string(),
        DgnValue::Instance(_) => "<instance>".to_string(),
        DgnValue::ByteArray(b) => format!("<ByteArray len={}>", b.lock().unwrap().len()),
       #[cfg(not(any(target_arch = "wasm32")))]
        DgnValue::NativeLib(_) => "<NativeLib>".to_string(),
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        DgnValue::Socket(_) => "<Socket>".to_string(),
        DgnValue::Regex(r) => format!("<Regex {}>", r.as_str()),
        DgnValue::Thread(_) => "<Thread>".to_string(),
        DgnValue::Promise(_) => "<Promise>".to_string(),
    }
}



pub fn get_attr(obj: DgnValue, name: &str) -> Result<DgnValue, DgnError> {
    match obj {

        DgnValue::Class(env) => {
            let e = env.lock().unwrap();
            if let Some(val) = e.get(name) {
                return Ok(val);
            }
            Ok(DgnValue::Null)
        },

        DgnValue::Promise(state_arc) => {
            match name {
                "then" => {
                    let state_arc_clone = state_arc.clone();
                    Ok(native_fn(move |args| {
                        let callback = args.get(0).cloned().ok_or(DgnError::new("Expected callback for .then", 0))?;
                        let state = state_arc_clone.clone();
                        
                        // Return a NEW Promise representing the chain
                        let handle = tokio::spawn(async move {
                            let mut s = state.lock().await;
                            
                            let extracted_handle = match &mut *s {
                                PromiseState::Resolved(r) => return r.clone(),
                                PromiseState::Pending(h_arc) => {
                                    let mut h_lock = h_arc.lock().await;
                                    h_lock.take()
                                }
                            };
                            
                            let res = if let Some(h) = extracted_handle {
                                let r = h.await.map_err(|_| DgnError::new("Promise panicked", 0))?;
                                *s = PromiseState::Resolved(r.clone());
                                r
                            } else {
                                Err(DgnError::new("Promise already consumed", 0))
                            };
                            
                            match res {
                                Ok(val) => call_dgn_func(callback, vec![val]).await,
                                Err(e) => Err(e),
                            }
                        });
                        
                        Ok(DgnValue::Promise(Arc::new(AsyncMutex::new(PromiseState::Pending(Arc::new(AsyncMutex::new(Some(handle))))))))
                    }))
                },
                "catch" => {
                    let state_arc_clone = state_arc.clone();
                    Ok(native_fn(move |args| {
                        let callback = args.get(0).cloned().ok_or(DgnError::new("Expected callback for .catch", 0))?;
                        let state = state_arc_clone.clone();
                        
                        let handle = tokio::spawn(async move {
                            let mut s = state.lock().await;
                            
                            let extracted_handle = match &mut *s {
                                PromiseState::Resolved(r) => return r.clone(),
                                PromiseState::Pending(h_arc) => {
                                    let mut h_lock = h_arc.lock().await;
                                    h_lock.take()
                                }
                            };
                            
                            let res = if let Some(h) = extracted_handle {
                                let r = h.await.map_err(|_| DgnError::new("Promise panicked", 0))?;
                                *s = PromiseState::Resolved(r.clone());
                                r
                            } else {
                                Err(DgnError::new("Promise already consumed", 0))
                            };
                            
                            match res {
                                Ok(val) => Ok(val), 
                                Err(e) => call_dgn_func(callback, vec![DgnValue::String(e.msg)]).await,
                            }
                        });
                        
                        Ok(DgnValue::Promise(Arc::new(AsyncMutex::new(PromiseState::Pending(Arc::new(AsyncMutex::new(Some(handle))))))))
                    }))
                },
                _ => Ok(DgnValue::Null)
            }
        },

        DgnValue::Map(m) => {
            match name {
                "keys" => return Ok(native_method(m, |m, _| {
                    let keys: Vec<DgnValue> = m.lock().unwrap().keys().map(|k| DgnValue::String(k.clone())).collect();
                    Ok(DgnValue::List(Arc::new(Mutex::new(keys))))
                })),
                "values" => return Ok(native_method(m, |m, _| {
                    let vals: Vec<DgnValue> = m.lock().unwrap().values().cloned().collect();
                    Ok(DgnValue::List(Arc::new(Mutex::new(vals))))
                })),
                "contains" => return Ok(native_method(m, |m, args| {
                    let key = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(m.lock().unwrap().contains_key(&key)))
                })),
                "set" => return Ok(native_method(m, |m, args| {
                    if args.len() == 2 {
                        if let DgnValue::String(k) = &args[0] {
                            m.lock().unwrap().insert(k.clone(), args[1].clone());
                        }
                    }
                    Ok(DgnValue::Map(m))
                })),
                "get" => return Ok(native_method(m, |m, args| {
                    let key = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(m.lock().unwrap().get(&key).cloned().unwrap_or(DgnValue::Null))
                })),
                "remove" => return Ok(native_method(m, |m, args| {
                    if let Some(DgnValue::String(k)) = args.get(0) {
                        m.lock().unwrap().remove(k);
                    }
                    Ok(DgnValue::Map(m))
                })),
                "clear" => return Ok(native_method(m, |m, _| {
                    m.lock().unwrap().clear();
                    Ok(DgnValue::Map(m))
                })),
                "length" => return Ok(DgnValue::Number(m.lock().unwrap().len() as f64)),
                _ => {}
            }
            // Not a built-in — look up as a stored data key
            Ok(m.lock().unwrap().get(name).cloned().unwrap_or(DgnValue::Null))
        },
        DgnValue::Instance(inst) => {

            if let Some(v) = inst.lock().unwrap().get(name) {
                return Ok(v.clone());
            }
            

            let mut cls_ref = inst.lock().unwrap().get("__class__").cloned();
            while let Some(DgnValue::Class(cls_env)) = cls_ref {
                 if let Some(mem) = cls_env.lock().unwrap().get(name) {
                     
   
                    if let DgnValue::Func(f) = mem { 
                         let inst_capture = inst.clone();
                         let f_clone = f.clone();
                     
                         return Ok(DgnValue::Func(DgnFunc {
                             name: f.name.clone(), params: f.params.clone(), req_params: f.req_params, body: None,
                             closure: f.closure.clone(),
                             is_native: true,
                             is_async: f.is_async,
                             compiled_chunk: None,
                             native_fn: Some(Arc::new(move |args| {
                                 let rt = tokio::runtime::Handle::current();
                                    tokio::task::block_in_place(|| {
                                        rt.block_on(call_dgn_func_impl(f_clone.clone(), args, Some(DgnValue::Instance(inst_capture.clone()))))
                                    })
                             }))
                         }));
                     }
                     
                     return Ok(mem);
                 }
                 cls_ref = cls_env.lock().unwrap().get("__parent_class__");
            }
            Ok(DgnValue::Null)
        },
        DgnValue::List(l) => {
            match name {
                "push" => Ok(native_method(l, |l, args| { 
                    l.lock().unwrap().push(args.get(0).unwrap_or(&DgnValue::Null).clone()); Ok(DgnValue::List(l))
                })),
                "pop" => Ok(native_method(l, |l, _| { 
                    Ok(l.lock().unwrap().pop().unwrap_or(DgnValue::Null)) 
                })),
                "insert" => Ok(native_method(l, |l, args| {
                    let idx = if let Some(DgnValue::Number(n)) = args.get(0) { *n as usize } else { 0 };
                    let val = args.get(1).cloned().unwrap_or(DgnValue::Null);
                    let mut vec = l.lock().unwrap();
                    if idx <= vec.len() { vec.insert(idx, val); }
                    Ok(DgnValue::Null)
                })),
                "remove_at" => Ok(native_method(l, |l, args| {
                    let idx = if let Some(DgnValue::Number(n)) = args.get(0) { *n as usize } else { 0 };
                    let mut vec = l.lock().unwrap();
                    if idx < vec.len() { Ok(vec.remove(idx)) } else { Ok(DgnValue::Null) }
                })),
                "length" => Ok(DgnValue::Number(l.lock().unwrap().len() as f64)),
                "join" => Ok(native_method(l, |l, args| {
                    let sep = if let Some(DgnValue::String(s)) = args.get(0) { s.clone() } else { "".to_string() };
                    let strs: Vec<String> = l.lock().unwrap().iter().map(val_to_string).collect();
                    Ok(DgnValue::String(strs.join(&sep)))
                })),
                "map" => Ok(native_method(l, |l, args| {
                    let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                    let old_vec = l.lock().unwrap().clone();
                    let mut new_vec = Vec::new();
                    
                    // Grab the current runtime to block on the async function safely
                    let rt = tokio::runtime::Handle::current(); 
                    
                    for item in old_vec {
                        let val = tokio::task::block_in_place(|| rt.block_on(call_dgn_func(func.clone(), vec![item])))?;
                        new_vec.push(val);
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(new_vec))))
                })),
                "filter" => Ok(native_method(l, |l, args| {
                        let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                        let old_vec = l.lock().unwrap().clone();
                        let mut new_vec = Vec::new();
                        
                        let rt = tokio::runtime::Handle::current(); // Grab runtime
                        
                        for item in old_vec {
                            // Block on the async call
                            let res = tokio::task::block_in_place(|| rt.block_on(call_dgn_func(func.clone(), vec![item.clone()])))?;
                            if is_truthy(&res) {
                                new_vec.push(item);
                            }
                        }
                        Ok(DgnValue::List(Arc::new(Mutex::new(new_vec))))
                    })),
                "sort" => Ok(native_method(l, |l, _| {
                    let mut vec = l.lock().unwrap();
                    vec.sort_by(|a, b| {
                        match (a, b) {
                            (DgnValue::Number(n1), DgnValue::Number(n2)) => n1.partial_cmp(n2).unwrap_or(std::cmp::Ordering::Equal),
                            (DgnValue::String(s1), DgnValue::String(s2)) => s1.cmp(s2),
                            _ => std::cmp::Ordering::Equal
                        }
                    });
                    Ok(DgnValue::Null)
                })),
                "reverse" => Ok(native_method(l, |l, _| {
                    l.lock().unwrap().reverse();
                    Ok(DgnValue::Null)
                })),
                "contains" => Ok(native_method(l, |l, args| {
                    let val = args.get(0).cloned().unwrap_or(DgnValue::Null);
                    let vec = l.lock().unwrap();
                    let found = vec.iter().any(|x| val_to_string(x) == val_to_string(&val));
                    Ok(DgnValue::Bool(found))
                })),
                _ => Ok(DgnValue::Null)
            }
        },
        DgnValue::String(s) => {
             match name {
                 "length" => Ok(DgnValue::Number(s.len() as f64)),
                 "upper" => { let s=s.clone(); Ok(native_fn(move |_| Ok(DgnValue::String(s.to_uppercase())))) },
                 "lower" => { let s=s.clone(); Ok(native_fn(move |_| Ok(DgnValue::String(s.to_lowercase())))) },
                 "trim" => { let s=s.clone(); Ok(native_fn(move |_| Ok(DgnValue::String(s.trim().to_string())))) },
                 "split" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let sep = if let Some(DgnValue::String(sp)) = args.get(0) { sp.clone() } else { " ".to_string() };
                     let parts = if sep.is_empty() {
                        s.chars().map(|c| DgnValue::String(c.to_string())).collect()
                     } else {
                        s.split(&sep).map(|p| DgnValue::String(p.to_string())).collect()
                     };
                     Ok(DgnValue::List(Arc::new(Mutex::new(parts))))
                 }))},
                 "replace" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let from = if let Some(DgnValue::String(f)) = args.get(0) { f } else { "" };
                     let to = if let Some(DgnValue::String(t)) = args.get(1) { t } else { "" };
                     Ok(DgnValue::String(s.replace(from, to)))
                 }))},
                 "contains" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let sub = if let Some(DgnValue::String(sb)) = args.get(0) { sb } else { "" };
                     Ok(DgnValue::Bool(s.contains(sub)))
                 }))},
                 "starts_with" => { let s=s.clone(); Ok(native_fn(move |args| {
                    let sub = if let Some(DgnValue::String(sb)) = args.get(0) { sb } else { "" };
                    Ok(DgnValue::Bool(s.starts_with(sub)))
                }))},
                "ends_with" => { let s=s.clone(); Ok(native_fn(move |args| {
                    let sub = if let Some(DgnValue::String(sb)) = args.get(0) { sb } else { "" };
                    Ok(DgnValue::Bool(s.ends_with(sub)))
                }))},
                 "index_of" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let sub = if let Some(DgnValue::String(sb)) = args.get(0) { sb } else { "" };
                     Ok(DgnValue::Number(s.find(sub).map(|i| i as f64).unwrap_or(-1.0)))
                 }))},
                 "substring" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let start = if let Some(DgnValue::Number(n)) = args.get(0) { *n as usize } else { 0 };
                     let end = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { s.len() };
                     if start < s.len() && end <= s.len() && start <= end {
                         Ok(DgnValue::String(s[start..end].to_string()))
                     } else {
                         Ok(DgnValue::String("".to_string()))
                     }
                 }))},
                 _ => Ok(DgnValue::Null)
             }
        },
        DgnValue::ByteArray(b) => {
            match name {
                "length" => Ok(DgnValue::Number(b.lock().unwrap().len() as f64)),
                "to_string" => Ok(native_method(b, |b, _| {
                    let v = b.lock().unwrap().clone();
                    Ok(DgnValue::String(String::from_utf8_lossy(&v).to_string()))
                })),
                "push" => Ok(native_method(b, |b, args| {
                    if let Some(DgnValue::Number(n)) = args.get(0) {
                        b.lock().unwrap().push(*n as u8);
                    }
                    Ok(DgnValue::Null)
                })),
                 _ => Ok(DgnValue::Null)
            }
        },
       #[cfg(not(any(target_arch = "wasm32")))]
        DgnValue::NativeLib(lib) => {
            let lib_ref = lib.clone();
            let func_name = CString::new(name).unwrap();

            Ok(native_fn(move |args| {
                let l = lib_ref.lock().unwrap();
                
                // 1. Prepare Arguments
                let mut raw_args: Vec<usize> = Vec::new();
                let mut keep_alive_strs = Vec::new(); // Keeps C-Strings valid in memory

                for arg in args {
                    match arg {
                        DgnValue::Number(n) => raw_args.push(n as usize),
                        DgnValue::Bool(b) => raw_args.push(if b { 1 } else { 0 }),
                        DgnValue::String(s) => {
                            let c = CString::new(s).unwrap();
                            raw_args.push(c.as_ptr() as usize);
                            keep_alive_strs.push(c);
                        },
                        DgnValue::ByteArray(b) => {
                             let ptr = b.lock().unwrap().as_ptr() as usize;
                             raw_args.push(ptr);
                        },
                        DgnValue::Null => raw_args.push(0),
                        _ => raw_args.push(0),
                    }
                }

                // 2. Pad to 8 arguments (The Magic Fix)
                // This ensures we can call functions with 0 to 8 arguments safely
                while raw_args.len() < 8 { raw_args.push(0); }

                unsafe {
                    // Define a generic function pointer that takes 8 usize args
                    type GenericFn = unsafe extern "C" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;

                    let symbol: Symbol<GenericFn> = match l.get(func_name.as_bytes()) {
                        Ok(s) => s,
                        Err(e) => return Err(DgnError::new(&format!("FFI Error: Symbol '{}' not found", func_name.to_str().unwrap()), 0))
                    };

                    // 3. Call the C function
                    let res = symbol(
                        raw_args[0], raw_args[1], raw_args[2], raw_args[3],
                        raw_args[4], raw_args[5], raw_args[6], raw_args[7]
                    );

                    // 4. Return Result
                    // Heuristic: If it looks like a boolean (0 or 1), return Bool, otherwise Number.
                    // You can adjust this logic if needed.
                    if res == 0 || res == 1 {
                        Ok(DgnValue::Bool(res != 0))
                    } else {
                        Ok(DgnValue::Number(res as f64))
                    }
                }
            }))
        },
        DgnValue::Regex(r) => {
            match name {
                "test" => Ok(native_method(r, |r, args| {
                    let s = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(r.is_match(&s)))
                })),
                "match" => Ok(native_method(r, |r, args| {
                    let s = args.get(0).map(val_to_string).unwrap_or_default();
                    if let Some(m) = r.find(&s) { Ok(DgnValue::String(m.as_str().to_string())) } else { Ok(DgnValue::Null) }
                })),
                "replace" => Ok(native_method(r, |r, args| {
                    let s = args.get(0).map(val_to_string).unwrap_or_default();
                    let repl = args.get(1).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::String(r.replace_all(&s, &repl).to_string()))
                })),
                _ => Ok(DgnValue::Null)
            }
        }
        _ => Ok(DgnValue::Null)
    }
}

pub fn native_method<T: Send + Sync + 'static>(target: Arc<T>, f: impl Fn(Arc<T>, Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync + 'static) -> DgnValue {
    DgnValue::Func(DgnFunc {
        name: Some("native_method".into()), params: vec![], req_params: 0, body: None,
        closure: Arc::new(Mutex::new(Env::new(None, None))), is_native: true,
        is_async: false,
        compiled_chunk: None,
        native_fn: Some(Arc::new(move |args| f(target.clone(), args)))
    })
}

pub fn native_fn(f: impl Fn(Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync + 'static) -> DgnValue {
    DgnValue::Func(DgnFunc {
        name: Some("native".into()), params: vec![], req_params: 0, body: None,
        closure: Arc::new(Mutex::new(Env::new(None, None))), is_native: true, is_async: false,
        compiled_chunk: None,
        native_fn: Some(Arc::new(f))
    })
}

pub async fn call_dgn_func(func: DgnValue, args: Vec<DgnValue>) -> Result<DgnValue, DgnError> {
    if let DgnValue::Func(f) = func {
        if f.is_native {
             return (f.native_fn.unwrap())(args);
        }
        call_dgn_func_impl(f, args, None).await
    } else {
        Err(DgnError::new("Not a function", 0))
    }
}

#[async_recursion]
pub async fn call_dgn_func_impl(f: DgnFunc, args: Vec<DgnValue>, this_ctx: Option<DgnValue>) -> Result<DgnValue, DgnError> {

    if args.len() < f.req_params {
        let fn_name = f.name.as_deref().unwrap_or("<anonymous>");
        return Err(DgnError::new(&format!("Function '{}' expects at least {} argument(s), but got {}", fn_name, f.req_params, args.len()), 0));
    }

    let scope = Arc::new(Mutex::new(Env::new(Some(f.closure.clone()), None)));

    for (i, param) in f.params.iter().enumerate() {
        let v = args.get(i).cloned().unwrap_or(DgnValue::Null);
        scope.lock().unwrap().store.insert(param.clone(), v);
    }

    if let Some(th) = this_ctx {
        scope.lock().unwrap().store.insert("this".to_string(), th);
    }

    // If this function has a compiled bytecode chunk, run it through the VM
    if let Some(chunk_arc) = &f.compiled_chunk {
        let chunk = chunk_arc.as_ref().clone();
        let mut vm = Vm::new();
        return vm.run(&chunk, scope).await;
    }


        Ok(DgnValue::Null)

}

#[cfg(not(any(target_arch = "wasm32")))]
fn get_ffi_type_info(name: &str) -> Option<(Type, usize, usize)> {
    match name {
        "u8" | "byte"   => Some((Type::u8(), 1, 1)),
        "i8" | "char"   => Some((Type::i8(), 1, 1)),
        "u16"           => Some((Type::u16(), 2, 2)),
        "i16" | "short" => Some((Type::i16(), 2, 2)),
        "u32"           => Some((Type::u32(), 4, 4)),
        "i32" | "int"   => Some((Type::i32(), 4, 4)),
        "u64"           => Some((Type::u64(), 8, 8)),
        "i64" | "long"  => Some((Type::i64(), 8, 8)),
        "f32" | "float" => Some((Type::f32(), 4, 4)),
        "f64" | "double"=> Some((Type::f64(), 8, 8)),
        "pointer" | "string" => Some((Type::pointer(), 8, 8)), 
        _ => None, 
    }
}

fn make_promise<F>(f: F) -> DgnValue
where
    F: FnOnce() -> Result<DgnValue, DgnError> + Send + 'static,
{
    let blocking_handle = tokio::task::spawn_blocking(f);
    
    let async_handle = tokio::spawn(async move {
        blocking_handle.await.unwrap_or_else(|_| Err(DgnError::new("Background thread panicked", 0)))
    });
    
    DgnValue::Promise(Arc::new(AsyncMutex::new(PromiseState::Pending(
        Arc::new(AsyncMutex::new(Some(async_handle)))
    ))))
}


pub fn get_builtin_module(name: &str) -> Option<DgnValue> {
  {
     let cache = BUILTIN_MODULE_CACHE.lock().unwrap();
        if let Some(v) = cache.get(name) {
            return Some(v.clone());
        }
    }
    // Not cached yet — build it
    let val = build_builtin_module(name)?;
    BUILTIN_MODULE_CACHE.lock().unwrap().insert(name.to_owned(), val.clone());
    Some(val)
}

/// Builds a built-in module map from scratch.
/// Called only ONCE per module name — result is cached by get_builtin_module.
fn build_builtin_module(name: &str) -> Option<DgnValue> {
    match name {
        "math" => {
            let mut map = HashMap::new();
            map.insert("sin".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.sin() } else { 0.0 }))));
            map.insert("cos".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.cos() } else { 0.0 }))));
            map.insert("sqrt".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.sqrt() } else { 0.0 }))));
            map.insert("abs".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.abs() } else { 0.0 }))));
            map.insert("floor".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.floor() } else { 0.0 }))));
            map.insert("ceil".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.ceil() } else { 0.0 }))));
            map.insert("round".to_string(), native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.round() } else { 0.0 }))));
            map.insert("pow".to_string(), native_fn(|args| {
                let base = if let Some(DgnValue::Number(n)) = args.get(0) { *n } else { 0.0 };
                let exp = if let Some(DgnValue::Number(n)) = args.get(1) { *n } else { 1.0 };
                Ok(DgnValue::Number(base.powf(exp)))
            }));
            map.insert("log".to_string(), native_fn(|args| {
                let n = if let Some(DgnValue::Number(n)) = args.get(0) { *n } else { 0.0 };
                Ok(DgnValue::Number(n.ln()))
            }));
            map.insert("min".to_string(), native_fn(|args| {
                let mut min = std::f64::INFINITY;
                for arg in args { if let DgnValue::Number(n) = arg { if n < min { min = n; } } }
                Ok(DgnValue::Number(min))
            }));
            map.insert("max".to_string(), native_fn(|args| {
                let mut max = std::f64::NEG_INFINITY;
                for arg in args { if let DgnValue::Number(n) = arg { if n > max { max = n; } } }
                Ok(DgnValue::Number(max))
            }));

             map.insert("range".to_string(), native_fn(|args| {
                let (start, stop, step) = match args.len() {
                    1 => (0.0, args[0].as_number().unwrap_or(0.0), 1.0),
                    2 => (args[0].as_number().unwrap_or(0.0), args[1].as_number().unwrap_or(0.0), 1.0),
                    3 => (args[0].as_number().unwrap_or(0.0), args[1].as_number().unwrap_or(0.0), args[2].as_number().unwrap_or(1.0)),
                    _ => return Err(DgnError::new("range() expects 1 to 3 arguments", 0)),
                };

                let mut nums = Vec::new();
                let mut curr = start;

                if step > 0.0 {
                    while curr < stop {
                        nums.push(DgnValue::Number(curr));
                        curr += step;
                    }
                } else if step < 0.0 {
                    while curr > stop {
                        nums.push(DgnValue::Number(curr));
                        curr += step;
                    }
                }

                Ok(DgnValue::List(Arc::new(Mutex::new(nums))))
            }));
            map.insert("random".to_string(), native_fn(|_| Ok(DgnValue::Number(rand::random::<f64>()))));
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "json" => {
            let mut map = HashMap::new();
            fn dgn_to_json(v: &DgnValue) -> serde_json::Value {
                match v {
                    DgnValue::Null => serde_json::Value::Null,
                    DgnValue::Bool(b) => serde_json::Value::Bool(*b),
                    DgnValue::Number(n) => serde_json::Number::from_f64(*n).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null),
                    DgnValue::String(s) => serde_json::Value::String(s.clone()),
                    DgnValue::List(l) => serde_json::Value::Array(l.lock().unwrap().iter().map(dgn_to_json).collect()),
                    DgnValue::Map(m) | DgnValue::Instance(m) => {
                        let mut map = serde_json::Map::new();
                        for (k, v) in m.lock().unwrap().iter() { map.insert(k.clone(), dgn_to_json(v)); }
                        serde_json::Value::Object(map)
                    },
                    _ => serde_json::Value::String(val_to_string(v)),
                }
            }

            map.insert("encode".to_string(), native_fn(|args| {
                let v = args.get(0).unwrap_or(&DgnValue::Null);
                Ok(DgnValue::String(dgn_to_json(v).to_string()))
            }));

            map.insert("decode".to_string(), native_fn(|args| {
                let json_str = args.get(0).map(val_to_string).unwrap_or_default();
                fn json_to_dgn(v: serde_json::Value) -> DgnValue {
                    match v {
                        serde_json::Value::Null => DgnValue::Null,
                        serde_json::Value::Bool(b) => DgnValue::Bool(b),
                        serde_json::Value::Number(n) => DgnValue::Number(n.as_f64().unwrap_or(0.0)),
                        serde_json::Value::String(s) => DgnValue::String(s),
                        serde_json::Value::Array(arr) => DgnValue::List(Arc::new(Mutex::new(arr.into_iter().map(json_to_dgn).collect()))),
                        serde_json::Value::Object(m) => {
                            let mut dgn_map = HashMap::new();
                            for (k, v) in m { dgn_map.insert(k, json_to_dgn(v)); }
                            DgnValue::Map(Arc::new(Mutex::new(dgn_map)))
                        }
                    }
                }
                match serde_json::from_str(&json_str) {
                    Ok(val) => Ok(json_to_dgn(val)),
                    Err(_) => Ok(DgnValue::Null)
                }
            }));
            
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "base64" => {
            let mut map = HashMap::new();
            map.insert("encode".to_string(), native_fn(|args| {
                use base64::{Engine as _, engine::general_purpose};
                let s = args.get(0).map(val_to_string).unwrap_or_default();
                Ok(DgnValue::String(general_purpose::STANDARD.encode(s)))
            }));
            map.insert("decode".to_string(), native_fn(|args| {
                use base64::{Engine as _, engine::general_purpose};
                let s = args.get(0).map(val_to_string).unwrap_or_default();
                match general_purpose::STANDARD.decode(s) {
                    Ok(bytes) => Ok(DgnValue::String(String::from_utf8_lossy(&bytes).to_string())),
                    Err(_) => Ok(DgnValue::Null)
                }
            }));
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "file" => {
            let mut map = HashMap::new();
            #[cfg(not(target_arch = "wasm32"))]
            {
                map.insert("read_all".to_string(), native_fn(|a| {
                    let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("File path required",0)) };
                    match fs::read_to_string(p) { Ok(c) => Ok(DgnValue::String(c)), Err(_) => Ok(DgnValue::Null) }
                }));
                map.insert("read_bytes".to_string(), native_fn(|a| {
                    let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("File path required",0)) };
                    match fs::read(p) { Ok(c) => Ok(DgnValue::ByteArray(Arc::new(Mutex::new(c)))), Err(_) => Ok(DgnValue::Null) }
                }));
                map.insert("write".to_string(), native_fn(|a| {
                    let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("Path required",0)) };
                    let c = if let Some(DgnValue::String(s)) = a.get(1) { s } else { "" };
                    Ok(DgnValue::Bool(fs::write(p, c).is_ok()))
                }));
                map.insert("exists".to_string(), native_fn(|args| {
                    let path = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(Path::new(&path).exists()))
                }));
                map.insert("delete".to_string(), native_fn(|args| {
                    let path = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(fs::remove_file(path).is_ok()))
                }));

                map.insert("append".to_string(), native_fn(|args| {
                    let p = if let Some(DgnValue::String(s)) = args.get(0) { s } else { return Err(DgnError::new("Path required", 0)) };
                    let c = if let Some(DgnValue::String(s)) = args.get(1) { s } else { "" };
                    let res = std::fs::OpenOptions::new().create(true).append(true).open(p)
                        .and_then(|mut f| std::io::Write::write_all(&mut f, c.as_bytes()));
                    Ok(DgnValue::Bool(res.is_ok()))
                }));
                
                map.insert("mkdir".to_string(), native_fn(|args| {
                    let p = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::fs::create_dir_all(p).is_ok()))
                }));
                
                map.insert("rmdir".to_string(), native_fn(|args| {
                    let p = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::fs::remove_dir_all(p).is_ok()))
                }));
                
                map.insert("rename".to_string(), native_fn(|args| {
                    let from = args.get(0).map(val_to_string).unwrap_or_default();
                    let to = args.get(1).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::fs::rename(from, to).is_ok()))
                }));
                
                map.insert("size".to_string(), native_fn(|args| {
                    let p = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Number(std::fs::metadata(p).map(|m| m.len() as f64).unwrap_or(-1.0)))
                }));
                
                map.insert("is_dir".to_string(), native_fn(|args| {
                    let p = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)))
                }));
            }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "uuid" => {
            let mut map = HashMap::new();
            map.insert("v4".to_string(), native_fn(|_| {
                let u1: u32 = rand::random();
                let u2: u32 = rand::random();
                let u3: u32 = rand::random();
                let u4: u32 = rand::random();
                let uuid = format!("{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                    u1,
                    (u2 >> 16) as u16,
                    (u2 & 0xFFF) as u16,
                    ((u3 >> 16) & 0x3FFF | 0x8000) as u16,
                    (((u3 & 0xFFFF) as u64) << 32) | (u4 as u64)
                );
                Ok(DgnValue::String(uuid))
            }));
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "os" => {
            let mut map = HashMap::new();
            #[cfg(not(any(target_arch = "wasm32")))]
            {
                map.insert("name".to_string(), native_fn(|_| Ok(DgnValue::String(std::env::consts::OS.to_string()))));
                map.insert("arch".to_string(), native_fn(|_| Ok(DgnValue::String(std::env::consts::ARCH.to_string()))));
                map.insert("family".to_string(), native_fn(|_| Ok(DgnValue::String(std::env::consts::FAMILY.to_string()))));
                map.insert("system".to_string(), native_fn(|args| {
                    let cmd = if let Some(DgnValue::String(s)) = args.get(0) { s } else { "" };
                    if cfg!(target_os = "windows") {
                        Command::new("cmd").args(["/C", cmd]).status().map(|s| DgnValue::Number(s.code().unwrap_or(0) as f64)).map_err(|e| DgnError::new(&e.to_string(),0))
                    } else {
                        Command::new("sh").args(["-c", cmd]).status().map(|s| DgnValue::Number(s.code().unwrap_or(0) as f64)).map_err(|e| DgnError::new(&e.to_string(),0))
                    }
                }));
                map.insert("setenv".to_string(), native_fn(|args| {
                    let k = args.get(0).map(val_to_string).unwrap_or_default();
                    let v = args.get(1).map(val_to_string).unwrap_or_default();
                    std::env::set_var(k, v);
                    Ok(DgnValue::Null)
                }));
                
                map.insert("cwd".to_string(), native_fn(|_| {
                    match std::env::current_dir() {
                        Ok(path) => Ok(DgnValue::String(path.to_string_lossy().into_owned())),
                        Err(_) => Ok(DgnValue::Null)
                    }
                }));
                
                map.insert("chdir".to_string(), native_fn(|args| {
                    let path = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(std::env::set_current_dir(path).is_ok()))
                }));
                    map.insert("listdir".to_string(), native_fn(|args| {
                        let path = args.get(0).map(val_to_string).unwrap_or_default();
                    let mut files = Vec::new();
                    if let Ok(entries) = fs::read_dir(path) {
                        for entry in entries {
                            if let Ok(e) = entry { if let Ok(name) = e.file_name().into_string() { files.push(DgnValue::String(name)); } }
                        }
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(files))))
                }));
            }
            #[cfg(target_arch = "wasm32")]
            { map.insert("name".to_string(), native_fn(|_| Ok(DgnValue::String("web-wasm".into())))); }
            map.insert("getenv".to_string(), native_fn(|args| {
                if let Some(DgnValue::String(k)) = args.get(0) { Ok(DgnValue::String(env::var(k).unwrap_or_default())) } else { Ok(DgnValue::Null) }
            }));
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "path" => {
            let mut map = HashMap::new();
            
            // Joins multiple path segments using the OS-specific separator
            map.insert("join".to_string(), native_fn(|args| {
                let mut path = std::path::PathBuf::new();
                for arg in args {
                    path.push(val_to_string(&arg));
                }
                Ok(DgnValue::String(path.to_string_lossy().into_owned()))
            }));

            // Gets the final component of a path (e.g., "file.txt" from "/dir/file.txt")
            map.insert("basename".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                let name = std::path::Path::new(&p)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(DgnValue::String(name))
            }));

            // Gets the directory portion of a path (e.g., "/dir" from "/dir/file.txt")
            map.insert("dirname".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                let dir = std::path::Path::new(&p)
                    .parent()
                    .map(|d| d.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Ok(DgnValue::String(dir))
            }));

            // Gets the extension of a file path (e.g., ".txt" from "file.txt")
            map.insert("extname".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                let ext = std::path::Path::new(&p)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                Ok(DgnValue::String(ext))
            }));

            // Checks if a path is absolute
            map.insert("is_absolute".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                Ok(DgnValue::Bool(std::path::Path::new(&p).is_absolute()))
            }));

            // Resolves a path to its absolute, normalized format (resolving symlinks and '..')
            #[cfg(not(target_arch = "wasm32"))]
            map.insert("resolve".to_string(), native_fn(|args| {
                let p = args.get(0).map(val_to_string).unwrap_or_default();
                match std::fs::canonicalize(&p) {
                    Ok(canon) => Ok(DgnValue::String(canon.to_string_lossy().into_owned())),
                    Err(_) => Ok(DgnValue::Null) 
                }
            }));

            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "time" =>{
             let mut map = HashMap::new();
            map.insert("now".to_string(), native_fn(|_| { Ok(DgnValue::Number(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64())) }));
            map.insert("unix".to_string(), native_fn(|_| {
                    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
                    Ok(DgnValue::Number(t))
                }));
            map.insert("date".to_string(), native_fn(|args| {
                let now = std::time::SystemTime::now();
                let elapsed = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
                
                let days = elapsed / 86400;
                let year = 1970 + (days / 365); 
                
                let mut map1 = HashMap::new();
                map1.insert("timestamp".to_string(), DgnValue::Number(elapsed as f64));
                map1.insert("approx_year".to_string(), DgnValue::Number(year as f64));
                
                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
            }));

            map.insert("date_now".to_string(), native_fn(|_| {
                let now: DateTime<Local> = Local::now();
                let mut map1 = HashMap::new();
                map1.insert("year".to_string(), DgnValue::Number(now.year() as f64));
                map1.insert("month".to_string(), DgnValue::Number(now.month() as f64));
                map1.insert("day".to_string(), DgnValue::Number(now.day() as f64));
                map1.insert("hour".to_string(), DgnValue::Number(now.hour() as f64));
                map1.insert("minute".to_string(), DgnValue::Number(now.minute() as f64));
                map1.insert("second".to_string(), DgnValue::Number(now.second() as f64));
                map1.insert("str".to_string(), DgnValue::String(now.format("%Y-%m-%d %H:%M:%S").to_string()));
                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
            }));

            map.insert("sleep_ms".to_string(), native_fn(|args| {
                let ms = if let Some(DgnValue::Number(n)) = args.get(0) { *n as u64 } else { 0 };
                thread::sleep(Duration::from_millis(ms));
                Ok(DgnValue::Null)
            }));
            
              Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "net" =>{
            let mut map = HashMap::new();
             #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        {
              map.insert("http_get".to_string(), native_fn(|args| {
                    let url = args.get(0).map(val_to_string).ok_or(DgnError::new("URL required", 0))?;
                    
                    Ok(make_promise(move || {
                        match ureq::get(&url).call() {
                            Ok(resp) => {
                                let status = resp.status();
                                let text = resp.into_string().unwrap_or_default();
                                
                                let mut map1 = HashMap::new();
                                map1.insert("status".to_string(), DgnValue::Number(status as f64));
                                map1.insert("body".to_string(), DgnValue::String(text));
                                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                            },
                            Err(e) => Err(DgnError::new(&format!("HTTP Request failed: {}", e), 0))
                        }
                    }))
                }));

                map.insert("http_post".to_string(), native_fn(|args| {
                    let url = args.get(0).map(val_to_string).ok_or(DgnError::new("URL required", 0))?;
                    let body = args.get(1).map(val_to_string).unwrap_or_default();
                    
                    // Extract headers safely before passing them to the background thread
                    let mut headers_vec = Vec::new();
                    if let Some(DgnValue::Map(headers)) = args.get(2) {
                        for (k, v) in headers.lock().unwrap().iter() {
                            headers_vec.push((k.clone(), val_to_string(v)));
                        }
                    }

                    Ok(make_promise(move || {
                        let mut req = ureq::post(&url);
                        for (k, v) in headers_vec {
                            req = req.set(&k, &v);
                        }

                        match req.send_string(&body) {
                            Ok(resp) => {
                                let status = resp.status();
                                let text = resp.into_string().unwrap_or_default();
                                let mut map1 = HashMap::new();
                                map1.insert("status".to_string(), DgnValue::Number(status as f64));
                                map1.insert("body".to_string(), DgnValue::String(text));
                                Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                            },
                            Err(e) => Err(DgnError::new(&format!("HTTP Request failed: {}", e), 0))
                        }
                    }))
                }));

       
                map.insert("socket_connect".to_string(), native_fn(|args| {
                    let addr = args.get(0).map(val_to_string).ok_or(DgnError::new("Address required", 0))?;
                    
                    Ok(make_promise(move || {
                        match TcpStream::connect(&addr) {
                            Ok(stream) => Ok(DgnValue::Socket(Arc::new(Mutex::new(stream)))),
                            Err(e) => Err(DgnError::new(&format!("Connection failed: {}", e), 0))
                        }
                    }))
                }));

                map.insert("socket_send".to_string(), native_fn(|args| {
                    if let Some(DgnValue::Socket(s)) = args.get(0) {
                        let msg = args.get(1).map(val_to_string).unwrap_or_default();
                        let socket_clone = s.clone();
                        
                        Ok(make_promise(move || {
                            let mut stream = socket_clone.lock().unwrap();
                            match stream.write_all(msg.as_bytes()) {
                                Ok(_) => Ok(DgnValue::Bool(true)),
                                Err(_) => Ok(DgnValue::Bool(false))
                            }
                        }))
                    } else {
                        Err(DgnError::new("Socket required", 0))
                    }
                }));

                map.insert("socket_read".to_string(), native_fn(|args| {
                    if let Some(DgnValue::Socket(s)) = args.get(0) {
                        let socket_clone = s.clone();
                        
                        Ok(make_promise(move || {
                            let mut stream = socket_clone.lock().unwrap();
                            let mut reader = BufReader::new(&*stream);
                            let mut buffer = String::new();
                            match reader.read_line(&mut buffer) {
                                Ok(_) => Ok(DgnValue::String(buffer)),
                                Err(_) => Ok(DgnValue::Null)
                            }
                        }))
                    } else {
                        Err(DgnError::new("Socket required", 0))
                    }
                }));
                
                map.insert("socket_close".to_string(), native_fn(|args| {
                    if let Some(DgnValue::Socket(s)) = args.get(0) {
                        let _ = s.lock().unwrap().shutdown(Shutdown::Both);
                        Ok(DgnValue::Bool(true))
                    } else { Ok(DgnValue::Bool(false)) }
                }));

                map.insert("socket_server".to_string(), native_fn(|args| {
                    let port = args.get(0).and_then(|v| v.as_number()).unwrap_or(8080.0) as u16;
                    let handler = args.get(1).cloned().ok_or(DgnError::new("Handler function required", 0))?;
                    
                    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
                        .map_err(|e| DgnError::new(&e.to_string(), 0))?;
                        
                    thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap(); 
                        for stream in listener.incoming() {
                            if let Ok(s) = stream {
                                let client_socket = DgnValue::Socket(Arc::new(Mutex::new(s)));
                                let handler_clone = handler.clone();
                                
                                rt.spawn(async move {
                                    let _ = call_dgn_func(handler_clone, vec![client_socket]).await; 
                                });
                            }
                        }
                    });

                    Ok(DgnValue::Bool(true))
                }));

                    map.insert("start_http_server".to_string(), native_fn(|args| {
                    let port = if let Some(DgnValue::Number(n)) = args.get(0) { *n as u16 } else { 8080 };
                    let handler = args.get(1).cloned().ok_or(DgnError::new("Handler required", 0))?;
                    
                    thread::spawn(move || {
                        let server = match Server::http(format!("0.0.0.0:{}", port)) {
                            Ok(s) => s,
                            Err(e) => { println!("Failed to start server: {}", e); return; }
                        };
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        for mut request in server.incoming_requests() {
                            let method = request.method().to_string();
                            let url = request.url().to_string();
                            let mut content = String::new();
                            request.as_reader().read_to_string(&mut content).unwrap_or(0);
                            
                            let headers_map = Arc::new(Mutex::new(HashMap::new()));
                            for h in request.headers() {
                                headers_map.lock().unwrap().insert(h.field.to_string(), DgnValue::String(h.value.to_string()));
                            }

                            let query_map = Arc::new(Mutex::new(HashMap::new()));
                            if let Some(pos) = url.find('?') {
                                for pair in url[pos+1..].split('&') {
                                    let mut parts = pair.split('=');
                                    if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                                        query_map.lock().unwrap().insert(k.to_string(), DgnValue::String(v.to_string()));
                                    }
                                }
                            }

                            let req_obj = DgnValue::Map(Arc::new(Mutex::new(HashMap::from([
                                ("method".to_string(), DgnValue::String(method)),
                                ("path".to_string(), DgnValue::String(url)), 
                                ("body".to_string(), DgnValue::String(content)),
                                ("headers".to_string(), DgnValue::Map(headers_map)),
                                ("query".to_string(), DgnValue::Map(query_map)),
                            ]))));
                            
                            let req_handle = Arc::new(Mutex::new(Some(request)));
                            let res_map = Arc::new(Mutex::new(HashMap::new()));
                            
                            let rh = req_handle.clone();
                            res_map.lock().unwrap().insert("send".into(), native_method(rh, move |rh, args| {
                                let body = val_to_string(args.get(0).unwrap_or(&DgnValue::String("".into())));
                                let status = if let Some(DgnValue::Number(n)) = args.get(1) { *n as u16 } else { 200 };
                                let headers_arg = args.get(2).cloned();

                                if let Some(req) = rh.lock().unwrap().take() {
                                    let mut response = Response::from_string(body).with_status_code(status);
                                    
                                    if let Some(DgnValue::Map(hm)) = headers_arg {
                                        for (k, v) in hm.lock().unwrap().iter() {
                                            if let Ok(h) = Header::from_bytes(k.as_bytes(), val_to_string(v).as_bytes()) {
                                                response.add_header(h);
                                            }
                                        }
                                    }

                                    let _ = req.respond(response);
                                }
                                Ok(DgnValue::Null)
                            }));

                            let handler_clone = handler.clone();
                            rt.spawn(async move {
                                let _ = call_dgn_func(handler_clone, vec![req_obj, DgnValue::Map(res_map)]).await;
                            });
                        }
                    });
                    Ok(DgnValue::Bool(true))
                }));
             }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },

        "zip" => {
            let mut map = HashMap::new();
            #[cfg(not(target_arch = "wasm32"))]
            {
                map.insert("list".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    
                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let mut files = Vec::new();
                    for i in 0..archive.len() {
                        if let Ok(z_file) = archive.by_index(i) {
                            files.push(DgnValue::String(z_file.name().to_string()));
                        }
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(files))))
                }));

                // Checks if a specific file exists inside the ZIP/JAR
                // Checks if a specific file exists inside the ZIP/JAR
                map.insert("exists".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    // FIX: Evaluate and drop the temporary borrow immediately
                    let file_exists = archive.by_name(&internal_path).is_ok();
                    
                    Ok(DgnValue::Bool(file_exists))
                }));
                // Returns metadata about a file inside the archive (size, compressed size, type)
                map.insert("info".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let z_file = archive.by_name(&internal_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    
                    let mut info_map = HashMap::new();
                    info_map.insert("size".to_string(), DgnValue::Number(z_file.size() as f64));
                    info_map.insert("compressed_size".to_string(), DgnValue::Number(z_file.compressed_size() as f64));
                    info_map.insert("is_dir".to_string(), DgnValue::Bool(z_file.is_dir()));
                    info_map.insert("is_file".to_string(), DgnValue::Bool(z_file.is_file()));

                    Ok(DgnValue::Map(Arc::new(Mutex::new(info_map))))
                }));

                // Extracts a single file from the ZIP/JAR to the host disk
                map.insert("extract".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;
                    let dest_path = args.get(2).map(val_to_string).ok_or(DgnError::new("Destination path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let mut z_file = archive.by_name(&internal_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut out_file = std::fs::File::create(&dest_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    
                    std::io::copy(&mut z_file, &mut out_file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    Ok(DgnValue::Bool(true))
                }));
                // Reads a file inside the ZIP/JAR as a String (Great for MANIFEST.MF)
                map.insert("read_string".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let mut z_file = archive.by_name(&internal_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut buffer = String::new();
                    std::io::Read::read_to_string(&mut z_file, &mut buffer).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    Ok(DgnValue::String(buffer))
                }));

                // Reads a file inside the ZIP/JAR as raw bytes (Great for .class files)
                map.insert("read_bytes".to_string(), native_fn(|args| {
                    let archive_path = args.get(0).map(val_to_string).ok_or(DgnError::new("Archive path required", 0))?;
                    let internal_path = args.get(1).map(val_to_string).ok_or(DgnError::new("Internal path required", 0))?;

                    let file = std::fs::File::open(&archive_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut archive = zip::ZipArchive::new(file).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    let mut z_file = archive.by_name(&internal_path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    let mut buffer = Vec::new();
                    std::io::Read::read_to_end(&mut z_file, &mut buffer).map_err(|e| DgnError::new(&e.to_string(), 0))?;

                    Ok(DgnValue::ByteArray(Arc::new(Mutex::new(buffer))))
                }));
            }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        
        "ffi" => {
            let mut map = HashMap::new();
            #[cfg(not(target_arch = "wasm32"))]
    {
            map.insert("loadlib".to_string(), native_fn(|args| {
                let path = if let Some(DgnValue::String(s)) = args.get(0) { s } else { return Err(DgnError::new("Path req", 0)) };
                unsafe {
                    let lib = Library::new(path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    
                    let mut map1 = HashMap::new();
                    map1.insert("path".to_string(), DgnValue::String(path.clone()));
                    map1.insert("ptr".to_string(), DgnValue::NativeLib(Arc::new(Mutex::new(lib))));
                    
                    Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                }
            }));
            
            map.insert("def".to_string(), native_fn(|args| {
                if args.len() < 4 { return Err(DgnError::new("Usage: ffi_def(lib, name, ret_type, [arg_types])", 0)); }
                
                let lib_map = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid Lib Object",0)); };
                let lib_path = val_to_string(lib_map.get("path").unwrap());
                
                let func_name = val_to_string(&args[1]);
                let ret_type = val_to_string(&args[2]);
                
                let arg_types_list = if let DgnValue::List(l) = &args[3] { l.lock().unwrap().clone() } else { vec![] };
                let arg_types: Vec<String> = arg_types_list.iter().map(val_to_string).collect();

                // Store in the global registry
                let key = format!("{}::{}", lib_path, func_name);
                FFI_SIGNATURES.lock().unwrap().insert(key, (ret_type, arg_types));
                
                Ok(DgnValue::Null)
            }));
            
           map.insert("loadlib_call".to_string(), native_fn(|args| {
                if args.len() < 2 { return Err(DgnError::new("loadlib_call(lib, symbol, args_json)", 0)); }
                
                let lib_val = &args[0];
                let symbol_name = val_to_string(&args[1]);
                let input_arg = val_to_string(args.get(2).unwrap_or(&DgnValue::String("".to_string())));

                if let DgnValue::NativeLib(lib) = lib_val {
                    let l = lib.lock().unwrap();
                    unsafe {
                        type DragonFunc = unsafe extern "C" fn(*const c_char) -> *mut c_char;
                        let func: Symbol<DragonFunc> = l.get(symbol_name.as_bytes()).map_err(|e| DgnError::new(&e.to_string(), 0))?;
                        
                        let c_input = CString::new(input_arg).unwrap();
                        let c_result = func(c_input.as_ptr());
                        
                        if c_result.is_null() {
                            return Ok(DgnValue::Null);
                        }
                        
                        let res_str = CStr::from_ptr(c_result).to_string_lossy().into_owned();
                        Ok(DgnValue::String(res_str))
                    }
                } else {
                    Err(DgnError::new("First arg must be NativeLib", 0))
                }
            }));
             
              map.insert("symbol_addr".to_string(), native_fn(|args| {
                // 1. Get Library
                let lib_map = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid Lib",0)); };
                let lib_arc = if let DgnValue::NativeLib(l) = lib_map.get("ptr").unwrap() { l } else { return Err(DgnError::new("Invalid Lib Ptr",0)); };
                
                // 2. Get Symbol Name
                let func_name = val_to_string(&args[1]);
                
                // 3. Return Address
                let l = lib_arc.lock().unwrap();
                unsafe {
                    // We cast the symbol to a generic pointer, then to usize (number)
                    let symbol: Symbol<*const c_void> = l.get(func_name.as_bytes())
                        .map_err(|e| DgnError::new(&e.to_string(), 0))?;
                    Ok(DgnValue::Number(*symbol as usize as f64))
                }
            }));

            map.insert("call".to_string(), native_fn(|args| {
                    if args.len() < 2 { return Err(DgnError::new("Usage: ffi_call(lib, name, ...args)", 0)); }

                    let lib_map = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid Lib",0)); };
                    let lib_path = val_to_string(lib_map.get("path").unwrap());
                    let lib_arc = if let DgnValue::NativeLib(l) = lib_map.get("ptr").unwrap() { l } else { return Err(DgnError::new("Invalid Lib Ptr",0)); };
                    
                    let func_name = val_to_string(&args[1]);
                    let call_args_values = &args[2..];

                    let key = format!("{}::{}", lib_path, func_name);
                    let sig_guard = FFI_SIGNATURES.lock().unwrap();
                    let (ret_type_str, arg_types_str) = sig_guard.get(&key)
                        .ok_or(DgnError::new(&format!("No signature for '{}'. Call ffi_def() first.", func_name), 0))?;

                    // 1. Prepare Types
                    let mut arg_types: Vec<Type> = Vec::new();
                    for t in arg_types_str {
                        match t.as_str() {
                            "int" | "i32" => arg_types.push(Type::i32()),
                            "long" | "i64" => arg_types.push(Type::i64()),
                            "float" | "f32" => arg_types.push(Type::f32()),
                            "double" | "f64" => arg_types.push(Type::f64()),
                            "pointer" | "void*" => arg_types.push(Type::pointer()),
                            "string" | "char*" => arg_types.push(Type::pointer()),
                            "bool" => arg_types.push(Type::u8()),
                            "void" => {}, 
                            name => {
                                // NEW: Check if it is a registered C-Struct
                                if let Some(def) = FFI_STRUCTS.lock().unwrap().get(name) {
                                    // We clone the pre-calculated LibFFI Type::structure
                                    arg_types.push(def.libffi_type.clone());
                                } else {
                                    return Err(DgnError::new(&format!("Unknown FFI type: {}", name), 0));
                                }
                            }
                        }
                    }

                    let ret_type = match ret_type_str.as_str() {
                        "void" => Type::void(),
                        "int" | "i32" => Type::i32(),
                        "long" | "i64" => Type::i64(),
                        "float" | "f32" => Type::f32(),
                        "double" | "f64" => Type::f64(),
                        "pointer" | "void*" => Type::pointer(),
                        "bool" => Type::u8(),
                        _ => return Err(DgnError::new("Unknown return type", 0))
                    };

                    // FIX: Use into_iter() to satisfy Cif::new requirements
                    let cif = Cif::new(arg_types.into_iter(), ret_type.clone());
                    
                    let l = lib_arc.lock().unwrap();
                    let code_ptr: CodePtr = unsafe {
                        let symbol: Symbol<*mut c_void> = l.get(func_name.as_bytes())
                            .map_err(|e| DgnError::new(&e.to_string(), 0))?;
                        CodePtr::from_ptr(*symbol)
                    };

                    // 2. Prepare Values safely
                    // We use an enum to keep the converted data alive during the call
                    enum FfiValue {
                        I32(i32), I64(i64),
                        F32(f32), F64(f64),
                        Ptr(usize), U8(u8),
                        Struct(Vec<u8>),
                    }

                    let mut values = Vec::new();
                    let mut keep_alive_strs: Vec<CString> = Vec::new();

                    for (i, val) in call_args_values.iter().enumerate() {
                        let t_str = arg_types_str.get(i).map(|s| s.as_str()).unwrap_or("int");
                        match t_str {
                            "int" | "i32" => values.push(FfiValue::I32(val.as_number().unwrap_or(0.0) as i32)),
                            "long" | "i64" => values.push(FfiValue::I64(val.as_number().unwrap_or(0.0) as i64)),
                            "float" | "f32" => values.push(FfiValue::F32(val.as_number().unwrap_or(0.0) as f32)),
                            "double" | "f64" => values.push(FfiValue::F64(val.as_number().unwrap_or(0.0) as f64)),
                            "pointer" | "void*" => values.push(FfiValue::Ptr(val.as_number().unwrap_or(0.0) as usize)),
                            "string" | "char*" => {
                                let s = val_to_string(val);
                                let c_str = CString::new(s).unwrap_or_default();
                                let ptr = c_str.as_ptr() as usize;
                                keep_alive_strs.push(c_str); 
                                values.push(FfiValue::Ptr(ptr));
                            },
                            "bool" => values.push(FfiValue::U8(if is_truthy(val) { 1 } else { 0 })),
                        name => {
                                if FFI_STRUCTS.lock().unwrap().contains_key(name) {
                                    if let DgnValue::Map(m) = val {
                                        let m = m.lock().unwrap();
                                        if let Some(DgnValue::ByteArray(b)) = m.get("__bytes__") {
                                            // COPY the bytes into our FfiValue holder
                                            let data = b.lock().unwrap().clone();
                                            values.push(FfiValue::Struct(data));
                                        } else {
                                            // Fallback: empty struct
                                            values.push(FfiValue::Struct(vec![])); 
                                        }
                                    } else {
                                        values.push(FfiValue::I32(0)); // Error fallback
                                    }
                                } else {
                                    values.push(FfiValue::I32(0));
                                }
                            }
                        }
                    }

                    // 3. Create LibFFI Arguments
                    // Arg::new() borrows the value from our 'values' vector
                    let mut libffi_args = Vec::new();
                    for v in &values {
                        match v {
                            FfiValue::I32(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::I64(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::F32(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::F64(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::Ptr(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::U8(x) => libffi_args.push(Arg::new(x)),
                            FfiValue::Struct(data) => {
                                if data.is_empty() {
                                    // Edge case handling
                                    static EMPTY: u8 = 0;
                                    libffi_args.push(Arg::new(&EMPTY));
                                } else {
                                    libffi_args.push(Arg::new(&data[0]));
                                }
                            }
                        }
                    }

                    // 4. Execute Call
                    unsafe {
                        match ret_type_str.as_str() {
                            "void" => {
                                cif.call::<()>(code_ptr, &libffi_args);
                                Ok(DgnValue::Null)
                            },
                            "int" | "i32" => {
                                let res = cif.call::<i32>(code_ptr, &libffi_args);
                                Ok(DgnValue::Number(res as f64))
                            },
                            "long" | "i64" => {
                                let res = cif.call::<i64>(code_ptr, &libffi_args);
                                Ok(DgnValue::Number(res as f64))
                            },
                            "float" | "f32" => {
                                let res = cif.call::<f32>(code_ptr, &libffi_args);
                                Ok(DgnValue::Number(res as f64))
                            },
                            "double" | "f64" => {
                                let res = cif.call::<f64>(code_ptr, &libffi_args);
                                Ok(DgnValue::Number(res))
                            },
                            "pointer" | "void*" => {
                                let res = cif.call::<usize>(code_ptr, &libffi_args);
                                Ok(DgnValue::Number(res as f64))
                            },
                            "bool" => {
                                let res = cif.call::<u8>(code_ptr, &libffi_args);
                                Ok(DgnValue::Bool(res != 0))
                            },
                            _ => Ok(DgnValue::Null)
                        }
                    }
                }));
            }
             Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
        "struct" => {
               let mut map = HashMap::new();

           #[cfg(not(target_arch = "wasm32"))]
        {    
                map.insert("def".to_string(), native_fn(|args| {
                    let name = val_to_string(args.get(0).unwrap());
                    let fields_list = if let DgnValue::List(l) = args.get(1).unwrap() { l.lock().unwrap().clone() } else { return Err(DgnError::new("Fields must be a list", 0)); };

                    let mut offset = 0;
                    let mut max_align = 1;
                    let mut struct_fields = Vec::new();
                    let mut field_types = Vec::new();

                    for field in fields_list {
                        if let DgnValue::List(def) = field {
                            let def = def.lock().unwrap();
                            let type_name = val_to_string(&def[0]);
                            let field_name = val_to_string(&def[1]);

                            let (ffi_type, size, align) = get_ffi_type_info(&type_name)
                                .ok_or(DgnError::new(&format!("Unknown type: {}", type_name), 0))?;

                            // 1. Add Padding for Alignment
                            let padding = (align - (offset % align)) % align;
                            offset += padding;

                            // 2. Store Field
                            struct_fields.push((field_name, offset, ffi_type.clone()));
                            field_types.push(ffi_type);

                            // 3. Advance Offset
                            offset += size;
                            if align > max_align { max_align = align; }
                        }
                    }

                    // 4. Final Padding (Structure size must be multiple of largest alignment)
                    let final_padding = (max_align - (offset % max_align)) % max_align;
                    let total_size = offset + final_padding;

                    // 5. Create LibFFI Structure Type
                    let struct_type = Type::structure(field_types);

                    let def = FfiStructDef {
                        size: total_size,
                        fields: struct_fields,
                        libffi_type: struct_type,
                    };

                    FFI_STRUCTS.lock().unwrap().insert(name, def);
                    Ok(DgnValue::Number(total_size as f64))
                }));


                map.insert("create".to_string(), native_fn(|args| {
                    let name = val_to_string(args.get(0).unwrap());
                    let structs = FFI_STRUCTS.lock().unwrap();
                    let def = structs.get(&name).ok_or(DgnError::new("Struct not defined", 0))?;

                    // Allocate zeroed memory
                    let bytes = vec![0u8; def.size];
                    
                    // Return a Map acting as the Struct Object
                    // It contains the Raw Bytes + Metadata
                    let mut map1 = HashMap::new();
                    map1.insert("__struct_id__".to_string(), DgnValue::String(name.clone()));
                    map1.insert("__bytes__".to_string(), DgnValue::ByteArray(Arc::new(Mutex::new(bytes))));
                    
                    Ok(DgnValue::Map(Arc::new(Mutex::new(map1))))
                }));

                map.insert("set".to_string(), native_fn(|args| {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let obj = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid struct", 0)); };
                    let struct_name = val_to_string(obj.get("__struct_id__").unwrap());
                    let buffer = if let DgnValue::ByteArray(b) = obj.get("__bytes__").unwrap() { b.clone() } else { return Err(DgnError::new("Invalid bytes", 0)); };
                    
                    let field_name = val_to_string(&args[1]);
                    let val = &args[2];

                    let structs = FFI_STRUCTS.lock().unwrap();
                    let def = structs.get(&struct_name).unwrap();
                    
                    // Find field offset and type
                    let (_, offset, type_info) = def.fields.iter().find(|(n, _, _)| n == &field_name)
                        .ok_or(DgnError::new("Field not found", 0))?;

                    // Write Data
                    let mut b = buffer.lock().unwrap();
                    let offset = *offset;
                    
                    // This is a simplified writer (Supports i32/f32 for now - expand for others)
                    // You can copy the logic from `mem_write_int` here but adapt for floats
                    match val {
                        DgnValue::Number(n) => {
                            let remaining = b.len().saturating_sub(offset);
                            let bytes = (*n as f32).to_ne_bytes();
                        
                            let write_len = std::cmp::min(4, remaining);
                            
                            for i in 0..write_len {
                                b[offset + i] = bytes[i];
                            }
                        },
                        _ => {}
                    }

                    Ok(DgnValue::Null)
                    }));
                    match result {
                        Ok(val) => val,
                        Err(_) => Err(DgnError::new("Dragon: Memory access violation in struct.set", 0))
                    }
                }));

            // [INSERT THIS AFTER struct_set block (~Line 1485)]
            map.insert("get".to_string(), native_fn(|args| {
                let obj = if let DgnValue::Map(m) = &args[0] { m.lock().unwrap() } else { return Err(DgnError::new("Invalid struct", 0)); };
                let struct_name = val_to_string(obj.get("__struct_id__").unwrap());
                let buffer = if let DgnValue::ByteArray(b) = obj.get("__bytes__").unwrap() { b.clone() } else { return Err(DgnError::new("Invalid bytes", 0)); };
                
                let field_name = val_to_string(&args[1]);

                let structs = FFI_STRUCTS.lock().unwrap();
                let def = structs.get(&struct_name).ok_or(DgnError::new("Struct def not found", 0))?;
                
                // Find field offset and type
                let (_, offset, type_info) = def.fields.iter().find(|(n, _, _)| n == &field_name)
                    .ok_or(DgnError::new("Field not found", 0))?;

                let b = buffer.lock().unwrap();
                let offset = *offset;
                
                // Read based on type size (Simplified for basics, expand logic as needed)
                // We look at the 'size' of the field type to decide how to read
                // Note: You might want to match on 'type_info' specifically for cleaner code
                if offset + 4 <= b.len() {
                    let bytes: [u8; 4] = b[offset..offset+4].try_into().unwrap();
                    let f_val = f32::from_ne_bytes(bytes); // Assume float for colors
                    let i_val = u32::from_ne_bytes(bytes);
                    
                    // Heuristic: If field name suggests float, return float, else int
                    // In a perfect world, you'd check type_info. 
                    // For Allegro colors (r,g,b,a are floats), we return float.
                    return Ok(DgnValue::Number(f_val as f64));
                }

                Ok(DgnValue::Null)
            }));

        }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
        },
         "mem" => {
               let mut map = HashMap::new();
            #[cfg(not(target_arch = "wasm32"))]
             {
              map.insert("alloc".to_string(), native_fn(|args| {
                    let size = args.get(0).and_then(|v| if let DgnValue::Number(n) = v { Some(*n as usize) } else { None }).unwrap_or(0);
                    let vec = vec![0u8; size];
                    Ok(DgnValue::ByteArray(Arc::new(Mutex::new(vec))))
                }));

                // Add this inside the "mem" map insertions
map.insert("read_ptr".to_string(), native_fn(|args| {
    let ptr = args.get(0).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
    let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
    
    if ptr == 0 { return Ok(DgnValue::Null); }

    unsafe {
        // Read 8 bytes for a 64-bit pointer
        let target_addr = (ptr + offset) as *const usize;
        let val = *target_addr;
        Ok(DgnValue::Number(val as f64))
    }
}));

map.insert("read_float".to_string(), native_fn(|args| {
    let ptr = args.get(0).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
    let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
    
    if ptr == 0 { return Ok(DgnValue::Null); }

    unsafe {
        // Read 4 bytes as a float (f32)
        let target_addr = (ptr + offset) as *const f32;
        let val = *target_addr;
        Ok(DgnValue::Number(val as f64))
    }
}));

map.insert("read_mem_int".to_string(), native_fn(|args| {
                    let ptr = args.get(0).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                    let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
                    
                    if ptr == 0 { return Ok(DgnValue::Null); }

                    unsafe {
                        // Read 4 bytes as a 32-bit integer from raw memory
                        let target_addr = (ptr + offset) as *const u32;
                        let val = *target_addr;
                        Ok(DgnValue::Number(val as f64))
                    }
                }));

                map.insert("addr".to_string(), native_fn(|args| {
                    match args.get(0) {
                        Some(DgnValue::ByteArray(b)) => {
                            let ptr = b.lock().unwrap().as_ptr() as usize;
                            Ok(DgnValue::Number(ptr as f64))
                        },
                        Some(DgnValue::String(s)) => {
                            let ptr = s.as_ptr() as usize;
                            Ok(DgnValue::Number(ptr as f64))
                        }
                        _ => Ok(DgnValue::Null)
                    }
                }));

                map.insert("read_int".to_string(), native_fn(|args| {
                    let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
                    let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
                    
                    let b = buffer.lock().unwrap();
                    if offset + 4 <= b.len() {
                        let bytes: [u8; 4] = b[offset..offset+4].try_into().unwrap();
                        let val = u32::from_ne_bytes(bytes); 
                        Ok(DgnValue::Number(val as f64))
                    } else {
                        Ok(DgnValue::Null)
                    }
                }));


                map.insert("write_int".to_string(), native_fn(|args| {
                    let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
                    let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
                    let val = if let Some(DgnValue::Number(n)) = args.get(2) { *n as u32 } else { 0 };

                    let mut b = buffer.lock().unwrap();
                    if offset + 4 <= b.len() {
                        let bytes = val.to_ne_bytes();
                        for i in 0..4 { b[offset + i] = bytes[i]; }
                    }
                    Ok(DgnValue::Null)
                }));


                map.insert("read_string".to_string(), native_fn(|args| {
                    let ptr = args.get(0).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
                    
                    if ptr == 0 { return Ok(DgnValue::Null); }

                    unsafe {
                        let c_str = std::ffi::CStr::from_ptr(ptr as *const std::os::raw::c_char);
                        Ok(DgnValue::String(c_str.to_string_lossy().into_owned()))
                    }
                }));

                map.insert("read_byte".to_string(), native_fn(|args| {
                    let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
                    let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
                    
                    let b = buffer.lock().unwrap();
                    if offset < b.len() {
                        Ok(DgnValue::Number(b[offset] as f64))
                    } else {
                        Ok(DgnValue::Null)
                    }
                }));

                map.insert("write_byte".to_string(), native_fn(|args| {
                    let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
                    let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
                    let val = if let Some(DgnValue::Number(n)) = args.get(2) { *n as u8 } else { 0 };

                    let mut b = buffer.lock().unwrap();
                    if offset < b.len() {
                        b[offset] = val;
                    }
                    Ok(DgnValue::Null)
                }));
             }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
         },
         
         "thread" => {
             let mut map = HashMap::new();
             #[cfg(not(target_arch = "wasm32"))]
             {
                    map.insert("spawn".to_string(), native_fn(|args| {
                        let func = args.get(0).cloned().ok_or(DgnError::new("Func required",0))?;
                        let t_args = if let Some(DgnValue::List(l)) = args.get(1) { l.lock().unwrap().clone() } else { vec![] };
                        let handle = thread::spawn(move || { 
                            // Create a runtime for the new thread
                            let rt = tokio::runtime::Runtime::new().unwrap(); 
                            let _ = rt.block_on(call_dgn_func(func, t_args)); 
                        });
                        Ok(DgnValue::Thread(Arc::new(handle)))
                    }));
            }
            Some(DgnValue::Map(Arc::new(Mutex::new(map))))
         }
        // To keep things truly modular, you can move memory, ffi, and struct globals here as well!
        // You just transfer the logic from `populate_builtins` verbatim under an "ffi" and "mem" match block.
        _ => None,
    }
}

pub fn populate_builtins(env: Arc<Mutex<Env>>) {
    let mut e = env.lock().unwrap();
    let gc_env = env.clone();

    e.set("show", native_fn(|args| { 
        let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&s.into()); 
        #[cfg(not(target_arch = "wasm32"))]
        print!("{}", s); 
        Ok(DgnValue::Null) 
    }));

    e.set("showln", native_fn(|args| { 
        let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&s.into());
        #[cfg(not(target_arch = "wasm32"))]
        println!("{}", s); 
        Ok(DgnValue::Null) 
    }));

  e.set("readln", native_fn(|args| {
    use std::io::{self, Write};

    if let Some(prompt) = args.get(0) {
        print!("{}", val_to_string(prompt));
        let _ = io::stdout().flush(); 
    }

    let mut b = String::new();
    match io::stdin().read_line(&mut b) {
        Ok(_) => Ok(DgnValue::String(b.trim_end_matches(|c| c == '\r' || c == '\n').to_string())),
        Err(e) => Err(DgnError::new(&e.to_string(), 0)),
    }
}));
    e.set("exit", native_fn(|args| { let c = if let Some(DgnValue::Number(n)) = args.get(0) { *n as i32 } else { 0 }; std::process::exit(c); }));
    e.set("gc", native_fn(move |_| {
        let count = gc_collect(gc_env.clone());
        Ok(DgnValue::Number(count as f64))
    }));


    e.set("sleep", native_fn(|args| { thread::sleep(Duration::from_secs_f64(if let Some(DgnValue::Number(n)) = args.get(0) { *n } else { 0.0 })); Ok(DgnValue::Null) }));

    e.set("hashmap", native_fn(|_| {
        let val = DgnValue::Map(Arc::new(Mutex::new(HashMap::new())));
        gc_register(&val);
        Ok(val)
    }));
    e.set("list", native_fn(|args| {
        let val = if let Some(DgnValue::List(l)) = args.get(0) { 
            DgnValue::List(l.clone()) 
        } else { 
            DgnValue::List(Arc::new(Mutex::new(Vec::new()))) 
        };
        gc_register(&val);
        Ok(val)
    }));
     e.set("regex", native_fn(|args| { let p = if let Some(DgnValue::String(s)) = args.get(0) { s } else { "" }; match Regex::new(p) { Ok(r) => Ok(DgnValue::Regex(Arc::new(r))), Err(e) => Err(DgnError::new(&e.to_string(), 0)) } }));
    

            e.set("bytes", native_fn(|args| {
            if let Some(DgnValue::List(l)) = args.get(0) {
     
                let list = l.lock().unwrap();
                let mut vec = Vec::with_capacity(list.len());
                for val in list.iter() {
                    if let DgnValue::Number(n) = val {
                        vec.push(*n as u8);
                    } else {
                        vec.push(0);
                    }
                }
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(vec))))
            } else if let Some(DgnValue::String(s)) = args.get(0) {
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(s.as_bytes().to_vec()))))
            } else if let Some(DgnValue::Number(n)) = args.get(0) {
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(vec![0; *n as usize]))))
            } else {
                Ok(DgnValue::ByteArray(Arc::new(Mutex::new(Vec::new()))))
            }
        }));

     e.set("typeof", native_fn(|args| {
        let val = args.get(0).unwrap_or(&DgnValue::Null);
        let type_name = match val {
            DgnValue::Null => "null",
            DgnValue::Bool(_) => "bool",
            DgnValue::Number(_) => "number",
            DgnValue::String(_) => "string",
            DgnValue::List(_) => "list",
            DgnValue::Map(_) => "map",
            DgnValue::ByteArray(_) => "bytes",
            DgnValue::Func(_) => "function",
            DgnValue::Class(_) => "class", 
            DgnValue::Instance(_) => "instance",
            #[cfg(not(any(target_arch = "wasm32")))]
            DgnValue::NativeLib(_) => "nativelib",
            #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
            DgnValue::Socket(_) => "socket",
            DgnValue::Regex(_) => "regex",
            DgnValue::Thread(_) => "thread",
            DgnValue::Promise(_) => "promise",
        };
        Ok(DgnValue::String(type_name.to_string()))
    }));


     e.set("int", native_fn(|args| {
        match args.get(0) {
            Some(DgnValue::Number(n)) => Ok(DgnValue::Number(n.trunc())),
            Some(DgnValue::String(s)) => {
                match s.parse::<f64>() {
                    Ok(n) => Ok(DgnValue::Number(n.trunc())),
                    Err(_) => Ok(DgnValue::Null)
                }
            },
            Some(DgnValue::Bool(b)) => Ok(DgnValue::Number(if *b { 1.0 } else { 0.0 })),
            _ => Ok(DgnValue::Null)
        }
    }));

     e.set("float", native_fn(|args| {
        match args.get(0) {
            Some(DgnValue::Number(n)) => Ok(DgnValue::Number(*n)),
            Some(DgnValue::String(s)) => Ok(DgnValue::Number(s.parse().unwrap_or(0.0))),
            _ => Ok(DgnValue::Null)
        }
    }));

     e.set("str", native_fn(|args| {
        let v = args.get(0).unwrap_or(&DgnValue::Null);
        Ok(DgnValue::String(val_to_string(v)))
    }));

     e.set("len", native_fn(|args| {
        match args.get(0) {
            Some(DgnValue::String(s)) => Ok(DgnValue::Number(s.len() as f64)),
            Some(DgnValue::List(l)) => Ok(DgnValue::Number(l.lock().unwrap().len() as f64)),
            Some(DgnValue::Map(m)) => Ok(DgnValue::Number(m.lock().unwrap().len() as f64)),
            Some(DgnValue::ByteArray(b)) => Ok(DgnValue::Number(b.lock().unwrap().len() as f64)),
            _ => Ok(DgnValue::Number(0.0))
        }
    }));


    e.set("chr", native_fn(|args| {
        if let Some(DgnValue::Number(n)) = args.get(0) {
            if let Some(c) = std::char::from_u32(*n as u32) {
                return Ok(DgnValue::String(c.to_string()));
            }
        }
        Ok(DgnValue::Null)
    }));

    e.set("ord", native_fn(|args| {
        if let Some(DgnValue::String(s)) = args.get(0) {
            if let Some(c) = s.chars().next() {
                return Ok(DgnValue::Number(c as u32 as f64));
            }
        }
        Ok(DgnValue::Null)
    }));
    

    e.set("parse_int", native_fn(|args| {
        let s = args.get(0).map(val_to_string).unwrap_or_default();
        match s.parse::<i64>() {
            Ok(n) => Ok(DgnValue::Number(n as f64)),
            Err(_) => Ok(DgnValue::Null)
        }
    }));

    e.set("parse_float", native_fn(|args| {
        let s = args.get(0).map(val_to_string).unwrap_or_default();
        match s.parse::<f64>() {
            Ok(n) => Ok(DgnValue::Number(n)),
            Err(_) => Ok(DgnValue::Null)
        }
    }));


#[cfg(target_arch = "wasm32")]
    {
        e.set("alert", native_fn(|args| {
            let msg = args.get(0).map(val_to_string).unwrap_or_default();
            web_sys::window().unwrap().alert_with_message(&msg).unwrap();
            Ok(DgnValue::Null)
        }));
        
    }

    e.define_const("PI".to_string(), DgnValue::Number(std::f64::consts::PI)).ok();
    e.define_const("E".to_string(), DgnValue::Number(std::f64::consts::E)).ok();

}


pub fn check_for_bundle(exe_path: &Path) -> Option<String> {
    let mut file = match File::open(exe_path) {
        Ok(f) => f,
        Err(_) => return None,
    };

    let len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return None,
    };

    if len < 24 { return None; }

    if file.seek(SeekFrom::End(-24)).is_err() { return None; }
    let mut footer = [0u8; 24];
    if file.read_exact(&mut footer).is_err() { return None; }

    if &footer[8..24] != BUNDLE_SIGNATURE { return None; }


    let script_len = u64::from_le_bytes(footer[0..8].try_into().unwrap());
    if script_len > len - 24 { return None; }


    if file.seek(SeekFrom::End(-(24 + script_len as i64))).is_err() { return None; }
    let mut script_buf = vec![0u8; script_len as usize];
    if file.read_exact(&mut script_buf).is_err() { return None; }


    let key = get_key(); 
    for (i, b) in script_buf.iter_mut().enumerate() {
        *b ^= key[i % key.len()];
    }

    Some(String::from_utf8_lossy(&script_buf).to_string())
}

pub fn gather_sources(path: &Path, sources: &mut HashMap<String, String>, is_root: bool) -> Result<(), String> {
    let code = fs::read_to_string(path)
        .map_err(|e| format!("Could not read source file '{}': {}", path.display(), e))?;

    let key = if is_root { "__entry__".to_string() } else { path.file_name().unwrap().to_string_lossy().to_string() };
    if sources.contains_key(&key) { return Ok(()); }
    sources.insert(key, code.clone());

    let re = Regex::new(r#"import[ \t]+["']([^"']+)["']"#).unwrap();
    for cap in re.captures_iter(&code) {
        let import_ref = &cap[1];
        
        // // Skip built-ins so the bundler doesn't try to look for them on disk
        // let builtins = ["math", "json", "base64", "file", "os", "path", "time", "net", "ffi", "struct", "mem", "thread", "uuid"];
        // if builtins.contains(&import_ref.as_str()) { continue; }

        let current_dir = path.parent().unwrap_or(Path::new("."));
        let mut search_dirs = vec![current_dir.to_path_buf(), current_dir.join("dgn_modules")];
        if let Ok(dp) = std::env::var("DRAGON_PATH") {
            for p in std::env::split_paths(&dp) { search_dirs.push(p); }
        }

        let mut target_files = Vec::new();
        let path_obj = Path::new(import_ref);
        if path_obj.extension().is_some() {
            target_files.push(path_obj.to_path_buf());
        } else {
            target_files.push(PathBuf::from(format!("{}.dgn", import_ref)));
            if let Some(name) = path_obj.file_name() {
                let n = name.to_string_lossy();
                target_files.push(PathBuf::from(import_ref).join(format!("{}.dgn", n)));
                target_files.push(PathBuf::from(import_ref).join("index.dgn"));
            }
        }

        let mut found_path = None;
        for dir in &search_dirs {
            for target in &target_files {
                let candidate = dir.join(target);
                if candidate.exists() && candidate.is_file() {
                    found_path = Some(candidate);
                    break;
                }
            }
            if found_path.is_some() { break; }
        }

        if let Some(import_path) = found_path {
            gather_sources(&import_path, sources, false)?;
        } else {
            println!("Warning: Bundler skipped missing import '{}'", import_ref);
        }
    }
    Ok(())
}

pub struct LintReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl LintReport {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
    
    pub fn print(&self) {
        for w in &self.warnings { println!("Warning: {}", w); }
        for e in &self.errors { println!("Error: {}", e); }
    }
}

struct SymbolInfo {
    is_used: bool,
    sym_type: String,
}

struct LinterScope {
    variables: HashMap<String, SymbolInfo>,
}

struct Linter {
    scopes: Vec<LinterScope>,
    report: LintReport,
}

impl Linter {
    fn new() -> Self {
        let mut l = Linter {
            scopes: vec![],
            report: LintReport { errors: vec![], warnings: vec![] },
        };
        l.enter_scope();
        let builtins = vec![
            "show", "showln", "readln", "exit", "time_now", "sleep", 
            "hashmap", "list", "regex", "bytes", "typeof", "int", "float", "str", "len",  "chr", "ord", "parse_int", "parse_float", "gc","argv",
         "alert"
        ];
        for b in builtins {
            l.define(b, "built-in function", true);
        }
        l
    }

    fn enter_scope(&mut self) {
        self.scopes.push(LinterScope { variables: HashMap::new() });
    }

    fn exit_scope(&mut self) {
        let is_global = self.scopes.len() == 1;

        if let Some(scope) = self.scopes.pop() {
            if !is_global { 
                for (name, info) in scope.variables {
                    if !info.is_used && !name.starts_with('_') { 
                        self.report.warnings.push(format!("Unused {} '{}'", info.sym_type, name));
                    }
                }
            }
        }
    }

    fn define(&mut self, name: &str, sym_type: &str, marked_used: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            let current_status = scope.variables.get(name).map(|s| s.is_used).unwrap_or(false);
            if marked_used || !current_status {
                scope.variables.insert(name.to_string(), SymbolInfo {
                    is_used: marked_used,
                    sym_type: sym_type.to_string(),
                });
            }
        }
    }

    fn use_identifier(&mut self, name: &str, line: usize, expected_type: &str) {
        for scope in self.scopes.iter_mut().rev() {
            // We use get_mut to modify the SymbolInfo directly instead of inserting a bool
            if let Some(info) = scope.variables.get_mut(name) {
                info.is_used = true; 
                return;
            }
        }
        self.report.errors.push(format!("Undefined {} '{}' at line {}", expected_type, name, line));
    }


    fn check_stmt(&mut self, stmt: &Stmt) {
        match &stmt.node {
            StmtNode::Block(stmts) => {
                self.enter_scope();
                
                // PASS 1: Hoist functions and classes in the linter
                for s in stmts {
                    match &s.node {
                        StmtNode::FuncDef(name, ..) => {
                            if !name.is_empty() { self.define(name, "function", false); }
                        }
                        StmtNode::ClassDef(name, ..) => {
                            self.define(name, "class", false);
                        }
                        _ => {}
                    }
                }

                // PASS 2: Check standard execution flow
                for s in stmts { 
                    self.check_stmt(s); 
                }
                
                self.exit_scope();
            },
            StmtNode::Expr(e) | StmtNode::Return(Some(e)) | StmtNode::Raise(e) => {
                self.check_expr(e);
            },
            StmtNode::IfChain(chain) => {
                for (cond, block) in chain {
                    if let Some(c) = cond { self.check_expr(c); }
                    self.enter_scope(); 
                    self.check_stmt(block);
                    self.exit_scope();
                }
            },
            StmtNode::While(cond, body) | StmtNode::DoWhile(body, cond) => {
                self.check_expr(cond);
                self.enter_scope();
                self.check_stmt(body);
                self.exit_scope();
            },
            StmtNode::For(name, iter, body) => {
                self.check_expr(iter);
                self.enter_scope();
                self.define(name, "variable", false);
                self.check_stmt(body);
                self.exit_scope();
            },
            StmtNode::CFor(init, cond, step, body) => {
                self.enter_scope();
                if let Some(i) = init { self.check_expr(i); }
                if let Some(c) = cond { self.check_expr(c); }
                if let Some(s) = step { self.check_expr(s); }
                self.check_stmt(body);
                self.exit_scope();
            },
            StmtNode::FuncDef(name, params, _req_params, body, _is_async) => {
                self.define(name, "function", false);
                self.enter_scope();
                for p in params { self.define(p, "variable", false); }
                self.check_stmt(body);
                self.exit_scope();
            },
            StmtNode::ClassDef(name, _, body) => {
                self.define(name, "class", false);
                self.enter_scope();
                self.define("this", "keyword", true); 
                self.check_stmt(body);
                self.exit_scope();
            },
            StmtNode::ConstDecl(name, expr) => {
                self.check_expr(expr);
                self.define(name, "constant", false);
            },
                StmtNode::Import(path, as_name) => {
                    // Determine the name the module is bound to (alias, or file stem)
                    let bind = as_name.clone().unwrap_or_else(|| {
                        std::path::Path::new(path).file_stem().unwrap().to_str().unwrap().to_string()
                    });
                    // Define the module in the current scope so the linter knows it exists
                    self.define(&bind, "module", false);
                },
            StmtNode::Switch(expr, cases, default) => {
                self.check_expr(expr);
                for (case_expr, block) in cases {
                    self.check_expr(case_expr);
                    self.enter_scope();
                    self.check_stmt(block);
                    self.exit_scope();
                }
                if let Some(def_block) = default {
                    self.enter_scope();
                    self.check_stmt(def_block);
                    self.exit_scope();
                }
            }
            _ => {}
        }
    }

    fn check_expr(&mut self, expr: &Expr) {
        match &expr.node {
            ExprNode::Var(name) => self.use_identifier(name, expr.line, "variable"),
            ExprNode::Assign(target, val) => {
                self.check_expr(val);

                if let ExprNode::Var(name) = &target.node {

                    let mut found = false;
                    for scope in self.scopes.iter() {
                        if scope.variables.contains_key(name) { found = true; break; }
                    }
                    if !found {

                        self.define(name, "variable", false);
                    } else {

                        self.use_identifier(name, expr.line, "variable");
                    }
                } else {
                    self.check_expr(target);
                }
            },
            ExprNode::Binary(_, l, r) => { self.check_expr(l); self.check_expr(r); },
            ExprNode::Unary(_, e) => self.check_expr(e),
            ExprNode::Call(callee, args) => {
                if let ExprNode::Var(name) = &callee.node {
                    self.use_identifier(name, callee.line, "function");
                } else {
                    self.check_expr(callee);
                }
                for a in args { self.check_expr(a); }
            },
            ExprNode::List(items) => { for i in items { self.check_expr(i); } },
            ExprNode::Obj(props) => { for (_, v) in props { self.check_expr(v); } },
            ExprNode::Index(obj, idx) => { self.check_expr(obj); self.check_expr(idx); },
            ExprNode::GetAttr(obj, _) => self.check_expr(obj),
            ExprNode::FuncExpr(params, _req_params, body) => {
                self.enter_scope();
                for p in params { self.define(p, "variable", false); }
                self.check_stmt(body);
                self.exit_scope();
            },
            ExprNode::New(cls_expr, args) => {
                if let ExprNode::Var(name) = &cls_expr.node {
                    self.use_identifier(name, cls_expr.line, "class");
                } else {
                    self.check_expr(cls_expr);
                }
                for a in args { self.check_expr(a); }
            },
            ExprNode::Super(_, args) => {
                for a in args { self.check_expr(a); }
            },

            _ => {}
        }
    }
}

pub fn lint_ast(ast: &Stmt) -> LintReport {
    let mut linter = Linter::new();
    linter.check_stmt(ast);
    linter.exit_scope(); 
    linter.report
}

pub fn get_key() -> Vec<u8> {

    let mut key = Vec::new();

    key.push(68);  
    key.push(82);  
    key.push(65);  
    key.push(71); 
    key.push(79); 
    key.push(78); 
    key.push(95); 
    key.push(50);  
    key.push(48);  
    key.push(50);  
    key.push(54); 
    
    key.push(10+20);

    key
}