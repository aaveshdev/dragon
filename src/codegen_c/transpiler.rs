#![allow(dead_code)]

use super::*;

fn collect_field_defaults(s: &Stmt, out: &mut Vec<Stmt>) {
    match &s.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for st in stmts { collect_field_defaults(st, out); }
        }
        StmtNode::FuncDef(..) => { }
        StmtNode::Expr(e) => {
            if let ExprNode::Assign(target, value) = &e.node {
                if let ExprNode::Var(name) = &target.node {
                    let this_expr = Expr { node: ExprNode::This, line: e.line };
                    let new_target = Expr {
                        node: ExprNode::GetAttr(Box::new(this_expr), name.clone()),
                        line: target.line,
                    };
                    let new_assign = Expr {
                        node: ExprNode::Assign(Box::new(new_target), value.clone()),
                        line: e.line,
                    };
                    out.push(Stmt { node: StmtNode::Expr(new_assign), line: s.line });
                    return;
                }
            }
            out.push(s.clone());
        }
        _ => { out.push(s.clone()); }
    }
}

struct Transpiler {
    out: String,
    hoisted: String,
    indent: usize,
    tmp_counter: usize,
    lbl_counter: usize,
    declared: Vec<std::collections::HashSet<String>>,
    loop_ctx: Vec<(Option<String>, String)>,
    try_frames: Vec<String>,

