use super::*;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct AliasBinding {
    pub prefix: String,
    pub exports: HashSet<String>,
}

pub fn top_level_names(stmts: &[Stmt]) -> Vec<String> {
    let mut out = Vec::new();
    fn visit(s: &Stmt, out: &mut Vec<String>) {
        match &s.node {
            StmtNode::FuncDef(n, ..) | StmtNode::ClassDef(n, ..) | StmtNode::ConstDecl(n, _) => out.push(n.clone()),
            StmtNode::Block(v) | StmtNode::Multi(v) => for x in v { visit(x, out) },
            StmtNode::Expr(e) => {
                if let ExprNode::Assign(t, _) = &e.node {
                    if let ExprNode::Var(n) = &t.node { out.push(n.clone()); }
                }
            }
            _ => {}
        }
    }
    for s in stmts { visit(s, &mut out); }
    out.sort();
    out.dedup();
    out
}

pub struct Rewriter<'a> {
    pub vars: &'a HashMap<String, String>,
    pub aliases: &'a HashMap<String, AliasBinding>,
    pub errors: Vec<String>,
}

impl<'a> Rewriter<'a> {
    fn name(&self, n: &mut String) {
        if let Some(r) = self.vars.get(n.as_str()) { *n = r.clone(); }
    }

    fn params(&self, ps: &mut Vec<(String, Option<String>)>) {
        for p in ps.iter_mut() { self.name(&mut p.0); }
    }

    pub fn stmts(&mut self, v: &mut Vec<Stmt>) { for s in v.iter_mut() { self.stmt(s); } }

    pub fn stmt(&mut self, s: &mut Stmt) {
        match &mut s.node {
            StmtNode::Block(v) | StmtNode::Multi(v) => self.stmts(v),
            StmtNode::Expr(e) => self.expr(e),
            StmtNode::Return(e) => { if let Some(e) = e { self.expr(e) } }
            StmtNode::Raise(e) => self.expr(e),
            StmtNode::IfChain(arms) => for (c, b) in arms.iter_mut() { if let Some(c) = c { self.expr(c) } self.stmt(b) },
            StmtNode::While(c, b) => { self.expr(c); self.stmt(b) }
            StmtNode::DoWhile(b, c) => { self.stmt(b); self.expr(c) }
            StmtNode::For(v, i, it, b) => { self.name(v); if let Some(i) = i { self.name(i) } self.expr(it); self.stmt(b) }
            StmtNode::CFor(a, b, c, body) => {
                for x in [a, b, c] { if let Some(x) = x { self.expr(x) } }
                self.stmt(body)
            }
            StmtNode::Switch(e, cases, def) => {
                self.expr(e);
                for (c, b) in cases.iter_mut() { self.expr(c); self.stmt(b) }
                if let Some(d) = def { self.stmt(d) }
            }
            StmtNode::Try(b, v, c, f) => {
                self.stmt(b);
                if let Some(v) = v { self.name(v) }
                if let Some(c) = c { self.stmt(c) }
                if let Some(f) = f { self.stmt(f) }
            }
            StmtNode::FuncDef(_name, ps, _n, body, _a) => { self.params(ps); self.stmt(body) }
            StmtNode::ClassDef(_name, parent, body) => {
                if let Some(p) = parent {
                    if let Some((a, n)) = p.clone().split_once('.') {
                        if let Some(b) = self.aliases.get(a) {
                            if b.exports.contains(n) { *p = format!("{}{}", b.prefix, n); }
                            else { self.errors.push(format!("module '{}' has no public name '{}'", a, n)); }
                        }
                    } else {
                        self.name(p);
                    }
                }
                self.stmt(body)
            }
            StmtNode::Augment(n, body) => { self.name(n); self.stmts(body) }
            StmtNode::ConstDecl(_n, e) => self.expr(e),
            StmtNode::Break | StmtNode::Continue | StmtNode::Import(..) => {}
        }
    }

    pub fn expr(&mut self, e: &mut Expr) {
        if let ExprNode::GetAttr(obj, attr) = &e.node {
            if let ExprNode::Var(a) = &obj.node {
                if let Some(b) = self.aliases.get(a.as_str()) {
                    if b.exports.contains(attr.as_str()) {
                        let line = e.line;
                        *e = Expr { node: ExprNode::Var(format!("{}{}", b.prefix, attr)), line };
                        return;
                    } else {
                        self.errors.push(format!("line {}: module '{}' has no public name '{}'", e.line, a, attr));
                        return;
                    }
                }
            }
        }
        match &mut e.node {
            ExprNode::Var(n) => self.name(n),
            ExprNode::List(v) => for x in v.iter_mut() { self.expr(x) },
            ExprNode::Obj(v) => for (_, x) in v.iter_mut() { self.expr(x) },
            ExprNode::Unary(_, a) | ExprNode::PreOp(_, a) | ExprNode::PostOp(_, a) | ExprNode::Await(a) => self.expr(a),
            ExprNode::Binary(_, a, b) | ExprNode::Assign(a, b) | ExprNode::Index(a, b) => { self.expr(a); self.expr(b) }
            ExprNode::Ternary(a, b, c) => { self.expr(a); self.expr(b); self.expr(c) }
            ExprNode::Call(f, args) | ExprNode::New(f, args) => { self.expr(f); for x in args.iter_mut() { self.expr(x) } }
            ExprNode::GetAttr(o, _) | ExprNode::OptionalGetAttr(o, _) => self.expr(o),
            ExprNode::FuncExpr(ps, _n, body) => { self.params(ps); self.stmt(body) }
            ExprNode::Super(_, args) => for x in args.iter_mut() { self.expr(x) },
            ExprNode::Number(_) | ExprNode::String(_) | ExprNode::Bool(_) | ExprNode::Null | ExprNode::This => {}
        }
    }

    pub fn decls(&mut self, v: &mut Vec<Stmt>) {
        for s in v.iter_mut() {
            match &mut s.node {
                StmtNode::FuncDef(n, ..) | StmtNode::ClassDef(n, ..) | StmtNode::ConstDecl(n, _) => self.name(n),
                StmtNode::Block(b) | StmtNode::Multi(b) => self.decls(b),
                _ => {}
            }
        }
    }
}

pub fn rewrite_file(
    stmts: &mut Vec<Stmt>,
    vars: &HashMap<String, String>,
    aliases: &HashMap<String, AliasBinding>,
) -> Result<(), String> {
    if vars.is_empty() && aliases.is_empty() { return Ok(()); }
    let mut r = Rewriter { vars, aliases, errors: Vec::new() };
    r.decls(stmts);
    r.stmts(stmts);
    if r.errors.is_empty() { Ok(()) } else { Err(r.errors.join("\n")) }
}
