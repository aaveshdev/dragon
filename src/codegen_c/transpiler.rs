#![allow(dead_code)]

use super::*;

// ─────────────────────────────────────────────────────────────────────────────
//  Transpiler state
// ─────────────────────────────────────────────────────────────────────────────

struct Transpiler {
    out: String,
    hoisted: String,
    indent: usize,
    tmp_counter: usize,
    lbl_counter: usize,
    declared: Vec<std::collections::HashSet<String>>,
    loop_ctx: Vec<(Option<String>, String)>,
    closure_vars: std::collections::HashSet<String>,
    hoisted_modules: std::collections::HashSet<String>,
}

impl Transpiler {
    fn new() -> Self {
        Transpiler {
            out: String::new(),
            hoisted: String::new(),
            indent: 0,
            tmp_counter: 0,
            lbl_counter: 0,
            declared: vec![std::collections::HashSet::new()],
            loop_ctx: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            hoisted_modules: std::collections::HashSet::new(),
        }
    }

    fn line(&mut self, s: &str) {
        let indent = "    ".repeat(self.indent);
        writeln!(self.out, "{}{}", indent, s).unwrap();
    }

    fn raw(&mut self, s: &str) {
        self.out.push_str(s);
    }

    fn fresh_tmp(&mut self) -> String {
        self.tmp_counter += 1;
        format!("_t{}", self.tmp_counter)
    }

    #[allow(dead_code)]
    fn fresh_lbl(&mut self) -> String {
        self.lbl_counter += 1;
        format!("_lbl{}", self.lbl_counter)
    }

    fn push_scope(&mut self) {
        self.declared.push(std::collections::HashSet::new());
    }
    fn pop_scope(&mut self) {
        self.declared.pop();
    }

    // --- ADD THIS HELPER METHOD ---
    fn is_declared(&self, c_name: &str) -> bool {
        self.declared.iter().any(|scope| scope.contains(c_name))
    }

    // --- REPLACE YOUR EXISTING decl_var WITH THIS ---
    fn decl_var(&mut self, name: &str) -> String {
        let c_name = mangle(name);
        // Check all parent scopes to prevent shadowing outer variables!
        if !self.is_declared(&c_name) {
            if let Some(current) = self.declared.last_mut() {
                current.insert(c_name.clone());
                return format!("DgnVal {}", c_name);
            }
        }
        c_name
    }

    // ── statement codegen ────────────────────────────────────────────────────

    fn emit_stmt(&mut self, stmt: &Stmt) {
        match &stmt.node {
            StmtNode::Block(stmts) => {
                self.line("{");
                self.indent += 1;
                self.push_scope();
                for s in stmts {
                    self.emit_stmt(s);
                }
                self.pop_scope();
                self.indent -= 1;
                self.line("}");
            }
            StmtNode::Multi(stmts) => {
                for s in stmts {
                    self.emit_stmt(s);
                }
            }
            StmtNode::Expr(expr) => {
                let (pre, val) = self.emit_expr(expr);
                self.raw(&pre);
                let indent = "    ".repeat(self.indent);
                writeln!(self.out, "{}(void){};", indent, val).unwrap();
            }
            StmtNode::Return(maybe_expr) => match maybe_expr {
                None => self.line("return dgn_null();"),
                Some(e) => {
                    let (pre, val) = self.emit_expr(e);
                    self.raw(&pre);
                    let indent = "    ".repeat(self.indent);
                    writeln!(self.out, "{}return {};", indent, val).unwrap();
                }
            },
            StmtNode::Break => {
                if let Some((_, brk_lbl)) = self.loop_ctx.last() {
                    let indent = "    ".repeat(self.indent);
                    writeln!(self.out, "{}goto {};", indent, brk_lbl).unwrap();
                } else {
                    self.line("break;");
                }
            }
            StmtNode::Continue => {
                let mut found = false;
                for (cont_opt, _) in self.loop_ctx.iter().rev() {
                    if let Some(cont_lbl) = cont_opt {
                        let indent = "    ".repeat(self.indent);
                        writeln!(self.out, "{}goto {};", indent, cont_lbl).unwrap();
                        found = true;
                        break;
                    }
                }
                if !found {
                    self.line("continue;");
                }
            }
            StmtNode::Raise(e) => {
                let (pre, val) = self.emit_expr(e);
                self.raw(&pre);
                let indent = "    ".repeat(self.indent);
                writeln!(self.out, "{}dgn_raise({});", indent, val).unwrap();
            }
            StmtNode::ConstDecl(name, expr) => {
                let c_name = mangle(name);
                let (pre, val) = self.emit_expr(expr);
                self.raw(&pre);
                let indent = "    ".repeat(self.indent);
                let decl = self.decl_var(name);
                writeln!(self.out, "{}{} = {};  /* const */", indent, decl, val).unwrap();
                writeln!(self.out, "{}(void){};", indent, c_name).unwrap();
            }
            StmtNode::FuncDef(name, params, _req, body, _is_async) => {
                let c_name = mangle(name);

                // Hoist the C-level function. For recursive nested functions the body
                // refers to the function's own DgnVal by name. We inject a self-binding
                // as the very first line of the hoisted body so the recursive call resolves.
                // Strategy: emit_named_fn writes into self.hoisted; we patch in the
                // self-binding by temporarily prepending it to the body via a wrapper.
                {
                    let saved_indent = self.indent;
                    self.indent = 0;
                    let mut saved_out = std::mem::take(&mut self.out);

                    writeln!(
                        self.out,
                        "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
                        c_name
                    ).unwrap();
                    self.indent = 1;
                    self.push_scope();

                    // Emit parameters
                    for (i, (pname, default_val)) in params.iter().enumerate() {
                        let pname_clean = pname.split(':').next().unwrap().trim();
                        let is_rest = pname_clean.starts_with("...");
                        let actual_pname = if is_rest { pname_clean[3..].trim() } else { pname_clean };
                        let pc = mangle(actual_pname);
                        let _ = self.decl_var(actual_pname);
                        let ind = "    ".repeat(self.indent);
                        if is_rest {
                            writeln!(self.out, "{}DgnVal {} = dgn_list_new((_nargs > {}) ? (_nargs - {}) : 0);", ind, pc, i, i).unwrap();
                            writeln!(self.out, "{}for (int _i = {}; _i < _nargs; _i++) dgn_list_push(&{}, _args[_i]);", ind, i, pc).unwrap();
                        } else if let Some(def_str) = default_val {
                            writeln!(self.out, "{}DgnVal {} = (_nargs > {}) ? _args[{}] : dgn_str(\"{}\");", ind, pc, i, i, escape_c_string(def_str)).unwrap();
                        } else {
                            writeln!(self.out, "{}DgnVal {} = (_nargs > {}) ? _args[{}] : dgn_null();", ind, pc, i, i).unwrap();
                        }
                    }

                    // Self-binding: lets the function call itself by name recursively
                    {
                        let ind = "    ".repeat(self.indent);
                        let self_decl = self.decl_var(name);
                        writeln!(self.out, "{}{} = dgn_func(_fn_{}, {});", ind, self_decl, c_name, params.len()).unwrap();
                    }

                    self.emit_stmt(body);
                    self.line("return dgn_null();");
                    self.pop_scope();
                    self.indent = 0;
                    self.line("}");
                    self.line("");

                    let generated_fn = std::mem::take(&mut self.out);
                    self.out = saved_out;
                    self.hoisted.push_str(&generated_fn);
                    self.indent = saved_indent;
                }

                // Runtime binding in the current scope (caller side)
                let decl = self.decl_var(name);
                let indent = "    ".repeat(self.indent);
                writeln!(
                    self.out,
                    "{}{} = dgn_func(_fn_{}, {});",
                    indent, decl, c_name, params.len()
                ).unwrap();
            }
            StmtNode::ClassDef(name, parent, body) => {
                self.emit_classdef_body(name, parent.as_deref(), body);
            }
            StmtNode::IfChain(branches) => {
                self.emit_ifchain(branches);
            }
            StmtNode::While(cond, body) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

                let tmp = self.fresh_tmp();
                let (pre, cval) = self.emit_expr(cond);
                self.raw(&pre);
                let indent = "    ".repeat(self.indent);
                writeln!(self.out, "{}DgnVal {} = {};", indent, tmp, cval).unwrap();
                writeln!(self.out, "{}while (dgn_truthy({})) {{", indent, tmp).unwrap();
                self.indent += 1;
                self.push_scope();
                
                self.emit_stmt(body);
                
                self.indent -= 1;
                writeln!(self.out, "    {}{}:;", indent, cont_lbl).unwrap();
                self.indent += 1;
                
                let (pre2, cval2) = self.emit_expr(cond);
                self.raw(&pre2);
                let indent2 = "    ".repeat(self.indent);
                writeln!(self.out, "{}{} = {};", indent2, tmp, cval2).unwrap();
                self.pop_scope();
                self.indent -= 1;
                self.line("}");
                
                writeln!(self.out, "{}{}:;", indent, brk_lbl).unwrap();
                self.loop_ctx.pop();
            }
            StmtNode::DoWhile(body, cond) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

                self.line("do {");
                self.indent += 1;
                self.push_scope();
                
                self.emit_stmt(body);
                
                self.pop_scope();
                self.indent -= 1;
                let indent = "    ".repeat(self.indent);
                writeln!(self.out, "    {}{}:;", indent, cont_lbl).unwrap();
                
                let (pre, cval) = self.emit_expr(cond);
                self.raw(&pre);
                writeln!(self.out, "{}}} while (dgn_truthy({}));", indent, cval).unwrap();
                writeln!(self.out, "{}{}:;", indent, brk_lbl).unwrap();
                self.loop_ctx.pop();
            }
            StmtNode::CFor(init, cond, step, body) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