    finally_at: Vec<(usize, Stmt)>,
    loop_try_base: Vec<usize>,
    closure_vars: std::collections::HashSet<String>,
    boxed_vars: std::collections::HashSet<String>,
    closure_boxed: std::collections::HashSet<String>,
    box_candidates: std::collections::HashSet<String>,
    shadow_stack: Vec<std::collections::HashSet<String>>,
    hoisted_modules: std::collections::HashSet<String>,
    local_names: Vec<std::collections::HashSet<String>>,
    direct_callable: std::collections::HashSet<String>,
    numeric_callable: std::collections::HashSet<String>,
    numeric_params: std::collections::HashMap<String, Vec<String>>,
    numeric_callable_float: std::collections::HashMap<String, FnSig>,
    numeric_params_float: std::collections::HashMap<String, Vec<String>>,
    classes: std::collections::HashMap<String, ClassInfo>,
    overridable_methods: std::collections::HashSet<(String, String)>,
    current_class: Option<String>,
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
            try_frames: Vec::new(),
            finally_at: Vec::new(),
            loop_try_base: Vec::new(),
            closure_vars: std::collections::HashSet::new(),
            boxed_vars: std::collections::HashSet::new(),
            closure_boxed: std::collections::HashSet::new(),
            box_candidates: std::collections::HashSet::new(),
            shadow_stack: Vec::new(),
            hoisted_modules: std::collections::HashSet::new(),
            local_names: vec![std::collections::HashSet::new()],
            direct_callable: std::collections::HashSet::new(),
            numeric_callable: std::collections::HashSet::new(),
            numeric_params: std::collections::HashMap::new(),
            numeric_callable_float: std::collections::HashMap::new(),
            numeric_params_float: std::collections::HashMap::new(),
            classes: std::collections::HashMap::new(),
            overridable_methods: std::collections::HashSet::new(),
            current_class: None,
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
        self.local_names.push(std::collections::HashSet::new());
    }
    fn pop_scope(&mut self) {
        self.declared.pop();
        self.local_names.pop();
    }

    fn is_declared(&self, c_name: &str) -> bool {
        self.declared.iter().any(|scope| scope.contains(c_name))
    }

    fn user_defines(&self, name: &str) -> bool {
        let c = mangle(name);
        self.direct_callable.contains(&c) || self.is_shadowed_locally(&c) || self.is_declared(&c)
    }

    fn is_shadowed_locally(&self, c_name: &str) -> bool {
        self.local_names.iter().skip(1).any(|scope| scope.contains(c_name))
    }

    fn var_read(&mut self, c_name: &str) -> (String, String) {
        if self.closure_vars.contains(c_name) {
            let tmp = self.fresh_tmp();
            let indent = "    ".repeat(self.indent);
            let mut pre = String::new();
            if self.closure_boxed.contains(c_name) {
                writeln!(pre, "{}DgnVal {} = (_cenv) ? dgn_box_get(dgn_map_get_raw(_cenv, \"{}\")) : dgn_null();", indent, tmp, c_name).unwrap();
            } else {
                writeln!(pre, "{}DgnVal {} = (_cenv) ? dgn_map_get_raw(_cenv, \"{}\") : dgn_null();", indent, tmp, c_name).unwrap();
            }
            (pre, tmp)
        } else if self.boxed_vars.contains(c_name) {
            (String::new(), format!("dgn_box_get({})", c_name))
        } else {
            (String::new(), c_name.to_string())
        }
    }

    fn var_store(&self, c_name: &str, val: &str) -> String {
        if self.closure_vars.contains(c_name) {
            if self.closure_boxed.contains(c_name) {
                format!("if (_cenv) dgn_box_set(dgn_map_get_raw(_cenv, \"{}\"), {});", c_name, val)
            } else {
                format!("if (_cenv) dgn_map_set_raw(_cenv, \"{}\", {});", c_name, val)
            }
        } else if self.boxed_vars.contains(c_name) {
            format!("dgn_box_set({}, {});", c_name, val)
        } else {
            format!("{} = {};", c_name, val)
        }
    }

    fn is_global_name(&self, c_name: &str) -> bool {
        self.declared.first().map_or(false, |g| g.contains(c_name))
            && !self.shadow_stack.iter().any(|s| s.contains(c_name))
    }

    fn fn_enter(&mut self, params: &[(String, Option<String>)], body: &Stmt)
        -> (std::collections::HashSet<String>, std::collections::HashSet<String>, std::collections::HashSet<String>, std::collections::HashSet<String>)
    {
        let saved = (
            std::mem::take(&mut self.closure_vars),
            std::mem::take(&mut self.closure_boxed),
            std::mem::take(&mut self.boxed_vars),
            std::mem::take(&mut self.box_candidates),
        );
        let mut shadow = std::collections::HashSet::new();
        for (p, _) in params.iter() {
            let pname = p.split(':').next().unwrap_or(p).trim();
            let actual = if pname.starts_with("...") { &pname[3..] } else { pname };
            shadow.insert(mangle(actual.trim()));
        }
        self.shadow_stack.push(shadow);
        let mut mutated = std::collections::HashSet::new();
        collect_all_mutated_names(body, &mut mutated);
        let captured = closure_captured_names(body);
        self.box_candidates = mutated.intersection(&captured).cloned().collect();
        saved
    }

    fn fn_leave(&mut self, saved: (std::collections::HashSet<String>, std::collections::HashSet<String>, std::collections::HashSet<String>, std::collections::HashSet<String>)) {
        self.shadow_stack.pop();
        self.closure_vars = saved.0;
        self.closure_boxed = saved.1;
        self.boxed_vars = saved.2;
        self.box_candidates = saved.3;
    }

    fn box_params(&mut self, params: &[(String, Option<String>)]) {
        let indent = "    ".repeat(self.indent);
        for (p, _) in params.iter() {
            let pname = p.split(':').next().unwrap_or(p).trim();
            let actual = if pname.starts_with("...") { &pname[3..] } else { pname };
            let c = mangle(actual.trim());
            if self.box_candidates.contains(&c) && !self.closure_vars.contains(&c) {
                writeln!(self.out, "{}{} = dgn_box_new({});", indent, c, c).unwrap();
                self.boxed_vars.insert(c);
            }
        }
    }

    fn is_cell_var(&self, c_name: &str) -> bool {
        self.closure_vars.contains(c_name) || self.boxed_vars.contains(c_name)
    }

    fn decl_var(&mut self, name: &str) -> String {
        let c_name = mangle(name);
        if let Some(current) = self.local_names.last_mut() {
            current.insert(c_name.clone());
        }
        if !self.is_declared(&c_name) {
            if let Some(current) = self.declared.last_mut() {
                current.insert(c_name.clone());
                self.boxed_vars.remove(&c_name);
                return format!("DgnVal {}", c_name);
            }
        }
        c_name
    }

    fn try_emit_strbuf_loop(&mut self, stmts: &[Stmt], i: usize) -> Option<usize> {
        let seed_stmt = &stmts[i];

        let (name, seed_lit): (String, String) = match &seed_stmt.node {
            StmtNode::ConstDecl(n, e) => match &e.node {
                ExprNode::String(s) => (n.clone(), s.clone()),
                _ => return None,
            },
            StmtNode::Expr(e) => match &e.node {
                ExprNode::Assign(t, v) => match (&t.node, &v.node) {
                    (ExprNode::Var(n), ExprNode::String(s)) => (n.clone(), s.clone()),
                    _ => return None,
                },
                _ => return None,
            },
            _ => return None,
        };

        let mut loop_idx = i + 1;
        while loop_idx < stmts.len() {
            match &stmts[loop_idx].node {
                StmtNode::While(_, _) | StmtNode::DoWhile(_, _) | StmtNode::CFor(_, _, _, _) => break,
                _ => {
                    if stmt_references_var(&stmts[loop_idx], &name) { return None; }
                    loop_idx += 1;
                }
            }
        }
        if loop_idx >= stmts.len() { return None; }
        let loop_stmt = &stmts[loop_idx];

        enum LoopShape<'a> { While(&'a Expr, &'a Stmt), DoWhile(&'a Stmt, &'a Expr), CFor(&'a Option<Expr>, &'a Option<Expr>, &'a Option<Expr>, &'a Stmt) }
        let shape = match &loop_stmt.node {
            StmtNode::While(c, b) => LoopShape::While(c, b),
            StmtNode::DoWhile(b, c) => LoopShape::DoWhile(b, c),
            StmtNode::CFor(init, c, step, b) => LoopShape::CFor(init, c, step, b),
            _ => return None,
        };
        let (cond_refs, body): (bool, &Stmt) = match &shape {
            LoopShape::While(c, b) => (expr_references_var(c, &name), b),
            LoopShape::DoWhile(b, c) => (expr_references_var(c, &name), b),
            LoopShape::CFor(init, c, step, b) => (
                init.as_ref().map_or(false, |e| expr_references_var(e, &name))
                    || c.as_ref().map_or(false, |e| expr_references_var(e, &name))
                    || step.as_ref().map_or(false, |e| expr_references_var(e, &name)),
                b,
            ),
        };
        if cond_refs { return None; }

        let body_stmts: Vec<Stmt> = match &body.node {
            StmtNode::Block(s) | StmtNode::Multi(s) => s.clone(),
            _ => vec![(*body).clone()],
        };
        let (accum_idx, pieces) = find_and_validate_accumulator(&body_stmts, &name)?;

        self.emit_stmt(seed_stmt);
        for s in &stmts[i + 1..loop_idx] {
            self.emit_stmt(s);
        }
        let buf = self.fresh_tmp();
        let ind = "    ".repeat(self.indent);
        writeln!(self.out, "{}DgnStrBuf {}; dgn_strbuf_init(&{}, \"{}\");", ind, buf, buf, escape_c_string(&seed_lit)).unwrap();

        let emit_rewritten_body = |t: &mut Self| {
            t.line("{");
            t.indent += 1;
            t.push_scope();
            for (j, s) in body_stmts.iter().enumerate() {
                if j == accum_idx {
                    for piece in &pieces {
                        let (ppre, pval) = t.emit_expr(piece);
                        t.raw(&ppre);
                        let ind2 = "    ".repeat(t.indent);
                        writeln!(t.out, "{}dgn_strbuf_append_val(&{}, {});", ind2, buf, pval).unwrap();
                    }
                } else {
                    t.emit_stmt(s);
                }
            }
            t.pop_scope();
            t.indent -= 1;
            t.line("}");
        };

        match &shape {
            LoopShape::While(c, _) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_try_base.push(self.try_frames.len()); self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

                let tmp = self.fresh_tmp();
                let (pre, cval) = self.emit_expr(c);
                self.raw(&pre);
                let indent = "    ".repeat(self.indent);
                writeln!(self.out, "{}DgnVal {} = {};", indent, tmp, cval).unwrap();
                writeln!(self.out, "{}while (dgn_truthy({})) {{", indent, tmp).unwrap();
                self.indent += 1;
                self.push_scope();

                emit_rewritten_body(self);

                self.indent -= 1;
                writeln!(self.out, "    {}{}:;", indent, cont_lbl).unwrap();
                self.indent += 1;

                let (pre2, cval2) = self.emit_expr(c);
                self.raw(&pre2);
                let indent2 = "    ".repeat(self.indent);
                writeln!(self.out, "{}{} = {};", indent2, tmp, cval2).unwrap();
                self.pop_scope();
                self.indent -= 1;
                self.line("}");

                writeln!(self.out, "{}{}:;", indent, brk_lbl).unwrap();
                self.loop_ctx.pop(); self.loop_try_base.pop();
            }
            LoopShape::DoWhile(_, c) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_try_base.push(self.try_frames.len()); self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

                self.line("do {");
                self.indent += 1;
                self.push_scope();

                emit_rewritten_body(self);

                self.pop_scope();
                self.indent -= 1;
                let indent = "    ".repeat(self.indent);
                writeln!(self.out, "    {}{}:;", indent, cont_lbl).unwrap();

                let (pre, cval) = self.emit_expr(c);
                self.raw(&pre);
                writeln!(self.out, "{}}} while (dgn_truthy({}));", indent, cval).unwrap();
                writeln!(self.out, "{}{}:;", indent, brk_lbl).unwrap();
                self.loop_ctx.pop(); self.loop_try_base.pop();
            }
            LoopShape::CFor(init, cond, step, _) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_try_base.push(self.try_frames.len()); self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

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

                    emit_rewritten_body(self);

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

                    emit_rewritten_body(self);

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
                self.loop_ctx.pop(); self.loop_try_base.pop();
            }
        }

        let c_name = mangle(&name);
        let ind_final = "    ".repeat(self.indent);
        writeln!(self.out, "{}{} = dgn_strbuf_to_dgnval(&{});", ind_final, c_name, buf).unwrap();

        Some(loop_idx - i + 1)
    }

    fn emit_stmt(&mut self, stmt: &Stmt) {
        match &stmt.node {
            StmtNode::Block(stmts) => {
                self.line("{");
                self.indent += 1;
                self.push_scope();
                let mut i = 0;
                while i < stmts.len() {
                    if let Some(consumed) = self.try_emit_strbuf_loop(stmts, i) {
                        i += consumed;
                        continue;
                    }
                    self.emit_stmt(&stmts[i]);
                    i += 1;
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
                None => {
                    self.unwind_try_frames(0);
                    self.line("return dgn_null();")
                }
                Some(e) => {
                    let (pre, val) = self.emit_expr(e);
                    self.raw(&pre);
                    let indent = "    ".repeat(self.indent);
                    if self.try_frames.is_empty() {
                        writeln!(self.out, "{}return {};", indent, val).unwrap();
                    } else {
                        let r = self.fresh_tmp();
                        writeln!(self.out, "{}DgnVal {} = {};", indent, r, val).unwrap();
                        self.unwind_try_frames(0);
                        let indent = "    ".repeat(self.indent);
                        writeln!(self.out, "{}return {};", indent, r).unwrap();
                    }
                }
            },
            StmtNode::Break => {
                self.pop_try_frames_to_loop();
                if let Some((_, brk_lbl)) = self.loop_ctx.last() {
                    let indent = "    ".repeat(self.indent);
                    writeln!(self.out, "{}goto {};", indent, brk_lbl).unwrap();
                } else {
                    self.line("break;");
                }
            }
            StmtNode::Continue => {
                self.pop_try_frames_to_loop();
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

                if self.numeric_callable.contains(&c_name) {
                    let saved_indent = self.indent;
                    self.indent = 0;
                    let saved_out = std::mem::take(&mut self.out);

                    let pnames = self.numeric_params.get(&c_name).cloned().unwrap_or_default();

                    let sig = if pnames.is_empty() {
                        "void".to_string()
                    } else {
                        pnames.iter().map(|p| format!("int64_t {}", mangle(p))).collect::<Vec<_>>().join(", ")
                    };
                    writeln!(self.out, "static int64_t _fnum_{}({}) {{", c_name, sig).unwrap();
                    self.indent = 1;
                    let mut it = IntSpecTranspiler::new();
                    it.indent = self.indent;
                    it.emit_body(body);
                    self.out.push_str(&it.out);
                    self.indent = 0;
                    self.line("}");
                    self.line("");

                    writeln!(
                        self.out,
                        "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
                        c_name
                    ).unwrap();
                    self.indent = 1;
                    for (i, pname) in pnames.iter().enumerate() {
                        let ind = "    ".repeat(self.indent);
                        writeln!(self.out, "{}int64_t {} = (_nargs > {}) ? _args[{}].i : 0;", ind, mangle(pname), i, i).unwrap();
                    }
                    let ind = "    ".repeat(self.indent);
                    let call_args: Vec<String> = pnames.iter().map(|p| mangle(p)).collect();
                    writeln!(self.out, "{}return dgn_int(_fnum_{}({}));", ind, c_name, call_args.join(", ")).unwrap();
                    self.indent = 0;
                    self.line("}");
                    self.line("");

                    let generated_fn = std::mem::take(&mut self.out);
                    self.out = saved_out;
                    self.hoisted.push_str(&generated_fn);
                    self.indent = saved_indent;

                    let decl = self.decl_var(name);
                    let indent = "    ".repeat(self.indent);
                    writeln!(
                        self.out,
                        "{}{} = dgn_func_n(_fn_{}, {}, \"{}\");",
                        indent, decl, c_name, params.len(), param_csv(params)
                    ).unwrap();
                    return;
                }

                {
                    let saved_indent = self.indent;
                    self.indent = 0;
                    let mut saved_out = std::mem::take(&mut self.out);
                    let saved_tf = std::mem::take(&mut self.try_frames);
                    let saved_lt = std::mem::take(&mut self.loop_try_base);
                    let saved_fa = std::mem::take(&mut self.finally_at);

                    writeln!(
                        self.out,
                        "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
                        c_name
                    ).unwrap();
                    self.indent = 1;
                    self.push_scope();

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

                    let elide_self_binding = self.direct_callable.contains(&c_name)
                        && !stmt_shadows_name(body, &c_name)
                        && !body_uses_name_as_value(body, &c_name);
                    {
                        let ind = "    ".repeat(self.indent);
                        let self_decl = self.decl_var(name);
                        if elide_self_binding {
                            writeln!(self.out, "{}{} = dgn_null();  /* self-binding elided: never read as a value, only called directly */", ind, self_decl).unwrap();
                            writeln!(self.out, "{}(void){};", ind, c_name).unwrap();
                        } else {
                            writeln!(self.out, "{}{} = dgn_func_n(_fn_{}, {}, \"{}\");", ind, self_decl, c_name, params.len(), param_csv(params)).unwrap();
                        }
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
                    self.try_frames = saved_tf;
                    self.loop_try_base = saved_lt;
                    self.finally_at = saved_fa;
                }

                let decl = self.decl_var(name);
                let indent = "    ".repeat(self.indent);
                writeln!(
                    self.out,
                    "{}{} = dgn_func_n(_fn_{}, {}, \"{}\");",
                    indent, decl, c_name, params.len(), param_csv(params)
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
                self.loop_try_base.push(self.try_frames.len()); self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

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
                self.loop_ctx.pop(); self.loop_try_base.pop();
            }
            StmtNode::DoWhile(body, cond) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_try_base.push(self.try_frames.len()); self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

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
                self.loop_ctx.pop(); self.loop_try_base.pop();
            }
            StmtNode::CFor(init, cond, step, body) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_try_base.push(self.try_frames.len()); self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

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
                self.loop_ctx.pop(); self.loop_try_base.pop();
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
                (String::new(), format!("dgn_lit(\"{}\")", escaped))
            }
            ExprNode::This => {
                let c_name = mangle("this");
                self.var_read(&c_name)
            }
            ExprNode::Var(name) => {
                let c_name = mangle(name);
                self.var_read(&c_name)
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
                    "!" | "not" => format!("dgn_not({})", val),
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
                let (mut vpre, mut vval) = self.emit_expr(value);
                if vval.contains('(') {
                    let t = self.fresh_tmp();
                    let ind = "    ".repeat(self.indent);
                    writeln!(vpre, "{}DgnVal {} = {};", ind, t, vval).unwrap();
                    vval = t;
                }
                match &target.node {
                    ExprNode::Var(name) => {
                        let c_name = mangle(name);
                        let indent = "    ".repeat(self.indent);
                        let mut pre = vpre;
                        if self.closure_vars.contains(&c_name) {
                            let st = self.var_store(&c_name, &vval);
                            writeln!(pre, "{}{}", indent, st).unwrap();
                            (pre, vval)
                        } else {
                            let decl = self.decl_var(name);
                            if decl.starts_with("DgnVal ") && self.box_candidates.contains(&c_name) {
                                self.boxed_vars.insert(c_name.clone());
                            }
                            if self.boxed_vars.contains(&c_name) {
                                if decl.starts_with("DgnVal ") {
                                    writeln!(pre, "{}{} = dgn_box_new({});", indent, decl, vval).unwrap();
                                } else {
                                    writeln!(pre, "{}dgn_box_set({}, {});", indent, c_name, vval).unwrap();
                                }
                                (pre, vval)
                            } else {
                                writeln!(pre, "{}{} = {};", indent, decl, vval).unwrap();
                                (pre, c_name.clone())
                            }
                        }
                    }
                    ExprNode::GetAttr(obj, attr) => {
                        let (opre, oval) = self.emit_expr(obj);
                        let mut pre = opre;
                        pre.push_str(&vpre);
                        let indent = "    ".repeat(self.indent);
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

                    ExprNode::List(items) => {
                        let mut pre = vpre;
                        let rhs_tmp = self.fresh_tmp();
                        let indent = "    ".repeat(self.indent);

                        writeln!(pre, "{}DgnVal {} = {};", indent, rhs_tmp, vval).unwrap();

                        for (i, item) in items.iter().enumerate() {
                            let mut var_name = String::new();
                            let mut current_node = &item.node;

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
                                break;
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
                    if self.is_cell_var(&c_name) {
                        let tmp = self.fresh_tmp();
                        let (rp, rv) = self.var_read(&c_name);
                        pre.push_str(&rp);
                        let op_fn = if is_inc { "dgn_add" } else { "dgn_sub" };
                        writeln!(pre, "{}DgnVal {} = {}({}, dgn_int(1));", indent, tmp, op_fn, rv).unwrap();
                        let st = self.var_store(&c_name, &tmp);
                        writeln!(pre, "{}{}", indent, st).unwrap();
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
                ExprNode::GetAttr(obj, attr) => {
                    let (opre, oval) = self.emit_expr(obj);
                    let is_inc = op.contains('+') || op.to_lowercase().contains("inc");
                    let indent = "    ".repeat(self.indent);
                    let mut pre = opre;
                    let is_call = oval.contains('(');
                    let obj_lval = if is_call {
                        let tmp = self.fresh_tmp();
                        writeln!(pre, "{}DgnVal {} = {};", indent, tmp, oval).unwrap();
                        tmp
                    } else { oval };
                    let old_tmp = self.fresh_tmp();
                    writeln!(pre, "{}DgnVal {} = dgn_get_attr({}, \"{}\");", indent, old_tmp, obj_lval, escape_c_string(attr)).unwrap();
                    let new_tmp = self.fresh_tmp();
                    let op_fn = if is_inc { "dgn_add" } else { "dgn_sub" };
                    writeln!(pre, "{}DgnVal {} = {}({}, dgn_int(1));", indent, new_tmp, op_fn, old_tmp).unwrap();
                    writeln!(pre, "{}dgn_set_attr(&{}, \"{}\", {});", indent, obj_lval, escape_c_string(attr), new_tmp).unwrap();
                    (pre, new_tmp)
                }
                ExprNode::Index(obj, idx) => {
                    let (opre, oval) = self.emit_expr(obj);
                    let (ipre, ival) = self.emit_expr(idx);
                    let is_inc = op.contains('+') || op.to_lowercase().contains("inc");
                    let indent = "    ".repeat(self.indent);
                    let mut pre = opre;
                    pre.push_str(&ipre);
                    let is_call = oval.contains('(');
                    let obj_lval = if is_call {
                        let tmp = self.fresh_tmp();
                        writeln!(pre, "{}DgnVal {} = {};", indent, tmp, oval).unwrap();
                        tmp
                    } else { oval };
                    let idx_tmp = self.fresh_tmp();
                    writeln!(pre, "{}DgnVal {} = {};", indent, idx_tmp, ival).unwrap();
                    let old_tmp = self.fresh_tmp();
                    writeln!(pre, "{}DgnVal {} = dgn_get_index({}, {});", indent, old_tmp, obj_lval, idx_tmp).unwrap();
                    let new_tmp = self.fresh_tmp();
                    let op_fn = if is_inc { "dgn_add" } else { "dgn_sub" };
                    writeln!(pre, "{}DgnVal {} = {}({}, dgn_int(1));", indent, new_tmp, op_fn, old_tmp).unwrap();
                    writeln!(pre, "{}dgn_set_index(&{}, {}, {});", indent, obj_lval, idx_tmp, new_tmp).unwrap();
                    (pre, new_tmp)
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
                    if self.is_cell_var(&c_name) {
                        let (rp, rv) = self.var_read(&c_name);
                        pre.push_str(&rp);
                        writeln!(pre, "{}DgnVal {} = {};", indent, tmp, rv).unwrap();
                        let new_tmp = self.fresh_tmp();
                        let op_fn = if is_inc { "dgn_add" } else { "dgn_sub" };
                        writeln!(pre, "{}DgnVal {} = {}({}, dgn_int(1));", indent, new_tmp, op_fn, tmp).unwrap();
                        let st = self.var_store(&c_name, &new_tmp);
                        writeln!(pre, "{}{}", indent, st).unwrap();
                        (pre, tmp)
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
                ExprNode::GetAttr(obj, attr) => {
                    let (opre, oval) = self.emit_expr(obj);
                    let is_inc = op.contains('+') || op.to_lowercase().contains("inc");
                    let indent = "    ".repeat(self.indent);
                    let mut pre = opre;
                    let is_call = oval.contains('(');
                    let obj_lval = if is_call {
                        let tmp = self.fresh_tmp();
                        writeln!(pre, "{}DgnVal {} = {};", indent, tmp, oval).unwrap();
                        tmp
                    } else { oval };
                    let old_tmp = self.fresh_tmp();
                    writeln!(pre, "{}DgnVal {} = dgn_get_attr({}, \"{}\");", indent, old_tmp, obj_lval, escape_c_string(attr)).unwrap();
                    let new_tmp = self.fresh_tmp();
                    let op_fn = if is_inc { "dgn_add" } else { "dgn_sub" };
                    writeln!(pre, "{}DgnVal {} = {}({}, dgn_int(1));", indent, new_tmp, op_fn, old_tmp).unwrap();
                    writeln!(pre, "{}dgn_set_attr(&{}, \"{}\", {});", indent, obj_lval, escape_c_string(attr), new_tmp).unwrap();
                    (pre, old_tmp)
                }
                ExprNode::Index(obj, idx) => {
                    let (opre, oval) = self.emit_expr(obj);
                    let (ipre, ival) = self.emit_expr(idx);
                    let is_inc = op.contains('+') || op.to_lowercase().contains("inc");
                    let indent = "    ".repeat(self.indent);
                    let mut pre = opre;
                    pre.push_str(&ipre);
                    let is_call = oval.contains('(');
                    let obj_lval = if is_call {
                        let tmp = self.fresh_tmp();
                        writeln!(pre, "{}DgnVal {} = {};", indent, tmp, oval).unwrap();
                        tmp
                    } else { oval };
                    let idx_tmp = self.fresh_tmp();
                    writeln!(pre, "{}DgnVal {} = {};", indent, idx_tmp, ival).unwrap();
                    let old_tmp = self.fresh_tmp();
                    writeln!(pre, "{}DgnVal {} = dgn_get_index({}, {});", indent, old_tmp, obj_lval, idx_tmp).unwrap();
                    let new_tmp = self.fresh_tmp();
                    let op_fn = if is_inc { "dgn_add" } else { "dgn_sub" };
                    writeln!(pre, "{}DgnVal {} = {}({}, dgn_int(1));", indent, new_tmp, op_fn, old_tmp).unwrap();
                    writeln!(pre, "{}dgn_set_index(&{}, {}, {});", indent, obj_lval, idx_tmp, new_tmp).unwrap();
                    (pre, old_tmp)
                }
                _ => {
                    let (pre, val) = self.emit_expr(operand);
                    (pre, val)
                }
            },
            ExprNode::FuncExpr(params, _req, body) => {
                let fn_tag = self.fresh_tmp();
                let fn_c = format!("anon_{}", fn_tag);

                let mut param_set = std::collections::HashSet::new();
                for (p, _) in params.iter() {
                    let pname = p.split(':').next().unwrap_or(p).trim();
                    let actual = if pname.starts_with("...") { &pname[3..] } else { pname };
                    param_set.insert(mangle(actual));
                }

                let mut free_c_names = std::collections::HashSet::new();
                collect_free_vars(body, &param_set, &mut free_c_names);

                let captured: Vec<String> = free_c_names
                    .into_iter()
                    .filter(|cn| self.is_declared(cn) && !self.is_global_name(cn))
                    .collect();
                let captured_boxed: std::collections::HashSet<String> = captured.iter()
                    .filter(|cn| if self.closure_vars.contains(*cn) { self.closure_boxed.contains(*cn) } else { self.boxed_vars.contains(*cn) })
                    .cloned().collect();

                let has_closure = !captured.is_empty();

                self.emit_named_fn_closure(&fn_c, params, body, &captured, &captured_boxed);

                let indent = "    ".repeat(self.indent);
                let mut pre = String::new();

                if has_closure {
                    let env_tmp = format!("{}_env", fn_tag);
                    writeln!(pre, "{}DgnVal {} = dgn_map_new();", indent, env_tmp).unwrap();
                    for cn in &captured {
                        if self.closure_vars.contains(cn) {
                            writeln!(pre, "{}dgn_map_set(&{}, \"{}\", (_cenv) ? dgn_map_get_raw(_cenv, \"{}\") : dgn_null());", indent, env_tmp, cn, cn).unwrap();
                        } else {
                            writeln!(pre, "{}dgn_map_set(&{}, \"{}\", {});", indent, env_tmp, cn, cn).unwrap();
                        }
                    }
                    writeln!(
                        pre,
                        "{}DgnVal {} = dgn_closure_n(_fn_{}, {}, {}.map, \"{}\");",
                        indent, fn_tag, fn_c, params.len(), env_tmp, param_csv(params)
                    ).unwrap();
                } else {
                    writeln!(
                        pre,
                        "{}DgnVal {} = dgn_func_n(_fn_{}, {}, \"{}\");",
                        indent, fn_tag, fn_c, params.len(), param_csv(params)
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
                let call = if let Some(cur) = self.current_class.clone() {
                    format!(
                        "dgn_super_call_from(_v_this, {}, \"{}\", {}, {})",
                        mangle(&cur),
                        escape_c_string(method),
                        args_tmp,
                        args.len()
                    )
                } else {
                    format!(
                        "dgn_super_call(_v_this, \"{}\", {}, {})",
                        escape_c_string(method),
                        args_tmp,
                        args.len()
                    )
                };
                (pre, call)
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

            let lt = self.fresh_tmp();
            writeln!(pre, "{}DgnVal {} = {};", indent, lt, lval).unwrap();
            writeln!(pre, "{}DgnVal {};", indent, tmp).unwrap();
            writeln!(
                pre,
                "{}if (dgn_truthy({})) {{ {} = {}; }}",
                indent, lt, tmp, lt
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
            let lt = self.fresh_tmp();
            writeln!(pre, "{}DgnVal {} = {};", indent, lt, lval).unwrap();
            writeln!(pre, "{}DgnVal {};", indent, tmp).unwrap();
            writeln!(
                pre,
                "{}if ({}.tag != DGN_NULL) {{ {} = {}; }}",
                indent, lt, tmp, lt
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

    fn resolve_devirtualized_method(&self, from_class: &str, method: &str) -> Option<String> {
        let resolved_class = resolve_method_owner(&self.classes, from_class, method)?;
        if self.overridable_methods.contains(&(resolved_class.clone(), method.to_string())) {
            return None;
        }
        Some(format!("{}_{}", mangle(&resolved_class), mangle(method)))
    }

    fn emit_call(&mut self, callee: &Expr, args: &[Expr]) -> (String, String) {
        let mut pre = String::new();
        let indent = "    ".repeat(self.indent);

        if let ExprNode::GetAttr(obj, method) = &callee.node {
            if matches!(obj.node, ExprNode::This) && !has_named_pack(args) {
                if let Some(cur) = self.current_class.clone() {
                    if let Some(target_fn) = self.resolve_devirtualized_method(&cur, method) {
                        let (opre, oval) = self.emit_expr(obj);
                        pre.push_str(&opre);
                        let args_tmp = self.fresh_tmp();
                        let indent2 = "    ".repeat(self.indent);
                        writeln!(pre, "{}DgnVal {}[{}];", indent2, args_tmp, args.len() + 1).unwrap();
                        writeln!(pre, "{}{}[0] = {};", indent2, args_tmp, oval).unwrap();
                        for (i, a) in args.iter().enumerate() {
                            let (apre, aval) = self.emit_expr(a);
                            pre.push_str(&apre);
                            writeln!(pre, "{}{}[{}] = {};", indent2, args_tmp, i + 1, aval).unwrap();
                        }
                        let result = format!("_fn_{}({}, {}, NULL)", target_fn, args_tmp, args.len() + 1);
                        return (pre, result);
                    }
                }
            }
        }

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
                "class_name" if !self.user_defines(name) => format!("dgn_b_class_name({}[0])", args_tmp),
                "is_a" if !self.user_defines(name) => format!("dgn_b_is_a({}[0], {}[1])", args_tmp, args_tmp),
                "has_attr" if !self.user_defines(name) => format!("dgn_b_has_attr({}[0], {}[1])", args_tmp, args_tmp),
                "set_attr" if !self.user_defines(name) => format!("dgn_b_set_attr({}[0], {}[1], {}[2])", args_tmp, args_tmp, args_tmp),
                "same" if !self.user_defines(name) => format!("dgn_b_same({}[0], {}[1])", args_tmp, args_tmp),
                "json_compact" if !self.user_defines(name) => format!("dgn_b_json_compact({}[0])", args_tmp),
                "now_ms" if !self.user_defines(name) => "dgn_b_now_ms()".to_string(),
                "floor_int" if !self.user_defines(name) => format!("dgn_b_floor_int({}[0])", args_tmp),
                "round_int" if !self.user_defines(name) => format!("dgn_b_round_int({}[0])", args_tmp),
                "min" if !self.user_defines(name) => format!("dgn_b_min({}, {})", args_tmp, args.len()),
                "max" if !self.user_defines(name) => format!("dgn_b_max({}, {})", args_tmp, args.len()),
                "abs" if !self.user_defines(name) => format!("dgn_b_abs({}[0])", args_tmp),
                "pow" if !self.user_defines(name) => format!("dgn_b_pow({}[0], {}[1])", args_tmp, args_tmp),
                "sin" if !self.user_defines(name) => format!("dgn_b_sin({}[0])", args_tmp),
                _ => {
                    let c_name = mangle(name);
                    if self.closure_vars.contains(&c_name) {
                        let (fpre, fval) = self.emit_expr(callee);
                        pre.push_str(&fpre);
                        format!("dgn_call_val({}, {}, {})", fval, args_tmp, args.len())
                    } else if self.direct_callable.contains(&c_name)
                        && !self.is_shadowed_locally(&c_name)
                        && !has_named_pack(args)
                    {
                        format!("_fn_{}({}, {}, NULL)", c_name, args_tmp, args.len())
                    } else {
                        format!("dgn_call_val({}, {}, {})", c_name, args_tmp, args.len())
                    }
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

    fn emit_named_fn_closure(
        &mut self,
        fn_c: &str,
        params: &[(String, Option<String>)],
        body: &Stmt,
        captured: &[String],
        captured_boxed: &std::collections::HashSet<String>,
    ) {
        let saved_tf = std::mem::take(&mut self.try_frames);
        let saved_lt = std::mem::take(&mut self.loop_try_base);
        let saved_fa = std::mem::take(&mut self.finally_at);
        self.emit_named_fn_closure_inner(fn_c, params, body, captured, captured_boxed);
        self.try_frames = saved_tf;
        self.loop_try_base = saved_lt;
        self.finally_at = saved_fa;
    }

    fn emit_named_fn_closure_inner(
        &mut self,
        fn_c: &str,
        params: &[(String, Option<String>)],
        body: &Stmt,
        captured: &[String],
        captured_boxed: &std::collections::HashSet<String>,
    ) {
        if captured.is_empty() {
            self.emit_named_fn(fn_c, params, body);
            return;
        }

        let saved_indent = self.indent;
        self.indent = 0;
        let mut saved_out = std::mem::take(&mut self.out);

        let saved_state = self.fn_enter(params, body);
        for cn in captured { self.closure_vars.insert(cn.clone()); }
        self.closure_boxed = captured_boxed.clone();

        writeln!(
            self.out,
            "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
            fn_c
        ).unwrap();
        self.indent = 1;
        self.push_scope();

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

        self.box_params(params);

        let indent = "    ".repeat(self.indent);
        writeln!(self.out, "{}DgnMap* _cenv = _env ? _env->map : 0;", indent).unwrap();

        self.emit_stmt(body);
        self.line("return dgn_null();");
        self.pop_scope();
        self.indent = 0;
        self.line("}");
        self.line("");

        self.fn_leave(saved_state);
        let generated_fn = std::mem::take(&mut self.out);
        self.out = saved_out;
        self.hoisted.push_str(&generated_fn);
        self.indent = saved_indent;
    }

   fn emit_named_fn(
        &mut self,
        fn_c: &str,
        params: &[(String, Option<String>)],
        body: &Stmt,
    ) {
        let saved_tf = std::mem::take(&mut self.try_frames);
        let saved_lt = std::mem::take(&mut self.loop_try_base);
        let saved_fa = std::mem::take(&mut self.finally_at);
        self.emit_named_fn_inner(fn_c, params, body);
        self.try_frames = saved_tf;
        self.loop_try_base = saved_lt;
        self.finally_at = saved_fa;
    }

    fn emit_named_fn_inner(
        &mut self,
        fn_c: &str,
        params: &[(String, Option<String>)],
        body: &Stmt,
    ) {
        let saved_indent = self.indent;
        self.indent = 0;

        let mut saved_out = std::mem::take(&mut self.out);

        if self.numeric_callable.contains(fn_c) {
            let pnames = self.numeric_params.get(fn_c).cloned().unwrap_or_default();

            let sig = if pnames.is_empty() {
                "void".to_string()
            } else {
                pnames.iter().map(|p| format!("int64_t {}", mangle(p))).collect::<Vec<_>>().join(", ")
            };
            writeln!(self.out, "static int64_t _fnum_{}({}) {{", fn_c, sig).unwrap();
            self.indent = 1;
            let mut it = IntSpecTranspiler::new();
            it.indent = self.indent;
            it.emit_body(body);
            self.out.push_str(&it.out);
            self.indent = 0;
            self.line("}");
            self.line("");

            writeln!(
                self.out,
                "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
                fn_c
            ).unwrap();
            self.indent = 1;
            for (i, pname) in pnames.iter().enumerate() {
                let ind = "    ".repeat(self.indent);
                writeln!(self.out, "{}int64_t {} = (_nargs > {}) ? _args[{}].i : 0;", ind, mangle(pname), i, i).unwrap();
            }
            let ind = "    ".repeat(self.indent);
            let call_args: Vec<String> = pnames.iter().map(|p| mangle(p)).collect();
            writeln!(self.out, "{}return dgn_int(_fnum_{}({}));", ind, fn_c, call_args.join(", ")).unwrap();
            self.indent = 0;
            self.line("}");
            self.line("");

            let generated_fn = std::mem::take(&mut self.out);
            self.out = saved_out;
            self.hoisted.push_str(&generated_fn);
            self.indent = saved_indent;
            return;
        }

        if let Some(sig) = self.numeric_callable_float.get(fn_c).cloned() {
            let pnames = self.numeric_params_float.get(fn_c).cloned().unwrap_or_default();
            let params_c: Vec<String> = pnames.iter().zip(sig.params.iter())
                .map(|(p, t)| format!("{} {}", num_c_type(*t), mangle(p)))
                .collect();
            let params_str = if params_c.is_empty() { "void".to_string() } else { params_c.join(", ") };
            writeln!(self.out, "static {} _fnumx_{}({}) {{", num_c_type(sig.ret.unwrap()), fn_c, params_str).unwrap();
            self.indent = 1;
            let types: std::collections::HashMap<String, NumType> =
                pnames.iter().cloned().zip(sig.params.iter().copied()).collect();
            let mut nt = NumSpecTranspiler::new(types);
            nt.indent = self.indent;
            nt.emit_body(body, &self.numeric_callable_float);
            self.out.push_str(&nt.out);
            self.indent = 0;
            self.line("}");
            self.line("");

            writeln!(
                self.out,
                "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
                fn_c
            ).unwrap();
            self.indent = 1;
            for (i, (pname, ptype)) in pnames.iter().zip(sig.params.iter()).enumerate() {
                let ind = "    ".repeat(self.indent);
                let coerced = match ptype {
                    NumType::Int => format!("(_args[{i}].tag==DGN_FLOAT) ? (int64_t)_args[{i}].f : _args[{i}].i", i = i),
                    NumType::Float => format!("(_args[{i}].tag==DGN_FLOAT) ? _args[{i}].f : (double)_args[{i}].i", i = i),
                };
                writeln!(self.out, "{}{} {} = (_nargs > {}) ? ({}) : 0;", ind, num_c_type(*ptype), mangle(pname), i, coerced).unwrap();
            }
            let ind = "    ".repeat(self.indent);
            let call_args: Vec<String> = pnames.iter().map(|p| mangle(p)).collect();
            let wrap = match sig.ret.unwrap() {
                NumType::Int => "dgn_int",
                NumType::Float => "dgn_float",
            };
            writeln!(self.out, "{}return {}(_fnumx_{}({}));", ind, wrap, fn_c, call_args.join(", ")).unwrap();
            self.indent = 0;
            self.line("}");
            self.line("");

            let generated_fn = std::mem::take(&mut self.out);
            self.out = saved_out;
            self.hoisted.push_str(&generated_fn);
            self.indent = saved_indent;
            return;
        }

        writeln!(
            self.out,
            "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env) {{",
            fn_c
        )
        .unwrap();
        self.indent = 1;
        self.push_scope();
        let saved_state = self.fn_enter(params, body);

        for (i, (pname, default_val)) in params.iter().enumerate() {
            let pname_clean = pname.split(':').next().unwrap().trim();
            let is_rest = pname_clean.starts_with("...");
            let actual_pname = if is_rest { pname_clean[3..].trim() } else { pname_clean };

            let c_name = mangle(actual_pname);
            let _ = self.decl_var(actual_pname);
            let indent = "    ".repeat(self.indent);

            if is_rest {
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

        self.box_params(params);
        self.emit_stmt(body);
        self.line("return dgn_null();");
        self.fn_leave(saved_state);
        self.pop_scope();
        self.indent = 0;
        self.line("}");
        self.line("");

        let generated_fn = std::mem::take(&mut self.out);
        self.out = saved_out;
        self.hoisted.push_str(&generated_fn);

        self.indent = saved_indent;
    }

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
                        "{}dgn_class_add_method_n(&{}, \"{}\", _fn_{}, {}, \"{}\");",
                        indent,
                        c_name,
                        escape_c_string(mname),
                        fn_c,
                        params.len(),
                        param_csv(params)
                    )
                    .unwrap();
                }
            }
            let mut field_stmts = Vec::new();
            for s in stmts { collect_field_defaults(s, &mut field_stmts); }
            let fi_fn_c = format!("{}___fieldinit__", mangle(name));
            let fi_body = Stmt { node: StmtNode::Block(field_stmts), line: body.line };
            let fi_params = vec![("this".to_string(), None)];
            self.emit_named_fn(&fi_fn_c, &fi_params, &fi_body);
            let c_name = mangle(name);
            writeln!(
                self.out,
                "{}dgn_class_add_method(&{}, \"__field_init__\", _fn_{}, 1);",
                indent, c_name, fi_fn_c
            )
            .unwrap();
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
        writeln!(self.out, "{}dgn_class_init_statics(&{});", indent, mangle(name)).unwrap();
    }

    fn emit_ifchain(&mut self, branches: &[(Option<Expr>, Stmt)]) {
        let mut first = true;
        let mut extra_close = 0;
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
                    let nest = !first && !pre.trim().is_empty();
                    if nest { self.line("else {"); self.indent += 1; extra_close += 1; }
                    self.raw(&pre);
                    let keyword = if first || nest { "if" } else { "else if" };
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
        for _ in 0..extra_close { self.indent -= 1; self.line("}"); }
    }

    fn emit_for_in(
        &mut self,
        var: &str,
        var2: Option<&str>,
        iter_expr: &Expr,
        body: &Stmt,
    ) {
        let cont_lbl = self.fresh_lbl();
        let brk_lbl = self.fresh_lbl();
        self.loop_try_base.push(self.try_frames.len()); self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

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
        writeln!(self.out, "{}DGN_SAFEPOINT();", indent2).unwrap();

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
        self.loop_ctx.pop(); self.loop_try_base.pop();
    }

    fn emit_switch(
        &mut self,
        val_expr: &Expr,
        cases: &[(Expr, Stmt)],
        default: Option<&Stmt>,
    ) {
        let brk_lbl = self.fresh_lbl();
        self.loop_try_base.push(self.try_frames.len()); self.loop_ctx.push((None, brk_lbl.clone()));

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

        self.loop_ctx.pop(); self.loop_try_base.pop();
    }

    fn pop_try_frames_to_loop(&mut self) {
        let base = self.loop_try_base.last().copied().unwrap_or(0);
        self.unwind_try_frames(base);
    }

    fn unwind_try_frames(&mut self, lo: usize) {
        let frames: Vec<String> = self.try_frames.clone();
        let saved_fa = self.finally_at.clone();
        for i in (lo..frames.len()).rev() {
            let line = format!("dgn_try_pop(&{});", frames[i]);
            self.line(&line);
            if let Some((_, fb)) = saved_fa.iter().find(|(idx, _)| *idx == i) {

                self.try_frames.truncate(i);
                self.finally_at.retain(|(idx, _)| *idx < i);
                self.line("{");
                self.indent += 1;
                self.push_scope();
                self.emit_stmt(fb);
                self.pop_scope();
                self.indent -= 1;
                self.line("}");
            }
        }
        self.try_frames = frames;
        self.finally_at = saved_fa;
    }

    fn emit_try(
        &mut self,
        try_body: &Stmt,
        catch_var: Option<&str>,
        catch_body: Option<&Stmt>,
        finally_body: Option<&Stmt>,
    ) {
        if let Some(fb) = finally_body {

            let outer = self.fresh_tmp();
            let pend = self.fresh_tmp();
            let has = self.fresh_tmp();
            let indent = "    ".repeat(self.indent);
            writeln!(self.out, "{}DgnTryFrame {}; {}.thrown = dgn_null();", indent, outer, outer).unwrap();
            writeln!(self.out, "{}DgnVal {} = dgn_null(); volatile int {} = 0;", indent, pend, has).unwrap();
            writeln!(self.out, "{}dgn_try_push(&{});", indent, outer).unwrap();
            writeln!(self.out, "{}if (setjmp({}.buf) == 0) {{", indent, outer).unwrap();
            self.indent += 1;
            self.push_scope();
            self.try_frames.push(outer.clone());
            self.finally_at.push((self.try_frames.len() - 1, fb.clone()));
            if catch_body.is_some() {
                self.emit_try(try_body, catch_var, catch_body, None);
            } else {
                self.emit_stmt(try_body);
            }
            self.finally_at.pop();
            self.try_frames.pop();
            let indent_inner = "    ".repeat(self.indent);
            writeln!(self.out, "{}dgn_try_pop(&{});", indent_inner, outer).unwrap();
            self.pop_scope();
            self.indent -= 1;
            writeln!(self.out, "{}}} else {{ {} = {}.thrown; {} = 1; }}", indent, pend, outer, has).unwrap();
            self.line("/* finally */");
            self.line("{");
            self.indent += 1;
            self.push_scope();
            self.emit_stmt(fb);
            self.pop_scope();
            self.indent -= 1;
            self.line("}");
            let rethrow = format!("if ({}) dgn_raise({});", has, pend);
            self.line(&rethrow);
            return;
        }
        let jmp_tmp = self.fresh_tmp();
        let indent = "    ".repeat(self.indent);

        writeln!(
            self.out,
            "{}DgnTryFrame {}; {}.thrown = dgn_null();",
            indent, jmp_tmp, jmp_tmp
        )
        .unwrap();
        writeln!(self.out, "{}dgn_try_push(&{});", indent, jmp_tmp).unwrap();

        writeln!(
            self.out,
            "{}if (setjmp({}.buf) == 0) {{",
            indent, jmp_tmp
        )
        .unwrap();

        self.indent += 1;
        self.push_scope();
        self.try_frames.push(jmp_tmp.clone());
        self.emit_stmt(try_body);
        self.try_frames.pop();

        let indent_inner = "    ".repeat(self.indent);
        writeln!(self.out, "{}dgn_try_pop(&{});", indent_inner, jmp_tmp).unwrap();

        self.pop_scope();
        self.indent -= 1;
        self.line("}");

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

        if let Some(fb) = finally_body {
            self.line("/* finally */");
            self.push_scope();
            self.emit_stmt(fb);
            self.pop_scope();
        }
    }
}

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
fn collect_top_level_funcs(stmt: &Stmt, out: &mut std::collections::HashSet<String>) {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { collect_top_level_funcs(s, out); }
        }
        StmtNode::FuncDef(name, _, _, _, _) => { out.insert(mangle(name)); }
        _ => {}
    }
}

fn collect_all_mutated_names(stmt: &Stmt, out: &mut std::collections::HashSet<String>) {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { collect_all_mutated_names(s, out); }
        }
        StmtNode::Expr(e) | StmtNode::Return(Some(e)) | StmtNode::Raise(e) => {
            scan_expr_mutations(e, out);
        }
        StmtNode::Return(None) | StmtNode::Break | StmtNode::Continue => {}
        StmtNode::ConstDecl(_, e) => { scan_expr_mutations(e, out); }
        StmtNode::IfChain(branches) => {
            for (cond, body) in branches {
                if let Some(c) = cond { scan_expr_mutations(c, out); }
                collect_all_mutated_names(body, out);
            }
        }
        StmtNode::While(cond, body) | StmtNode::DoWhile(body, cond) => {
            scan_expr_mutations(cond, out);
            collect_all_mutated_names(body, out);
        }
        StmtNode::For(_, _, iter, body) => {
            scan_expr_mutations(iter, out);
            collect_all_mutated_names(body, out);
        }
        StmtNode::CFor(init, cond, step, body) => {
            if let Some(e) = init { scan_expr_mutations(e, out); }
            if let Some(e) = cond { scan_expr_mutations(e, out); }
            if let Some(e) = step { scan_expr_mutations(e, out); }
            collect_all_mutated_names(body, out);
        }
        StmtNode::Switch(val, cases, default) => {
            scan_expr_mutations(val, out);
            for (case_e, body) in cases {
                scan_expr_mutations(case_e, out);
                collect_all_mutated_names(body, out);
            }
            if let Some(d) = default { collect_all_mutated_names(d, out); }
        }
        StmtNode::Try(tb, _cv, cb, fb) => {
            collect_all_mutated_names(tb, out);
            if let Some(cb_stmt) = cb { collect_all_mutated_names(cb_stmt, out); }
            if let Some(fb_stmt) = fb { collect_all_mutated_names(fb_stmt, out); }
        }
        StmtNode::FuncDef(_, _, _, body, _) => { collect_all_mutated_names(body, out); }
        StmtNode::ClassDef(_, _, body) => { collect_all_mutated_names(body, out); }
        StmtNode::Augment(_, methods) => {
            for m in methods { collect_all_mutated_names(m, out); }
        }
        StmtNode::Import(_, _) => {}
    }
}

fn scan_expr_mutations(expr: &Expr, out: &mut std::collections::HashSet<String>) {
    match &expr.node {
        ExprNode::Number(_) | ExprNode::String(_) | ExprNode::Bool(_)
        | ExprNode::Null | ExprNode::This | ExprNode::Var(_) => {}
        ExprNode::List(items) => { for i in items { scan_expr_mutations(i, out); } }
        ExprNode::Obj(pairs) => { for (_, v) in pairs { scan_expr_mutations(v, out); } }
        ExprNode::Unary(_, e) => { scan_expr_mutations(e, out); }
        ExprNode::Binary(_, l, r) => { scan_expr_mutations(l, out); scan_expr_mutations(r, out); }
        ExprNode::Ternary(c, t, e) => {
            scan_expr_mutations(c, out); scan_expr_mutations(t, out); scan_expr_mutations(e, out);
        }
        ExprNode::Assign(target, val) => {
            if let ExprNode::Var(name) = &target.node {
                out.insert(mangle(name));
            } else {
                scan_expr_mutations(target, out);
            }
            scan_expr_mutations(val, out);
        }
        ExprNode::Call(callee, args) => {
            scan_expr_mutations(callee, out);
            for a in args { scan_expr_mutations(a, out); }
        }
        ExprNode::GetAttr(obj, _) | ExprNode::OptionalGetAttr(obj, _) => {
            scan_expr_mutations(obj, out);
        }
        ExprNode::Index(obj, idx) => { scan_expr_mutations(obj, out); scan_expr_mutations(idx, out); }
        ExprNode::PreOp(op, e) | ExprNode::PostOp(op, e) => {
            if op == "++" || op == "--" {
                if let ExprNode::Var(name) = &e.node {
                    out.insert(mangle(name));
                }
            }
            scan_expr_mutations(e, out);
        }
        ExprNode::FuncExpr(_, _, body) => { collect_all_mutated_names(body, out); }
        ExprNode::New(callee, args) => {
            scan_expr_mutations(callee, out);
            for a in args { scan_expr_mutations(a, out); }
        }
        ExprNode::Super(_, args) => { for a in args { scan_expr_mutations(a, out); } }
        ExprNode::Await(e) => { scan_expr_mutations(e, out); }
    }
}

pub(crate) fn stmt_shadows_name(stmt: &Stmt, mangled: &str) -> bool {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            stmts.iter().any(|s| stmt_shadows_name(s, mangled))
        }
        StmtNode::ConstDecl(n, _) => mangle(n) == mangled,
        StmtNode::IfChain(branches) => branches.iter().any(|(_, b)| stmt_shadows_name(b, mangled)),
        StmtNode::While(_, body) | StmtNode::DoWhile(body, _) => stmt_shadows_name(body, mangled),
        StmtNode::For(var, _, _, body) => mangle(var) == mangled || stmt_shadows_name(body, mangled),
        StmtNode::CFor(_, _, _, body) => stmt_shadows_name(body, mangled),
        StmtNode::Switch(_, cases, default) => {
            cases.iter().any(|(_, b)| stmt_shadows_name(b, mangled))
                || default.as_ref().map_or(false, |d| stmt_shadows_name(d, mangled))
        }
        StmtNode::Try(tb, cv, cb, fb) => {
            cv.as_ref().map_or(false, |v| mangle(v) == mangled)
                || stmt_shadows_name(tb, mangled)
                || cb.as_ref().map_or(false, |b| stmt_shadows_name(b, mangled))
                || fb.as_ref().map_or(false, |b| stmt_shadows_name(b, mangled))
        }
        StmtNode::FuncDef(n, params, _, body, _) => {
            mangle(n) == mangled
                || params.iter().any(|(p, _)| {
                    let clean = p.split(':').next().unwrap_or(p).trim();
                    let clean = clean.strip_prefix("...").unwrap_or(clean).trim();
                    mangle(clean) == mangled
                })
                || stmt_shadows_name(body, mangled)
        }
        StmtNode::ClassDef(_, _, body) => stmt_shadows_name(body, mangled),
        StmtNode::Augment(_, methods) => methods.iter().any(|m| stmt_shadows_name(m, mangled)),
        StmtNode::Expr(e) | StmtNode::Return(Some(e)) | StmtNode::Raise(e) => expr_shadows_name(e, mangled),
        StmtNode::Return(None) | StmtNode::Break | StmtNode::Continue | StmtNode::Import(_, _) => false,
    }
}

pub(crate) fn expr_shadows_name(expr: &Expr, mangled: &str) -> bool {
    match &expr.node {
        ExprNode::FuncExpr(params, _, body) => {
            params.iter().any(|(p, _)| {
                let clean = p.split(':').next().unwrap_or(p).trim();
                let clean = clean.strip_prefix("...").unwrap_or(clean).trim();
                mangle(clean) == mangled
            }) || stmt_shadows_name(body, mangled)
        }
        ExprNode::List(items) => items.iter().any(|e| expr_shadows_name(e, mangled)),
        ExprNode::Obj(pairs) => pairs.iter().any(|(_, v)| expr_shadows_name(v, mangled)),
        ExprNode::Unary(_, e) | ExprNode::PreOp(_, e) | ExprNode::PostOp(_, e)
        | ExprNode::GetAttr(e, _) | ExprNode::OptionalGetAttr(e, _) | ExprNode::Await(e) => expr_shadows_name(e, mangled),
        ExprNode::Binary(_, l, r) | ExprNode::Assign(l, r) | ExprNode::Index(l, r) => {
            expr_shadows_name(l, mangled) || expr_shadows_name(r, mangled)
        }
        ExprNode::Ternary(c, t, e) => {
            expr_shadows_name(c, mangled) || expr_shadows_name(t, mangled) || expr_shadows_name(e, mangled)
        }
        ExprNode::Call(callee, args) | ExprNode::New(callee, args) => {
            expr_shadows_name(callee, mangled) || args.iter().any(|a| expr_shadows_name(a, mangled))
        }
        ExprNode::Super(_, args) => args.iter().any(|a| expr_shadows_name(a, mangled)),
        ExprNode::Number(_) | ExprNode::String(_) | ExprNode::Bool(_)
        | ExprNode::Null | ExprNode::This | ExprNode::Var(_) => false,
    }
}

pub(crate) fn body_uses_name_as_value(stmt: &Stmt, mangled: &str) -> bool {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            stmts.iter().any(|s| body_uses_name_as_value(s, mangled))
        }
        StmtNode::Expr(e) | StmtNode::Return(Some(e)) | StmtNode::Raise(e) => expr_uses_name_as_value(e, mangled),
        StmtNode::Return(None) | StmtNode::Break | StmtNode::Continue | StmtNode::Import(_, _) => false,
        StmtNode::ConstDecl(_, e) => expr_uses_name_as_value(e, mangled),
        StmtNode::IfChain(branches) => branches.iter().any(|(cond, b)| {
            cond.as_ref().map_or(false, |c| expr_uses_name_as_value(c, mangled))
                || body_uses_name_as_value(b, mangled)
        }),
        StmtNode::While(cond, body) | StmtNode::DoWhile(body, cond) => {
            expr_uses_name_as_value(cond, mangled) || body_uses_name_as_value(body, mangled)
        }
        StmtNode::For(_, _, iter, body) => {
            expr_uses_name_as_value(iter, mangled) || body_uses_name_as_value(body, mangled)
        }
        StmtNode::CFor(init, cond, step, body) => {
            init.as_ref().map_or(false, |e| expr_uses_name_as_value(e, mangled))
                || cond.as_ref().map_or(false, |e| expr_uses_name_as_value(e, mangled))
                || step.as_ref().map_or(false, |e| expr_uses_name_as_value(e, mangled))
                || body_uses_name_as_value(body, mangled)
        }
        StmtNode::Switch(val, cases, default) => {
            expr_uses_name_as_value(val, mangled)
                || cases.iter().any(|(ce, b)| expr_uses_name_as_value(ce, mangled) || body_uses_name_as_value(b, mangled))
                || default.as_ref().map_or(false, |d| body_uses_name_as_value(d, mangled))
        }
        StmtNode::Try(tb, _, cb, fb) => {
            body_uses_name_as_value(tb, mangled)
                || cb.as_ref().map_or(false, |b| body_uses_name_as_value(b, mangled))
                || fb.as_ref().map_or(false, |b| body_uses_name_as_value(b, mangled))
        }
        StmtNode::FuncDef(_, _, _, body, _) => body_uses_name_as_value(body, mangled),
        StmtNode::ClassDef(_, _, body) => body_uses_name_as_value(body, mangled),
        StmtNode::Augment(_, methods) => methods.iter().any(|m| body_uses_name_as_value(m, mangled)),
    }
}

pub(crate) fn expr_uses_name_as_value(expr: &Expr, mangled: &str) -> bool {
    match &expr.node {
        ExprNode::Var(n) => mangle(n) == mangled,
        ExprNode::Call(callee, args) => {
            let callee_is_bare_self = matches!(&callee.node, ExprNode::Var(n) if mangle(n) == mangled);
            (!callee_is_bare_self && expr_uses_name_as_value(callee, mangled))
                || args.iter().any(|a| expr_uses_name_as_value(a, mangled))
        }
        ExprNode::FuncExpr(_, _, body) => body_uses_name_as_value(body, mangled),
        ExprNode::List(items) => items.iter().any(|e| expr_uses_name_as_value(e, mangled)),
        ExprNode::Obj(pairs) => pairs.iter().any(|(_, v)| expr_uses_name_as_value(v, mangled)),
        ExprNode::Unary(_, e) | ExprNode::PreOp(_, e) | ExprNode::PostOp(_, e)
        | ExprNode::GetAttr(e, _) | ExprNode::OptionalGetAttr(e, _) | ExprNode::Await(e) => expr_uses_name_as_value(e, mangled),
        ExprNode::Binary(_, l, r) | ExprNode::Assign(l, r) | ExprNode::Index(l, r) => {
            expr_uses_name_as_value(l, mangled) || expr_uses_name_as_value(r, mangled)
        }
        ExprNode::Ternary(c, t, e) => {
            expr_uses_name_as_value(c, mangled) || expr_uses_name_as_value(t, mangled) || expr_uses_name_as_value(e, mangled)
        }
        ExprNode::New(callee, args) => {
            expr_uses_name_as_value(callee, mangled) || args.iter().any(|a| expr_uses_name_as_value(a, mangled))
        }
        ExprNode::Super(_, args) => args.iter().any(|a| expr_uses_name_as_value(a, mangled)),
        ExprNode::Number(_) | ExprNode::String(_) | ExprNode::Bool(_) | ExprNode::Null | ExprNode::This => false,
    }
}

struct ClassInfo {
    parent: Option<String>,
    own_methods: std::collections::HashSet<String>,
}

fn collect_classes(stmt: &Stmt, out: &mut std::collections::HashMap<String, ClassInfo>) {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { collect_classes(s, out); }
        }
        StmtNode::ClassDef(name, parent, body) => {
            let mut methods = std::collections::HashSet::new();
            if let StmtNode::Block(stmts) = &body.node {
                for s in stmts {
                    if let StmtNode::FuncDef(mname, _, _, _, _) = &s.node {
                        methods.insert(mname.clone());
                    }
                }
            }
            out.insert(name.clone(), ClassInfo { parent: parent.clone(), own_methods: methods });
        }
        _ => {}
    }
}

fn resolve_method_owner(
    classes: &std::collections::HashMap<String, ClassInfo>,
    start_class: &str,
    method: &str,
) -> Option<String> {
    let mut cur = Some(start_class.to_string());
    let mut guard = 0;
    while let Some(name) = cur {
        guard += 1;
        if guard > 1000 { return None; }
        let info = classes.get(&name)?;
        if info.own_methods.contains(method) { return Some(name); }
        cur = info.parent.clone();
    }
    None
}

fn compute_overridable_methods(
    classes: &std::collections::HashMap<String, ClassInfo>,
) -> std::collections::HashSet<(String, String)> {
    let mut children: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for (name, info) in classes {
        if let Some(p) = &info.parent {
            children.entry(p.clone()).or_default().push(name.clone());
        }
    }
    let mut overridable = std::collections::HashSet::new();
    for (name, info) in classes {
        for method in &info.own_methods {
            let mut stack = children.get(name).cloned().unwrap_or_default();
            let mut visited = std::collections::HashSet::new();
            while let Some(desc) = stack.pop() {
                if !visited.insert(desc.clone()) { continue; }
                if let Some(dinfo) = classes.get(&desc) {
                    if dinfo.own_methods.contains(method) {
                        overridable.insert((name.clone(), method.clone()));
                        break;
                    }
                    if let Some(more) = children.get(&desc) { stack.extend(more.clone()); }
                }
            }
        }
    }
    overridable
}

fn extract_accum_chain(expr: &Expr, name: &str) -> Option<Vec<Expr>> {
    if let ExprNode::Binary(op, l, r) = &expr.node {
        if op == "+" {
            return match &l.node {
                ExprNode::Var(n) if n == name => Some(vec![(**r).clone()]),
                ExprNode::Binary(op2, _, _) if op2 == "+" => {
                    let mut chain = extract_accum_chain(l, name)?;
                    chain.push((**r).clone());
                    Some(chain)
                }
                _ => None,
            };
        }
    }
    None
}

fn expr_references_var(expr: &Expr, name: &str) -> bool {
    match &expr.node {
        ExprNode::Var(n) => n == name,
        ExprNode::Binary(_, l, r) | ExprNode::Assign(l, r) | ExprNode::Index(l, r) => {
            expr_references_var(l, name) || expr_references_var(r, name)
        }
        ExprNode::Unary(_, e) | ExprNode::PreOp(_, e) | ExprNode::PostOp(_, e)
        | ExprNode::GetAttr(e, _) | ExprNode::OptionalGetAttr(e, _) | ExprNode::Await(e) => expr_references_var(e, name),
        ExprNode::Ternary(c, t, e) => expr_references_var(c, name) || expr_references_var(t, name) || expr_references_var(e, name),
        ExprNode::Call(callee, args) | ExprNode::New(callee, args) => {
            expr_references_var(callee, name) || args.iter().any(|a| expr_references_var(a, name))
        }
        ExprNode::Super(_, args) => args.iter().any(|a| expr_references_var(a, name)),
        ExprNode::List(items) => items.iter().any(|i| expr_references_var(i, name)),
        ExprNode::Obj(pairs) => pairs.iter().any(|(_, v)| expr_references_var(v, name)),
        ExprNode::FuncExpr(_, _, body) => stmt_references_var(body, name),
        _ => false,
    }
}

fn stmt_references_var(stmt: &Stmt, name: &str) -> bool {
    match &stmt.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => s.iter().any(|st| stmt_references_var(st, name)),
        StmtNode::Expr(e) | StmtNode::Return(Some(e)) | StmtNode::Raise(e) => expr_references_var(e, name),
        StmtNode::ConstDecl(_, e) => expr_references_var(e, name),
        StmtNode::IfChain(branches) => branches.iter().any(|(c, b)| {
            c.as_ref().map_or(false, |c| expr_references_var(c, name)) || stmt_references_var(b, name)
        }),
        StmtNode::While(c, b) | StmtNode::DoWhile(b, c) => expr_references_var(c, name) || stmt_references_var(b, name),
        StmtNode::For(_, _, it, b) => expr_references_var(it, name) || stmt_references_var(b, name),
        StmtNode::CFor(i, c, s, b) => {
            i.as_ref().map_or(false, |e| expr_references_var(e, name))
                || c.as_ref().map_or(false, |e| expr_references_var(e, name))
                || s.as_ref().map_or(false, |e| expr_references_var(e, name))
                || stmt_references_var(b, name)
        }
        StmtNode::Switch(v, cases, default) => {
            expr_references_var(v, name)
                || cases.iter().any(|(ce, b)| expr_references_var(ce, name) || stmt_references_var(b, name))
                || default.as_ref().map_or(false, |d| stmt_references_var(d, name))
        }
        StmtNode::Try(tb, _, cb, fb) => {
            stmt_references_var(tb, name)
                || cb.as_ref().map_or(false, |b| stmt_references_var(b, name))
                || fb.as_ref().map_or(false, |b| stmt_references_var(b, name))
        }
        StmtNode::FuncDef(_, _, _, body, _) => stmt_references_var(body, name),
        StmtNode::ClassDef(_, _, body) => stmt_references_var(body, name),
        StmtNode::Augment(_, methods) => methods.iter().any(|m| stmt_references_var(m, name)),
        _ => false,
    }
}

fn find_and_validate_accumulator(body_stmts: &[Stmt], name: &str) -> Option<(usize, Vec<Expr>)> {
    let mut found: Option<(usize, Vec<Expr>)> = None;
    for (i, s) in body_stmts.iter().enumerate() {
        let mut matched_here = false;
        if let StmtNode::Expr(e) = &s.node {
            if let ExprNode::Assign(target, val) = &e.node {
                if let ExprNode::Var(n) = &target.node {
                    if n == name {
                        let chain = extract_accum_chain(val, name)?;
                        if chain.iter().any(|p| expr_references_var(p, name)) { return None; }
                        if found.is_some() { return None; }
                        found = Some((i, chain));
                        matched_here = true;
                    }
                }
            }
        }
        if !matched_here && stmt_references_var(s, name) { return None; }
    }
    found
}

fn int_param_names(params: &[(String, Option<String>)]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for (p, default) in params {
        if default.is_some() { return None; }
        let clean = p.split(':').next().unwrap_or(p).trim();
        if clean.starts_with("...") { return None; }
        if clean.is_empty() { return None; }
        out.push(clean.to_string());
    }
    Some(out)
}

fn int_safe_expr(expr: &Expr, locals: &std::collections::HashSet<String>, called: &mut std::collections::HashSet<String>) -> bool {
    match &expr.node {
        ExprNode::Number(n) => n.fract() == 0.0,
        ExprNode::Var(name) => locals.contains(name),
        ExprNode::Binary(op, l, r) => match op.as_str() {
            "+" | "-" | "*" | "%" | "&" | "|" | "^" | "<<" | ">>" => {
                int_safe_expr(l, locals, called) && int_safe_expr(r, locals, called)
            }
            _ => false,
        },
        ExprNode::Unary(op, e) => (op == "-" || op == "~") && int_safe_expr(e, locals, called),
        ExprNode::Ternary(cond, t, e) => {
            cond_safe_expr(cond, locals, called)
                && int_safe_expr(t, locals, called)
                && int_safe_expr(e, locals, called)
        }
        ExprNode::Call(callee, args) => {
            if let ExprNode::Var(name) = &callee.node {
                called.insert(mangle(name));
                args.iter().all(|a| int_safe_expr(a, locals, called))
            } else {
                false
            }
        }
        _ => false,
    }
}

fn cond_safe_expr(expr: &Expr, locals: &std::collections::HashSet<String>, called: &mut std::collections::HashSet<String>) -> bool {
    match &expr.node {
        ExprNode::Binary(op, l, r) => match op.as_str() {
            "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                int_safe_expr(l, locals, called) && int_safe_expr(r, locals, called)
            }
            "&&" | "||" | "and" | "or" => {
                cond_safe_expr(l, locals, called) && cond_safe_expr(r, locals, called)
            }
            _ => int_safe_expr(expr, locals, called),
        },
        ExprNode::Unary(op, e) if op == "!" || op == "not" => cond_safe_expr(e, locals, called),
        ExprNode::Bool(_) => true,
        _ => int_safe_expr(expr, locals, called),
    }
}

fn int_safe_stmt(stmt: &Stmt, locals: &mut std::collections::HashSet<String>, called: &mut std::collections::HashSet<String>) -> bool {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            stmts.iter().all(|s| int_safe_stmt(s, locals, called))
        }
        StmtNode::Expr(e) => match &e.node {
            ExprNode::Assign(target, val) => {
                matches!(&target.node, ExprNode::Var(n) if locals.contains(n))
                    && int_safe_expr(val, locals, called)
            }
            ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
                (op == "++" || op == "--") && matches!(&inner.node, ExprNode::Var(n) if locals.contains(n))
            }
            _ => false,
        },
        StmtNode::Return(Some(e)) => int_safe_expr(e, locals, called),
        StmtNode::Return(None) => false,
        StmtNode::Break | StmtNode::Continue => true,
        StmtNode::ConstDecl(name, e) => {
            if int_safe_expr(e, locals, called) {
                locals.insert(name.clone());
                true
            } else {
                false
            }
        }
        StmtNode::IfChain(branches) => branches.iter().all(|(cond, b)| {
            cond.as_ref().map_or(true, |c| cond_safe_expr(c, locals, called))
                && int_safe_stmt(b, locals, called)
        }),
        StmtNode::While(cond, body) | StmtNode::DoWhile(body, cond) => {
            cond_safe_expr(cond, locals, called) && int_safe_stmt(body, locals, called)
        }
        StmtNode::CFor(init, cond, step, body) => {
            init.as_ref().map_or(true, |e| int_safe_stmt_expr_toplevel(e, locals, called))
                && cond.as_ref().map_or(true, |e| cond_safe_expr(e, locals, called))
                && step.as_ref().map_or(true, |e| int_safe_stmt_expr_toplevel(e, locals, called))
                && int_safe_stmt(body, locals, called)
        }
        _ => false,
    }
}

