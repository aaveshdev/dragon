#![allow(dead_code)]

use super::*;

//  Native shared library transpiler
//  Emits plain C with no Dragon runtime — each top-level func becomes a real
//  exported C function callable from any language via dlopen/LoadLibrary.
//  Top-level variable assignments become C static globals.
// ─────────────────────────────────────────────────────────────────────────────

fn mangle_plain(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
        .collect()
}

fn dragon_type_to_c(t: &str) -> &'static str {
    match t {
        "int" | "i32" | "i64" | "long" => "int64_t",
        "float" | "double" | "f64"     => "double",
        "str" | "string"               => "const char*",
        "bool"                         => "int",
        _                              => "int64_t",
    }
}

fn infer_native_expr_type(e: &Expr) -> &'static str {
    match &e.node {
        ExprNode::Number(n) => {
            if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                "int64_t"
            } else {
                "double"
            }
        }
        ExprNode::String(_) => "const char*",
        ExprNode::Bool(_)   => "int",
        ExprNode::Binary(_, l, r) => {
            let lt = infer_native_expr_type(l);
            let rt = infer_native_expr_type(r);
            if lt == "double" || rt == "double" { "double" }
            else if lt == "const char*" || rt == "const char*" { "const char*" }
            else { "int64_t" }
        }
        _ => "int64_t",
    }
}

fn infer_native_return_type(body: &Stmt) -> &'static str {
    let mut ret: &'static str = "int64_t";
    visit_native_returns(body, &mut ret);
    ret
}

fn visit_native_returns(stmt: &Stmt, ret: &mut &'static str) {
    match &stmt.node {
        StmtNode::Return(Some(e)) => {
            *ret = infer_native_expr_type(e);
        }
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { visit_native_returns(s, ret); }
        }
        StmtNode::IfChain(branches) => {
            for (_, b) in branches { visit_native_returns(b, ret); }
        }
        _ => {}
    }
}

fn expr_to_native_literal(e: &Expr) -> Option<String> {
    match &e.node {
        ExprNode::Number(n) => {
            if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                Some(format!("{}LL", *n as i64))
            } else {
                Some(format!("{:.17}", n))
            }
        }
        ExprNode::String(s) => Some(format!("\"{}\"", escape_c_string(s))),
        ExprNode::Bool(b)   => Some(if *b { "1".into() } else { "0".into() }),
        ExprNode::Null      => Some("0".into()),
        _ => None,
    }
}

struct NativeTranspiler {
    out:     String,
    indent:  usize,
    tmp_ctr: usize,
}

impl NativeTranspiler {
    fn new() -> Self { Self { out: String::new(), indent: 1, tmp_ctr: 0 } }

    fn line(&mut self, s: &str) {
        let pad = "    ".repeat(self.indent);
        self.out.push_str(&pad);
        self.out.push_str(s);
        self.out.push('\n');
    }

    fn fresh(&mut self) -> String {
        self.tmp_ctr += 1;
        format!("_t{}", self.tmp_ctr)
    }

    fn emit_body(&mut self, body: &Stmt, ret_type: &str) {
        self.emit_stmt(body, ret_type);
        // default return so the compiler never warns about missing return
        match ret_type {
            "const char*" => self.line("return \"\";"),
            "double"      => self.line("return 0.0;"),
            _             => self.line("return 0;"),
        }
    }