                self.line("{");
                self.indent += 1;
                self.push_scope();
                if let Some(init_e) = init {
                    let (pre, val) = self.emit_expr(init_e);
                    self.raw(&pre);
                    let indent = "    ".repeat(self.indent);
                    writeln!(self.out, "{}(void){};", indent, val).unwrap();
                }
                if let Some(c) = cond {
                    let (pre, cv) = self.emit_expr(c);
                    self.raw(&pre);
                    let tmp = self.fresh_tmp();
                    let indent = "    ".repeat(self.indent);
                    writeln!(self.out, "{}DgnVal {} = {};", indent, tmp, cv).unwrap();
                    writeln!(self.out, "{}while (dgn_truthy({})) {{", indent, tmp).unwrap();
                    self.indent += 1;
                    self.push_scope();
                    
                    self.emit_stmt(body);
                    
                    self.indent -= 1;
                    writeln!(self.out, "    {}{}:;", indent, cont_lbl).unwrap();
                    self.indent += 1;
                    
                    if let Some(step_e) = step {
                        let (spre, sval) = self.emit_expr(step_e);
                        self.raw(&spre);
                        let indent2 = "    ".repeat(self.indent);
                        writeln!(self.out, "{}(void){};", indent2, sval).unwrap();
                    }
                    let (pre2, cv2) = self.emit_expr(c);
                    self.raw(&pre2);
                    let indent2 = "    ".repeat(self.indent);
                    writeln!(self.out, "{}{} = {};", indent2, tmp, cv2).unwrap();
                    self.pop_scope();
                    self.indent -= 1;
                    self.line("}");
                } else {
                    self.line("while (1) {");
                    self.indent += 1;
                    self.push_scope();
                    
                    self.emit_stmt(body);
                    
                    self.indent -= 1;
                    let indent = "    ".repeat(self.indent);
                    writeln!(self.out, "    {}{}:;", indent, cont_lbl).unwrap();
                    self.indent += 1;
                    
                    if let Some(step_e) = step {
                        let (spre, sval) = self.emit_expr(step_e);
                        self.raw(&spre);
                        let indent2 = "    ".repeat(self.indent);
                        writeln!(self.out, "{}(void){};", indent2, sval).unwrap();
                    }
                    self.pop_scope();
                    self.indent -= 1;
                    self.line("}");
                }
                let indent = "    ".repeat(self.indent);
                writeln!(self.out, "{}{}:;", indent, brk_lbl).unwrap();
                