fn int_safe_stmt_expr_toplevel(e: &Expr, locals: &mut std::collections::HashSet<String>, called: &mut std::collections::HashSet<String>) -> bool {
    match &e.node {
        ExprNode::Assign(target, val) => {
            matches!(&target.node, ExprNode::Var(n) if locals.contains(n))
                && int_safe_expr(val, locals, called)
        }
        ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
            (op == "++" || op == "--") && matches!(&inner.node, ExprNode::Var(n) if locals.contains(n))
        }
        _ => false,
    }
}

fn block_definitely_returns(stmts: &[Stmt]) -> bool {
    for st in stmts {
        match &st.node {
            StmtNode::Return(_) => return true,
            StmtNode::IfChain(branches) => {
                let has_else = branches.last().map_or(false, |(c, _)| c.is_none());
                if has_else && branches.iter().all(|(_, b)| stmt_definitely_returns(b)) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

fn stmt_definitely_returns(stmt: &Stmt) -> bool {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => block_definitely_returns(stmts),
        StmtNode::Return(_) => true,
        StmtNode::IfChain(branches) => {
            let has_else = branches.last().map_or(false, |(c, _)| c.is_none());
            has_else && branches.iter().all(|(_, b)| stmt_definitely_returns(b))
        }
        _ => false,
    }
}

fn collect_top_level_funcdefs(stmt: &Stmt, out: &mut Vec<(String, Vec<(String, Option<String>)>, Stmt)>) {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { collect_top_level_funcdefs(s, out); }
        }
        StmtNode::FuncDef(name, params, _, body, _) => {
            out.push((name.clone(), params.clone(), (**body).clone()));
        }
        _ => {}
    }
}

fn compute_numeric_callable(
    ast: &Stmt,
    direct_callable: &std::collections::HashSet<String>,
) -> (std::collections::HashSet<String>, std::collections::HashMap<String, Vec<String>>) {
    let mut funcdefs = Vec::new();
    collect_top_level_funcdefs(ast, &mut funcdefs);

    let mut candidates: std::collections::HashMap<String, (Vec<String>, std::collections::HashSet<String>)> =
        std::collections::HashMap::new();
    let mut param_map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();

    for (name, params, body) in &funcdefs {
        let c_name = mangle(name);
        if !direct_callable.contains(&c_name) { continue; }
        let Some(pnames) = int_param_names(params) else { continue; };
        let mut locals: std::collections::HashSet<String> = pnames.iter().cloned().collect();
        let mut called = std::collections::HashSet::new();
        if int_safe_stmt(body, &mut locals, &mut called) && stmt_definitely_returns(body) {
            called.remove(&c_name);
            candidates.insert(c_name.clone(), (pnames.clone(), called));
            param_map.insert(c_name, pnames);
        }
    }

    loop {
        let mut to_remove = Vec::new();
        for (name, (_, called)) in &candidates {
            if called.iter().any(|c| !candidates.contains_key(c)) {
                to_remove.push(name.clone());
            }
        }
        if to_remove.is_empty() { break; }
        for name in to_remove { candidates.remove(&name); }
    }

    let numeric_callable: std::collections::HashSet<String> = candidates.keys().cloned().collect();
    param_map.retain(|k, _| numeric_callable.contains(k));
    (numeric_callable, param_map)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NumType { Int, Float }

fn num_c_type(t: NumType) -> &'static str {
    match t { NumType::Int => "int64_t", NumType::Float => "double" }
}

#[derive(Default)]
struct TypeEvidence {
    float_ev: std::collections::HashSet<String>,
    int_only_ev: std::collections::HashSet<String>,
}

fn ev_mark_float(e: &Expr, ev: &mut TypeEvidence) {
    if let ExprNode::Var(n) = &e.node { ev.float_ev.insert(n.clone()); }
}
fn ev_mark_int_only(e: &Expr, ev: &mut TypeEvidence) {
    if let ExprNode::Var(n) = &e.node { ev.int_only_ev.insert(n.clone()); }
}

fn collect_evidence_expr(expr: &Expr, ev: &mut TypeEvidence) {
    match &expr.node {
        ExprNode::Binary(op, l, r) => {
            match op.as_str() {
                "&" | "|" | "^" | "<<" | ">>" => { ev_mark_int_only(l, ev); ev_mark_int_only(r, ev); }
                _ => {
                    if let ExprNode::Number(n) = &l.node { if n.fract() != 0.0 { ev_mark_float(r, ev); } }
                    if let ExprNode::Number(n) = &r.node { if n.fract() != 0.0 { ev_mark_float(l, ev); } }
                }
            }
            collect_evidence_expr(l, ev);
            collect_evidence_expr(r, ev);
        }
        ExprNode::Unary(op, inner) => {
            if op == "~" { ev_mark_int_only(inner, ev); }
            collect_evidence_expr(inner, ev);
        }
        ExprNode::Ternary(c, t, e2) => { collect_evidence_expr(c, ev); collect_evidence_expr(t, ev); collect_evidence_expr(e2, ev); }
        ExprNode::Call(_, args) => { for a in args { collect_evidence_expr(a, ev); } }
        _ => {}
    }
}

fn collect_evidence_stmt(stmt: &Stmt, ev: &mut TypeEvidence) {
    match &stmt.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => { for st in s { collect_evidence_stmt(st, ev); } }
        StmtNode::Expr(e) => collect_evidence_expr(e, ev),
        StmtNode::Return(Some(e)) => collect_evidence_expr(e, ev),
        StmtNode::ConstDecl(_, e) => collect_evidence_expr(e, ev),
        StmtNode::IfChain(branches) => {
            for (c, b) in branches {
                if let Some(c) = c { collect_evidence_expr(c, ev); }
                collect_evidence_stmt(b, ev);
            }
        }
        StmtNode::While(c, b) | StmtNode::DoWhile(b, c) => { collect_evidence_expr(c, ev); collect_evidence_stmt(b, ev); }
        StmtNode::CFor(i, c, s, b) => {
            if let Some(e) = i { collect_evidence_expr(e, ev); }
            if let Some(e) = c { collect_evidence_expr(e, ev); }
            if let Some(e) = s { collect_evidence_expr(e, ev); }
            collect_evidence_stmt(b, ev);
        }
        _ => {}
    }
}

