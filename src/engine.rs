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

use regex::Regex;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use tiny_http::{Server, Response, Header, Method};
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use libloading::{Library, Symbol};
use lazy_static::lazy_static;

#[cfg(target_arch = "wasm32")]
use web_sys;

#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use libffi::middle::{Cif, Type, CodePtr, Arg};
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use std::ffi::c_void;

lazy_static! {
    static ref GC_STATE: Mutex<GcState> = Mutex::new(GcState::new());
    static ref FFI_SIGNATURES: Mutex<HashMap<String, (String, Vec<String>)>> = Mutex::new(HashMap::new());

    static ref FFI_STRUCTS: Mutex<HashMap<String, FfiStructDef>> = Mutex::new(HashMap::new());
}


struct FfiStructDef {
    size: usize,
    fields: Vec<(String, usize, Type)>, // Name, Offset, LibFFI Type
    libffi_type: Type, // The constructed Type::structure
}
// We need Send/Sync to store Type in a global Mutex, forcing unsafe impl if necessary
// (libffi Types are generally thread-safe as they are read-only descriptors)
unsafe impl Send for FfiStructDef {}
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
    FuncExpr(Vec<String>, Box<Stmt>),
    New(Box<Expr>, Vec<Expr>),
    Super(String, Vec<Expr>),
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
    FuncDef(String, Vec<String>, Box<Stmt>),
    ClassDef(String, Option<String>, Box<Stmt>),
    Import(String, Option<String>),
    ConstDecl(String, Expr),
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
    #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    NativeLib(Arc<Mutex<Library>>),
    Regex(Arc<Regex>),
    Thread(Arc<thread::JoinHandle<()>>), 
   #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    Socket(Arc<Mutex<TcpStream>>),
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
    pub body: Stmt,
    pub closure: Arc<Mutex<Env>>, 
    pub is_native: bool,
    pub native_fn: Option<Arc<dyn Fn(Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync>>,
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
    match val {
        DgnValue::List(l) => {
            let mut state = GC_STATE.lock().unwrap();
            state.objects.push(WeakContainer::List(Arc::downgrade(l)));
            state.allocations += 1;
        },
        DgnValue::Map(m) | DgnValue::Instance(m) => {
            let mut state = GC_STATE.lock().unwrap();
            state.objects.push(WeakContainer::Map(Arc::downgrade(m)));
            state.allocations += 1;
        },
        _ => {}
    }
}

pub fn gc_check_auto(env: Arc<Mutex<Env>>) {
    let should = {
        let state = GC_STATE.lock().unwrap();
        state.allocations >= state.threshold
    };
    
    if should {
        gc_collect(env);
    }
}

pub fn gc_collect(root_env: Arc<Mutex<Env>>) -> usize {
    let mut visited_objs = HashSet::new(); 
    let mut visited_envs = HashSet::new(); 
    
    mark_env(root_env, &mut visited_objs, &mut visited_envs);
    
    let mut state = GC_STATE.lock().unwrap();
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
    "super", "const", "var"
];