                self.pop_scope();
                self.indent -= 1;
                self.line("}");
                self.loop_ctx.pop();
            }
            StmtNode::For(var, var2, iter_expr, body) => {
                self.emit_for_in(var, var2.as_deref(), iter_expr, body);
            }
            StmtNode::Switch(val_expr, cases, default) => {
                self.emit_switch(val_expr, cases, default.as_deref());
            }
            StmtNode::Try(try_body, catch_var, catch_body, finally_body) => {
                self.emit_try(
                    try_body,
                    catch_var.as_deref(),
                    catch_body.as_deref(),
                    finally_body.as_deref(),
                );
            }
            StmtNode::Augment(type_name, methods) => {
                for m in methods {
                    if let StmtNode::FuncDef(mname, params, _req, body, _) = &m.node {
                        let fn_c = format!("{}_{}", mangle(type_name), mangle(mname));
                        
                        // FIX: Explicitly inject 'this' as the first parameter
                        let mut all_params = vec![("this".to_string(), None)];
                        all_params.extend(params.iter().cloned());
                        
                        self.emit_named_fn(&fn_c, &all_params, body);
                        
                        let indent = "    ".repeat(self.indent);
                        writeln!(
                            self.out,
                            "{}dgn_augment_register(\"{}\", \"{}\", _fn_{});",
                            indent, type_name, mname, fn_c
                        )
                        .unwrap();
                    }
                }
            }
            StmtNode::Import(module, alias) => {
                let bind_name = alias.as_deref().unwrap_or(module.as_str());
                let c_name = mangle(bind_name);
                // Already declared as a static global — just assign
                let indent = "    ".repeat(self.indent);
                writeln!(
                    self.out,
                    "{}{} = dgn_load_module(\"{}\");",
                    indent, c_name, module
                )
                .unwrap();
                writeln!(self.out, "{}(void){};", indent, c_name).unwrap();
            }
        }
    }

    // ── expression codegen ────────────────────────────────────────────────────

    fn emit_expr(&mut self, expr: &Expr) -> (String, String) {
        match &expr.node {
            ExprNode::Null => (String::new(), "dgn_null()".into()),
            ExprNode::Bool(b) => (
                String::new(),
                format!("dgn_bool({})", if *b { 1 } else { 0 }),
            ),
            ExprNode::Number(n) => {
                if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n <= i64::MAX as f64 {
                    (String::new(), format!("dgn_int({}LL)", *n as i64))
                } else {
                    (String::new(), format!("dgn_float({:.17})", n))
                }
            }
            ExprNode::String(s) => {
                let escaped = escape_c_string(s);
                (String::new(), format!("dgn_str(\"{}\")", escaped))
            }
            ExprNode::This => (
                String::new(),
                mangle("this"),
            ),
            ExprNode::Var(name) => {
                let c_name = mangle(name);
                if self.closure_vars.contains(&c_name) {
                    // Read from closure env
                    let tmp = self.fresh_tmp();
                    let indent = "    ".repeat(self.indent);
                    let mut pre = String::new();
                    writeln!(pre, "{}DgnVal {} = (_cenv) ? dgn_map_get_raw(_cenv, \"{}\") : dgn_null();",
                        indent, tmp, c_name).unwrap();
                    (pre, tmp)
                } else {
                    (String::new(), c_name)
                }
            }
           ExprNode::List(items) => {
                let mut pre = String::new();
                let tmp = self.fresh_tmp();
                let indent = "    ".repeat(self.indent);
                writeln!(pre, "{}DgnVal {} = dgn_list_new({});", indent, tmp, items.len()).unwrap();
                for item in items {
                    let (ipre, ival) = self.emit_expr(item);
                    pre.push_str(&ipre);
                    writeln!(pre, "{}dgn_list_push(&{}, {});", indent, tmp, ival).unwrap();
                }
                (pre, tmp)
            }
            ExprNode::Obj(pairs) => {
                let mut pre = String::new();
                let tmp = self.fresh_tmp();
                let indent = "    ".repeat(self.indent);
                writeln!(pre, "{}DgnVal {} = dgn_map_new();", indent, tmp).unwrap();
                for (k, v) in pairs {
                    let (vpre, vval) = self.emit_expr(v);
                    pre.push_str(&vpre);
                    writeln!(
                        pre,
                        "{}dgn_map_set(&{}, \"{}\", {});",
                        indent,
                        tmp,
                        escape_c_string(k),
                        vval
                    )
                    .unwrap();
                }
                (pre, tmp)
            }
            ExprNode::Unary(op, operand) => {
                let (pre, val) = self.emit_expr(operand);
                let result = match op.as_str() {
                    "!" => format!("dgn_not({})", val),
                    "-" => format!("dgn_neg({})", val),
                    "~" => format!("dgn_bitnot({})", val),
                    _ => format!("dgn_unary(\"{}\", {})", op, val),
                };
                (pre, result)
            }
            ExprNode::Binary(op, lhs, rhs) => self.emit_binary(op, lhs, rhs),
            ExprNode::Ternary(cond, then_e, else_e) => {
                let tmp = self.fresh_tmp();
                let (cpre, cval) = self.emit_expr(cond);
                let (tpre, tval) = self.emit_expr(then_e);
                let (epre, eval) = self.emit_expr(else_e);
                let indent = "    ".repeat(self.indent);
                let mut pre = cpre;
                writeln!(pre, "{}DgnVal {};", indent, tmp).unwrap();
                writeln!(pre, "{}if (dgn_truthy({})) {{", indent, cval).unwrap();
                pre.push_str(&tpre);
                writeln!(pre, "{}  {} = {};", indent, tmp, tval).unwrap();
                writeln!(pre, "{}}} else {{", indent).unwrap();
                pre.push_str(&epre);
                writeln!(pre, "{}  {} = {};", indent, tmp, eval).unwrap();
                writeln!(pre, "{}}}", indent).unwrap();
                (pre, tmp)
            }
            ExprNode::Assign(target, value) => {
                let (vpre, vval) = self.emit_expr(value);
                match &target.node {
                    ExprNode::Var(name) => {
                        let c_name = mangle(name);
                        let indent = "    ".repeat(self.indent);
                        let mut pre = vpre;
                        if self.closure_vars.contains(&c_name) {
                            writeln!(pre, "{}if (_cenv) dgn_map_set_raw(_cenv, \"{}\", {});",
                                indent, c_name, vval).unwrap();
                        } else {
                            let decl = self.decl_var(name);
                            writeln!(pre, "{}{} = {};", indent, decl, vval).unwrap();
                        }
                        (pre, if self.closure_vars.contains(&c_name) { vval } else { c_name.clone() })
                    }
                    ExprNode::GetAttr(obj, attr) => {
                        let (opre, oval) = self.emit_expr(obj);
                        let mut pre = opre;
                        pre.push_str(&vpre);
                        let indent = "    ".repeat(self.indent);
                        // FIX: if oval is a call expression (rvalue), materialise it into
                        // an addressable temporary before taking its address.
                        let is_call = oval.contains('(');
                        if is_call {
                            let tmp = self.fresh_tmp();
                            writeln!(pre, "{}DgnVal {} = {};", indent, tmp, oval).unwrap();
                            writeln!(
                                pre,
                                "{}dgn_set_attr(&{}, \"{}\", {});",
                                indent,
                                tmp,
                                escape_c_string(attr),
                                vval
                            )
                            .unwrap();
                        } else {
                            writeln!(
                                pre,
                                "{}dgn_set_attr(&{}, \"{}\", {});",
                                indent,
                                oval,
                                escape_c_string(attr),
                                vval
                            )
                            .unwrap();
                        }
                        (pre, vval)
                    }
                    ExprNode::Index(obj, idx) => {
                        let (opre, oval) = self.emit_expr(obj);
                        let (ipre, ival) = self.emit_expr(idx);
                        let mut pre = opre;
                        pre.push_str(&ipre);
                        pre.push_str(&vpre);
                        let indent = "    ".repeat(self.indent);
                        // MSVC C2102: '&' requires an l-value.
                        // dgn_get_index() returns DgnVal by value (a temporary/rvalue),
                        // so &dgn_get_index(...) is illegal under strict C rules.
                        // If oval looks like a function call (contains '('), materialise
                        // it into a named temporary first so we have an addressable lvalue.
                        let is_call = oval.contains('(');
                        if is_call {
                            let tmp = self.fresh_tmp();
                            writeln!(pre, "{}DgnVal {} = {};", indent, tmp, oval).unwrap();
                            writeln!(
                                pre,
                                "{}dgn_set_index(&{}, {}, {});",
                                indent, tmp, ival, vval
                            )
                            .unwrap();
                        } else {
                            writeln!(
                                pre,
                                "{}dgn_set_index(&{}, {}, {});",
                                indent, oval, ival, vval
                            )
                            .unwrap();
                        }
                        (pre, vval)
                    }
                    
                    // --- NEW DESTRUCTURING LOGIC LIVES HERE ---
                   // --- NEW DESTRUCTURING LOGIC LIVES HERE ---
                    ExprNode::List(items) => {
                        let mut pre = vpre;
                        let rhs_tmp = self.fresh_tmp();
                        let indent = "    ".repeat(self.indent);
                        
                        // Store the evaluated right-hand side array in a temporary variable
                        writeln!(pre, "{}DgnVal {} = {};", indent, rhs_tmp, vval).unwrap();

                        for (i, item) in items.iter().enumerate() {
                            let mut var_name = String::new();
                            let mut current_node = &item.node;
                            
                            // Deep extraction loop: drill down through ANY wrappers to find the variable name
                            for _ in 0..5 {
                                match current_node {
                                    ExprNode::Var(name) => { var_name = name.clone(); break; },
                                    ExprNode::String(name) => { var_name = name.clone(); break; },
                                    ExprNode::PreOp(_, op) | ExprNode::Unary(_, op) | ExprNode::PostOp(_, op) => { 
                                        current_node = &op.node; 
                                    },
                                    ExprNode::Binary(_, lhs, rhs) => { 
                                        if let ExprNode::Var(n) = &lhs.node { var_name = n.clone(); break; }
                                        current_node = &rhs.node;
                                    },
                                    ExprNode::Call(callee, args) => {
                                        if let Some(arg) = args.first() { current_node = &arg.node; }
                                        else { current_node = &callee.node; }
                                    },
                                    _ => break,
                                }
                            }

                            // If we couldn't find a variable name, or it's a blank slot `_`, skip it
                            if var_name.is_empty() || var_name == "_" {
                                continue;
                            }

                            let clean_name = var_name.trim();
                            let is_rest = clean_name.starts_with("...");
                            let actual_name = if is_rest { clean_name[3..].trim() } else { clean_name };
                            
                            let decl = self.decl_var(actual_name);
                            let c_name = mangle(actual_name);

                            if is_rest {
                                writeln!(
                                    pre,
                                    "{}{} = dgn_list_slice({}, {});",
                                    indent, decl, rhs_tmp, i
                                ).unwrap();
                                writeln!(pre, "{}(void){};", indent, c_name).unwrap();
                                break; // Rest parameter must be the final element, stop iterating
                            } else {
                                writeln!(
                                    pre,
                                    "{}{} = dgn_get_index({}, dgn_int({}));",
                                    indent, decl, rhs_tmp, i
                                ).unwrap();
                                writeln!(pre, "{}(void){};", indent, c_name).unwrap();
                            }
                        }
                        (pre, rhs_tmp)
                    }
                    
                    _ => (vpre, vval),
                }
            }
            ExprNode::Call(callee, args) => self.emit_call(callee, args),
            ExprNode::GetAttr(obj, attr) => {
                let (pre, oval) = self.emit_expr(obj);
                (
                    pre,
                    format!(
                        "dgn_get_attr({}, \"{}\")",
                        oval,
                        escape_c_string(attr)
                    ),
                )
            }
            ExprNode::OptionalGetAttr(obj, attr) => {
                let tmp = self.fresh_tmp();
                let (pre, oval) = self.emit_expr(obj);
                let indent = "    ".repeat(self.indent);
                let mut preamble = pre;
                writeln!(
                    preamble,
                    "{}DgnVal {} = ({}.tag == DGN_NULL) ? dgn_null() : dgn_get_attr({}, \"{}\");",
                    indent,
                    tmp,
                    oval,
                    oval,
                    escape_c_string(attr)
                )
                .unwrap();
                (preamble, tmp)
            }
            ExprNode::Index(obj, idx) => {
                let (opre, oval) = self.emit_expr(obj);
                let (ipre, ival) = self.emit_expr(idx);
                let mut pre = opre;
                pre.push_str(&ipre);
                (pre, format!("dgn_get_index({}, {})", oval, ival))
            }
            ExprNode::PreOp(op, operand) => match &operand.node {
                ExprNode::Var(name) => {
                    let c_name = mangle(name);
                    let indent = "    ".repeat(self.indent);
                    let mut pre = String::new();
                    let is_inc = op.contains('+') || op.to_lowercase().contains("inc");
                    if self.closure_vars.contains(&c_name) {
                        let tmp = self.fresh_tmp();
                        writeln!(pre, "{}DgnVal {} = (_cenv) ? dgn_map_get_raw(_cenv, \"{}\") : dgn_null();", indent, tmp, c_name).unwrap();
                        let op_fn = if is_inc { "dgn_add" } else { "dgn_sub" };
                        writeln!(pre, "{}{} = {}({}, dgn_int(1));", indent, tmp, op_fn, tmp).unwrap();
                        writeln!(pre, "{}if (_cenv) dgn_map_set_raw(_cenv, \"{}\", {});", indent, c_name, tmp).unwrap();
                        (pre, tmp)
                    } else {
                        if is_inc {
                            writeln!(pre, "{}{} = dgn_add({}, dgn_int(1));", indent, c_name, c_name).unwrap();
                        } else {
                            writeln!(pre, "{}{} = dgn_sub({}, dgn_int(1));", indent, c_name, c_name).unwrap();
                        }
                        (pre, c_name)
                    }
                }
                _ => {
                    let (pre, val) = self.emit_expr(operand);
                    let is_inc = op.contains('+') || op.to_lowercase().contains("inc");
                    let result = if is_inc { format!("dgn_add({}, dgn_int(1))", val) }
                                 else      { format!("dgn_sub({}, dgn_int(1))", val) };
                    (pre, result)
                }
            },
            ExprNode::PostOp(op, operand) => match &operand.node {
                ExprNode::Var(name) => {
                    let c_name = mangle(name);
                    let tmp = self.fresh_tmp();
                    let indent = "    ".repeat(self.indent);
                    let mut pre = String::new();
                    let is_inc = op.contains('+') || op.to_lowercase().contains("inc");
                    if self.closure_vars.contains(&c_name) {
                        writeln!(pre, "{}DgnVal {} = (_cenv) ? dgn_map_get_raw(_cenv, \"{}\") : dgn_null();", indent, tmp, c_name).unwrap();
                        let new_tmp = self.fresh_tmp();
                        let op_fn = if is_inc { "dgn_add" } else { "dgn_sub" };
                        writeln!(pre, "{}DgnVal {} = {}({}, dgn_int(1));", indent, new_tmp, op_fn, tmp).unwrap();
                        writeln!(pre, "{}if (_cenv) dgn_map_set_raw(_cenv, \"{}\", {});", indent, c_name, new_tmp).unwrap();
                        (pre, tmp) // PostOp returns old value
                    } else {
                        writeln!(pre, "{}DgnVal {} = {};", indent, tmp, c_name).unwrap();
                        if is_inc {
                            writeln!(pre, "{}{} = dgn_add({}, dgn_int(1));", indent, c_name, c_name).unwrap();
                        } else {
                            writeln!(pre, "{}{} = dgn_sub({}, dgn_int(1));", indent, c_name, c_name).unwrap();
                        }
                        (pre, tmp)
                    }
                }
                _ => {
                    let (pre, val) = self.emit_expr(operand);
                    (pre, val)
                }
            },
            ExprNode::FuncExpr(params, _req, body) => {
                let fn_tag = self.fresh_tmp();
                let fn_c = format!("anon_{}", fn_tag);

                // ── free-variable analysis ──────────────────────────────────
                let mut param_set = std::collections::HashSet::new();
                for (p, _) in params.iter() {
                    let pname = p.split(':').next().unwrap_or(p).trim();
                    let actual = if pname.starts_with("...") { &pname[3..] } else { pname };
                    param_set.insert(mangle(actual));
                }
                // Also treat "this" as bound
                param_set.insert("_v_this".to_string());

                let mut free_c_names = std::collections::HashSet::new();
                collect_free_vars(body, &param_set, &mut free_c_names);

                // Only capture vars that are actually declared in an enclosing scope
                let captured: Vec<String> = free_c_names
                    .into_iter()
                    .filter(|cn| self.is_declared(cn))
                    .collect();

                let has_closure = !captured.is_empty();

                // Emit the hoisted C function (closure-aware)
                self.emit_named_fn_closure(&fn_c, params, body, &captured);

                let indent = "    ".repeat(self.indent);
                let mut pre = String::new();

                if has_closure {
                    let env_tmp = format!("{}_env", fn_tag);
                    // Create env map and populate it with captured variable values
                    writeln!(pre, "{}DgnVal {} = dgn_map_new();", indent, env_tmp).unwrap();
                    for cn in &captured {
                        writeln!(pre, "{}dgn_map_set(&{}, \"{}\", {});", indent, env_tmp, cn, cn).unwrap();
                    }
                    writeln!(
                        pre,
                        "{}DgnVal {} = dgn_closure(_fn_{}, {}, {}.map);",
                        indent, fn_tag, fn_c, params.len(), env_tmp
                    ).unwrap();
                } else {
                    writeln!(
                        pre,
                        "{}DgnVal {} = dgn_func(_fn_{}, {});",
                        indent, fn_tag, fn_c, params.len()
                    ).unwrap();
                }
                (pre, fn_tag)
            }
            ExprNode::New(cls_expr, args) => {
                let (cpre, cval) = self.emit_expr(cls_expr);
                let mut pre = cpre;
                let args_tmp = self.fresh_tmp();
                let indent = "    ".repeat(self.indent);
                writeln!(
                    pre,
                    "{}DgnVal {}[{}];",
                    indent,
                    args_tmp,
                    args.len().max(1)
                )
                .unwrap();
                for (i, a) in args.iter().enumerate() {
                    let (apre, aval) = self.emit_expr(a);
                    pre.push_str(&apre);
                    writeln!(pre, "{}{}[{}] = {};", indent, args_tmp, i, aval).unwrap();
                }
                (
                    pre,
                    format!(
                        "dgn_new_instance({}, {}, {})",
                        cval,
                        args_tmp,
                        args.len()
                    ),
                )
            }
            ExprNode::Super(method, args) => {
                let mut pre = String::new();
                let args_tmp = self.fresh_tmp();
                let indent = "    ".repeat(self.indent);
                writeln!(
                    pre,
                    "{}DgnVal {}[{}];",
                    indent,
                    args_tmp,
                    args.len().max(1)
                )
                .unwrap();
                for (i, a) in args.iter().enumerate() {
                    let (apre, aval) = self.emit_expr(a);
                    pre.push_str(&apre);
                    writeln!(pre, "{}{}[{}] = {};", indent, args_tmp, i, aval).unwrap();
                }
                (
                    pre,
                    format!(
                        "dgn_super_call(_v_this, \"{}\", {}, {})",
                        escape_c_string(method),
                        args_tmp,
                        args.len()
                    ),
                )
            }
            ExprNode::Await(inner) => {
                let (pre, val) = self.emit_expr(inner);
                let tmp = self.fresh_tmp();
                let indent = "    ".repeat(self.indent);
                let mut preamble = pre;
                writeln!(preamble, "{}DgnVal {} = dgn_promise_await({});", indent, tmp, val).unwrap();
                (preamble, tmp)
            }
        }
    }

    // ── binary expressions ────────────────────────────────────────────────────

    fn emit_binary(&mut self, op: &str, lhs: &Expr, rhs: &Expr) -> (String, String) {
        if op == "and" || op == "&&" {
            let tmp = self.fresh_tmp();
            let (lpre, lval) = self.emit_expr(lhs);
            let (rpre, rval) = self.emit_expr(rhs);
            let indent = "    ".repeat(self.indent);
            let mut pre = lpre;
            writeln!(pre, "{}DgnVal {};", indent, tmp).unwrap();
            writeln!(
                pre,
                "{}if (!dgn_truthy({})) {{ {} = dgn_bool(0); }}",
                indent, lval, tmp
            )
            .unwrap();
            writeln!(pre, "{}else {{", indent).unwrap();
            pre.push_str(&rpre);
            writeln!(pre, "{}  {} = {};", indent, tmp, rval).unwrap();
            writeln!(pre, "{}}}", indent).unwrap();
            return (pre, tmp);
        }
        if op == "or" || op == "||" {
            let tmp = self.fresh_tmp();
            let (lpre, lval) = self.emit_expr(lhs);
            let (rpre, rval) = self.emit_expr(rhs);
            let indent = "    ".repeat(self.indent);
            let mut pre = lpre;
            writeln!(pre, "{}DgnVal {};", indent, tmp).unwrap();
            writeln!(
                pre,
                "{}if (dgn_truthy({})) {{ {} = {}; }}",
                indent, lval, tmp, lval
            )
            .unwrap();
            writeln!(pre, "{}else {{", indent).unwrap();
            pre.push_str(&rpre);
            writeln!(pre, "{}  {} = {};", indent, tmp, rval).unwrap();
            writeln!(pre, "{}}}", indent).unwrap();
            return (pre, tmp);
        }
        if op == "??" {
            let tmp = self.fresh_tmp();
            let (lpre, lval) = self.emit_expr(lhs);
            let (rpre, rval) = self.emit_expr(rhs);
            let indent = "    ".repeat(self.indent);
            let mut pre = lpre;
            writeln!(pre, "{}DgnVal {};", indent, tmp).unwrap();
            writeln!(
                pre,
                "{}if ({}.tag != DGN_NULL) {{ {} = {}; }}",
                indent, lval, tmp, lval
            )
            .unwrap();
            writeln!(pre, "{}else {{", indent).unwrap();
            pre.push_str(&rpre);
            writeln!(pre, "{}  {} = {};", indent, tmp, rval).unwrap();
            writeln!(pre, "{}}}", indent).unwrap();
            return (pre, tmp);
        }

        let (lpre, lval) = self.emit_expr(lhs);
        let (rpre, rval) = self.emit_expr(rhs);
        let mut pre = lpre;
        pre.push_str(&rpre);
        let c_fn = match op {
            "+" => "dgn_add",
            "-" => "dgn_sub",
            "*" => "dgn_mul",
            "/" => "dgn_div",
            "%" => "dgn_mod",
            "==" => "dgn_eq",
            "!=" => "dgn_neq",
            "<" => "dgn_lt",
            "<=" => "dgn_lte",
            ">" => "dgn_gt",
            ">=" => "dgn_gte",
            "&" => "dgn_band",
            "|" => "dgn_bor",
            "^" => "dgn_bxor",
            "<<" => "dgn_shl",
            ">>" => "dgn_shr",
            "in" => "dgn_in",
            _ => "dgn_binop",
        };
        let result = if c_fn == "dgn_binop" {
            format!("dgn_binop(\"{}\", {}, {})", op, lval, rval)
        } else {
            format!("{}({}, {})", c_fn, lval, rval)
        };
        (pre, result)
    }

    // ── function calls ────────────────────────────────────────────────────────

    fn emit_call(&mut self, callee: &Expr, args: &[Expr]) -> (String, String) {
        let mut pre = String::new();
        let indent = "    ".repeat(self.indent);

        let args_tmp = self.fresh_tmp();
        writeln!(
            pre,
            "{}DgnVal {}[{}];",
            indent,
            args_tmp,
            args.len().max(1)
        )
        .unwrap();
        for (i, a) in args.iter().enumerate() {
            let (apre, aval) = self.emit_expr(a);
            pre.push_str(&apre);
            writeln!(pre, "{}{}[{}] = {};", indent, args_tmp, i, aval).unwrap();
        }

        let result = match &callee.node {
            ExprNode::GetAttr(obj, method) => {
                let (opre, oval) = self.emit_expr(obj);
                pre.push_str(&opre);
                format!(
                    "dgn_call_method({}, \"{}\", {}, {})",
                    oval,
                    escape_c_string(method),
                    args_tmp,
                    args.len()
                )
            }
            ExprNode::Var(name) => match name.as_str() {
                "regex" => format!("dgn_regex_new({}, {}, NULL)", args_tmp, args.len()),
                "bytes" => format!("dgn_bytes({}, {})", args_tmp, args.len()),

                "int" => format!("dgn_builtin_int({}, {}, NULL)", args_tmp, args.len()),
                "float" => format!("dgn_builtin_float({}, {}, NULL)", args_tmp, args.len()),
                "str" => format!("dgn_builtin_str({}, {}, NULL)", args_tmp, args.len()),
                "chr" => format!("dgn_builtin_chr({}, {}, NULL)", args_tmp, args.len()),
                "ord" => format!("dgn_builtin_ord({}, {}, NULL)", args_tmp, args.len()),
                "typeof" => format!("dgn_builtin_typeof({}, {}, NULL)", args_tmp, args.len()),
                "parse_int" => format!("dgn_builtin_parse_int({}, {}, NULL)", args_tmp, args.len()),
                "parse_float" => format!("dgn_builtin_parse_float({}, {}, NULL)", args_tmp, args.len()),
                "format" => format!("dgn_builtin_format({}, {}, NULL)", args_tmp, args.len()),

                "showln" => format!("dgn_showln({}, {})", args_tmp, args.len()),
                "show" => format!("dgn_show({}, {})", args_tmp, args.len()),
                "readln" => {
                    if args.is_empty() {
                        "dgn_readln(dgn_null())".into()
                    } else {
                        format!("dgn_readln({}[0])", args_tmp)
                    }
                }
                "len" => format!("dgn_len({}[0])", args_tmp),
                "typeof" => format!("dgn_typeof({}[0])", args_tmp),
                "str" => format!("dgn_to_str({}[0])", args_tmp),
                "int" => format!("dgn_to_int({}[0])", args_tmp),
                "float" => format!("dgn_to_float({}[0])", args_tmp),
                "chr" => format!("dgn_chr({}[0])", args_tmp),
                "ord" => format!("dgn_ord({}[0])", args_tmp),
                "exit" => {
                    if args.is_empty() {
                        "dgn_exit(dgn_int(0))".into()
                    } else {
                        format!("dgn_exit({}[0])", args_tmp)
                    }
                }
                "sleep" => format!("dgn_sleep({}[0])", args_tmp),
                "format" => format!("dgn_format({}, {})", args_tmp, args.len()),
                "hashmap" => "dgn_map_new()".into(),
                "list" => "dgn_list_new(0)".into(),
                "bool" => format!("dgn_bool(dgn_truthy({}[0]))", args_tmp),
                "range" => {
                    if args.len() == 1 {
                        format!("dgn_range(dgn_int(0), {}[0], dgn_int(1))", args_tmp)
                    } else if args.len() == 2 {
                        format!("dgn_range({}[0], {}[1], dgn_int(1))", args_tmp, args_tmp)
                    } else {
                        format!(
                            "dgn_range({}[0], {}[1], {}[2])",
                            args_tmp, args_tmp, args_tmp
                        )
                    }
                }
                _ => {
                    let c_name = mangle(name);
                    format!("dgn_call_val({}, {}, {})", c_name, args_tmp, args.len())
                }
            },
            _ => {
                let (fpre, fval) = self.emit_expr(callee);
                pre.push_str(&fpre);
                format!("dgn_call_val({}, {}, {})", fval, args_tmp, args.len())
            }
        };
        (pre, result)
    }

    // Like emit_named_fn but the generated C function reads/writes captured
    // variables through the DgnEnv* _env parameter (a DgnMap).
    fn emit_named_fn_closure(
        &mut self,
        fn_c: &str,
        params: &[(String, Option<String>)],
        body: &Stmt,
        captured: &[String],
    ) {
        if captured.is_empty() {
            // No captures — delegate to regular emitter, nothing changes
            self.emit_named_fn(fn_c, params, body);
            return;
        }

        let saved_indent = self.indent;
        self.indent = 0;
        let mut saved_out = std::mem::take(&mut self.out);

        // Track which names are captured so emit_expr/emit_stmt can route reads/writes
        let prev_closure_vars = std::mem::take(&mut self.closure_vars);
        for cn in captured { self.closure_vars.insert(cn.clone()); }

        writeln!(
            self.out,
            "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
            fn_c
        ).unwrap();
        self.indent = 1;
        self.push_scope();

        // Emit parameters normally
        for (i, (pname, default_val)) in params.iter().enumerate() {
            let pname_clean = pname.split(':').next().unwrap().trim();
            let is_rest = pname_clean.starts_with("...");
            let actual_pname = if is_rest { pname_clean[3..].trim() } else { pname_clean };
            let c_name = mangle(actual_pname);
            let _ = self.decl_var(actual_pname);
            let indent = "    ".repeat(self.indent);

            if is_rest {
                writeln!(self.out, "{}DgnVal {} = dgn_list_new((_nargs > {}) ? (_nargs - {}) : 0);", indent, c_name, i, i).unwrap();
                writeln!(self.out, "{}for (int _i = {}; _i < _nargs; _i++) dgn_list_push(&{}, _args[_i]);", indent, i, c_name).unwrap();
            } else if let Some(def_str) = default_val {
                writeln!(self.out, "{}DgnVal {} = (_nargs > {}) ? _args[{}] : dgn_str(\"{}\");", indent, c_name, i, i, escape_c_string(def_str)).unwrap();
            } else {
                writeln!(self.out, "{}DgnVal {} = (_nargs > {}) ? _args[{}] : dgn_null();", indent, c_name, i, i).unwrap();
            }
        }

        // Load captured vars from env as local mutable aliases at function entry.
        // All reads/writes in the body go through these locals, and we flush them
        // back to the env at every return point by wrapping with a macro trick.
        // Simpler: we emit proxy read/write helpers inline via the closure_vars set.
        // The body emitter (emit_stmt/emit_expr) checks closure_vars and routes
        // Var reads to dgn_env_get and Assign/PreOp/PostOp to dgn_env_set.
        let indent = "    ".repeat(self.indent);
        // Make env non-null safe
        writeln!(self.out, "{}DgnMap* _cenv = _env ? _env->map : 0;", indent).unwrap();

        self.emit_stmt(body);
        self.line("return dgn_null();");
        self.pop_scope();
        self.indent = 0;
        self.line("}");
        self.line("");

        // Restore
        self.closure_vars = prev_closure_vars;
        let generated_fn = std::mem::take(&mut self.out);
        self.out = saved_out;
        self.hoisted.push_str(&generated_fn);
        self.indent = saved_indent;
    }


    // ── function definition helpers ───────────────────────────────────────────

   fn emit_named_fn(
        &mut self,
        fn_c: &str,
        params: &[(String, Option<String>)],
        body: &Stmt,
    ) {
        let saved_indent = self.indent;
        self.indent = 0;

        // Swap out the current buffer so nested lambdas don't interrupt the current C scope
        let mut saved_out = std::mem::take(&mut self.out);

        writeln!(
            self.out,
            "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
            fn_c
        )
        .unwrap();
        self.indent = 1;
        self.push_scope();

        for (i, (pname, default_val)) in params.iter().enumerate() {
            // Strip out type annotations and detect rest parameters
            let pname_clean = pname.split(':').next().unwrap().trim();
            let is_rest = pname_clean.starts_with("...");
            let actual_pname = if is_rest { pname_clean[3..].trim() } else { pname_clean };
            
            let c_name = mangle(actual_pname);
            let _ = self.decl_var(actual_pname);
            let indent = "    ".repeat(self.indent);

            if is_rest {
                // Collect remaining arguments into a list
                writeln!(
                    self.out,
                    "{}DgnVal {} = dgn_list_new((_nargs > {}) ? (_nargs - {}) : 0);",
                    indent, c_name, i, i
                ).unwrap();
                writeln!(
                    self.out,
                    "{}for (int _i = {}; _i < _nargs; _i++) dgn_list_push(&{}, _args[_i]);",
                    indent, i, c_name
                ).unwrap();
            } else if let Some(def_str) = default_val {
                writeln!(
                    self.out,
                    "{}DgnVal {} = (_nargs > {}) ? _args[{}] : dgn_str(\"{}\");",
                    indent,
                    c_name,
                    i,
                    i,
                    escape_c_string(def_str)
                )
                .unwrap();
            } else {
                writeln!(
                    self.out,
                    "{}DgnVal {} = (_nargs > {}) ? _args[{}] : dgn_null();",
                    indent, c_name, i, i
                )
                .unwrap();
            }
        }

        self.emit_stmt(body);
        self.line("return dgn_null();");
        self.pop_scope();
        self.indent = 0;
        self.line("}");
        self.line("");

        // Move the generated function to the global hoisted buffer and restore the previous scope buffer
        let generated_fn = std::mem::take(&mut self.out);
        self.out = saved_out;
        self.hoisted.push_str(&generated_fn);

        self.indent = saved_indent;
    }

    // ── class definition ─────────────────────────────────────────────────────

    fn emit_classdef_body(&mut self, name: &str, parent: Option<&str>, body: &Stmt) {
        let indent = "    ".repeat(self.indent);
        let decl = self.decl_var(name);
        writeln!(
            self.out,
            "{}{} = dgn_class_new(\"{}\", {});",
            indent,
            decl,
            escape_c_string(name),
            parent
                .map(|p| format!("\"{}\"", escape_c_string(p)))
                .unwrap_or_else(|| "NULL".into())
        )
        .unwrap();

        if let StmtNode::Block(stmts) = &body.node {
            for s in stmts {
                if let StmtNode::FuncDef(mname, params, _req, _, _) = &s.node {
                    let fn_c = format!("{}_{}", mangle(name), mangle(mname));
                    let c_name = mangle(name);
                    writeln!(
                        self.out,
                        "{}dgn_class_add_method(&{}, \"{}\", _fn_{}, {});",
                        indent,
                        c_name,
                        escape_c_string(mname),
                        fn_c,
                        params.len()
                    )
                    .unwrap();
                }
            }
        }

        if let Some(p) = parent {
            let c_name = mangle(name);
            writeln!(
                self.out,
                "{}dgn_class_set_parent(&{}, {});",
                indent,
                c_name,
                mangle(p)
            )
            .unwrap();
        }
    }

    // ── if-chain ─────────────────────────────────────────────────────────────

    fn emit_ifchain(&mut self, branches: &[(Option<Expr>, Stmt)]) {
        let mut first = true;
        for (cond, body) in branches {
            match cond {
                None => {
                    self.line("else {");
                    self.indent += 1;
                    self.push_scope();
                    self.emit_stmt(body);
                    self.pop_scope();
                    self.indent -= 1;
                    self.line("}");
                }
                Some(c) => {
                    let (pre, cval) = self.emit_expr(c);
                    self.raw(&pre);
                    let keyword = if first { "if" } else { "else if" };
                    first = false;
                    let indent = "    ".repeat(self.indent);
                    writeln!(
                        self.out,
                        "{}{} (dgn_truthy({})) {{",
                        indent, keyword, cval
                    )
                    .unwrap();
                    self.indent += 1;
                    self.push_scope();
                    self.emit_stmt(body);
                    self.pop_scope();
                    self.indent -= 1;
                    self.line("}");
                }
            }
        }
    }

    // ── for-in ───────────────────────────────────────────────────────────────

    fn emit_for_in(
        &mut self,
        var: &str,
        var2: Option<&str>,
        iter_expr: &Expr,
        body: &Stmt,
    ) {
        let cont_lbl = self.fresh_lbl();
        let brk_lbl = self.fresh_lbl();
        self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));
        
        let (pre, ival) = self.emit_expr(iter_expr);
        self.raw(&pre);
        let iter_tmp = self.fresh_tmp();
        let idx_tmp = self.fresh_tmp();
        let len_tmp = self.fresh_tmp();
        let indent = "    ".repeat(self.indent);
        writeln!(self.out, "{}DgnVal {} = {};", indent, iter_tmp, ival).unwrap();
        writeln!(
            self.out,
            "{}int {} = 0, {} = (int)dgn_len_raw({});",
            indent, idx_tmp, len_tmp, iter_tmp
        )
        .unwrap();
        writeln!(
            self.out,
            "{}for (; {} < {}; {}++) {{",
            indent, idx_tmp, len_tmp, idx_tmp
        )
        .unwrap();
        self.indent += 1;
        self.push_scope();

        let c_var = self.decl_var(var);
        let indent2 = "    ".repeat(self.indent);

        if let Some(v2) = var2 {
            writeln!(
                self.out,
                "{}DgnVal _pair_{} = dgn_iter_pair({}, {});",
                indent2, idx_tmp, iter_tmp, idx_tmp
            )
            .unwrap();
            writeln!(
                self.out,
                "{}{} = dgn_get_index(_pair_{}, dgn_int(0));",
                indent2, c_var, idx_tmp
            )
            .unwrap();
            let c_var2 = self.decl_var(v2);
            writeln!(
                self.out,
                "{}{} = dgn_get_index(_pair_{}, dgn_int(1));",
                indent2, c_var2, idx_tmp
            )
            .unwrap();
        } else {
            writeln!(
                self.out,
                "{}{} = dgn_iter_get({}, {});",
                indent2, c_var, iter_tmp, idx_tmp
            )
            .unwrap();
        }

        self.emit_stmt(body);
        
        self.indent -= 1;
        writeln!(self.out, "    {}{}:;", indent2, cont_lbl).unwrap();
        self.indent += 1;
        
        self.pop_scope();
        self.indent -= 1;
        self.line("}");
        
        writeln!(self.out, "{}{}:;", indent, brk_lbl).unwrap();
        self.loop_ctx.pop();
    }

    // ── switch ───────────────────────────────────────────────────────────────

    fn emit_switch(
        &mut self,
        val_expr: &Expr,
        cases: &[(Expr, Stmt)],
        default: Option<&Stmt>,
    ) {
        let brk_lbl = self.fresh_lbl();
        self.loop_ctx.push((None, brk_lbl.clone()));
        
        let (pre, sval) = self.emit_expr(val_expr);
        self.raw(&pre);
        let sw_tmp = self.fresh_tmp();
        let indent = "    ".repeat(self.indent);
        writeln!(self.out, "{}DgnVal {} = {};", indent, sw_tmp, sval).unwrap();
        self.line("{");
        self.indent += 1;
        let mut hit = false;
        for (case_expr, case_body) in cases {
            let (cpre, cval) = self.emit_expr(case_expr);
            self.raw(&cpre);
            let kw = if hit { "else if" } else { "if" };
            hit = true;
            let indent2 = "    ".repeat(self.indent);
            writeln!(
                self.out,
                "{}{} (dgn_truthy(dgn_eq({}, {}))) {{",
                indent2, kw, sw_tmp, cval
            )
            .unwrap();
            self.indent += 1;
            self.push_scope();
            self.emit_stmt(case_body);
            self.pop_scope();
            self.indent -= 1;
            self.line("}");
        }
        if let Some(def) = default {
            let kw = if hit { "else {" } else { "{" };
            self.line(kw);
            self.indent += 1;
            self.push_scope();
            self.emit_stmt(def);
            self.pop_scope();
            self.indent -= 1;
            self.line("}");
        }
        self.indent -= 1;
        self.line("}");
        
        let indent = "    ".repeat(self.indent);
        writeln!(self.out, "{}{}:;", indent, brk_lbl).unwrap();
        
        self.loop_ctx.pop();
    }

    // ── try/catch/finally ────────────────────────────────────────────────────

    fn emit_try(
        &mut self,
        try_body: &Stmt,
        catch_var: Option<&str>,
        catch_body: Option<&Stmt>,
        finally_body: Option<&Stmt>,
    ) {
        let jmp_tmp = self.fresh_tmp();
        let indent = "    ".repeat(self.indent);
        
        // 1. Declare and push the try frame
        writeln!(
            self.out,
            "{}DgnTryFrame {}; {}.thrown = dgn_null();",
            indent, jmp_tmp, jmp_tmp
        )
        .unwrap();
        writeln!(self.out, "{}dgn_try_push(&{});", indent, jmp_tmp).unwrap();
        
        // 2. setjmp returns 0 initially, and > 0 when longjmp occurs
        writeln!(
            self.out,
            "{}if (setjmp({}.buf) == 0) {{",
            indent, jmp_tmp
        )
        .unwrap();
        
        self.indent += 1;
        self.push_scope();
        self.emit_stmt(try_body);
        
        // Pop the try frame if try_body finishes without raising an exception
        let indent_inner = "    ".repeat(self.indent);
        writeln!(self.out, "{}dgn_try_pop(&{});", indent_inner, jmp_tmp).unwrap();
        
        self.pop_scope();
        self.indent -= 1;
        self.line("}");

        // 3. Emitting the catch block
        if let (Some(cv), Some(cb)) = (catch_var, catch_body) {
            let c_name = mangle(cv);
            writeln!(self.out, "{}else {{", indent).unwrap();
            self.indent += 1;
            self.push_scope();
            let decl = self.decl_var(cv);
            let indent2 = "    ".repeat(self.indent);
            writeln!(self.out, "{}{} = {}.thrown;", indent2, decl, jmp_tmp).unwrap();
            writeln!(self.out, "{}(void){};", indent2, c_name).unwrap();
            self.emit_stmt(cb);
            self.pop_scope();
            self.indent -= 1;
            self.line("}");
        } else if let Some(cb) = catch_body {
            self.line("else {");
            self.indent += 1;
            self.push_scope();
            self.emit_stmt(cb);
            self.pop_scope();
            self.indent -= 1;
            self.line("}");
        }

        // 4. Emitting the finally block
        if let Some(fb) = finally_body {
            self.line("/* finally */");
            self.push_scope();
            self.emit_stmt(fb);
            self.pop_scope();
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Top-level transpile driver
// ─────────────────────────────────────────────────────────────────────────────
fn collect_assign_targets(expr: &Expr, out: &mut std::collections::HashSet<String>) {
    match &expr.node {
        ExprNode::Var(name) => { out.insert(mangle(name)); }
        ExprNode::List(items) => {
            for item in items { collect_assign_targets(item, out); }
        }
        _ => {}
    }
}

fn collect_top_level_decls(stmt: &Stmt, out: &mut std::collections::HashSet<String>) {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { collect_top_level_decls(s, out); }
        }
        StmtNode::FuncDef(name, _, _, _, _) => { out.insert(mangle(name)); }
        StmtNode::ClassDef(name, _, _) => { out.insert(mangle(name)); }
        StmtNode::ConstDecl(name, _) => { out.insert(mangle(name)); }
        StmtNode::Import(module, alias) => {
            let bind_name = alias.as_deref().unwrap_or(module.as_str());
            out.insert(mangle(bind_name));
        }
        StmtNode::Expr(e) => {
            if let ExprNode::Assign(target, _) = &e.node {
                collect_assign_targets(target, out);
            }
        }
        StmtNode::IfChain(branches) => {
            for (_, body) in branches { collect_top_level_decls(body, out); }
        }
        StmtNode::While(_, body) | StmtNode::DoWhile(body, _) => {
            collect_top_level_decls(body, out);
        }
        StmtNode::For(var, var2, _, body) => {
            out.insert(mangle(var));
            if let Some(v2) = var2 { out.insert(mangle(v2)); }
            collect_top_level_decls(body, out);
        }
        StmtNode::CFor(init, _, _, body) => {
            if let Some(i) = init {
                if let ExprNode::Assign(target, _) = &i.node {
                    collect_assign_targets(target, out);
                }
            }
            collect_top_level_decls(body, out);
        }
        StmtNode::Switch(_, cases, default) => {
            for (_, body) in cases { collect_top_level_decls(body, out); }
            if let Some(d) = default { collect_top_level_decls(d, out); }
        }
        StmtNode::Try(tb, cv, cb, fb) => {
            collect_top_level_decls(tb, out);
            if let Some(v) = cv { out.insert(mangle(v)); }
            if let Some(cb_stmt) = cb { collect_top_level_decls(cb_stmt, out); }
            if let Some(fb_stmt) = fb { collect_top_level_decls(fb_stmt, out); }
        }
        _ => {}
    }
}
pub(crate) fn transpile(ast: &Stmt, _is_lib: bool, base_dir: &PathBuf) -> String {
    let mut out = String::new();
    out.push_str(RUNTIME_HEADER);

    let mut t = Transpiler::new();
    
    // 1. Collect all top-level variables, functions, and imports
    let mut top_level_vars = std::collections::HashSet::new();
    collect_top_level_decls(ast, &mut top_level_vars);
    
    // Ensure standard system globals are included
    top_level_vars.insert("_v_PI".into());
    top_level_vars.insert("_v_E".into());
    top_level_vars.insert("_v_argv".into());
    
    // Pre-fill the global transpiler scope so `decl_var` knows they exist!
    if let Some(scope) = t.declared.last_mut() {
        for v in &top_level_vars {
            scope.insert(v.clone());
        }
    }

    t.indent = 0;
    hoist_top_level(ast, &mut t, base_dir);
    
    t.indent = 1;
    t.push_scope();

    t.line("DgnVal _v_PI = dgn_float(3.14159265358979323846);");
    t.line("DgnVal _v_E  = dgn_float(2.71828182845904523536);");
    t.line("(void)_v_PI; (void)_v_E;");

    emit_top_level_body(ast, &mut t, base_dir);
    t.pop_scope();

    // -- Assemble the final C file --

    // Emit the true C globals — NOT static, so dgn_gc_collect can root them.
    // Static globals live in BSS/data which the conservative stack scanner
    // never sees; they must be explicitly marked or the GC frees them.
    let mut sorted_globals: Vec<String> = top_level_vars.into_iter().collect();
    sorted_globals.sort();

    // Extern forward declarations consumed by the GC root block in RUNTIME_HEADER
    out.push_str("/* GC extern forward declarations */\n");
    for g in &sorted_globals {
        out.push_str(&format!("extern DgnVal {};\n", g));
    }
    out.push_str("\n");

    // Actual definitions — no static, so extern linkage is satisfied
    for g in &sorted_globals {
        out.push_str(&format!("DgnVal {};\n", g));
    }
    out.push_str("\n");

    // Emit the sentinel-terminated global roots array for dgn_gc_collect
    out.push_str("/* Sentinel-terminated array of all top-level globals for GC rooting */\n");
    out.push_str("DgnVal* _dgn_global_roots[] = {\n");
    for g in &sorted_globals {
        out.push_str(&format!("    &{},\n", g));
    }
    out.push_str("    NULL\n};\n\n");

    // Emit the hoisted functions
    out.push_str(&t.hoisted);

    out.push_str("int main(int argc, char** argv) {\n");
    out.push_str("    int _dgn_dummy; dgn_runtime_init(argc, argv, &_dgn_dummy);\n");
    out.push_str("    DgnEnv* _env = NULL;\n");
    out.push_str("    _v_argv = dgn_list_new(argc > 1 ? argc - 1 : 0);\n");
    out.push_str("    for(int _ai=1;_ai<argc;_ai++) dgn_list_push(&_v_argv, dgn_str(_dgn_argv[_ai]));\n");
    out.push_str("    _v_PI = dgn_float(3.14159265358979323846);\n");
    out.push_str("    _v_E  = dgn_float(2.71828182845904523536);\n");

    out.push_str(&t.out);

    out.push_str("    return 0;\n");
    out.push_str("}\n");

    out
}