fn collect_call_site_evidence_expr(expr: &Expr, fn_name: &str, pnames: &[String], ev: &mut TypeEvidence) {
    if let ExprNode::Call(callee, args) = &expr.node {
        if let ExprNode::Var(n) = &callee.node {
            if n == fn_name {
                for (i, a) in args.iter().enumerate() {
                    if let ExprNode::Number(num) = &a.node {
                        if num.fract() != 0.0 {
                            if let Some(p) = pnames.get(i) { ev.float_ev.insert(p.clone()); }
                        }
                    }
                }
            }
        }
    }
    match &expr.node {
        ExprNode::Binary(_, l, r) => { collect_call_site_evidence_expr(l, fn_name, pnames, ev); collect_call_site_evidence_expr(r, fn_name, pnames, ev); }
        ExprNode::Unary(_, e2) | ExprNode::PreOp(_, e2) | ExprNode::PostOp(_, e2)
        | ExprNode::GetAttr(e2, _) | ExprNode::OptionalGetAttr(e2, _) | ExprNode::Await(e2) => collect_call_site_evidence_expr(e2, fn_name, pnames, ev),
        ExprNode::Ternary(c, t, e2) => { collect_call_site_evidence_expr(c, fn_name, pnames, ev); collect_call_site_evidence_expr(t, fn_name, pnames, ev); collect_call_site_evidence_expr(e2, fn_name, pnames, ev); }
        ExprNode::Call(callee, args) => { collect_call_site_evidence_expr(callee, fn_name, pnames, ev); for a in args { collect_call_site_evidence_expr(a, fn_name, pnames, ev); } }
        ExprNode::Assign(l, r) | ExprNode::Index(l, r) => { collect_call_site_evidence_expr(l, fn_name, pnames, ev); collect_call_site_evidence_expr(r, fn_name, pnames, ev); }
        ExprNode::List(items) => { for i in items { collect_call_site_evidence_expr(i, fn_name, pnames, ev); } }
        ExprNode::Obj(pairs) => { for (_, v) in pairs { collect_call_site_evidence_expr(v, fn_name, pnames, ev); } }
        ExprNode::FuncExpr(_, _, body) => collect_call_site_evidence_stmt(body, fn_name, pnames, ev),
        ExprNode::New(callee, args) => { collect_call_site_evidence_expr(callee, fn_name, pnames, ev); for a in args { collect_call_site_evidence_expr(a, fn_name, pnames, ev); } }
        ExprNode::Super(_, args) => { for a in args { collect_call_site_evidence_expr(a, fn_name, pnames, ev); } }
        _ => {}
    }
}
fn collect_call_site_evidence_stmt(stmt: &Stmt, fn_name: &str, pnames: &[String], ev: &mut TypeEvidence) {
    match &stmt.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => { for st in s { collect_call_site_evidence_stmt(st, fn_name, pnames, ev); } }
        StmtNode::Expr(e) | StmtNode::Return(Some(e)) | StmtNode::Raise(e) => collect_call_site_evidence_expr(e, fn_name, pnames, ev),
        StmtNode::ConstDecl(_, e) => collect_call_site_evidence_expr(e, fn_name, pnames, ev),
        StmtNode::IfChain(branches) => { for (c, b) in branches { if let Some(c) = c { collect_call_site_evidence_expr(c, fn_name, pnames, ev); } collect_call_site_evidence_stmt(b, fn_name, pnames, ev); } }
        StmtNode::While(c, b) | StmtNode::DoWhile(b, c) => { collect_call_site_evidence_expr(c, fn_name, pnames, ev); collect_call_site_evidence_stmt(b, fn_name, pnames, ev); }
        StmtNode::For(_, _, it, b) => { collect_call_site_evidence_expr(it, fn_name, pnames, ev); collect_call_site_evidence_stmt(b, fn_name, pnames, ev); }
        StmtNode::CFor(i, c, s, b) => {
            if let Some(e) = i { collect_call_site_evidence_expr(e, fn_name, pnames, ev); }
            if let Some(e) = c { collect_call_site_evidence_expr(e, fn_name, pnames, ev); }
            if let Some(e) = s { collect_call_site_evidence_expr(e, fn_name, pnames, ev); }
            collect_call_site_evidence_stmt(b, fn_name, pnames, ev);
        }
        StmtNode::Switch(v, cases, default) => {
            collect_call_site_evidence_expr(v, fn_name, pnames, ev);
            for (ce, b) in cases { collect_call_site_evidence_expr(ce, fn_name, pnames, ev); collect_call_site_evidence_stmt(b, fn_name, pnames, ev); }
            if let Some(d) = default { collect_call_site_evidence_stmt(d, fn_name, pnames, ev); }
        }
        StmtNode::Try(tb, _, cb, fb) => {
            collect_call_site_evidence_stmt(tb, fn_name, pnames, ev);
            if let Some(b) = cb { collect_call_site_evidence_stmt(b, fn_name, pnames, ev); }
            if let Some(b) = fb { collect_call_site_evidence_stmt(b, fn_name, pnames, ev); }
        }
        StmtNode::FuncDef(_, _, _, body, _) => collect_call_site_evidence_stmt(body, fn_name, pnames, ev),
        StmtNode::ClassDef(_, _, body) => collect_call_site_evidence_stmt(body, fn_name, pnames, ev),
        StmtNode::Augment(_, methods) => { for m in methods { collect_call_site_evidence_stmt(m, fn_name, pnames, ev); } }
        _ => {}
    }
}