    fn emit_stmt(&mut self, stmt: &Stmt, ret_type: &str) {
        match &stmt.node {
            StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
                for s in stmts { self.emit_stmt(s, ret_type); }
            }
            StmtNode::Return(Some(e)) => {
                let v = self.emit_expr(e, ret_type);
                self.line(&format!("return {};", v));
            }
            StmtNode::Return(None) => {
                match ret_type {
                    "const char*" => self.line("return \"\";"),
                    "double"      => self.line("return 0.0;"),
                    _             => self.line("return 0;"),
                }
            }
            StmtNode::Expr(e) => {
                let v = self.emit_expr(e, "int64_t");
                self.line(&format!("(void){};", v));
            }
            StmtNode::ConstDecl(name, e) => {
                let ctype = infer_native_expr_type(e);
                let v = self.emit_expr(e, ctype);
                self.line(&format!("{} _{} = {};", ctype, mangle_plain(name), v));
            }
            StmtNode::IfChain(branches) => {
                let mut first = true;
                for (cond_opt, body) in branches {
                    if let Some(cond) = cond_opt {
                        let cv = self.emit_expr(cond, "int64_t");
                        if first {
                            self.line(&format!("if ({}) {{", cv));
                            first = false;
                        } else {
                            self.line(&format!("}} else if ({}) {{", cv));
                        }
                    } else {
                        self.line("} else {");
                    }
                    self.indent += 1;
                    self.emit_stmt(body, ret_type);
                    self.indent -= 1;
                }
                if !first { self.line("}"); }
            }
            StmtNode::While(cond, body) => {
                let cv = self.emit_expr(cond, "int64_t");
                self.line(&format!("while ({}) {{", cv));
                self.indent += 1;
                self.emit_stmt(body, ret_type);
                self.indent -= 1;
                self.line("}");
            }
            StmtNode::For(var, _, iter, body) => {
                // simple numeric for-in: iterate integers
                let iv = self.emit_expr(iter, "int64_t");
                let idx = format!("_{}", mangle_plain(var));
                self.line(&format!("for(int64_t {} = 0; {} < {}; {}++) {{", idx, idx, iv, idx));
                self.indent += 1;
                self.emit_stmt(body, ret_type);
                self.indent -= 1;
                self.line("}");
            }
            _ => {} // classes, imports, etc. are not meaningful inside a native lib func
        }
    }

    fn emit_expr(&mut self, expr: &Expr, hint: &str) -> String {
        match &expr.node {
            ExprNode::Number(n) => {
                if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                    format!("{}LL", *n as i64)
                } else {
                    format!("{:.17}", n)
                }
            }
            ExprNode::String(s) => format!("\"{}\"", escape_c_string(s)),
            ExprNode::Bool(b)   => if *b { "1".into() } else { "0".into() },
            ExprNode::Null      => "0".into(),
            ExprNode::Var(name) => {
                // Could be a local param (_{name}) or a module-level static (_g_{name})
                format!("_{}", mangle_plain(name))
            }
            ExprNode::Binary(op, l, r) => {
                let lv = self.emit_expr(l, hint);
                let rv = self.emit_expr(r, hint);
                let lt = infer_native_expr_type(l);
                let rt = infer_native_expr_type(r);
                match op.as_str() {
                    "+" => {
                        if lt == "const char*" || rt == "const char*" {
                            // string concat via a local buffer
                            let tmp = self.fresh();
                            self.line(&format!(
                                "char {}[4096]; snprintf({}, sizeof({}), \"%s%s\", (const char*)(uintptr_t)({}), (const char*)(uintptr_t)({}));",
                                tmp, tmp, tmp, lv, rv
                            ));
                            tmp
                        } else {
                            format!("({} + {})", lv, rv)
                        }
                    }
                    "-"  => format!("({} - {})", lv, rv),
                    "*"  => format!("({} * {})", lv, rv),
                    "/"  => format!("({} / {})", lv, rv),
                    "%"  => format!("({} % {})", lv, rv),
                    "==" => format!("({} == {})", lv, rv),
                    "!=" => format!("({} != {})", lv, rv),
                    "<"  => format!("({} < {})", lv, rv),
                    "<=" => format!("({} <= {})", lv, rv),
                    ">"  => format!("({} > {})", lv, rv),
                    ">=" => format!("({} >= {})", lv, rv),
                    "&&" | "and" => format!("({} && {})", lv, rv),
                    "||" | "or"  => format!("({} || {})", lv, rv),
                    _    => format!("({} + {})", lv, rv),
                }
            }
            ExprNode::Unary(op, e) => {
                let v = self.emit_expr(e, hint);
                match op.as_str() {
                    "!" | "not" => format!("(!{})", v),
                    "-"         => format!("(-{})", v),
                    _           => v,
                }
            }
            ExprNode::Assign(target, val) => {
                let vv = self.emit_expr(val, hint);
                if let ExprNode::Var(name) = &target.node {
                    let n = format!("_{}", mangle_plain(name));
                    self.line(&format!("{} = {};", n, vv));
                    n
                } else {
                    vv
                }
            }
            ExprNode::PostOp(op, e) => {
                let v = self.emit_expr(e, hint);
                match op.as_str() {
                    "++" => format!("({}++)", v),
                    "--" => format!("({}--)", v),
                    _    => v,
                }
            }
            ExprNode::PreOp(op, e) => {
                let v = self.emit_expr(e, hint);
                match op.as_str() {
                    "++" => format!("(++{})", v),
                    "--" => format!("(--{})", v),
                    _    => v,
                }
            }
            ExprNode::Ternary(cond, then_e, else_e) => {
                let cv = self.emit_expr(cond, "int64_t");
                let tv = self.emit_expr(then_e, hint);
                let ev = self.emit_expr(else_e, hint);
                format!("({} ? {} : {})", cv, tv, ev)
            }
            _ => "0".into(),
        }
    }
}