// ─────────────────────────────────────────────────────────────────────────────
// ── Free-variable analysis ────────────────────────────────────────────────────
// Collects all variable names *used* in `body` that are not bound by `params`.
// "Used" means appearing as ExprNode::Var, including as targets of assignments,
// pre/post-ops, etc. Recurses into nested lambdas but does NOT collect variables
// that are only defined inside a deeper lambda (they are local to that lambda).
fn collect_free_vars(
    body: &Stmt,
    params: &std::collections::HashSet<String>,
    out: &mut std::collections::HashSet<String>,
) {
    collect_free_vars_stmt(body, params, out);
}

fn collect_free_vars_stmt(
    stmt: &Stmt,
    bound: &std::collections::HashSet<String>,
    out: &mut std::collections::HashSet<String>,
) {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { collect_free_vars_stmt(s, bound, out); }
        }
        StmtNode::Expr(e) | StmtNode::Return(Some(e)) | StmtNode::Raise(e) => {
            collect_free_vars_expr(e, bound, out);
        }
        StmtNode::Return(None) | StmtNode::Break | StmtNode::Continue => {}
        StmtNode::ConstDecl(_, e) => { collect_free_vars_expr(e, bound, out); }
        StmtNode::FuncDef(_, params, _, body, _) => {
            // Don't descend into nested named functions — they form their own scope
            let _ = (params, body);
        }
        StmtNode::IfChain(branches) => {
            for (cond, body) in branches {
                if let Some(c) = cond { collect_free_vars_expr(c, bound, out); }
                collect_free_vars_stmt(body, bound, out);
            }
        }
        StmtNode::While(cond, body) | StmtNode::DoWhile(body, cond) => {
            collect_free_vars_expr(cond, bound, out);
            collect_free_vars_stmt(body, bound, out);
        }
        StmtNode::For(var, var2, iter, body) => {
            collect_free_vars_expr(iter, bound, out);
            let mut inner_bound = bound.clone();
            inner_bound.insert(mangle(var));
            if let Some(v2) = var2 { inner_bound.insert(mangle(v2)); }
            collect_free_vars_stmt(body, &inner_bound, out);
        }
        StmtNode::Try(tb, cv, cb, fb) => {
            collect_free_vars_stmt(tb, bound, out);
            let mut inner = bound.clone();
            if let Some(v) = cv { inner.insert(mangle(v)); }
            if let Some(stmt) = cb { collect_free_vars_stmt(stmt, &inner, out); }
            if let Some(f) = fb { collect_free_vars_stmt(f, bound, out); }
        }
        StmtNode::CFor(init, cond, step, body) => {
            if let Some(i) = init { collect_free_vars_expr(i, bound, out); }
                if let Some(c) = cond { collect_free_vars_expr(c, bound, out); }
                if let Some(s) = step { collect_free_vars_expr(s, bound, out); }
                collect_free_vars_stmt(body, bound, out);
        }
        StmtNode::Switch(val, cases, default) => {
            collect_free_vars_expr(val, bound, out);
            for (cv, body) in cases { collect_free_vars_expr(cv, bound, out); collect_free_vars_stmt(body, bound, out); }
            if let Some(d) = default { collect_free_vars_stmt(d, bound, out); }
        }
        _ => {}
    }
}