fn resolve_param_types(pnames: &[String], ev: &TypeEvidence) -> Option<Vec<NumType>> {
    let mut out = Vec::with_capacity(pnames.len());
    for p in pnames {
        let is_float = ev.float_ev.contains(p);
        let is_int_only = ev.int_only_ev.contains(p);
        if is_float && is_int_only { return None; }
        out.push(if is_float { NumType::Float } else { NumType::Int });
    }
    Some(out)
}

#[derive(Clone)]
struct FnSig { params: Vec<NumType>, ret: Option<NumType> }

fn num_cond_safe(
    expr: &Expr,
    types: &std::collections::HashMap<String, NumType>,
    sigs: &std::collections::HashMap<String, FnSig>,
) -> bool {
    match &expr.node {
        ExprNode::Binary(op, l, r) => match op.as_str() {
            "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                num_infer_expr_type(l, types, sigs).is_some() && num_infer_expr_type(r, types, sigs).is_some()
            }
            "&&" | "||" | "and" | "or" => num_cond_safe(l, types, sigs) && num_cond_safe(r, types, sigs),
            _ => num_infer_expr_type(expr, types, sigs).is_some(),
        },
        ExprNode::Unary(op, e2) if op == "!" || op == "not" => num_cond_safe(e2, types, sigs),
        ExprNode::Bool(_) => true,
        _ => num_infer_expr_type(expr, types, sigs).is_some(),
    }
}