pub fn lex(src: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
  let token_spec = vec![
        ("INC", r"^\+\+"), 
        ("DEC", r"^--"), 
        
        ("COMMENT_ML", r"(?s)^/\*.*?\*/"), 
        
        ("COMMENT_SL", r"^//[^\n]*"), 
        ("HEX", r"^0x[0-9a-fA-F]+"),
        ("NUMBER", r"^[0-9]+(\.[0-9]+)?"), 
        
        ("STRING", r#"^"([^"\\]|\\.)*"|^'([^'\\]|\\.)*'"#), 
        ("NAME", r"^[A-Za-z_][A-Za-z0-9_]*"),
        ("OP", r"^(==|!=|<=|>=|=>|<<|>>|[!+\-*/%<>=?:;,&|^~(){}\[\].])"),
        ("NEWLINE", r"^\n"),
        ("SKIP", r"^[ \t\r\u{FEFF}\x00]+"), 
        ("MISMATCH", r"^."),
    ];
    
    let mut pos = 0;
    let mut line = 1;
    let mut col = 0; 

    while pos < src.len() {
        let remainder = &src[pos..];
        let mut matched = false;

        for (name, pat) in &token_spec {
            let re = Regex::new(pat).unwrap();
            if let Some(mat) = re.find(remainder) {
                let val = mat.as_str();
                let len = val.len();
                let token_col = col + 1;

                match *name {
                    "HEX" => {
                        // Strip '0x' and parse as base-16 integer, then cast to f64
                        let clean = val.trim_start_matches("0x");
                        let n = u64::from_str_radix(clean, 16).unwrap_or(0) as f64;
                        tokens.push(Token{typ: TokenType::Number, value: val.to_string(), line, col: token_col, num_val: Some(n)});
                    },
                    "NUMBER" => {
                        let n : f64 = val.parse().unwrap_or(0.0);
                        tokens.push(Token{typ: TokenType::Number, value: val.to_string(), line, col: token_col, num_val: Some(n)});
                    },
                    "STRING" => {
                        let content = &val[1..val.len()-1];
                        let unescaped = content.replace("\\n", "\n").replace("\\t", "\t").replace("\\\"", "\"").replace("\\'", "'");
                        tokens.push(Token{typ: TokenType::String, value: unescaped, line, col: token_col, num_val: None});
                    },
                    "NAME" => {
                        if KEYWORDS.contains(&val) {
                            tokens.push(Token{typ: TokenType::Keyword(val.to_string()), value: val.to_string(), line, col: token_col, num_val: None});
                        } else {
                            tokens.push(Token{typ: TokenType::Name, value: val.to_string(), line, col: token_col, num_val: None});
                        }
                    },
                    "OP" | "INC" | "DEC" => {
                        let t = match val {
                            "++" => TokenType::Inc, "--" => TokenType::Dec,
                            _ => TokenType::Op
                        };
                        tokens.push(Token{typ: t, value: val.to_string(), line, col: token_col, num_val: None});
                    },
                    "NEWLINE" => {
                        line += 1;
                        col = 0;
                        pos += len;
                        matched = true;
                        continue;
                    },
                    "COMMENT_ML" => {
                        line += val.matches('\n').count();
                        col = 0; 
                    },
                    "COMMENT_SL" | "SKIP" => {},
                    "MISMATCH" => {
                        let char_code = val.chars().next().map(|c| c as u32).unwrap_or(0);
                        panic!("Unexpected character '{}' (Code: {}) at {}:{}", val, char_code, line, col);
                    },
                    _ => {}
                }

                if *name != "NEWLINE" {
                    col += len;
                }
                pos += len;
                matched = true;
                break;
            }
        }
        if !matched {
            break; 
        }
    }
    tokens.push(Token{typ: TokenType::Eof, value: "".to_string(), line, col, num_val: None});
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
        if cur == "func" { return self.parse_func(); }
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

    fn parse_func(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("func")?;
        let mut name = String::new();
        if let TokenType::Name = self.current().typ {
            name = self.advance().value;
        }
        self.expect("(")?;
        let mut params = Vec::new();
        if self.current().value != ")" {
            loop {
                params.push(self.expect("NAME")?.value);
                if !self.match_token(",") { break; }
            }
        }
        self.expect(")")?;
        let body = self.parse_block()?;
        Ok(Stmt { node: StmtNode::FuncDef(name, params, Box::new(body)), line })
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
        let node = self.parse_or()?;
        if self.match_token("?") {
            let t = self.parse_expr()?;
            self.expect(":")?;
            let f = self.parse_expr()?;
            return Ok(Expr { node: ExprNode::Ternary(Box::new(node.clone()), Box::new(t), Box::new(f)), line: node.line });
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
        if self.current().typ == TokenType::Op && ["+", "-"].contains(&self.current().value.as_str()) {
            let op = self.advance().value;
            let v = self.parse_unary()?;
            Ok(Expr { node: ExprNode::Unary(op, Box::new(v)), line: 0 })
        } else if self.match_token("new") {
            let mut cls_expr = self.parse_primary()?;
            
            while self.match_token(".") {
                let attr = self.expect("NAME")?.value;
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
                let attr = self.expect("NAME")?.value;
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
                    if self.current().value != ")" {
                        loop {
                            params.push(self.expect("NAME")?.value);
                            if !self.match_token(",") { break; }
                        }
                    }
                    self.expect(")")?;
                    let body = self.parse_block()?;
                    Ok(Expr { node: ExprNode::FuncExpr(params, Box::new(body)), line })
                },
                _ => { self.advance(); Ok(Expr { node: ExprNode::Var(t.value), line }) }
            },
            TokenType::Name => { self.advance(); Ok(Expr { node: ExprNode::Var(t.value), line }) },
            TokenType::Op => {
                if t.value == "(" {
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
       #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        DgnValue::NativeLib(_) => "<NativeLib>".to_string(),
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        DgnValue::Socket(_) => "<Socket>".to_string(),
        DgnValue::Regex(r) => format!("<Regex {}>", r.as_str()),
        DgnValue::Thread(_) => "<Thread>".to_string(),
    }
}

pub fn eval_stmt(stmt: &Stmt, env: Arc<Mutex<Env>>) -> Result<EvalResult, DgnError> {
    if let StmtNode::Block(_) = &stmt.node {
        gc_check_auto(env.clone());
    }
    match &stmt.node {
          StmtNode::ConstDecl(name, expr) => {
            let val = eval_expr(expr, env.clone())?;
            env.lock().unwrap().define_const(name.clone(), val)
                .map_err(|msg| DgnError::new(&msg, stmt.line))?;
            Ok(EvalResult::None)
        },
        StmtNode::Block(stmts) => {
            let block_env = Arc::new(Mutex::new(Env::new(Some(env.clone()), None)));
            gc_check_auto(block_env.clone());
            for s in stmts {
                let r = eval_stmt(s, block_env.clone())?;
                match r {
                    EvalResult::None => continue,
                    _ => return Ok(r),
                }
            }
            Ok(EvalResult::None)
        },
        StmtNode::Expr(e) => {
            eval_expr(e, env)?;
            Ok(EvalResult::None)
        },
        StmtNode::Return(e) => {
            let val = if let Some(expr) = e { eval_expr(expr, env)? } else { DgnValue::Null };
            Ok(EvalResult::Return(val))
        },
        StmtNode::Break => Ok(EvalResult::Break),
        StmtNode::Continue => Ok(EvalResult::Continue),
        StmtNode::Raise(e) => {
            let msg = val_to_string(&eval_expr(e, env)?);
            Err(DgnError::new(&msg, stmt.line))
        },
        StmtNode::IfChain(chain) => {
            for (cond, block) in chain {
                if let Some(c) = cond {
                    if is_truthy(&eval_expr(c, env.clone())?) {
                        return eval_stmt(block, env);
                    }
                } else {
                    return eval_stmt(block, env);
                }
            }
            Ok(EvalResult::None)
        },
        StmtNode::While(cond, body) => {
            while is_truthy(&eval_expr(cond, env.clone())?) {
                let res = eval_stmt(body, env.clone())?;
                match res {
                    EvalResult::Break => break,
                    EvalResult::Return(v) => return Ok(EvalResult::Return(v)),
                    _ => {}
                }
            }
            Ok(EvalResult::None)
        },
        StmtNode::DoWhile(body, cond) => {
            loop {
                let res = eval_stmt(body, env.clone())?;
                match res {
                    EvalResult::Break => break,
                    EvalResult::Return(v) => return Ok(EvalResult::Return(v)),
                    _ => {}
                }
                if !is_truthy(&eval_expr(cond, env.clone())?) { break; }
            }
            Ok(EvalResult::None)
        },
        StmtNode::For(name, iter_expr, body) => {
            let iter = eval_expr(iter_expr, env.clone())?;
            let items = match iter {
                DgnValue::List(l) => l.lock().unwrap().clone(),
                DgnValue::String(s) => s.chars().map(|c| DgnValue::String(c.to_string())).collect(),
                DgnValue::Number(n) => (0..n as i64).map(|i| DgnValue::Number(i as f64)).collect(),
                DgnValue::ByteArray(b) => b.lock().unwrap().iter().map(|byte| DgnValue::Number(*byte as f64)).collect(),
                _ => return Err(DgnError::new("Not iterable", stmt.line)),
            };
            
            for item in items {
                let new_scope = Arc::new(Mutex::new(Env::new(Some(env.clone()), None)));
                new_scope.lock().unwrap().set(name.clone(), item).map_err(|e| DgnError::new(&e, 0))?;
                let res = eval_stmt(body, new_scope)?;
                match res {
                    EvalResult::Break => break,
                    EvalResult::Return(v) => return Ok(EvalResult::Return(v)),
                    _ => {}
                }
            }
            Ok(EvalResult::None)
        },
        StmtNode::CFor(init, cond, step, body) => {
            let loop_env = Arc::new(Mutex::new(Env::new(Some(env.clone()), None)));
            if let Some(i) = init { eval_expr(i, loop_env.clone())?; }
            
            loop {
                if let Some(c) = cond {
                    if !is_truthy(&eval_expr(c, loop_env.clone())?) { break; }
                }
                let res = eval_stmt(body, loop_env.clone())?;
                match res {
                    EvalResult::Break => break,
                    EvalResult::Return(v) => return Ok(EvalResult::Return(v)),
                    _ => {}
                }
                if let Some(s) = step { eval_expr(s, loop_env.clone())?; }
            }
            Ok(EvalResult::None)
        },
        StmtNode::Switch(val_expr, cases, default) => {
            let val = eval_expr(val_expr, env.clone())?;
            let mut matched = false;
            for (case_expr, block) in cases {
                let c_val = eval_expr(case_expr, env.clone())?;
                let eq = match (&val, &c_val) {
                    (DgnValue::Number(a), DgnValue::Number(b)) => (a-b).abs() < f64::EPSILON,
                    (DgnValue::String(a), DgnValue::String(b)) => a == b,
                    (DgnValue::Bool(a), DgnValue::Bool(b)) => a == b,
                    (DgnValue::Null, DgnValue::Null) => true,
                    _ => false 
                };
                if eq {
                    let r = eval_stmt(block, Arc::new(Mutex::new(Env::new(Some(env.clone()), None))))?;
                    if let EvalResult::Break = r { return Ok(EvalResult::None); }
                    if let EvalResult::Return(_) = r { return Ok(r); }
                    matched = true;
                }
            }
            if !matched {
                if let Some(def) = default {
                    let r = eval_stmt(def, Arc::new(Mutex::new(Env::new(Some(env.clone()), None))))?;
                    if let EvalResult::Break = r { return Ok(EvalResult::None); }
                    if let EvalResult::Return(_) = r { return Ok(r); }
                }
            }
            Ok(EvalResult::None)
        },
        StmtNode::Try(try_b, catch_var, catch_b, fin_b) => {
            let res = eval_stmt(try_b, env.clone());
            let mut final_res = Ok(EvalResult::None);

            if let Err(e) = res {
                if let Some(cb) = catch_b {
                    let catch_env = Arc::new(Mutex::new(Env::new(Some(env.clone()), None)));
                    if let Some(v) = catch_var {
                        catch_env.lock().unwrap().set(v.clone(), DgnValue::String(e.msg)).map_err(|e| DgnError::new(&e, 0))?;
                    }
                    final_res = eval_stmt(cb, catch_env);
                } else {
                    final_res = Err(e);
                }
            } else {
                final_res = res;
            }

            if let Some(fb) = fin_b {
                eval_stmt(fb, env.clone())?;
            }
            final_res
        },
        StmtNode::FuncDef(name, params, body) => {
            let func = DgnFunc {
                name: Some(name.clone()),
                params: params.clone(),
                body: *body.clone(),
                closure: env.clone(),
                is_native: false,
                native_fn: None
            };
            if !name.is_empty() {
                env.lock().unwrap().set(name, DgnValue::Func(func)).map_err(|e| DgnError::new(&e, 0))?;
            }
            Ok(EvalResult::None)
        },
        StmtNode::ClassDef(name, parent_name, body) => {
            let mut parent_env = None;
            if let Some(p) = parent_name {
                 if let Some(pv) = env.lock().unwrap().get(p) {
                     if let DgnValue::Class(penv) = pv {
                         parent_env = Some(penv);
                     } else {
                         return Err(DgnError::new("Parent is not a class", stmt.line));
                     }
                 } else {
                     return Err(DgnError::new("Parent class not found", stmt.line));
                 }
            }
            
            let class_env = Arc::new(Mutex::new(Env::new(Some(env.clone()), None)));
            if let Some(pe) = parent_env {
                class_env.lock().unwrap().store.insert("__parent_class__".into(), DgnValue::Class(pe));
            }
            
            if let StmtNode::Block(stmts) = &body.node {
                for s in stmts {
                    eval_stmt(s, class_env.clone())?;
                }
            } else {
                eval_stmt(body, class_env.clone())?;
            }
            env.lock().unwrap().set(name.clone(), DgnValue::Class(class_env)).map_err(|e| DgnError::new(&e, 0))?;
            Ok(EvalResult::None)
        },
          StmtNode::Import(path, as_name) => {
            let root = Env::get_root(env.clone());
            let bundled_src = {
                let r = root.lock().unwrap();
                if let Some(DgnValue::Map(map)) = r.store.get("__bundled_sources__") {
                     let m = map.lock().unwrap();
                     let key1 = path.clone();
                     let key2 = format!("{}.dgn", path);
                     
                     if let Some(DgnValue::String(s)) = m.get(&key1).or(m.get(&key2)) {
                         Some(s.clone())
                     } else { None }
                } else { None }
            };

            let (code, new_env_path) = if let Some(src) = bundled_src {
                (src, PathBuf::from(format!("bundle:///{}", path)))
            } else {
                let current_dir = env.lock().unwrap().current_path.parent().unwrap_or(Path::new(".")).to_path_buf();
                let mut full_path = current_dir.join(path);
                if full_path.extension().is_none() {
                    full_path.set_extension("dgn");
                }
                let canon = full_path.canonicalize()
                    .map_err(|e| DgnError::new(&format!("Import not found: {:?} ({})", full_path, e), stmt.line))?;
                
                let c = fs::read_to_string(&canon).map_err(|e| DgnError::new(&e.to_string(), stmt.line))?;
                (c, canon)
            };

            let cache_key = new_env_path.to_string_lossy().to_string();
            
            let cached_mod = {
                let r = root.lock().unwrap();
                if let Some(DgnValue::Map(modules)) = r.store.get("__modules__") {
                    modules.lock().unwrap().get(&cache_key).cloned()
                } else { None }
            };

            let mod_val = if let Some(m) = cached_mod {
                m
            } else {
                let tokens = lex(&code);
                let mut parser = Parser::new(tokens);
                let module_body = parser.parse().map_err(|e| DgnError::new(&e, stmt.line))?;

                let mod_env = Arc::new(Mutex::new(Env::new(None, Some(new_env_path))));
                populate_builtins(mod_env.clone());
                
                let mod_val_placeholder = DgnValue::Class(mod_env.clone());
                
                {
                    let mut r = root.lock().unwrap();
                    if let Some(DgnValue::Map(modules)) = r.store.get("__modules__") {
                        modules.lock().unwrap().insert(cache_key, mod_val_placeholder.clone());
                    }
                }

                if let StmtNode::Block(stmts) = module_body.node {
                    for s in stmts {
                        eval_stmt(&s, mod_env.clone())?;
                    }
                }
                mod_val_placeholder
            };

            let bind = as_name.clone().unwrap_or_else(|| {
                Path::new(&path).file_stem().unwrap().to_str().unwrap().to_string()
            });
            
            env.lock().unwrap().set(bind, mod_val).map_err(|msg| DgnError::new(&msg, stmt.line))?;
            Ok(EvalResult::None)
        },
    }
}

pub fn eval_expr(expr: &Expr, env: Arc<Mutex<Env>>) -> Result<DgnValue, DgnError> {
    match &expr.node {
        ExprNode::Number(n) => Ok(DgnValue::Number(*n)),
        ExprNode::String(s) => Ok(DgnValue::String(s.clone())),
        ExprNode::Bool(b) => Ok(DgnValue::Bool(*b)),
        ExprNode::Null => Ok(DgnValue::Null),
        ExprNode::This => {
            env.lock().unwrap().get("this").ok_or(DgnError::new("this not found", expr.line))
        },
        
        ExprNode::Super(method_name, args_expr) => {
            let this_val = env.lock().unwrap().get("this")
                .ok_or(DgnError::new("'super' used outside of a class method", 0))?;

            if let DgnValue::Instance(inst) = &this_val {
                let cls_val = inst.lock().unwrap().get("__class__").cloned()
                    .ok_or(DgnError::new("Instance missing class metadata", 0))?;

                if let DgnValue::Class(cls_env) = cls_val {
                    let parent_cls = cls_env.lock().unwrap().get("__parent_class__")
                        .ok_or(DgnError::new("No parent class found for 'super' call", 0))?;

                    if let DgnValue::Class(parent_env) = parent_cls {
                        let method = parent_env.lock().unwrap().get(method_name)
                            .ok_or(DgnError::new(&format!("Method '{}' not found in parent", method_name), 0))?;

                        if let DgnValue::Func(f) = method {
                            let mut args = Vec::new();
                            for a in args_expr { args.push(eval_expr(a, env.clone())?); }
                            return call_dgn_func_impl(f, args, Some(this_val));
                        }
                    }
                }
            }
            Err(DgnError::new("Invalid super call", 0))
        },
        ExprNode::Var(n) => {
            env.lock().unwrap().get(n).ok_or(DgnError::new(&format!("Var '{}' not defined", n), expr.line))
        },
        ExprNode::List(es) => {
            let mut vec = Vec::new();
            for e in es { vec.push(eval_expr(e, env.clone())?); }
            let val = DgnValue::List(Arc::new(Mutex::new(vec)));
            gc_register(&val); 
            Ok(val)
        },
        ExprNode::Obj(entries) => {
            let mut map = HashMap::new();
            for (k, v) in entries {
                map.insert(k.clone(), eval_expr(v, env.clone())?);
            }
            let val = DgnValue::Map(Arc::new(Mutex::new(map)));
            gc_register(&val); 
            Ok(val)
        },
        ExprNode::Assign(target, val_expr) => {
            let val = eval_expr(val_expr, env.clone())?;
            match &target.node {
                ExprNode::Var(name) => {
                    env.lock().unwrap()
                .set(name.clone(), val.clone())
                .map_err(|msg| DgnError::new(&msg, expr.line))?;
                    Ok(val)
                },
                ExprNode::Index(obj, idx) => {
                    let o = eval_expr(obj, env.clone())?;
                    let i = eval_expr(idx, env.clone())?;
                    match o {
                        DgnValue::List(l) => {
                            if let DgnValue::Number(n) = i {
                                let mut arr = l.lock().unwrap();
                                let idx = n as usize;
                                if idx < arr.len() { arr[idx] = val.clone(); }
                            }
                        },
                        DgnValue::Map(m) | DgnValue::Instance(m) => {
                             if let DgnValue::String(k) = i {
                                 m.lock().unwrap().insert(k, val.clone());
                             }
                        },
                        DgnValue::ByteArray(b) => {
                            if let DgnValue::Number(n) = i {
                                let mut arr = b.lock().unwrap();
                                let idx = n as usize;
                                if idx < arr.len() { 
                                    if let DgnValue::Number(byte_val) = val {
                                        arr[idx] = byte_val as u8;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                    Ok(val)
                },
                ExprNode::GetAttr(obj, name) => {
                    let o = eval_expr(obj, env.clone())?;
                    if let DgnValue::Map(m) | DgnValue::Instance(m) = o {
                        m.lock().unwrap().insert(name.clone(), val.clone());
                        Ok(val)
                    } else {
                        Err(DgnError::new("Cannot set attr on this type", expr.line))
                    }
                },
                _ => Err(DgnError::new("Invalid assignment target", expr.line))
            }
        },
        ExprNode::Unary(op, rhs) => {
             let v = eval_expr(rhs, env)?;
             match (op.as_str(), v) {
                 ("-", DgnValue::Number(n)) => Ok(DgnValue::Number(-n)),
                 ("not", x) => Ok(DgnValue::Bool(!is_truthy(&x))),
                 _ => Err(DgnError::new("Invalid unary op", expr.line))
             }
        },
        ExprNode::Binary(op, lhs, rhs) => {
            let l = eval_expr(lhs, env.clone())?;
            if op == "and" { return Ok(if !is_truthy(&l) { l } else { eval_expr(rhs, env)? }); }
            if op == "or" { return Ok(if is_truthy(&l) { l } else { eval_expr(rhs, env)? }); }
            
            let r = eval_expr(rhs, env)?;
            
            match op.as_str() {
                "+" => match (l, r) {
                    (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Number(a+b)),
                    (DgnValue::String(a), b) => Ok(DgnValue::String(a + &val_to_string(&b))),
                    (a, DgnValue::String(b)) => Ok(DgnValue::String(val_to_string(&a) + &b)),
                    (DgnValue::List(a), DgnValue::List(b)) => {
                        let mut new_list = a.lock().unwrap().clone();
                        new_list.extend(b.lock().unwrap().clone());
                        Ok(DgnValue::List(Arc::new(Mutex::new(new_list))))
                    },
                    (DgnValue::ByteArray(a), DgnValue::ByteArray(b)) => {
                        let mut new_bytes = a.lock().unwrap().clone();
                        new_bytes.extend(b.lock().unwrap().clone());
                        Ok(DgnValue::ByteArray(Arc::new(Mutex::new(new_bytes))))
                    }
                    _ => Ok(DgnValue::Null)
                },
                "-" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(a-b)) } else { Ok(DgnValue::Null) },
                "*" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(a*b)) } else { Ok(DgnValue::Null) },
                "/" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(a/b)) } else { Ok(DgnValue::Null) },
                "%" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Number(a%b)) } else { Ok(DgnValue::Null) },
                "<" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Bool(a<b)) } else { Ok(DgnValue::Bool(false)) },
                ">" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Bool(a>b)) } else { Ok(DgnValue::Bool(false)) },
                "<=" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Bool(a<=b)) } else { Ok(DgnValue::Bool(false)) },
                ">=" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l,r) { Ok(DgnValue::Bool(a>=b)) } else { Ok(DgnValue::Bool(false)) },
                "==" => match (l, r) {
                    (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Bool((a-b).abs() < f64::EPSILON)),
                    (DgnValue::String(a), DgnValue::String(b)) => Ok(DgnValue::Bool(a==b)),
                    (DgnValue::Null, DgnValue::Null) => Ok(DgnValue::Bool(true)),
                    _ => Ok(DgnValue::Bool(false)),
                },
                "!=" => match (l, r) { 
                    (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Bool((a-b).abs() >= f64::EPSILON)),
                    (DgnValue::String(a), DgnValue::String(b)) => Ok(DgnValue::Bool(a!=b)),
                    (DgnValue::Null, DgnValue::Null) => Ok(DgnValue::Bool(false)),
                    _ => Ok(DgnValue::Bool(true)),
                },
                "&" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l, r) { 
                    Ok(DgnValue::Number((a as i64 & b as i64) as f64)) 
                } else { Ok(DgnValue::Null) },
                
                "|" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l, r) { 
                    Ok(DgnValue::Number((a as i64 | b as i64) as f64)) 
                } else { Ok(DgnValue::Null) },
                
                "^" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l, r) { 
                    Ok(DgnValue::Number((a as i64 ^ b as i64) as f64)) 
                } else { Ok(DgnValue::Null) },
                
                "<<" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l, r) { 
                    Ok(DgnValue::Number(((a as i64) << (b as i64)) as f64)) 
                } else { Ok(DgnValue::Null) },
                
                ">>" => if let (DgnValue::Number(a), DgnValue::Number(b)) = (l, r) { 
                    Ok(DgnValue::Number(((a as i64) >> (b as i64)) as f64)) 
                } else { Ok(DgnValue::Null) },
                _ => Ok(DgnValue::Null)
            }
        },
        ExprNode::Call(func_expr, args_expr) => {
            let func_val = eval_expr(func_expr, env.clone())?;
            let mut args = Vec::new();
            for a in args_expr { args.push(eval_expr(a, env.clone())?); }
            
            call_dgn_func(func_val, args).map_err(|mut e| { 
                if e.line == 0 { e.line = expr.line; } 
                e 
            })
        },
        ExprNode::GetAttr(obj_expr, name) => {
            let obj = eval_expr(obj_expr, env.clone())?;
            get_attr(obj, name).map_err(|mut e| { e.line = expr.line; e })
        },
        ExprNode::Index(obj, idx) => {
             let o = eval_expr(obj, env.clone())?;
             let i = eval_expr(idx, env.clone())?;
             match (o, i) {
                 (DgnValue::List(l), DgnValue::Number(n)) => {
                     let vec = l.lock().unwrap();
                     let idx = n as usize;
                     if idx < vec.len() { Ok(vec[idx].clone()) } else { Ok(DgnValue::Null) }
                 },
                 (DgnValue::ByteArray(b), DgnValue::Number(n)) => {
                     let vec = b.lock().unwrap();
                     let idx = n as usize;
                     if idx < vec.len() { Ok(DgnValue::Number(vec[idx] as f64)) } else { Ok(DgnValue::Null) }
                 },
                 (DgnValue::Map(m) | DgnValue::Instance(m), DgnValue::String(k)) => {
                     let val = m.lock().unwrap().get(&k).cloned();
                     Ok(val.unwrap_or(DgnValue::Null))
                 },
                 (DgnValue::String(s), DgnValue::Number(n)) => {
                    let idx = n as usize;
                    if idx < s.len() {
                        Ok(DgnValue::String(s.chars().nth(idx).unwrap().to_string()))
                    } else { Ok(DgnValue::Null) }
                 }
                 _ => Ok(DgnValue::Null)
             }
        },
        ExprNode::Ternary(c, t, f) => {
            if is_truthy(&eval_expr(c, env.clone())?) { eval_expr(t, env) } else { eval_expr(f, env) }
        },
        ExprNode::FuncExpr(params, body) => {
             Ok(DgnValue::Func(DgnFunc {
                 name: None, params: params.clone(), body: *body.clone(), closure: env.clone(), is_native: false, native_fn: None
             }))
        },
        ExprNode::New(cls_expr, args) => {

             let cls_val = eval_expr(cls_expr, env.clone())?;
             
             if let DgnValue::Class(cls_env) = cls_val {
                 let instance_map = Arc::new(Mutex::new(HashMap::new()));
                 instance_map.lock().unwrap().insert("__class__".to_string(), DgnValue::Class(cls_env.clone()));
                 
                 let instance = DgnValue::Instance(instance_map);
                 gc_register(&instance);
                 
                 if let Ok(init_func) = get_attr(instance.clone(), "init") {
                     if let DgnValue::Func(_) = init_func {
                        call_dgn_func(init_func, args_expr_to_val(args, env.clone())?).map_err(|mut e| { e.line = expr.line; e })?;
                     }
                 }
                 
                 Ok(instance)
             } else {
                 Err(DgnError::new("Operand is not a class", expr.line))
             }
        },
        ExprNode::PreOp(op, target) => {
             handle_inc_dec(op, target, env, true).map_err(|mut e| { e.line = expr.line; e })
        },
        ExprNode::PostOp(op, target) => {
             handle_inc_dec(op, target, env, false).map_err(|mut e| { e.line = expr.line; e })
        },
    }
}