/// Collect top-level function definitions for the native lib export table.
fn collect_native_fns(ast: &Stmt) -> Vec<(String, Vec<(String, Option<String>)>, Stmt)> {
    let mut out = Vec::new();
    match &ast.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { out.extend(collect_native_fns(s)); }
        }
        StmtNode::FuncDef(name, params, _, body, _) => {
            out.push((name.clone(), params.clone(), *body.clone()));
        }
        _ => {}
    }
    out
}

/// Collect top-level variable assignments (var x = val or x = val at top level).
fn collect_native_globals(ast: &Stmt) -> Vec<(String, &'static str, String)> {
    let mut out = Vec::new();
    match &ast.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { out.extend(collect_native_globals(s)); }
        }
        StmtNode::ConstDecl(name, e) => {
            if let Some(lit) = expr_to_native_literal(e) {
                out.push((name.clone(), infer_native_expr_type(e), lit));
            }
        }
        StmtNode::Expr(e) => {
            if let ExprNode::Assign(target, val) = &e.node {
                if let ExprNode::Var(name) = &target.node {
                    if let Some(lit) = expr_to_native_literal(val) {
                        out.push((name.clone(), infer_native_expr_type(val), lit));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Generate a fully-native shared library — no Dragon runtime, no GC, no init protocol.
/// Each top-level Dragon function becomes a plain exported C function.
/// Callers use it exactly like any other native .so/.dll.
pub(crate) fn transpile_native_lib(ast: &Stmt) -> String {
    let mut out = String::new();

    out.push_str("#include <stdint.h>\n");
    out.push_str("#include <string.h>\n");
    out.push_str("#include <stdlib.h>\n");
    out.push_str("#include <stdio.h>\n");
    out.push_str("#include <math.h>\n\n");

    // Platform export macro
    out.push_str("#ifdef _WIN32\n");
    out.push_str("#  define DGN_EXPORT __declspec(dllexport)\n");
    out.push_str("#else\n");
    out.push_str("#  define DGN_EXPORT __attribute__((visibility(\"default\")))\n");
    out.push_str("#endif\n\n");

    // Top-level static globals (counter = 0, etc.)
    let globals = collect_native_globals(ast);
    for (name, ctype, init) in &globals {
        out.push_str(&format!("static {} _g_{} = {};\n", ctype, mangle_plain(name), init));
    }
    if !globals.is_empty() { out.push('\n'); }

    // One exported C function per Dragon func
    let fns = collect_native_fns(ast);
    for (name, params, body) in &fns {
        let ret_type = infer_native_return_type(body);

        // Build the C parameter list.
        // Dragon param syntax:  name  or  name:type
        // Default type when unannotated: int64_t
        let c_params: Vec<String> = params.iter().map(|(pname, _default)| {
            let clean = pname.split(':').next().unwrap_or(pname).trim();
            let (ptype, pname_clean) = if let Some(colon_pos) = clean.find(':') {
                (dragon_type_to_c(clean[colon_pos+1..].trim()), clean[..colon_pos].trim())
            } else {
                ("int64_t", clean)
            };
            format!("{} _{}", ptype, mangle_plain(pname_clean))
        }).collect();

        let params_str = if c_params.is_empty() {
            "void".to_string()
        } else {
            c_params.join(", ")
        };

        out.push_str(&format!("DGN_EXPORT {} {}({}) {{\n", ret_type, name, params_str));

        // Re-expose globals as local aliases so the function body can write them
        for (gname, gtype, _) in &globals {
            out.push_str(&format!(
                "    /* alias for global */ (void)0; /* _{} is _g_{} */\n",
                mangle_plain(gname), mangle_plain(gname)
            ));
            // Actually emit a pointer alias so assignments inside the func hit the global
            out.push_str(&format!(
                "    {}* const __{}_ptr = &_g_{};\n",
                gtype, mangle_plain(gname), mangle_plain(gname)
            ));
            out.push_str(&format!(
                "#define _{} (*__{}_ptr)\n",
                mangle_plain(gname), mangle_plain(gname)
            ));
        }

        let mut nt = NativeTranspiler::new();
        nt.emit_body(body, ret_type);
        out.push_str(&nt.out);

        // Undefine the global aliases so they don't leak into subsequent functions
        for (gname, _, _) in &globals {
            out.push_str(&format!("#undef _{}\n", mangle_plain(gname)));
        }

        out.push_str("}\n\n");
    }

    out
}