fn num_infer_expr_type(
    expr: &Expr,
    types: &std::collections::HashMap<String, NumType>,
    sigs: &std::collections::HashMap<String, FnSig>,
) -> Option<NumType> {
    match &expr.node {
        ExprNode::Number(n) => Some(if n.fract() == 0.0 { NumType::Int } else { NumType::Float }),
        ExprNode::Var(name) => types.get(name).copied(),
        ExprNode::Binary(op, l, r) => {
            let lt = num_infer_expr_type(l, types, sigs)?;
            let rt = num_infer_expr_type(r, types, sigs)?;
            match op.as_str() {
                "+" | "-" | "*" | "%" => Some(if lt == NumType::Float || rt == NumType::Float { NumType::Float } else { NumType::Int }),
                "/" => if lt == NumType::Int && rt == NumType::Int { None } else { Some(NumType::Float) },
                "&" | "|" | "^" | "<<" | ">>" => if lt == NumType::Int && rt == NumType::Int { Some(NumType::Int) } else { None },
                _ => None,
            }
        }
        ExprNode::Unary(op, inner) => {
            let it = num_infer_expr_type(inner, types, sigs)?;
            match op.as_str() {
                "-" => Some(it),
                "~" => if it == NumType::Int { Some(NumType::Int) } else { None },
                _ => None,
            }
        }
        ExprNode::Ternary(cond, t, e2) => {
            if !num_cond_safe(cond, types, sigs) { return None; }
            let tt = num_infer_expr_type(t, types, sigs)?;
            let et = num_infer_expr_type(e2, types, sigs)?;
            if tt == et { Some(tt) } else { None }
        }
        ExprNode::Call(callee, args) => {
            let ExprNode::Var(name) = &callee.node else { return None; };
            let mangled = mangle(name);
            let sig = sigs.get(&mangled)?;
            let ret = sig.ret?;
            if args.len() != sig.params.len() { return None; }
            for (a, pt) in args.iter().zip(sig.params.iter()) {
                let at = num_infer_expr_type(a, types, sigs)?;
                if at != *pt { return None; }
            }
            Some(ret)
        }
        _ => None,
    }
}