pub fn args_expr_to_val(args: &[Expr], env: Arc<Mutex<Env>>) -> Result<Vec<DgnValue>, DgnError> {
    let mut v = Vec::new();
    for a in args { v.push(eval_expr(a, env.clone())?); }
    Ok(v)
}

pub fn handle_inc_dec(op: &str, target: &Expr, env: Arc<Mutex<Env>>, is_pre: bool) -> Result<DgnValue, DgnError> {
    match &target.node {
        // Case 1: Simple Variables (i++)
        ExprNode::Var(name) => {
            let cur = env.lock().unwrap().get(name).unwrap_or(DgnValue::Number(0.0));
            if let DgnValue::Number(n) = cur {
                let new_n = if op == "INC" { n + 1.0 } else { n - 1.0 };
                env.lock().unwrap().set(name.clone(), DgnValue::Number(new_n)).map_err(|e| DgnError::new(&e, 0))?;
                Ok(DgnValue::Number(if is_pre { new_n } else { n }))
            } else { Err(DgnError::new("Cannot ++/-- non-number", target.line)) }
        },
        
        // Case 2: Object Properties (this.pc++)
        ExprNode::GetAttr(obj_expr, name) => {
            let obj = eval_expr(obj_expr, env.clone())?;
            // Use existing get_attr helper to resolve value (handles inheritance)
            let current = get_attr(obj.clone(), name)?; 
            
            if let DgnValue::Number(n) = current {
                let new_n = if op == "INC" { n + 1.0 } else { n - 1.0 };
                
                // Write the new value back to the object
                match obj {
                    DgnValue::Instance(map) | DgnValue::Map(map) => {
                        map.lock().unwrap().insert(name.clone(), DgnValue::Number(new_n));
                    },
                    _ => return Err(DgnError::new("Cannot assign property on this type", target.line))
                }
                
                Ok(DgnValue::Number(if is_pre { new_n } else { n }))
            } else {
                Err(DgnError::new("Cannot increment/decrement non-number property", target.line))
            }
        },

        // Case 3: Index/Array Access (arr[0]++)
        ExprNode::Index(obj_expr, idx_expr) => {
            let obj = eval_expr(obj_expr, env.clone())?;
            let idx = eval_expr(idx_expr, env.clone())?;

            match obj {
                DgnValue::List(list) => {
                    if let DgnValue::Number(i) = idx {
                        let i = i as usize;
                        let mut vec = list.lock().unwrap();
                        if i < vec.len() {
                             if let DgnValue::Number(n) = vec[i] {
                                 let new_n = if op == "INC" { n + 1.0 } else { n - 1.0 };
                                 vec[i] = DgnValue::Number(new_n);
                                 Ok(DgnValue::Number(if is_pre { new_n } else { n }))
                             } else {
                                 Err(DgnError::new("Element is not a number", target.line))
                             }
                        } else {
                             Err(DgnError::new("Index out of bounds", target.line))
                        }
                    } else {
                        Err(DgnError::new("List index must be a number", target.line))
                    }
                },
                 DgnValue::Instance(map) | DgnValue::Map(map) => {
                    if let DgnValue::String(k) = idx {
                        let mut m = map.lock().unwrap();
                        let cur = m.get(&k).cloned().unwrap_or(DgnValue::Number(0.0));
                        if let DgnValue::Number(n) = cur {
                            let new_n = if op == "INC" { n + 1.0 } else { n - 1.0 };
                            m.insert(k, DgnValue::Number(new_n));
                            Ok(DgnValue::Number(if is_pre { new_n } else { n }))
                        } else {
                             Err(DgnError::new("Value is not a number", target.line))
                        }
                    } else {
                        Err(DgnError::new("Map key must be string", target.line))
                    }
                },
                _ => Err(DgnError::new("Cannot index this type", target.line))
            }
        },
        _ => Err(DgnError::new("Invalid op target", target.line))
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

        DgnValue::Map(m) => {
            let val = m.lock().unwrap().get(name).cloned();
            if val.is_none() {
                 return match name {
                     "keys" => Ok(native_method(m, |m, _| {
                         let keys: Vec<DgnValue> = m.lock().unwrap().keys().map(|k| DgnValue::String(k.clone())).collect();
                         Ok(DgnValue::List(Arc::new(Mutex::new(keys))))
                     })),
                     "values" => Ok(native_method(m, |m, _| {
                        let vals: Vec<DgnValue> = m.lock().unwrap().values().cloned().collect();
                        Ok(DgnValue::List(Arc::new(Mutex::new(vals))))
                    })),
                    "contains" => Ok(native_method(m, |m, args| {
                        let key = args.get(0).map(val_to_string).unwrap_or_default();
                        Ok(DgnValue::Bool(m.lock().unwrap().contains_key(&key)))
                     })),
                     "set" => Ok(native_method(m, |m, args| {
                         if args.len() == 2 {
                             if let DgnValue::String(k) = &args[0] {
                                 m.lock().unwrap().insert(k.clone(), args[1].clone());
                             }
                         }
                         Ok(DgnValue::Map(m))
                     })),
                     "get" => Ok(native_method(m, |m, args| {
                            let key = args.get(0).map(val_to_string).unwrap_or_default();
                            Ok(m.lock().unwrap().get(&key).cloned().unwrap_or(DgnValue::Null))
                        })),
                     "remove" => Ok(native_method(m, |m, args| {
                        if let Some(DgnValue::String(k)) = args.get(0) {
                            m.lock().unwrap().remove(k);
                        }
                        Ok(DgnValue::Map(m))
                    })),
                     "clear" => Ok(native_method(m, |m, _| {
                        m.lock().unwrap().clear();
                        Ok(DgnValue::Map(m))
                     })),
                     "length" => Ok(DgnValue::Number(m.lock().unwrap().len() as f64)),
                     _ => Ok(DgnValue::Null)
                 };
            }
            Ok(val.unwrap())
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
                             name: f.name.clone(), params: f.params.clone(), body: Stmt { node: StmtNode::Block(vec![]), line: 0 },
                             closure: f.closure.clone(),
                             is_native: true,
                             native_fn: Some(Arc::new(move |args| {
                                 call_dgn_func_impl(f_clone.clone(), args, Some(DgnValue::Instance(inst_capture.clone())))
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
                    for item in old_vec {
                        new_vec.push(call_dgn_func(func.clone(), vec![item])?);
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(new_vec))))
                })),
                "filter" => Ok(native_method(l, |l, args| {
                    let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                    let old_vec = l.lock().unwrap().clone();
                    let mut new_vec = Vec::new();
                    for item in old_vec {
                        if is_truthy(&call_dgn_func(func.clone(), vec![item.clone()])?) {
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
       #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
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
        name: Some("native_method".into()), params: vec![], body: Stmt { node: StmtNode::Block(vec![]), line: 0 },
        closure: Arc::new(Mutex::new(Env::new(None, None))), is_native: true,
        native_fn: Some(Arc::new(move |args| f(target.clone(), args)))
    })
}

pub fn native_fn(f: impl Fn(Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync + 'static) -> DgnValue {
    DgnValue::Func(DgnFunc {
        name: Some("native".into()), params: vec![], body: Stmt { node: StmtNode::Block(vec![]), line: 0 },
        closure: Arc::new(Mutex::new(Env::new(None, None))), is_native: true,
        native_fn: Some(Arc::new(f))
    })
}

pub fn call_dgn_func(func: DgnValue, args: Vec<DgnValue>) -> Result<DgnValue, DgnError> {
    if let DgnValue::Func(f) = func {
        if f.is_native {
             return (f.native_fn.unwrap())(args);
        }
        call_dgn_func_impl(f, args, None)
    } else {
        Err(DgnError::new("Not a function", 0))
    }
}

pub fn call_dgn_func_impl(f: DgnFunc, args: Vec<DgnValue>, this_ctx: Option<DgnValue>) -> Result<DgnValue, DgnError> {

    let scope = Arc::new(Mutex::new(Env::new(Some(f.closure.clone()), None)));
    
    for (i, param) in f.params.iter().enumerate() {
        let v = args.get(i).cloned().unwrap_or(DgnValue::Null);
        scope.lock().unwrap().store.insert(param.clone(), v);
    }
    
   
    if let Some(th) = this_ctx {

        scope.lock().unwrap().store.insert("this".to_string(), th);
    }
    
    match eval_stmt(&f.body, scope) {
        Ok(EvalResult::Return(v)) => Ok(v),
        Ok(_) => Ok(DgnValue::Null),
        Err(e) => Err(e),
    }
}

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

    e.set("readln", native_fn(|_| { let mut b = String::new(); std::io::stdin().read_line(&mut b).unwrap(); Ok(DgnValue::String(b.trim().to_string())) }));
    e.set("exit", native_fn(|args| { let c = if let Some(DgnValue::Number(n)) = args.get(0) { *n as i32 } else { 0 }; std::process::exit(c); }));
    e.set("gc", native_fn(move |_| {
        let count = gc_collect(gc_env.clone());
        Ok(DgnValue::Number(count as f64))
    }));

    e.set("time_now", native_fn(|_| { Ok(DgnValue::Number(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64())) }));
    e.set("sleep", native_fn(|args| { thread::sleep(Duration::from_secs_f64(if let Some(DgnValue::Number(n)) = args.get(0) { *n } else { 0.0 })); Ok(DgnValue::Null) }));
    e.set("random", native_fn(|_| Ok(DgnValue::Number(rand::random::<f64>()))));

    e.set("range", native_fn(|args| {
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
    
    // e.set("bytes", native_fn(|args| {
    //     if let Some(DgnValue::String(s)) = args.get(0) {
    //          Ok(DgnValue::ByteArray(Arc::new(Mutex::new(s.as_bytes().to_vec()))))
    //     } else if let Some(DgnValue::Number(n)) = args.get(0) {
    //          Ok(DgnValue::ByteArray(Arc::new(Mutex::new(vec![0; *n as usize]))))
    //     } else {
    //          Ok(DgnValue::ByteArray(Arc::new(Mutex::new(Vec::new()))))
    //     }
    // }));

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

    e.set("math_sin", native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.sin() } else { 0.0 }))));
    e.set("math_cos", native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.cos() } else { 0.0 }))));
    e.set("math_sqrt", native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.sqrt() } else { 0.0 }))));