fn collect_free_vars_expr(
    expr: &Expr,
    bound: &std::collections::HashSet<String>,
    out: &mut std::collections::HashSet<String>,
) {
    match &expr.node {
        ExprNode::Var(name) => {
            let c = mangle(name);
            if !bound.contains(&c) { out.insert(c); }
        }
        ExprNode::Assign(target, val) => {
            collect_free_vars_expr(target, bound, out);
            collect_free_vars_expr(val, bound, out);
        }
        ExprNode::Binary(_, a, b) | ExprNode::Index(a, b) => {
            collect_free_vars_expr(a, bound, out);
            collect_free_vars_expr(b, bound, out);
        }
        ExprNode::Unary(_, e) | ExprNode::PreOp(_, e) | ExprNode::PostOp(_, e)
        | ExprNode::GetAttr(e, _) | ExprNode::Await(e) => {
            collect_free_vars_expr(e, bound, out);
        }
        ExprNode::Ternary(c, t, e) => {
            collect_free_vars_expr(c, bound, out);
            collect_free_vars_expr(t, bound, out);
            collect_free_vars_expr(e, bound, out);
        }
        ExprNode::Call(callee, args) => {
            collect_free_vars_expr(callee, bound, out);
            for a in args { collect_free_vars_expr(a, bound, out); }
        }
        ExprNode::List(items) | ExprNode::New(_, items) => {
            // For New, the first sub-expr is the class - handle both uniformly
            if let ExprNode::New(cls, args) = &expr.node {
                collect_free_vars_expr(cls, bound, out);
                for a in args { collect_free_vars_expr(a, bound, out); }
            } else {
                for i in items { collect_free_vars_expr(i, bound, out); }
            }
        }
        ExprNode::Obj(pairs) => {
            for (_, v) in pairs { collect_free_vars_expr(v, bound, out); }
        }
        // FuncExpr: collect free vars of the *inner* lambda relative to *its* params,
        // then anything that inner lambda captures from OUR scope is a free var for us too
        ExprNode::FuncExpr(params, _, body) => {
            let mut inner_bound = bound.clone();
            for (p, _) in params {
                let pname = p.split(':').next().unwrap_or(p).trim();
                let actual = if pname.starts_with("...") { &pname[3..] } else { pname };
                inner_bound.insert(mangle(actual));
            }
            collect_free_vars_stmt(body, &inner_bound, out);
        }
        _ => {}
    }
}

