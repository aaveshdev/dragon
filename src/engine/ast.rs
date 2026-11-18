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
    OptionalGetAttr(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    PreOp(String, Box<Expr>),
    PostOp(String, Box<Expr>),
    FuncExpr(Vec<(String, Option<String>)>, usize, Box<Stmt>),
    New(Box<Expr>, Vec<Expr>),
    Super(String, Vec<Expr>),
    Await(Box<Expr>),
}

#[derive(Debug, Clone)]
pub enum StmtNode {
    Block(Vec<Stmt>),
    Multi(Vec<Stmt>),
    Expr(Expr),
    Return(Option<Expr>),
    Break,
    Continue,
    Raise(Expr),
    IfChain(Vec<(Option<Expr>, Stmt)>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(String, Option<String>, Expr, Box<Stmt>),
    CFor(Option<Expr>, Option<Expr>, Option<Expr>, Box<Stmt>),
    Switch(Expr, Vec<(Expr, Stmt)>, Option<Box<Stmt>>),
    Try(Box<Stmt>, Option<String>, Option<Box<Stmt>>, Option<Box<Stmt>>),
    FuncDef(String, Vec<(String, Option<String>)>, usize, Box<Stmt>, bool),
    ClassDef(String, Option<String>, Box<Stmt>),
    Augment(String, Vec<Stmt>),
    Import(String, Option<String>),
    ConstDecl(String, Expr),
}
