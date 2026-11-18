use super::*;

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
    star_import: bool,
}

impl Linter {
    fn new() -> Self {
        Self::new_with_extra_builtins(&[])
    }

    fn new_with_extra_builtins(extra: &[&str]) -> Self {
        let mut l = Linter {
            scopes: vec![],
            report: LintReport { errors: vec![], warnings: vec![] },
            star_import: false,
        };
        l.enter_scope();
        let builtins = vec![
            "show", "showln", "readln",
            "exit", "sleep",
            "int", "float", "str", "bool", "chr", "ord", "parse_int", "parse_float", "typeof",
            "hashmap", "list", "bytes", "len", "class_name", "is_a", "has_attr", "set_attr", "same", "json_compact", "now_ms", "floor_int", "round_int", "min", "max", "abs", "pow", "sin",
            "time_now", "gc", "format", "regex",
            "argv", "PI", "E",
        ];
        for b in builtins {
            l.define(b, "built-in function", true);
        }
        for b in extra {
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
            if let Some(info) = scope.variables.get_mut(name) {
                info.is_used = true;
                return;
            }
        }
        if self.star_import { return; }
        self.report.errors.push(format!("Undefined {} '{}' at line {}", expected_type, name, line));
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match &stmt.node {
            StmtNode::Block(stmts) => {
                self.enter_scope();

                for s in stmts {
                    match &s.node {
                        StmtNode::FuncDef(name, ..) => {
                            if !name.is_empty() { self.define(name, "function", false); }
                        }
                        StmtNode::ClassDef(name, ..) => {
                            self.define(name, "class", false);
                        }
                        StmtNode::Expr(e) if self.scopes.len() == 2 => {
                            if let ExprNode::Assign(target, _) = &e.node {
                                if let ExprNode::Var(n) = &target.node {
                                    self.define(n, "variable", false);
                                }
                            }
                        }
                        StmtNode::ConstDecl(n, _) if self.scopes.len() == 2 => {
                            self.define(n, "variable", false);
                        }
                        _ => {}
                    }
                }

                for s in stmts {
                    self.check_stmt(s);
                }

                self.exit_scope();
            },
            StmtNode::Multi(stmts) => {
                for s in stmts {
                    self.check_stmt(s);
                }
            }
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
            StmtNode::For(name, value_var, iter, body) => {
                self.check_expr(iter);
                self.enter_scope();
                self.define(name, "variable", false);
                if let Some(v) = value_var { self.define(v, "variable", false); }
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
                for (p, _) in params {
                    let clean = p.strip_prefix("...").unwrap_or(p.as_str());
                    self.define(clean, "variable", false);
                }
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
            StmtNode::Augment(_, methods) => {
                for m in methods { self.check_stmt(m); }
            },
            StmtNode::ConstDecl(name, expr) => {
                self.check_expr(expr);
                self.define(name, "constant", false);
            },
                StmtNode::Import(_, Some(a)) if a == "*" => {
                    self.star_import = true;
                },
                StmtNode::Import(path, as_name) => {
                    let bind = as_name.clone().unwrap_or_else(|| {
                        std::path::Path::new(path).file_stem().unwrap().to_str().unwrap().to_string()
                    });
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
            ExprNode::OptionalGetAttr(obj, _) => self.check_expr(obj),
            ExprNode::FuncExpr(params, _req_params, body) => {
                self.enter_scope();
                for (p, _) in params {
                        let clean = p.strip_prefix("...").unwrap_or(p.as_str());
                        self.define(clean, "variable", false);
                    }
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

fn has_star_import(ast: &Stmt) -> bool {
    match &ast.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => stmts.iter().any(has_star_import),
        StmtNode::Import(_, Some(a)) => a == "*",
        _ => false,
    }
}

pub fn lint_ast(ast: &Stmt) -> LintReport {
    let mut linter = Linter::new();
    linter.star_import = has_star_import(ast);
    linter.check_stmt(ast);
    linter.exit_scope();
    linter.report
}

pub fn lint_ast_with_extra_builtins(ast: &Stmt, extra: &[&str]) -> LintReport {
    let mut linter = Linter::new_with_extra_builtins(extra);
    linter.check_stmt(ast);
    linter.exit_scope();
    linter.report
}