fn hoist_top_level(ast: &Stmt, t: &mut Transpiler, base_dir: &PathBuf) {
    match &ast.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { hoist_top_level(s, t, base_dir); }
        }
        StmtNode::FuncDef(name, params, _req, body, _) => {
            t.emit_named_fn(&mangle(name), params, body);
        }
        StmtNode::ClassDef(name, _parent, body) => {
            if let StmtNode::Block(stmts) = &body.node {
                for s in stmts {
                    if let StmtNode::FuncDef(mname, params, _req, mbody, _) = &s.node {
                        let fn_c = format!("{}_{}", mangle(name), mangle(mname));
                        let mut all_params = vec![("this".to_string(), None)];
                        all_params.extend(params.iter().cloned());
                        t.emit_named_fn(&fn_c, &all_params, mbody);
                    }
                }
            }
        }
        // ── NEW: hoist functions from user .dgn modules ──────────────────
        StmtNode::Import(module, _alias) if !is_builtin_module(module) => {
            if !t.hoisted_modules.contains(module) {
                if let Some(mod_ast) = load_user_module(module, base_dir) {
                    hoist_module_decls(module, &mod_ast, t, base_dir);
                }
                t.hoisted_modules.insert(module.to_string());
            }
        }
        _ => {}
    }
}

