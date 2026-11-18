use super::*;

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or, NullCoalesce,
    BitAnd, BitOr, BitXor, Shl, Shr,
}

#[derive(Debug, Clone)]
pub enum Instruction {
    LoadConst(DgnValue),
    LoadVar(String),
    StoreVar(String),
    DefineConst(String),
    Pop,

    BinaryOp(BinOp),
    UnaryOp(String),

    MakeList(usize),
    MakeMap(usize),

    Jump(usize),
    JumpIfFalse(usize),
    JumpIfTrue(usize),
    JumpIfNotNull(usize),

    Call(usize),
    Return,

    GetAttr(String),
    GetOptionalAttr(String),
    AddExtension(String),
    SetAttr(String),
    GetIndex,
    SetIndex,

    ForIter(String, Option<String>, usize),
    Break,
    Continue,
    DoWhileStart,

    MakeClosure(Option<String>, Vec<(String, Option<String>)>, usize, bool, Arc<Chunk>),
    SnapshotNext(Vec<String>),
    MakeClosureRef(String, String),
    MakeClassRef(String, String, String),
    MakeClass(String, Option<String>, Box<Vec<Instruction>>, Box<Vec<usize>>),
    NewObject(usize),
    SuperCall(String, usize),

    PushTry(usize),
    PopTry,
    Raise,
    EnterCatch(String),

    Await,

    Import(String, Option<String>),

    IncVar(String),
    DecVar(String),
    PostIncVar(String),
    PostDecVar(String),
    ReadModWriteAttr(String, bool, bool),
    ReadModWriteIndex(bool, bool),

    Dup,
    This,
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