fn num_safe_stmt(
    stmt: &Stmt,
    types: &mut std::collections::HashMap<String, NumType>,
    sigs: &std::collections::HashMap<String, FnSig>,
    return_type: NumType,
) -> bool {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => stmts.iter().all(|s| num_safe_stmt(s, types, sigs, return_type)),
        StmtNode::Expr(e) => match &e.node {
            ExprNode::Assign(target, val) => {
                if let ExprNode::Var(n) = &target.node {
                    if let Some(existing) = types.get(n).copied() {
                        num_infer_expr_type(val, types, sigs) == Some(existing)
                    } else { false }
                } else { false }
            }
            ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
                (op == "++" || op == "--") && matches!(&inner.node, ExprNode::Var(n) if types.get(n) == Some(&NumType::Int))
            }
            _ => false,
        },
        StmtNode::Return(Some(e)) => num_infer_expr_type(e, types, sigs) == Some(return_type),
        StmtNode::Return(None) => false,
        StmtNode::Break | StmtNode::Continue => true,
        StmtNode::ConstDecl(name, e) => {
            match num_infer_expr_type(e, types, sigs) {
                Some(t) => { types.insert(name.clone(), t); true }
                None => false,
            }
        }
        StmtNode::IfChain(branches) => branches.iter().all(|(cond, b)| {
            cond.as_ref().map_or(true, |c| num_cond_safe(c, types, sigs)) && num_safe_stmt(b, types, sigs, return_type)
        }),
        StmtNode::While(cond, body) | StmtNode::DoWhile(body, cond) => {
            num_cond_safe(cond, types, sigs) && num_safe_stmt(body, types, sigs, return_type)
        }
        StmtNode::CFor(init, cond, step, body) => {
            init.as_ref().map_or(true, |e| num_safe_toplevel_expr(e, types, sigs))
                && cond.as_ref().map_or(true, |e| num_cond_safe(e, types, sigs))
                && step.as_ref().map_or(true, |e| num_safe_toplevel_expr(e, types, sigs))
                && num_safe_stmt(body, types, sigs, return_type)
        }
        _ => false,
    }
}
fn num_safe_toplevel_expr(
    e: &Expr,
    types: &std::collections::HashMap<String, NumType>,
    sigs: &std::collections::HashMap<String, FnSig>,
) -> bool {
    match &e.node {
        ExprNode::Assign(target, val) => {
            if let ExprNode::Var(n) = &target.node {
                types.get(n).copied().is_some() && num_infer_expr_type(val, types, sigs) == types.get(n).copied()
            } else { false }
        }
        ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
            (op == "++" || op == "--") && matches!(&inner.node, ExprNode::Var(n) if types.get(n) == Some(&NumType::Int))
        }
        _ => false,
    }
}

fn compute_numeric_callable_float(
    ast: &Stmt,
    direct_callable: &std::collections::HashSet<String>,
    already_handled: &std::collections::HashSet<String>,
) -> (std::collections::HashMap<String, FnSig>, std::collections::HashMap<String, Vec<String>>) {
    let mut funcdefs = Vec::new();
    collect_top_level_funcdefs(ast, &mut funcdefs);

    let mut param_types: std::collections::HashMap<String, Vec<NumType>> = std::collections::HashMap::new();
    let mut param_names: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut bodies: std::collections::HashMap<String, Stmt> = std::collections::HashMap::new();

    for (name, params, body) in &funcdefs {
        let c_name = mangle(name);
        if already_handled.contains(&c_name) { continue; }
        if !direct_callable.contains(&c_name) { continue; }
        let Some(pnames) = int_param_names(params) else { continue; };

        let mut ev = TypeEvidence::default();
        collect_evidence_stmt(body, &mut ev);
        collect_call_site_evidence_stmt(ast, name, &pnames, &mut ev);
        let Some(ptypes) = resolve_param_types(&pnames, &ev) else { continue; };
        if !stmt_definitely_returns(body) { continue; }

        param_types.insert(c_name.clone(), ptypes);
        param_names.insert(c_name.clone(), pnames);
        bodies.insert(c_name, body.clone());
    }

    let mut sigs: std::collections::HashMap<String, FnSig> = param_types
        .iter()
        .map(|(k, v)| (k.clone(), FnSig { params: v.clone(), ret: None }))
        .collect();

    let mut progressed = true;
    while progressed {
        progressed = false;
        let names: Vec<String> = sigs.keys().cloned().collect();
        for name in &names {
            if sigs[name].ret.is_some() { continue; }
            let pnames = &param_names[name];
            let ptypes = &param_types[name];
            let mut types: std::collections::HashMap<String, NumType> =
                pnames.iter().cloned().zip(ptypes.iter().copied()).collect();
            let body = &bodies[name];
            let mut collected_returns: Vec<NumType> = Vec::new();
            if collect_return_types(body, &mut types.clone(), &sigs, &mut collected_returns) {
                if let Some(&first) = collected_returns.first() {
                    if collected_returns.iter().all(|t| *t == first) {
                        sigs.get_mut(name).unwrap().ret = Some(first);
                        progressed = true;
                    }
                }
            }
        }
    }
    sigs.retain(|_, s| s.ret.is_some());

    loop {
        let mut to_remove = Vec::new();
        for name in sigs.keys() {
            let pnames = &param_names[name];
            let ptypes = &sigs[name].params;
            let ret = sigs[name].ret.unwrap();
            let mut types: std::collections::HashMap<String, NumType> =
                pnames.iter().cloned().zip(ptypes.iter().copied()).collect();
            let body = &bodies[name];
            if !num_safe_stmt(body, &mut types, &sigs, ret) {
                to_remove.push(name.clone());
            }
        }
        if to_remove.is_empty() { break; }
        for name in &to_remove { sigs.remove(name); }
    }

    param_names.retain(|k, _| sigs.contains_key(k));
    (sigs, param_names)
}

fn collect_return_types(
    stmt: &Stmt,
    types: &mut std::collections::HashMap<String, NumType>,
    sigs: &std::collections::HashMap<String, FnSig>,
    out: &mut Vec<NumType>,
) -> bool {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { if !collect_return_types(s, types, sigs, out) { return false; } }
            true
        }
        StmtNode::Return(Some(e)) => {
            match num_infer_expr_type(e, types, sigs) {
                Some(t) => { out.push(t); true }
                None => false,
            }
        }
        StmtNode::Return(None) => false,
        StmtNode::ConstDecl(name, e) => {
            match num_infer_expr_type(e, types, sigs) {
                Some(t) => { types.insert(name.clone(), t); true }
                None => false,
            }
        }
        StmtNode::IfChain(branches) => {
            for (_, b) in branches {
                let mut sub = types.clone();
                if !collect_return_types(b, &mut sub, sigs, out) { return false; }
            }
            true
        }
        StmtNode::While(_, body) | StmtNode::DoWhile(body, _) | StmtNode::CFor(_, _, _, body) => {
            let mut sub = types.clone();
            collect_return_types(body, &mut sub, sigs, out)
        }
        StmtNode::Expr(_) | StmtNode::Break | StmtNode::Continue => true,
        _ => true,
    }
}

struct NumSpecTranspiler {
    out: String,
    indent: usize,
    types: std::collections::HashMap<String, NumType>,
}
impl NumSpecTranspiler {
    fn new(types: std::collections::HashMap<String, NumType>) -> Self { Self { out: String::new(), indent: 1, types } }
    fn line(&mut self, s: &str) {
        let pad = "    ".repeat(self.indent);
        self.out.push_str(&pad);
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn emit_body(&mut self, body: &Stmt, sigs: &std::collections::HashMap<String, FnSig>) {
        self.emit_stmt(body, sigs);
        self.line("return 0; /* unreachable */");
    }
    fn emit_stmt(&mut self, stmt: &Stmt, sigs: &std::collections::HashMap<String, FnSig>) {
        match &stmt.node {
            StmtNode::Block(stmts) | StmtNode::Multi(stmts) => { for s in stmts { self.emit_stmt(s, sigs); } }
            StmtNode::Return(Some(e)) => {
                let v = self.emit_expr(e, sigs);
                self.line(&format!("return {};", v));
            }
            StmtNode::Break => self.line("break;"),
            StmtNode::Continue => self.line("continue;"),
            StmtNode::ConstDecl(name, e) => {
                let v = self.emit_expr(e, sigs);
                let t = self.types[name];
                self.line(&format!("{} {} = {};", num_c_type(t), mangle(name), v));
            }
            StmtNode::Expr(e) => match &e.node {
                ExprNode::Assign(target, val) => {
                    if let ExprNode::Var(name) = &target.node {
                        let v = self.emit_expr(val, sigs);
                        self.line(&format!("{} = {};", mangle(name), v));
                    }
                }
                ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
                    if let ExprNode::Var(name) = &inner.node {
                        let opstr = if op == "++" { "++" } else { "--" };
                        self.line(&format!("{}{};", mangle(name), opstr));
                    }
                }
                _ => {}
            },
            StmtNode::IfChain(branches) => {
                let mut first = true;
                for (cond_opt, body) in branches {
                    if let Some(cond) = cond_opt {
                        let cv = self.emit_cond(cond, sigs);
                        if first { self.line(&format!("if ({}) {{", cv)); first = false; }
                        else { self.line(&format!("}} else if ({}) {{", cv)); }
                    } else {
                        self.line("} else {");
                    }
                    self.indent += 1;
                    self.emit_stmt(body, sigs);
                    self.indent -= 1;
                }
                if !first { self.line("}"); }
            }
            StmtNode::While(cond, body) => {
                let cv = self.emit_cond(cond, sigs);
                self.line(&format!("while ({}) {{", cv));
                self.indent += 1; self.emit_stmt(body, sigs); self.indent -= 1;
                self.line("}");
            }
            StmtNode::DoWhile(body, cond) => {
                self.line("do {");
                self.indent += 1; self.emit_stmt(body, sigs); self.indent -= 1;
                let cv = self.emit_cond(cond, sigs);
                self.line(&format!("}} while ({});", cv));
            }
            StmtNode::CFor(init, cond, step, body) => {
                let init_s = init.as_ref().map(|e| self.emit_bare_expr(e, sigs)).unwrap_or_default();
                let cond_s = cond.as_ref().map(|e| self.emit_cond(e, sigs)).unwrap_or_else(|| "1".to_string());
                let step_s = step.as_ref().map(|e| self.emit_bare_expr(e, sigs)).unwrap_or_default();
                self.line(&format!("for ({}; {}; {}) {{", init_s, cond_s, step_s));
                self.indent += 1; self.emit_stmt(body, sigs); self.indent -= 1;
                self.line("}");
            }
            _ => {}
        }
    }
    fn emit_bare_expr(&mut self, e: &Expr, sigs: &std::collections::HashMap<String, FnSig>) -> String {
        match &e.node {
            ExprNode::Assign(target, val) => {
                if let ExprNode::Var(name) = &target.node {
                    let v = self.emit_expr(val, sigs);
                    format!("{} = {}", mangle(name), v)
                } else { String::new() }
            }
            ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
                if let ExprNode::Var(name) = &inner.node {
                    let opstr = if op == "++" { "++" } else { "--" };
                    format!("{}{}", mangle(name), opstr)
                } else { String::new() }
            }
            _ => String::new(),
        }
    }
    fn expr_type(&self, e: &Expr, sigs: &std::collections::HashMap<String, FnSig>) -> NumType {
        num_infer_expr_type(e, &self.types, sigs).expect("emitter only runs on already-verified expressions")
    }
    fn emit_expr(&mut self, e: &Expr, sigs: &std::collections::HashMap<String, FnSig>) -> String {
        match &e.node {
            ExprNode::Number(n) => {
                if n.fract() == 0.0 { format!("{}LL", *n as i64) } else { format!("{:.17}", n) }
            }
            ExprNode::Var(name) => mangle(name),
            ExprNode::Binary(op, l, r) => {
                let lv = self.emit_expr(l, sigs);
                let rv = self.emit_expr(r, sigs);
                if op == "/" {
                    format!("((double)({}) / (double)({}))", lv, rv)
                } else {
                    format!("({} {} {})", lv, op, rv)
                }
            }
            ExprNode::Unary(op, inner) => {
                let v = self.emit_expr(inner, sigs);
                match op.as_str() { "-" => format!("(-{})", v), "~" => format!("(~{})", v), _ => v }
            }
            ExprNode::Ternary(cond, t, e2) => {
                let cv = self.emit_cond(cond, sigs);
                let tv = self.emit_expr(t, sigs);
                let ev = self.emit_expr(e2, sigs);
                format!("({} ? {} : {})", cv, tv, ev)
            }
            ExprNode::Call(callee, args) => {
                if let ExprNode::Var(name) = &callee.node {
                    let mangled = mangle(name);
                    let arg_strs: Vec<String> = args.iter().map(|a| self.emit_expr(a, sigs)).collect();
                    format!("_fnumx_{}({})", mangled, arg_strs.join(", "))
                } else { "0".to_string() }
            }
            _ => "0".to_string(),
        }
    }
    fn emit_cond(&mut self, e: &Expr, sigs: &std::collections::HashMap<String, FnSig>) -> String {
        match &e.node {
            ExprNode::Binary(op, l, r) => match op.as_str() {
                "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                    let lv = self.emit_expr(l, sigs);
                    let rv = self.emit_expr(r, sigs);
                    format!("({} {} {})", lv, op, rv)
                }
                "&&" | "and" => { let lv = self.emit_cond(l, sigs); let rv = self.emit_cond(r, sigs); format!("({} && {})", lv, rv) }
                "||" | "or" => { let lv = self.emit_cond(l, sigs); let rv = self.emit_cond(r, sigs); format!("({} || {})", lv, rv) }
                _ => self.emit_expr(e, sigs),
            },
            ExprNode::Unary(op, inner) if op == "!" || op == "not" => { let v = self.emit_cond(inner, sigs); format!("(!{})", v) }
            ExprNode::Bool(b) => if *b { "1".to_string() } else { "0".to_string() },
            _ => self.emit_expr(e, sigs),
        }
    }
}