#[cfg(not(target_arch = "wasm32"))]
    {
    e.set("file_read_all", native_fn(|a| {
         let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("File path required",0)) };
         match fs::read_to_string(p) { Ok(c) => Ok(DgnValue::String(c)), Err(_) => Ok(DgnValue::Null) }
    }));

    e.set("file_read_bytes", native_fn(|a| {
         let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("File path required",0)) };
         match fs::read(p) { 
             Ok(c) => Ok(DgnValue::ByteArray(Arc::new(Mutex::new(c)))), 
             Err(_) => Ok(DgnValue::Null) 
         }
    }));

    e.set("file_write", native_fn(|a| {
        let p = if let Some(DgnValue::String(s)) = a.get(0) { s } else { return Err(DgnError::new("Path required",0)) };
        let c = if let Some(DgnValue::String(s)) = a.get(1) { s } else { "" };
        Ok(DgnValue::Bool(fs::write(p, c).is_ok()))
    }));
    }

    e.set("json_encode", native_fn(|a| {
         let v = a.get(0).unwrap_or(&DgnValue::Null);
         Ok(DgnValue::String(serde_json::to_string(&val_to_string(v)).unwrap()))
    }));
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
    e.set("start_server", native_fn(|args| {
        let port = if let Some(DgnValue::Number(n)) = args.get(0) { *n as u16 } else { 8080 };
        let handler = args.get(1).cloned().ok_or(DgnError::new("Handler required", 0))?;
        
        thread::spawn(move || {
            let server = match Server::http(format!("0.0.0.0:{}", port)) {
                Ok(s) => s,
                Err(e) => { println!("Failed to start server: {}", e); return; }
            };
            
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

                 let _ = call_dgn_func(handler.clone(), vec![req_obj, DgnValue::Map(res_map)]);
            }
        });
        Ok(DgnValue::Bool(true))
    }));


    e.set("thread_spawn", native_fn(|args| {
        let func = args.get(0).cloned().ok_or(DgnError::new("Func required",0))?;
        let t_args = if let Some(DgnValue::List(l)) = args.get(1) { l.lock().unwrap().clone() } else { vec![] };
        let handle = thread::spawn(move || { let _ = call_dgn_func(func, t_args); });
        Ok(DgnValue::Thread(Arc::new(handle)))
    }));


    e.set("system", native_fn(|args| {
         let cmd = if let Some(DgnValue::String(s)) = args.get(0) { s } else { "" };
         if cfg!(target_os = "windows") {
             Command::new("cmd").args(["/C", cmd]).status().map(|s| DgnValue::Number(s.code().unwrap_or(0) as f64)).map_err(|e| DgnError::new(&e.to_string(),0))
         } else {
             Command::new("sh").args(["-c", cmd]).status().map(|s| DgnValue::Number(s.code().unwrap_or(0) as f64)).map_err(|e| DgnError::new(&e.to_string(),0))
         }
    }));
    

    e.set("loadlib", native_fn(|args| {
        let path = if let Some(DgnValue::String(s)) = args.get(0) { s } else { return Err(DgnError::new("Path req", 0)) };
        unsafe {
            let lib = Library::new(path).map_err(|e| DgnError::new(&e.to_string(), 0))?;
            
            let mut map = HashMap::new();
            map.insert("path".to_string(), DgnValue::String(path.clone()));
            map.insert("ptr".to_string(), DgnValue::NativeLib(Arc::new(Mutex::new(lib))));
            
            Ok(DgnValue::Map(Arc::new(Mutex::new(map))))
        }
    }));

    e.set("ffi_struct", native_fn(|args| {
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


    e.set("struct_new", native_fn(|args| {
        let name = val_to_string(args.get(0).unwrap());
        let structs = FFI_STRUCTS.lock().unwrap();
        let def = structs.get(&name).ok_or(DgnError::new("Struct not defined", 0))?;

        // Allocate zeroed memory
        let bytes = vec![0u8; def.size];
        
        // Return a Map acting as the Struct Object
        // It contains the Raw Bytes + Metadata
        let mut map = HashMap::new();
        map.insert("__struct_id__".to_string(), DgnValue::String(name.clone()));
        map.insert("__bytes__".to_string(), DgnValue::ByteArray(Arc::new(Mutex::new(bytes))));
        
        Ok(DgnValue::Map(Arc::new(Mutex::new(map))))
    }));

    e.set("struct_set", native_fn(|args| {
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
                 // Simple Heuristic: If field is float, write f32/f64 bytes. If int, write i32 bytes.
                 // In a real engine, check `type_info` structure to know exact write type.
                 let bytes = (*n as f32).to_ne_bytes(); // Assuming float for Allegro colors
                 for i in 0..4 { b[offset + i] = bytes[i]; }
            },
            _ => {}
        }

        Ok(DgnValue::Null)
    }));

    // [INSERT THIS AFTER struct_set block (~Line 1485)]
    e.set("struct_get", native_fn(|args| {
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

    e.set("ffi_def", native_fn(|args| {
        if args.len() < 4 { return Err(DgnError::new("Usage: ffi_def(lib, name, ret_type, [arg_types])", 0)); }
        
        // Extract library path from the map we created in loadlib
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
    
    e.set("loadlib_call", native_fn(|args| {
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
    }

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
            #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
            DgnValue::NativeLib(_) => "nativelib",
            #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
            DgnValue::Socket(_) => "socket",
            DgnValue::Regex(_) => "regex",
            DgnValue::Thread(_) => "thread",
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

     e.set("math_abs", native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.abs() } else { 0.0 }))));
     e.set("math_floor", native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.floor() } else { 0.0 }))));
     e.set("math_ceil", native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.ceil() } else { 0.0 }))));
     e.set("math_round", native_fn(|a| Ok(DgnValue::Number(if let Some(DgnValue::Number(n)) = a.get(0) { n.round() } else { 0.0 }))));
     e.set("math_min", native_fn(|args| {
        let mut min = std::f64::INFINITY;
        for arg in args {
            if let DgnValue::Number(n) = arg { if n < min { min = n; } }
        }
        Ok(DgnValue::Number(min))
    }));
     e.set("math_max", native_fn(|args| {
        let mut max = std::f64::NEG_INFINITY;
        for arg in args {
            if let DgnValue::Number(n) = arg { if n > max { max = n; } }
        }
        Ok(DgnValue::Number(max))
    }));

     e.set("env_get", native_fn(|args| {
        if let Some(DgnValue::String(k)) = args.get(0) {
            Ok(DgnValue::String(env::var(k).unwrap_or_default()))
        } else {
            Ok(DgnValue::Null)
        }
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

    e.set("time", native_fn(|_| {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
        Ok(DgnValue::Number(t))
    }));

    e.set("date", native_fn(|args| {
        let now = std::time::SystemTime::now();
        let elapsed = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        
        let days = elapsed / 86400;
        let year = 1970 + (days / 365); 
        
        let mut map = HashMap::new();
        map.insert("timestamp".to_string(), DgnValue::Number(elapsed as f64));
        map.insert("approx_year".to_string(), DgnValue::Number(year as f64));
        
        Ok(DgnValue::Map(Arc::new(Mutex::new(map))))
    }));
#[cfg(not(target_arch = "wasm32"))]
    {
    e.set("file_exists", native_fn(|args| {
        let path = args.get(0).map(val_to_string).unwrap_or_default();
        Ok(DgnValue::Bool(Path::new(&path).exists()))
    }));

    e.set("file_delete", native_fn(|args| {
        let path = args.get(0).map(val_to_string).unwrap_or_default();
        Ok(DgnValue::Bool(fs::remove_file(path).is_ok()))
    }));
    
    e.set("dir_list", native_fn(|args| {
        let path = args.get(0).map(val_to_string).unwrap_or_default();
        let mut files = Vec::new();
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries {
                if let Ok(e) = entry {
                    if let Ok(name) = e.file_name().into_string() {
                        files.push(DgnValue::String(name));
                    }
                }
            }
        }
        Ok(DgnValue::List(Arc::new(Mutex::new(files))))
    }));
    }

    e.set("math_pow", native_fn(|args| {
        let base = if let Some(DgnValue::Number(n)) = args.get(0) { *n } else { 0.0 };
        let exp = if let Some(DgnValue::Number(n)) = args.get(1) { *n } else { 1.0 };
        Ok(DgnValue::Number(base.powf(exp)))
    }));
    
    e.set("math_log", native_fn(|args| {
        let n = if let Some(DgnValue::Number(n)) = args.get(0) { *n } else { 0.0 };
        Ok(DgnValue::Number(n.ln()))
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

    e.set("date_now", native_fn(|_| {
        let now: DateTime<Local> = Local::now();
        let mut map = HashMap::new();
        map.insert("year".to_string(), DgnValue::Number(now.year() as f64));
        map.insert("month".to_string(), DgnValue::Number(now.month() as f64));
        map.insert("day".to_string(), DgnValue::Number(now.day() as f64));
        map.insert("hour".to_string(), DgnValue::Number(now.hour() as f64));
        map.insert("minute".to_string(), DgnValue::Number(now.minute() as f64));
        map.insert("second".to_string(), DgnValue::Number(now.second() as f64));
        map.insert("str".to_string(), DgnValue::String(now.format("%Y-%m-%d %H:%M:%S").to_string()));
        Ok(DgnValue::Map(Arc::new(Mutex::new(map))))
    }));
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    {
    e.set("os_name", native_fn(|_| {
        Ok(DgnValue::String(std::env::consts::OS.to_string()))
    }));

    e.set("os_arch", native_fn(|_| {
        Ok(DgnValue::String(std::env::consts::ARCH.to_string()))
    }));

    e.set("os_family", native_fn(|_| {
        Ok(DgnValue::String(std::env::consts::FAMILY.to_string()))
    }));

    e.set("mem_alloc", native_fn(|args| {
        let size = args.get(0).and_then(|v| if let DgnValue::Number(n) = v { Some(*n as usize) } else { None }).unwrap_or(0);
        let vec = vec![0u8; size];
        Ok(DgnValue::ByteArray(Arc::new(Mutex::new(vec))))
    }));

    e.set("mem_addr", native_fn(|args| {
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

    e.set("mem_read_int", native_fn(|args| {
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


    e.set("mem_write_int", native_fn(|args| {
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


    e.set("mem_read_string", native_fn(|args| {
        let ptr = args.get(0).and_then(|v| v.as_number()).unwrap_or(0.0) as usize;
        
        if ptr == 0 { return Ok(DgnValue::Null); }

        unsafe {
            let c_str = std::ffi::CStr::from_ptr(ptr as *const std::os::raw::c_char);
            Ok(DgnValue::String(c_str.to_string_lossy().into_owned()))
        }
    }));

    e.set("mem_read_byte", native_fn(|args| {
        let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
        let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
        
        let b = buffer.lock().unwrap();
        if offset < b.len() {
            Ok(DgnValue::Number(b[offset] as f64))
        } else {
            Ok(DgnValue::Null)
        }
    }));

    e.set("mem_write_byte", native_fn(|args| {
        let buffer = if let Some(DgnValue::ByteArray(b)) = args.get(0) { b } else { return Ok(DgnValue::Null); };
        let offset = if let Some(DgnValue::Number(n)) = args.get(1) { *n as usize } else { 0 };
        let val = if let Some(DgnValue::Number(n)) = args.get(2) { *n as u8 } else { 0 };

        let mut b = buffer.lock().unwrap();
        if offset < b.len() {
            b[offset] = val;
        }
        Ok(DgnValue::Null)
    }));

    e.set("ffi_symbol_addr", native_fn(|args| {
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

   e.set("ffi_call", native_fn(|args| {
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


    e.set("http_get", native_fn(|args| {
        let url = args.get(0).map(val_to_string).ok_or(DgnError::new("URL required", 0))?;
        
        match ureq::get(&url).call() {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.into_string().unwrap_or_default();
                
                let mut map = HashMap::new();
                map.insert("status".to_string(), DgnValue::Number(status as f64));
                map.insert("body".to_string(), DgnValue::String(text));
                Ok(DgnValue::Map(Arc::new(Mutex::new(map))))
            },
            Err(e) => Err(DgnError::new(&format!("HTTP Request failed: {}", e), 0))
        }
    }));


    e.set("http_post", native_fn(|args| {
        let url = args.get(0).map(val_to_string).ok_or(DgnError::new("URL required", 0))?;
        let body = args.get(1).map(val_to_string).unwrap_or_default();
        
        let mut req = ureq::post(&url);

        if let Some(DgnValue::Map(headers)) = args.get(2) {
            for (k, v) in headers.lock().unwrap().iter() {
                req = req.set(k, &val_to_string(v));
            }
        }

        match req.send_string(&body) {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.into_string().unwrap_or_default();
                let mut map = HashMap::new();
                map.insert("status".to_string(), DgnValue::Number(status as f64));
                map.insert("body".to_string(), DgnValue::String(text));
                Ok(DgnValue::Map(Arc::new(Mutex::new(map))))
            },
            Err(e) => Err(DgnError::new(&format!("HTTP Request failed: {}", e), 0))
        }
    }));


    e.set("socket_connect", native_fn(|args| {
        let addr = args.get(0).map(val_to_string).ok_or(DgnError::new("Address required", 0))?;
        match TcpStream::connect(&addr) {
            Ok(stream) => Ok(DgnValue::Socket(Arc::new(Mutex::new(stream)))),
            Err(e) => Err(DgnError::new(&format!("Connection failed: {}", e), 0))
        }
    }));

    e.set("socket_send", native_fn(|args| {
        if let Some(DgnValue::Socket(s)) = args.get(0) {
            let msg = args.get(1).map(val_to_string).unwrap_or_default();
            let mut stream = s.lock().unwrap();
            match stream.write_all(msg.as_bytes()) {
                Ok(_) => Ok(DgnValue::Bool(true)),
                Err(_) => Ok(DgnValue::Bool(false))
            }
        } else {
            Err(DgnError::new("Socket required", 0))
        }
    }));

    e.set("socket_read", native_fn(|args| {
        if let Some(DgnValue::Socket(s)) = args.get(0) {
            let mut stream = s.lock().unwrap();
            let mut reader = BufReader::new(&*stream);
            let mut buffer = String::new();
            match reader.read_line(&mut buffer) {
                Ok(_) => Ok(DgnValue::String(buffer)),
                Err(_) => Ok(DgnValue::Null)
            }
        } else {
            Err(DgnError::new("Socket required", 0))
        }
    }));
    
    e.set("socket_close", native_fn(|args| {
        if let Some(DgnValue::Socket(s)) = args.get(0) {
            let _ = s.lock().unwrap().shutdown(Shutdown::Both);
            Ok(DgnValue::Bool(true))
        } else { Ok(DgnValue::Bool(false)) }
    }));

    e.set("socket_server", native_fn(|args| {
        let port = args.get(0).and_then(|v| v.as_number()).unwrap_or(8080.0) as u16;
        let handler = args.get(1).cloned().ok_or(DgnError::new("Handler function required", 0))?;
        
        let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
            .map_err(|e| DgnError::new(&e.to_string(), 0))?;
            
        thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(s) = stream {
                    let client_socket = DgnValue::Socket(Arc::new(Mutex::new(s)));
                    let _ = call_dgn_func(handler.clone(), vec![client_socket]);
                }
            }
        });

        Ok(DgnValue::Bool(true))
    }));


   
}

    e.set("json_decode", native_fn(|args| {
        let json_str = args.get(0).map(val_to_string).unwrap_or_default();
        fn json_to_dgn(v: serde_json::Value) -> DgnValue {
            match v {
                serde_json::Value::Null => DgnValue::Null,
                serde_json::Value::Bool(b) => DgnValue::Bool(b),
                serde_json::Value::Number(n) => DgnValue::Number(n.as_f64().unwrap_or(0.0)),
                serde_json::Value::String(s) => DgnValue::String(s),
                serde_json::Value::Array(arr) => {
                    let list: Vec<DgnValue> = arr.into_iter().map(json_to_dgn).collect();
                    DgnValue::List(Arc::new(Mutex::new(list)))
                },
                serde_json::Value::Object(map) => {
                    let mut dgn_map = HashMap::new();
                    for (k, v) in map { dgn_map.insert(k, json_to_dgn(v)); }
                    DgnValue::Map(Arc::new(Mutex::new(dgn_map)))
                }
            }
        }

        match serde_json::from_str(&json_str) {
            Ok(val) => Ok(json_to_dgn(val)),
            Err(_) => Ok(DgnValue::Null)
        }
    }));

 e.set("base64_encode", native_fn(|args| {
        use base64::{Engine as _, engine::general_purpose};
        let s = args.get(0).map(val_to_string).unwrap_or_default();
        Ok(DgnValue::String(general_purpose::STANDARD.encode(s)))
    }));

    e.set("base64_decode", native_fn(|args| {
        use base64::{Engine as _, engine::general_purpose};
        let s = args.get(0).map(val_to_string).unwrap_or_default();
        match general_purpose::STANDARD.decode(s) {
            Ok(bytes) => Ok(DgnValue::String(String::from_utf8_lossy(&bytes).to_string())),
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
        
        e.set("os_name", native_fn(|_| Ok(DgnValue::String("web-wasm".into()))));
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

    let key = if is_root {
        "__entry__".to_string()
    } else {
        path.file_name().unwrap().to_string_lossy().to_string()
    };

    if sources.contains_key(&key) { return Ok(()); }
    sources.insert(key, code.clone());

    // let re = Regex::new(r#"import\s+["']([^"']+)["']"#).unwrap();
    let re = Regex::new(r#"import[ \t]+["']([^"']+)["']"#).unwrap();
    for cap in re.captures_iter(&code) {
        let import_ref = &cap[1];
        let mut import_path = path.parent().unwrap().join(import_ref);
        if import_path.extension().is_none() {
            import_path.set_extension("dgn");
        }
        
        if import_path.exists() {
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

struct LinterScope {
    variables: HashMap<String, bool>,
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
            "show", "showln", "readln", "exit", "time_now", "sleep", "random",
            "hashmap", "list", "regex", "bytes", "math_sin", "math_cos", "math_sqrt",
            "file_read_all", "file_write", "json_encode", "start_server", "thread_spawn",
            "system", "loadlib", "loadlib_call", "typeof", "int", "float", "str",
            "math_abs", "math_floor", "math_ceil", "math_round", "math_min", "math_max",
            "env_get", "len", "time", "date", "date_now", "file_exists", "file_delete",
            "dir_list", "math_pow", "math_log", "chr", "ord", "parse_int", "parse_float",
            "os_name", "os_arch", "os_family", "ffi_call","ffi_def","ffi_symbol_addr","argv","mem_alloc", "mem_addr", "mem_read_int", "mem_write_int", "range", "gc",
            "http_get", "http_post", "socket_connect", "socket_send", "socket_read", "socket_close", "socket_server","json_decode", "base64_encode", "base64_decode", "alert", "mem_read_string","mem_write_byte","mem_read_byte" , "file_read_bytes", "ffi_struct", "struct_new", "struct_set", "struct_get"
        ];
        for b in builtins {
            l.define(b, true); 
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
                for (name, used) in scope.variables {
                    if !used && !name.starts_with('_') { 
                        self.report.warnings.push(format!("Unused variable '{}'", name));
                    }
                }
            }
        }
    }

    fn define(&mut self, name: &str, marked_used: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.variables.insert(name.to_string(), marked_used);
        }
    }

    fn use_var(&mut self, name: &str, line: usize) {
        for scope in self.scopes.iter_mut().rev() {
            if scope.variables.contains_key(name) {
                scope.variables.insert(name.to_string(), true); 
                return;
            }
        }
        self.report.errors.push(format!("Undefined variable '{}' at line {}", name, line));
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match &stmt.node {
            StmtNode::Block(stmts) => {
                self.enter_scope();
                for s in stmts { self.check_stmt(s); }
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
                self.define(name, false); 
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
            StmtNode::FuncDef(name, params, body) => {
                self.define(name, false);
                self.enter_scope();
                for p in params { self.define(p, false); }
                self.check_stmt(body);
                self.exit_scope();
            },
            StmtNode::ClassDef(name, _, body) => {
                self.define(name, false);
                self.enter_scope();
                self.define("this", true); 
                self.check_stmt(body);
                self.exit_scope();
            },
            StmtNode::ConstDecl(name, expr) => {
                self.check_expr(expr);
                self.define(name, false);
            },
            StmtNode::Import(_, as_name) => {
                if let Some(n) = as_name {
                    self.define(n, false);
                }
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
            ExprNode::Var(name) => self.use_var(name, expr.line),
            ExprNode::Assign(target, val) => {
                self.check_expr(val);

                if let ExprNode::Var(name) = &target.node {

                    let mut found = false;
                    for scope in self.scopes.iter() {
                        if scope.variables.contains_key(name) { found = true; break; }
                    }
                    if !found {

                        self.define(name, false);
                    } else {

                        self.use_var(name, expr.line);
                    }
                } else {
                    self.check_expr(target);
                }
            },
            ExprNode::Binary(_, l, r) => { self.check_expr(l); self.check_expr(r); },
            ExprNode::Unary(_, e) => self.check_expr(e),
            ExprNode::Call(callee, args) => {
                self.check_expr(callee);
                for a in args { self.check_expr(a); }
            },
            ExprNode::List(items) => { for i in items { self.check_expr(i); } },
            ExprNode::Obj(props) => { for (_, v) in props { self.check_expr(v); } },
            ExprNode::Index(obj, idx) => { self.check_expr(obj); self.check_expr(idx); },
            ExprNode::GetAttr(obj, _) => self.check_expr(obj),
            ExprNode::FuncExpr(params, body) => {
                self.enter_scope();
                for p in params { self.define(p, false); }
                self.check_stmt(body);
                self.exit_scope();
            },
            ExprNode::New(cls_expr, args) => {
                self.check_expr(cls_expr);
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