/// Resolve and parse a user .dgn module file. Returns None if not found.
fn load_user_module(module: &str, base_dir: &PathBuf) -> Option<Stmt> {
    let candidates = [
        base_dir.join(format!("{}.dgn", module)),
        base_dir.join("dgn_modules").join(format!("{}.dgn", module)),
    ];
    for path in &candidates {
        if path.exists() {
            if let Ok(src) = fs::read_to_string(path) {
                if let Ok(tokens) = lex(&src) {
                    if let Ok(ast) = Parser::new(tokens).parse() {
                        return Some(ast);
                    }
                }
            }
        }
    }
    None
}

/// Hoist all functions from a user module with namespaced C names:
/// square() in math_tools → _fn__v_math_tools__v_square
fn hoist_module_decls(module: &str, ast: &Stmt, t: &mut Transpiler, base_dir: &PathBuf) {

    let stmts = match &ast.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => s,
        _ => return,
    };

    let mod_prefix = format!("_mod_{}", mangle(module));

    // 1. Collect module-scope names: vars, consts, imports, AND functions
    let mut mod_vars: Vec<(String, String)> = Vec::new();
    for s in stmts {
        match &s.node {
            StmtNode::FuncDef(fname, _, _, _, _) => {                    // ← FIX 3
                let m = mangle(fname);
                if !mod_vars.iter().any(|(k,_)| k == &m) {
                    mod_vars.push((m, fname.clone()));
                }
            }
            StmtNode::ConstDecl(name, _) => {
                mod_vars.push((mangle(name), name.clone()));
            }
            StmtNode::Import(dep, alias) => {
                let bind = alias.as_deref().unwrap_or(dep.as_str());
                mod_vars.push((mangle(bind), bind.to_string()));
            }
            StmtNode::Expr(e) => {
                if let ExprNode::Assign(target, _) = &e.node {
                    if let ExprNode::Var(name) = &target.node {
                        let m = mangle(name);
                        if !mod_vars.iter().any(|(k,_)| k == &m) {
                            mod_vars.push((m, name.clone()));
                        }
                    }
                }
            }
            _ => {}
        }
    }

     // 1b. PRE-PASS: recursively hoist user sub-modules BEFORE emitting anything
    //     for this module. This ensures sub-modules complete their full
    //     #define → function bodies → #undef cycle before this module's
    //     step 3 (#define) runs, preventing macro redefinition warnings.
    //     Also resolves sub-modules relative to the containing module's directory
    //     (e.g. providers/ollama.dgn when processing providers/router.dgn).
    let module_dir = base_dir.join(
        std::path::Path::new(module).parent().unwrap_or(std::path::Path::new(""))
    );
    for s in stmts {
        if let StmtNode::Import(dep, _) = &s.node {
            if !is_builtin_module(dep) && !t.hoisted_modules.contains(dep.as_str()) {
                let sub_ast = load_user_module(dep, &module_dir)
                    .or_else(|| load_user_module(dep, base_dir));
                if let Some(ast) = sub_ast {
                    let sub_base = base_dir.join(
                        std::path::Path::new(dep).parent().unwrap_or(std::path::Path::new(""))
                    );
                    let sub_base = if sub_base == *base_dir { module_dir.clone() } else { sub_base };
                    hoist_module_decls(dep, &ast, t, &sub_base);
                }
                t.hoisted_modules.insert(dep.to_string());
            }
        }
    }

    // 2. Emit static storage for every module-scope name
    for (var, _) in &mod_vars {
        writeln!(t.hoisted, "static DgnVal {}_{};", mod_prefix, var).unwrap();
    }

    // 3. #define short names → namespaced statics
    writeln!(t.hoisted, "/* {}: map short names → namespaced statics */", module).unwrap();
    for (var, _) in &mod_vars {
        writeln!(t.hoisted, "#define {} {}_{}", var, mod_prefix, var).unwrap();
    }
    
    t.push_scope();
    if let Some(scope) = t.declared.last_mut() {
        for (var, _) in &mod_vars {
            scope.insert(var.clone());
        }
    }
    for s in stmts {
        if let StmtNode::FuncDef(fname, params, _req, body, _) = &s.node {
            let fn_c = format!("{}_{}", mangle(module), mangle(fname));
            t.emit_named_fn(&fn_c, params, body);

            let wrap_c = format!("_wrap_{}", fn_c);
            writeln!(
                t.hoisted,
                "static DgnVal {}(DgnVal* _args, int _nargs, DgnEnv* _env) \
                 {{ return _fn_{}(_nargs > 0 ? _args+1 : _args, _nargs > 0 ? _nargs-1 : 0, _env); }}",
                wrap_c, fn_c
            ).unwrap();
        }
    }
    t.pop_scope();

    for s in stmts {
        if let StmtNode::Import(dep, alias) = &s.node {
            if !is_builtin_module(dep) && !t.hoisted_modules.contains(dep.as_str()) {
                if let Some(sub_ast) = load_user_module(dep, base_dir) {
                    hoist_module_decls(dep, &sub_ast, t, base_dir);
                }
                t.hoisted_modules.insert(dep.to_string());
            }
        }
    }
    // 5. Emit _init_<module>() BEFORE #undef so it can use the #defined names  ← FIX 1
    writeln!(t.hoisted, "static int _init_{}_done = 0;", mangle(module)).unwrap();
    writeln!(t.hoisted, "static void _init_{}(void) {{", mangle(module)).unwrap();
    writeln!(t.hoisted, "    if (_init_{}_done) return; _init_{}_done = 1;",
             mangle(module), mangle(module)).unwrap();

    for s in stmts {
        match &s.node {
            StmtNode::Import(dep, alias) => {
                let bind = alias.as_deref().unwrap_or(dep.as_str());
                let var = mangle(bind);
                if is_builtin_module(dep) {
                    writeln!(t.hoisted, "    {}_{} = dgn_load_module(\"{}\");",
                             mod_prefix, var, dep).unwrap();
                // AFTER (fixed):
} else {
                    // Sub-module was already hoisted in the pre-pass above.
                    // Build the module map inside _init_<module>().
                    writeln!(t.hoisted, "    {}_{} = dgn_map_new();", mod_prefix, var).unwrap();
                    writeln!(t.hoisted, "    _init_{}();", mangle(dep)).unwrap();
                    if let Some(sub_ast) = load_user_module(dep, base_dir) {
                        let sub_stmts = match &sub_ast.node {
                            StmtNode::Block(s) | StmtNode::Multi(s) => s.clone(),
                            _ => vec![],
                        };
                        for sub_s in &sub_stmts {
                            match &sub_s.node {
                                StmtNode::FuncDef(fname, params, _, _, _) => {
                                    let fn_c = format!("{}_{}", mangle(dep), mangle(fname));
                                    writeln!(t.hoisted,
                                        "    dgn_map_set(&{}_{}, \"{}\", dgn_func(_wrap_{}, {}));",
                                        mod_prefix, var, fname, fn_c, params.len() + 1
                                    ).unwrap();
                                }
                                StmtNode::ConstDecl(cname, expr) => {
                                    let val = match &expr.node {
                                        ExprNode::String(s) => format!("dgn_str(\"{}\")", escape_c_string(s)),
                                        ExprNode::Number(n) => {
                                            if n.fract() == 0.0 { format!("dgn_int({}LL)", *n as i64) }
                                            else { format!("dgn_float({:.17})", n) }
                                        }
                                        ExprNode::Bool(b) => format!("dgn_bool({})", if *b { 1 } else { 0 }),
                                        _ => format!("_mod_{}_{}", mangle(dep), mangle(cname)),
                                    };
                                    writeln!(t.hoisted,
                                        "    dgn_map_set(&{}_{}, \"{}\", {});",
                                        mod_prefix, var, cname, val
                                    ).unwrap();
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            StmtNode::ConstDecl(name, expr) => {
                let var = mangle(name);
                let (pre, val) = t.emit_expr(expr);
                for line in pre.lines() {
                    if !line.trim().is_empty() {
                        writeln!(t.hoisted, "    {}", line).unwrap();
                    }
                }
                writeln!(t.hoisted, "    {}_{} = {};", mod_prefix, var, val).unwrap();
            }
            StmtNode::Expr(e) => {
                if let ExprNode::Assign(target, rhs) = &e.node {
                    if let ExprNode::Var(name) = &target.node {
                        let var = mangle(name);
                        if mod_vars.iter().any(|(k,_)| k == &var) {
                            let (pre, val) = t.emit_expr(rhs);
                            for line in pre.lines() {
                                if !line.trim().is_empty() {
                                    writeln!(t.hoisted, "    {}", line).unwrap();
                                }
                            }
                            writeln!(t.hoisted, "    {}_{} = {};",
                                     mod_prefix, var, val).unwrap();
                        }
                    }
                }
            }
            StmtNode::FuncDef(fname, params, _, _, _) => {               // ← FIX 3
                let var = mangle(fname);
                let fn_c = format!("{}_{}", mangle(module), var);
                writeln!(t.hoisted,
                    "    {}_{} = dgn_func(_wrap_{}, {});",
                    mod_prefix, var, fn_c, params.len() + 1
                ).unwrap();
            }
            _ => {}
        }
    }
    writeln!(t.hoisted, "}}").unwrap();

    // 6. #undef AFTER _init_ closes                                      ← FIX 1
    for (var, _) in &mod_vars {
        writeln!(t.hoisted, "#undef {}", var).unwrap();
    }
}

/// Emit dgn_map_set calls to populate the module map from its exported symbols.
fn emit_module_map_entries(module: &str, bind: &str, ast: &Stmt, t: &mut Transpiler) {
    let stmts = match &ast.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => s,
        _ => return,
    };
    let indent = "    ".repeat(t.indent);
    let c_name = mangle(bind);

    // Run the module initializer first (idempotent — guarded by _done flag)
    writeln!(t.out, "{}    _init_{}();", indent, mangle(module)).unwrap();

    for s in stmts {
        match &s.node {
            StmtNode::FuncDef(fname, params, _req, _body, _) => {
                let fn_c = format!("{}_{}", mangle(module), mangle(fname));
                writeln!(
                    t.out,
                    "{}dgn_map_set(&{}, \"{}\", dgn_func(_wrap_{}, {}));",
                    indent, c_name, fname, fn_c, params.len() + 1
                ).unwrap();
            }
            StmtNode::ConstDecl(cname, expr) => {
                let val = match &expr.node {
                    ExprNode::String(s) => format!("dgn_str(\"{}\")", escape_c_string(s)),
                    ExprNode::Number(n) => {
                        if n.fract() == 0.0 { format!("dgn_int({}LL)", *n as i64) }
                        else { format!("dgn_float({:.17})", n) }
                    }
                    ExprNode::Bool(b) => format!("dgn_bool({})", if *b { 1 } else { 0 }),
                    _ => {
                        // Non-literal const: read from the namespaced static after init
                        format!("_mod_{}_{}", mangle(module), mangle(cname))
                    }
                };
                writeln!(t.out, "{}dgn_map_set(&{}, \"{}\", {});",
                         indent, c_name, cname, val).unwrap();
            }
            _ => {}
        }
    }
}
fn emit_top_level_body(ast: &Stmt, t: &mut Transpiler, base_dir: &PathBuf) {
    match &ast.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { emit_top_level_body(s, t, base_dir); }
        }
        StmtNode::FuncDef(name, params, _req, _body, _) => {
            let c_name = mangle(name);
            let decl = t.decl_var(name);
            let indent = "    ".repeat(t.indent);
            writeln!(t.out, "{}{} = dgn_func(_fn_{}, {});", indent, decl, c_name, params.len()).unwrap();
        }
        StmtNode::ClassDef(name, parent, body) => {
            let indent = "    ".repeat(t.indent);
            let decl = t.decl_var(name);
            writeln!(t.out, "{}{} = dgn_class_new(\"{}\", {});", indent, decl, escape_c_string(name), parent.as_deref().map(|p| format!("\"{}\"", escape_c_string(p))).unwrap_or_else(|| "NULL".into())).unwrap();
            if let StmtNode::Block(stmts) = &body.node {
                for s in stmts {
                    if let StmtNode::FuncDef(mname, params, _req, _, _) = &s.node {
                        let fn_c = format!("{}_{}", mangle(name), mangle(mname));
                        let c_name = mangle(name);
                        writeln!(t.out, "{}dgn_class_add_method(&{}, \"{}\", _fn_{}, {});", indent, c_name, escape_c_string(mname), fn_c, params.len()).unwrap();
                    }
                }
            }
            if let Some(p) = parent {
                let c_name = mangle(name);
                writeln!(t.out, "{}dgn_class_set_parent(&{}, {});", indent, c_name, mangle(p)).unwrap();
            }
        }
        StmtNode::Import(module, alias) if !is_builtin_module(module) => {
            let bind = alias.as_deref().unwrap_or(module.as_str());
            let c_name = mangle(bind);
            let indent = "    ".repeat(t.indent);
            writeln!(t.out, "{}{} = dgn_map_new();", indent, c_name).unwrap();
            if let Some(mod_ast) = load_user_module(module, base_dir) {
                emit_module_map_entries(module, bind, &mod_ast, t);  // already calls _init_
            }
            writeln!(t.out, "{}(void){};", indent, c_name).unwrap();
        }
        other_stmt => {
            t.emit_stmt(&Stmt { node: other_stmt.clone(), line: 0 });
        }
    }
}

