use super::*;

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
    JumpIfNotNull(usize),      // Pop, jump if not null (for `??` operator)
    
    // Functions
    // MakeFunc removed — replaced by MakeClosure
    Call(usize),               // Pop func + N args, call
    Return,                    // Return top of stack

    // Object access
    GetAttr(String),           // obj.name
    GetOptionalAttr(String),   //
    AddExtension(String),
    SetAttr(String),           // obj.name = val
    GetIndex,                  // obj[idx]
    SetIndex,                  // obj[idx] = val

    // Loops
    ForIter(String, Option<String>, usize),     // var_name, exit_ip
    Break,                     // break out of loop
    Continue,                  // continue to loop start
    DoWhileStart,              // marks do-while body start ip (patched after)

    // Functions / closures
    MakeClosure(Option<String>, Vec<(String, Option<String>)>, usize, bool, Arc<Chunk>),// params, is_async, instrs, lines
    MakeClosureRef(String, String), 
    MakeClassRef(String, String, String),
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
            Instruction::Jump(t) 
            | Instruction::JumpIfFalse(t) 
            | Instruction::JumpIfTrue(t) 
            | Instruction::JumpIfNotNull(t)
            | Instruction::PushTry(t) => {
                *t = target;
            }
            Instruction::ForIter(_, _, t) => {
                *t = target;
            }
            _ => {}
        }
    }
}