struct IntSpecTranspiler {
    out: String,
    indent: usize,
}
impl IntSpecTranspiler {
    fn new() -> Self { Self { out: String::new(), indent: 1 } }
    fn line(&mut self, s: &str) {
        let pad = "    ".repeat(self.indent);
        self.out.push_str(&pad);
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn emit_body(&mut self, body: &Stmt) {
        self.emit_stmt(body);
        self.line("return 0; /* unreachable */");
    }
    fn emit_stmt(&mut self, stmt: &Stmt) {
        match &stmt.node {
            StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
                for s in stmts { self.emit_stmt(s); }
            }
            StmtNode::Return(Some(e)) => {
                let v = self.emit_expr(e);
                self.line(&format!("return {};", v));
            }
            StmtNode::Break => self.line("break;"),
            StmtNode::Continue => self.line("continue;"),
            StmtNode::ConstDecl(name, e) => {
                let v = self.emit_expr(e);
                self.line(&format!("int64_t {} = {};", mangle(name), v));
            }
            StmtNode::Expr(e) => match &e.node {
                ExprNode::Assign(target, val) => {
                    if let ExprNode::Var(name) = &target.node {
                        let v = self.emit_expr(val);
                        self.line(&format!("{} = {};", mangle(name), v));
                    }
                }
                ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
                    if let ExprNode::Var(name) = &inner.node {
                        let opstr = if op == "++" { "++" } else { "--" };
                        self.line(&format!("{}{};", mangle(name), opstr));
                    }
                }
                _ => {}
            },
            StmtNode::IfChain(branches) => {
                let mut first = true;
                for (cond_opt, body) in branches {
                    if let Some(cond) = cond_opt {
                        let cv = self.emit_cond(cond);
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
                    self.emit_stmt(body);
                    self.indent -= 1;
                }
                if !first { self.line("}"); }
            }
            StmtNode::While(cond, body) => {
                let cv = self.emit_cond(cond);
                self.line(&format!("while ({}) {{", cv));
                self.indent += 1; self.emit_stmt(body); self.indent -= 1;
                self.line("}");
            }
            StmtNode::DoWhile(body, cond) => {
                self.line("do {");
                self.indent += 1; self.emit_stmt(body); self.indent -= 1;
                let cv = self.emit_cond(cond);
                self.line(&format!("}} while ({});", cv));
            }
            StmtNode::CFor(init, cond, step, body) => {
                let init_s = init.as_ref().map(|e| self.emit_bare_expr(e)).unwrap_or_default();
                let cond_s = cond.as_ref().map(|e| self.emit_cond(e)).unwrap_or_else(|| "1".to_string());
                let step_s = step.as_ref().map(|e| self.emit_bare_expr(e)).unwrap_or_default();
                self.line(&format!("for ({}; {}; {}) {{", init_s, cond_s, step_s));
                self.indent += 1; self.emit_stmt(body); self.indent -= 1;
                self.line("}");
            }
            _ => {}
        }
    }
    fn emit_bare_expr(&mut self, e: &Expr) -> String {
        match &e.node {
            ExprNode::Assign(target, val) => {
                if let ExprNode::Var(name) = &target.node {
                    let v = self.emit_expr(val);
                    format!("{} = {}", mangle(name), v)
                } else { String::new() }
            }
            ExprNode::PreOp(op, inner) | ExprNode::PostOp(op, inner) => {
                if let ExprNode::Var(name) = &inner.node {
                    let opstr = if op == "++" { "++" } else { "--" };
                    format!("{}{}", mangle(name), opstr)
                } else { String::new() }
            }
            _ => String::new(),
        }
    }
    fn emit_expr(&mut self, e: &Expr) -> String {
        match &e.node {
            ExprNode::Number(n) => format!("{}LL", *n as i64),
            ExprNode::Var(name) => mangle(name),
            ExprNode::Binary(op, l, r) => {
                let lv = self.emit_expr(l);
                let rv = self.emit_expr(r);
                format!("({} {} {})", lv, op, rv)
            }
            ExprNode::Unary(op, inner) => {
                let v = self.emit_expr(inner);
                match op.as_str() {
                    "-" => format!("(-{})", v),
                    "~" => format!("(~{})", v),
                    _ => v,
                }
            }
            ExprNode::Ternary(cond, t, e2) => {
                let cv = self.emit_cond(cond);
                let tv = self.emit_expr(t);
                let ev = self.emit_expr(e2);
                format!("({} ? {} : {})", cv, tv, ev)
            }
            ExprNode::Call(callee, args) => {
                if let ExprNode::Var(name) = &callee.node {
                    let mangled = mangle(name);
                    let arg_strs: Vec<String> = args.iter().map(|a| self.emit_expr(a)).collect();
                    format!("_fnum_{}({})", mangled, arg_strs.join(", "))
                } else {
                    "0".to_string()
                }
            }
            _ => "0".to_string(),
        }
    }
    fn emit_cond(&mut self, e: &Expr) -> String {
        match &e.node {
            ExprNode::Binary(op, l, r) => match op.as_str() {
                "==" | "!=" | "<" | "<=" | ">" | ">=" => {
                    let lv = self.emit_expr(l);
                    let rv = self.emit_expr(r);
                    format!("({} {} {})", lv, op, rv)
                }
                "&&" | "and" => {
                    let lv = self.emit_cond(l);
                    let rv = self.emit_cond(r);
                    format!("({} && {})", lv, rv)
                }
                "||" | "or" => {
                    let lv = self.emit_cond(l);
                    let rv = self.emit_cond(r);
                    format!("({} || {})", lv, rv)
                }
                _ => self.emit_expr(e),
            },
            ExprNode::Unary(op, inner) if op == "!" || op == "not" => {
                let v = self.emit_cond(inner);
                format!("(!{})", v)
            }
            ExprNode::Bool(b) => if *b { "1".to_string() } else { "0".to_string() },
            _ => self.emit_expr(e),
        }
    }
}

pub(crate) static APP_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) fn transpile(ast: &Stmt, _is_lib: bool, base_dir: &PathBuf) -> String {
    let mut out = String::new();
    out.push_str(RUNTIME_HEADER);

    let mut t = Transpiler::new();

    let mut top_level_vars = std::collections::HashSet::new();
    collect_top_level_decls(ast, &mut top_level_vars);

    top_level_vars.insert("_v_PI".into());
    top_level_vars.insert("_v_E".into());
    top_level_vars.insert("_v_argv".into());

    if let Some(scope) = t.declared.last_mut() {
        for v in &top_level_vars {
            scope.insert(v.clone());
        }
    }

    let mut top_fns = std::collections::HashSet::new();
    collect_top_level_funcs(ast, &mut top_fns);
    let mut mutated = std::collections::HashSet::new();
    collect_all_mutated_names(ast, &mut mutated);
    t.direct_callable = top_fns.difference(&mutated).cloned().collect();

    let app_mode = APP_MODE.load(std::sync::atomic::Ordering::Relaxed);
    let (numeric_callable, numeric_params) = if app_mode {
        (std::collections::HashSet::new(), std::collections::HashMap::new())
    } else {
        compute_numeric_callable(ast, &t.direct_callable)
    };
    t.numeric_callable = numeric_callable;
    t.numeric_params = numeric_params;

    if !t.direct_callable.is_empty() {
        let mut sorted_direct: Vec<&String> = t.direct_callable.iter().collect();
        sorted_direct.sort();
        out.push_str("/* Forward declarations for direct static calls */\n");
        for name in &sorted_direct {
            out.push_str(&format!(
                "static DgnVal _fn_{}(DgnVal* _args, int _nargs, DgnEnv* _env);\n",
                name
            ));
        }
        out.push_str("\n");
    }

    if !t.numeric_callable.is_empty() {
        let mut sorted_numeric: Vec<&String> = t.numeric_callable.iter().collect();
        sorted_numeric.sort();
        out.push_str("/* Forward declarations for the int64_t fast-path functions */\n");
        for name in &sorted_numeric {
            let arity = t.numeric_params.get(*name).map(|p| p.len()).unwrap_or(0);
            let sig: Vec<&str> = std::iter::repeat("int64_t").take(arity).collect();
            let sig_str = if sig.is_empty() { "void".to_string() } else { sig.join(", ") };
            out.push_str(&format!("static int64_t _fnum_{}({});\n", name, sig_str));
        }
        out.push_str("\n");
    }

    let (numeric_callable_float, numeric_params_float) = if app_mode {
        (std::collections::HashMap::new(), std::collections::HashMap::new())
    } else {
        compute_numeric_callable_float(ast, &t.direct_callable, &t.numeric_callable)
    };
    t.numeric_callable_float = numeric_callable_float;
    t.numeric_params_float = numeric_params_float;

    if !t.numeric_callable_float.is_empty() {
        let mut sorted: Vec<&String> = t.numeric_callable_float.keys().collect();
        sorted.sort();
        out.push_str("/* Forward declarations for the float-aware fast-path functions */\n");
        for name in &sorted {
            let sig = &t.numeric_callable_float[*name];
            let ret_c = num_c_type(sig.ret.unwrap());
            let params_c: Vec<&str> = sig.params.iter().map(|t| num_c_type(*t)).collect();
            let params_str = if params_c.is_empty() { "void".to_string() } else { params_c.join(", ") };
            out.push_str(&format!("static {} _fnumx_{}({});\n", ret_c, name, params_str));
        }
        out.push_str("\n");
    }

    let mut classes = std::collections::HashMap::new();
    collect_classes(ast, &mut classes);
    t.overridable_methods = compute_overridable_methods(&classes);
    t.classes = classes;

    if !t.classes.is_empty() {
        let mut sorted_classes: Vec<&String> = t.classes.keys().collect();
        sorted_classes.sort();
        out.push_str("/* Forward declarations for class methods (devirtualized-call targets) */\n");
        for cname in &sorted_classes {
            let info = &t.classes[*cname];
            let mut sorted_methods: Vec<&String> = info.own_methods.iter().collect();
            sorted_methods.sort();
            for mname in sorted_methods {
                out.push_str(&format!(
                    "static DgnVal _fn_{}_{}(DgnVal* _args, int _nargs, DgnEnv* _env);\n",
                    mangle(cname), mangle(mname)
                ));
            }
        }
        out.push_str("\n");
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

    let mut sorted_globals: Vec<String> = top_level_vars.into_iter().collect();
    sorted_globals.sort();

    out.push_str("/* GC extern forward declarations */\n");
    for g in &sorted_globals {
        out.push_str(&format!("extern DgnVal {};\n", g));
    }
    out.push_str("\n");

    for g in &sorted_globals {
        out.push_str(&format!("DgnVal {};\n", g));
    }
    out.push_str("\n");

    out.push_str("/* Sentinel-terminated array of all top-level globals for GC rooting */\n");
    out.push_str("DgnVal* _dgn_global_roots[] = {\n");
    for g in &sorted_globals {
        out.push_str(&format!("    &{},\n", g));
    }
    out.push_str("    NULL\n};\n\n");

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

fn has_named_pack(args: &[Expr]) -> bool {
    matches!(args.last().map(|a| &a.node), Some(ExprNode::Obj(pairs)) if pairs.first().map(|(k, _)| k == "__named__").unwrap_or(false))
}

fn param_csv(params: &[(String, Option<String>)]) -> String {
    params.iter().map(|(p, _)| {
        let n = p.split(':').next().unwrap_or(p).trim();
        let n = n.strip_prefix("...").unwrap_or(n);
        n.to_string()
    }).collect::<Vec<_>>().join(",")
}

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
            if !bound.contains(&c) && !skip_outside_closure() { out.insert(c); }
        }
        ExprNode::This => {
            let c = mangle("this");
            if !bound.contains(&c) && !skip_outside_closure() { out.insert(c); }
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
        ExprNode::FuncExpr(params, _, body) => {
            let mut inner_bound = bound.clone();
            for (p, _) in params {
                let pname = p.split(':').next().unwrap_or(p).trim();
                let actual = if pname.starts_with("...") { &pname[3..] } else { pname };
                inner_bound.insert(mangle(actual));
            }
            IN_CLOSURE.with(|c| c.set(c.get() + 1));
            collect_free_vars_stmt(body, &inner_bound, out);
            IN_CLOSURE.with(|c| c.set(c.get() - 1));
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
            t.current_class = None;
            t.emit_named_fn(&mangle(name), params, body);
        }
        StmtNode::ClassDef(name, _parent, body) => {
            if let StmtNode::Block(stmts) = &body.node {
                for s in stmts {
                    if let StmtNode::FuncDef(mname, params, _req, mbody, _) = &s.node {
                        let fn_c = format!("{}_{}", mangle(name), mangle(mname));
                        let mut all_params = vec![("this".to_string(), None)];
                        all_params.extend(params.iter().cloned());
                        t.current_class = Some(name.clone());
                        t.emit_named_fn(&fn_c, &all_params, mbody);
                    }
                }
                let mut field_stmts = Vec::new();
                for s in stmts { collect_field_defaults(s, &mut field_stmts); }
                let fi_fn_c = format!("{}___fieldinit__", mangle(name));
                let fi_body = Stmt { node: StmtNode::Block(field_stmts), line: body.line };
                let fi_params = vec![("this".to_string(), None)];
                t.current_class = Some(name.clone());
                t.emit_named_fn(&fi_fn_c, &fi_params, &fi_body);
                t.current_class = None;
            }
        }
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

pub(crate) fn load_user_module(module: &str, base_dir: &PathBuf) -> Option<Stmt> {
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

fn hoist_module_decls(module: &str, ast: &Stmt, t: &mut Transpiler, base_dir: &PathBuf) {

    let stmts = match &ast.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => s,
        _ => return,
    };

    let mod_prefix = format!("_mod_{}", mangle(module));

    let mut mod_vars: Vec<(String, String)> = Vec::new();
    for s in stmts {
        match &s.node {
            StmtNode::FuncDef(fname, _, _, _, _) => {
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

    for (var, _) in &mod_vars {
        writeln!(t.hoisted, "static DgnVal {}_{};", mod_prefix, var).unwrap();
    }

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
} else {
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
            StmtNode::FuncDef(fname, params, _, _, _) => {
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

    for (var, _) in &mod_vars {
        writeln!(t.hoisted, "#undef {}", var).unwrap();
    }
}

fn emit_module_map_entries(module: &str, bind: &str, ast: &Stmt, t: &mut Transpiler) {
    let stmts = match &ast.node {
        StmtNode::Block(s) | StmtNode::Multi(s) => s,
        _ => return,
    };
    let indent = "    ".repeat(t.indent);
    let c_name = mangle(bind);

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
            writeln!(t.out, "{}{} = dgn_func_n(_fn_{}, {}, \"{}\");", indent, decl, c_name, params.len(), param_csv(params)).unwrap();
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
                        writeln!(t.out, "{}dgn_class_add_method_n(&{}, \"{}\", _fn_{}, {}, \"{}\");", indent, c_name, escape_c_string(mname), fn_c, params.len(), param_csv(params)).unwrap();
                    }
                }
            }
            {
                let c_name = mangle(name);
                let fi_fn_c = format!("{}___fieldinit__", mangle(name));
                writeln!(t.out, "{}dgn_class_add_method(&{}, \"__field_init__\", _fn_{}, 1);", indent, c_name, fi_fn_c).unwrap();
            }
            if let Some(p) = parent {
                let c_name = mangle(name);
                writeln!(t.out, "{}dgn_class_set_parent(&{}, {});", indent, c_name, mangle(p)).unwrap();
            }
            writeln!(t.out, "{}dgn_class_init_statics(&{});", indent, mangle(name)).unwrap();
        }
        StmtNode::Import(module, alias) if !is_builtin_module(module) => {
            let bind = alias.as_deref().unwrap_or(module.as_str());
            let c_name = mangle(bind);
            let indent = "    ".repeat(t.indent);
            writeln!(t.out, "{}{} = dgn_map_new();", indent, c_name).unwrap();
            if let Some(mod_ast) = load_user_module(module, base_dir) {
                emit_module_map_entries(module, bind, &mod_ast, t);
            }
            writeln!(t.out, "{}(void){};", indent, c_name).unwrap();
        }
        other_stmt => {
            t.emit_stmt(&Stmt { node: other_stmt.clone(), line: 0 });
        }
    }
}

thread_local! {
    static CLOSURE_ONLY: std::cell::Cell<bool> = std::cell::Cell::new(false);
    static IN_CLOSURE: std::cell::Cell<i32> = std::cell::Cell::new(0);
}

fn closure_captured_names(body: &Stmt) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    CLOSURE_ONLY.with(|c| c.set(true));
    IN_CLOSURE.with(|c| c.set(0));
    collect_free_vars_stmt(body, &std::collections::HashSet::new(), &mut out);
    CLOSURE_ONLY.with(|c| c.set(false));
    out
}

fn skip_outside_closure() -> bool {
    CLOSURE_ONLY.with(|c| c.get()) && IN_CLOSURE.with(|c| c.get()) == 0
}
