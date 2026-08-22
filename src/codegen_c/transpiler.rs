#![allow(dead_code)]

use super::*;

fn collect_field_defaults(s: &Stmt, out: &mut Vec<Stmt>) {
    match &s.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for st in stmts { collect_field_defaults(st, out); }
        }
        StmtNode::FuncDef(..) => { /* handled separately as a method */ }
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
    closure_vars: std::collections::HashSet<String>,
    hoisted_modules: std::collections::HashSet<String>,
    // Mirrors `declared` 1:1 (pushed/popped alongside it in push_scope/
    // pop_scope) but, unlike `declared`, ALWAYS records every name bound in
    // a scope via `decl_var`, even when that name coincides with an
    // already-known outer/global name. `declared`+`is_declared` exists to
    // decide whether a *C* declaration needs a fresh `DgnVal` prefix or can
    // reuse an existing local, and short-circuits when a name is already
    // known anywhere (including globally) — which means a parameter that
    // happens to share a name with a top-level function silently never gets
    // registered as shadowing it there. `is_shadowed_locally` needs the
    // unconditional version, so it's tracked separately here rather than by
    // changing the semantics of `declared`/`decl_var` (which other codegen
    // paths already depend on).
    local_names: Vec<std::collections::HashSet<String>>,
    // Mangled names of top-level functions that are (a) defined exactly once
    // as a top-level `func`, and (b) never reassigned/mutated anywhere in the
    // whole program (verified by a whole-program scan — see
    // `collect_all_mutated_names`). Call sites naming one of these directly,
    // e.g. `fib(n - 1)`, can therefore skip the generic dynamic dispatch
    // (`dgn_call_val`, which tag-checks a DgnVal, chases a heap-allocated
    // DgnFunc pointer, then calls through a function pointer) and instead
    // call the mangled C function directly — `_fn_fib(...)`. This is only
    // applied when the name isn't shadowed by a local (param/let/for-var/
    // catch-var) in the current scope chain; see `is_shadowed_locally`.
    direct_callable: std::collections::HashSet<String>,
    // Mangled names of top-level functions proven safe for full int64_t
    // specialization (see the "Integer fast-path specialization" section
    // below for the exact soundness conditions). A strict subset of
    // `direct_callable` — every numeric_callable function is also
    // direct_callable, but not vice versa.
    numeric_callable: std::collections::HashSet<String>,
    // Parameter names (unmangled) for each numeric_callable function, in
    // declaration order — needed to emit both the boxed shell's unboxing
    // and the raw _fnum_ function's C parameter list.
    numeric_params: std::collections::HashMap<String, Vec<String>>,
    // Float-aware specialization (additive, separate set -- see the
    // "Float-aware numeric specialization" section). Only ever contains
    // functions numeric_callable (the int-only set) did NOT already claim.
    numeric_callable_float: std::collections::HashMap<String, FnSig>,
    numeric_params_float: std::collections::HashMap<String, Vec<String>>,
    // Method-call devirtualization (see "Method-call devirtualization"
    // section below). `classes` is the whole-program class hierarchy;
    // `overridable_methods` holds (unmangled_class, unmangled_method) pairs
    // that are UNSAFE to devirtualize because some descendant overrides
    // them. `current_class` tracks which class's method body is currently
    // being transpiled, so `this.foo(...)` call sites know their context.
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
            closure_vars: std::collections::HashSet::new(),
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

    // Like `is_declared`, but ignores scope 0 (the persistent top-level/global
    // scope, pre-filled once in `transpile()` with every module-scope name),
    // and — unlike `is_declared`/`declared` — is always accurate about local
    // bindings even when a local name coincides with a global one (see the
    // `local_names` field doc comment for why `declared` can't be reused for
    // this check). Used to check whether a top-level function name is
    // shadowed by a local binding (parameter, for-loop var, catch var, local
    // const) in the current function's own scope chain.
    fn is_shadowed_locally(&self, c_name: &str) -> bool {
        self.local_names.iter().skip(1).any(|scope| scope.contains(c_name))
    }

    fn decl_var(&mut self, name: &str) -> String {
        let c_name = mangle(name);
        if let Some(current) = self.local_names.last_mut() {
            current.insert(c_name.clone());
        }
        if !self.is_declared(&c_name) {
            if let Some(current) = self.declared.last_mut() {
                current.insert(c_name.clone());
                return format!("DgnVal {}", c_name);
            }
        }
        c_name
    }


    // Tries the string-accumulator loop optimization for stmts[i..]: a
    // `name = "literal"` seed immediately followed by a qualifying While/
    // DoWhile/CFor loop. Returns how many statements it consumed (2) and
    // emits the optimized form directly, or None (emits nothing) if the
    // pattern doesn't apply -- callers fall back to normal per-statement
    // emission in that case.
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

        // Scan forward for the loop: any number of intervening statements
        // are fine (e.g. `s = ""; i = 0; while (...) { s = s + x; i++ }`)
        // as long as NONE of them reference `name` -- if one does, bail
        // rather than guess what it means for the accumulator.
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

        // -- Safe to apply: emit the seed, then any intervening statements
        // verbatim (normal codegen), then the strbuf-optimized loop,
        // mirroring the real While/DoWhile/CFor codegen exactly (labels,
        // scoping, braces) so break/continue inside the loop keep working
        // identically -- the ONLY difference from the normal path is that
        // wherever the real codegen would call `self.emit_stmt(body)`,
        // this calls `emit_rewritten_body` instead, which is identical
        // except the one matched accumulation statement becomes
        // strbuf-append calls.
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
                self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

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
                self.loop_ctx.pop();
            }
            LoopShape::DoWhile(_, c) => {
                let cont_lbl = self.fresh_lbl();
                let brk_lbl = self.fresh_lbl();
                self.loop_ctx.push((Some(cont_lbl.clone()), brk_lbl.clone()));

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
                self.loop_ctx.pop();
            }
            LoopShape::CFor(init, cond, step, _) => {
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
                self.loop_ctx.pop();
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

                if self.numeric_callable.contains(&c_name) {
                    // Fully specialized path: the boxed function becomes a
                    // thin shell — unbox params once, delegate to the raw
                    // int64_t function, box the result once. No self-binding
                    // needed here at all: the shell never dispatches
                    // dynamically to itself, and the raw body's own
                    // recursive/sibling calls go straight to other
                    // _fnum_ functions (see IntSpecTranspiler), never through
                    // dgn_call_val or a DgnVal self-reference.
                    let saved_indent = self.indent;
                    self.indent = 0;
                    let saved_out = std::mem::take(&mut self.out);

                    let pnames = self.numeric_params.get(&c_name).cloned().unwrap_or_default();

                    // The raw int64_t function.
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

                    // The boxed shell.
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

                    // Runtime binding in the current scope (caller side) —
                    // unchanged: dynamic callers (passing this function as a
                    // value, e.g. to another function, or list.map(fib))
                    // still go through the normal boxed _fn_ entry point.
                    let decl = self.decl_var(name);
                    let indent = "    ".repeat(self.indent);
                    writeln!(
                        self.out,
                        "{}{} = dgn_func(_fn_{}, {});",
                        indent, decl, c_name, params.len()
                    ).unwrap();
                    return;
                }

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

                    // Self-binding: lets the function call itself by name
                    // recursively via a bare value (e.g. passed as a
                    // callback, stored, or called indirectly through a
                    // variable). This is dead weight — an extra dgn_gc_alloc
                    // on every single invocation — when the function is
                    // direct_callable (so every "name(...)" call already
                    // resolves statically to _fn_name(...) and never reads
                    // this binding) AND nothing inside its body ever refers
                    // to its own name except as that direct-call callee, AND
                    // nothing inside the body redeclares/shadows the name
                    // (which would make the shadowing-aware skip at call
                    // sites diverge from this whole-body analysis). All three
                    // must hold, or we conservatively keep the self-binding.
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
                            writeln!(self.out, "{}{} = dgn_func(_fn_{}, {});", ind, self_decl, c_name, params.len()).unwrap();
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
                    (pre, old_tmp) // PostOp returns old value
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
                    (pre, old_tmp) // PostOp returns old value
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

    // Returns the mangled `ClassMangled_MethodMangled` target to call
    // directly, if `method` called as `this.method(...)` from within a
    // method of `from_class` is provably safe to devirtualize.
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

        // Method-call devirtualization: `this.method(args)` inside a method
        // of class C can skip dgn_call_method's hashmap lookup + malloc'd
        // arg-array dance entirely IF we can prove the call always resolves
        // to the same implementation regardless of `this`'s actual runtime
        // subtype -- i.e. no descendant of the class that actually defines
        // `method` (found by walking up C's ancestor chain) overrides it.
        // See "Method-call devirtualization" section below for the analysis.
        if let ExprNode::GetAttr(obj, method) = &callee.node {
            if matches!(obj.node, ExprNode::This) {
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
                _ => {
                    let c_name = mangle(name);
                    if self.direct_callable.contains(&c_name)
                        && !self.is_shadowed_locally(&c_name)
                    {
                        // Statically-known, never-reassigned top-level function:
                        // call the generated C function directly instead of
                        // going through the generic dynamic dispatch path.
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

        // Fully-specialized int64_t path (see "Integer fast-path
        // specialization"): only ever true for a genuine top-level function
        // name (hoist_top_level passes fn_c == mangle(name) for those; class
        // methods and field-init get a composite fn_c that can never match
        // an entry in numeric_callable, so this can't misfire onto a method).
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

        // Float-aware specialized path (see "Float-aware numeric
        // specialization"). Same shell-wraps-raw-function shape as the
        // int-only path above, but the boxed shell defensively coerces
        // each incoming argument based on its ACTUAL runtime tag rather
        // than assuming it matches the statically-inferred type: nothing
        // stops a dynamically-typed caller outside the whole-program
        // evidence scan from passing an int where float was inferred (or
        // vice versa), and blindly reading the wrong union field would
        // read garbage bits, not a sane converted value.
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
// ── Direct static-call analysis ─────────────────────────────────────────────
// Collects the mangled names of every function defined with a top-level
// `func name(...)` statement (not nested inside another function/lambda, not
// a class method — those are dispatched differently already).
fn collect_top_level_funcs(stmt: &Stmt, out: &mut std::collections::HashSet<String>) {
    match &stmt.node {
        StmtNode::Block(stmts) | StmtNode::Multi(stmts) => {
            for s in stmts { collect_top_level_funcs(s, out); }
        }
        StmtNode::FuncDef(name, _, _, _, _) => { out.insert(mangle(name)); }
        _ => {}
    }
}

// Whole-program scan for every name that is ever reassigned or incremented/
// decremented as a bare identifier — i.e. `name = ...`, `name++`, `--name`,
// anywhere in the entire program, including inside function bodies, nested
// lambdas, and class methods (unlike `collect_free_vars`, this does NOT stop
// at nested function boundaries: a reassignment inside a nested function
// still means the outer name is mutable). A name that never appears here
// can never change value after its initial binding, so it's safe to resolve
// statically at every call site.
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

// ── Self-binding elision analysis ───────────────────────────────────────────
// A recursive top-level function normally binds its own name to a fresh
// `dgn_func()` value (heap-allocated) on every single call, purely so the
// body *could* reference itself as a plain value. When the function is
// direct_callable, every "name(...)" call inside it already resolves
// statically to the C function (see `direct_callable`), so that binding goes
// completely unread — except it still pays for a GC-tracked allocation on
// every invocation. These two checks tell us whether it's safe to skip it:
// `mangled` is the already-mangled C name being searched for.

// True if `name` is redeclared/shadowed anywhere inside the body (as a
// param, local const, for-loop var, catch var, or a nested function's own
// name/params). If so we conservatively keep the self-binding, since the
// call-site shadowing check (`is_shadowed_locally`) is scope-position-aware
// in a way this whole-body pass isn't, and diverging between the two could
// silently make a shadowed inner call try to read the (now-missing) outer
// binding.
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

// True if `name` is ever referenced as a plain value anywhere in the body —
// i.e. any `Var(name)` that is NOT exactly the callee of a `Call(Var(name),
// args)` node. Passing the function itself around (as a callback, stored in
// a list, compared, returned bare, etc.) all count as "used as a value" and
// force us to keep the real self-binding.
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
            // The callee slot is exempt ONLY when it's exactly `name` — that's
            // the direct-call pattern that never needs the bound value.
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


// ── Method-call devirtualization ────────────────────────────────────────────
// `obj.method(args)` normally goes through dgn_call_method, which does a
// hashmap lookup by string AND an unconditional malloc/memcpy/free to
// prepend `obj` onto the argument array -- real cost on every single call.
// For the common, safe case of `this.method(...)` called from within a
// method body, we can often prove the call always resolves to exactly one
// implementation no matter what `this`'s actual runtime subtype is, and
// call that C function directly with no allocation and no lookup.
//
// Soundness: `this` inside a method literally written on class C could, at
// runtime, be an instance of C or of any subclass of C that doesn't itself
// override the CURRENTLY EXECUTING method (if it did, this code wouldn't
// run for it -- a different override would run instead). `method` itself
// resolves, starting from C, to the nearest ancestor-or-self that defines
// it (`resolve_method_owner`). It's then safe to call that resolved class's
// implementation directly as long as NO descendant of it overrides
// `method` (`overridable_methods`) -- checking descendants of the
// resolving class rather than of C is deliberately the broader, safer
// check (a superset of what's strictly necessary), not the tightest
// possible one.

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

// Walks up the ancestor chain starting at `start_class` (inclusive) looking
// for the nearest class that defines `method` directly. This mirrors the
// runtime's own resolution order (dgn_call_method's parent-chain walk).
fn resolve_method_owner(
    classes: &std::collections::HashMap<String, ClassInfo>,
    start_class: &str,
    method: &str,
) -> Option<String> {
    let mut cur = Some(start_class.to_string());
    let mut guard = 0; // defends against a malformed/cyclic parent chain
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


// For a top-level function where every parameter, local, and return value is
// PROVABLY a plain integer (no strings/lists/maps/closures/floats/division/
// exceptions/dynamic each-loops anywhere in the body, and every code path
// definitely returns an int-shaped value), we emit a second, fully unboxed
// C function operating on raw int64_t alongside the normal boxed DgnVal
// entry point (_fn_<name>). The boxed entry point becomes a thin shell:
// unbox params once, delegate to the raw function (_fnum_<name>), box the
// result once. Calls between two specializable functions route directly
// through the raw path, so a whole call tree of numeric recursion (fib,
// factorial, gcd, ...) pays the boxing tax exactly once at the outer
// boundary instead of on every single operation and every call.
//
// This is conservative BY CONSTRUCTION: anything the analysis can't prove
// safe disqualifies the WHOLE function, which then falls back to the
// existing, unchanged, fully-boxed codegen. In particular:
//   - No floats or "/" (true division can produce a float; unsafe to assume
//     int without knowing divisibility at compile time).
//   - No strings, lists, maps, attribute/index access, closures (FuncExpr),
//     exceptions (Try/Raise), new/super/await, switch, import, class,
//     augment, or "each" for-loops (dynamic iterable type).
//   - No default or rest parameters.
//   - Boolean-shaped values (comparisons, &&/||/and/or, `!`/`not`, bool
//     literals) are only permitted in condition position (if/while/ternary
//     conditions) — never stored in a local, passed as an argument, or
//     returned — because DGN_BOOL and DGN_INT are different runtime tags,
//     and comparing a wrongly-tagged result against `true`/`false` from
//     outside would silently give the wrong answer. See `cond_safe_expr`
//     vs `int_safe_expr` below.
//   - Every `Var` reference must resolve to a parameter or a local this
//     same pass has already seen declared (params pre-seed the set; a
//     ConstDecl adds to it as the walk proceeds) — anything else (globals,
//     a name from an enclosing/unrelated scope) disqualifies. This tracking
//     is deliberately flat/conservative rather than precisely scope-aware;
//     if it's ever too permissive for some corner case, the raw C emitted
//     from it would reference an undeclared C identifier and fail to
//     *compile* — a loud build-time failure, not a silent runtime bug.
//   - A function must have at least one branch that provably returns on
//     every path (see `block_definitely_returns`) — no implicit "falls off
//     the end returning null" allowed, since that's not representable as
//     an int64_t.
//   - Whole-program fixpoint: a function that's otherwise structurally safe
//     but calls a function that ISN'T (transitively) numeric-safe is also
//     excluded, so mutual recursion is only fast-pathed when every function
//     in the cycle is provably numeric.

// ── String-building loop optimization ───────────────────────────────────────
// `s = s + x` inside a loop, via the ordinary boxed path, does a full
// malloc+strcpy+strcat+free EVERY iteration (dgn_add's string-concat
// branch has to build a brand new buffer big enough for the WHOLE
// accumulated string so far, every single time) -- the classic O(n^2)
// string-building antipattern for n concatenations. When we can prove a
// loop's body does nothing to a variable except this exact accumulation
// pattern, we rewrite it to append into a growable DgnStrBuf instead
// (amortized O(1) per append) and materialize back into a real Dragon
// string exactly once, after the loop.
//
// Soundness hinges entirely on `name` not being observably read as a
// "live" Dragon string anywhere inside the loop except as the immediate
// left operand of the accumulation itself -- if it were, that read would
// see a stale pre-loop value once we stop keeping the DgnVal itself
// updated each iteration. `find_and_validate_accumulator` enforces this by
// requiring `name` doesn't appear ANYWHERE else in the loop body (a full,
// conservative scan) -- and only fires on a variable already known to be a
// plain string (seeded immediately before the loop by a literal), never on
// something whose type could be anything else.

// True if `expr` is a left-associative chain of `+` whose leftmost operand
// is exactly `Var(name)` -- e.g. `s + a + "_" + b`. Returns the ordered
// list of appended pieces (a, "_", b) if so.
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
        ExprNode::FuncExpr(_, _, body) => stmt_references_var(body, name), // a closure capturing `name`: bail
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

// Scans the loop body's top-level statements for exactly one accumulation
// of `name` in the required shape, and verifies `name` doesn't appear
// anywhere else in the body at all. The accumulation must be a direct
// top-level statement of the loop body (not nested inside an inner if/
// loop) -- deliberately conservative: it means some real accumulator loops
// won't qualify, but every one that DOES is unambiguously safe.
fn find_and_validate_accumulator(body_stmts: &[Stmt], name: &str) -> Option<(usize, Vec<Expr>)> {
    let mut found: Option<(usize, Vec<Expr>)> = None;
    for (i, s) in body_stmts.iter().enumerate() {
        let mut matched_here = false;
        if let StmtNode::Expr(e) = &s.node {
            if let ExprNode::Assign(target, val) = &e.node {
                if let ExprNode::Var(n) = &target.node {
                    if n == name {
                        let chain = extract_accum_chain(val, name)?; // wrong shape assignment to name -> bail entirely
                        if chain.iter().any(|p| expr_references_var(p, name)) { return None; }
                        if found.is_some() { return None; } // more than one assignment to name -> bail
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
        if default.is_some() { return None; } // default params: out of scope for v1
        let clean = p.split(':').next().unwrap_or(p).trim();
        if clean.starts_with("...") { return None; } // rest params: out of scope for v1
        if clean.is_empty() { return None; }
        out.push(clean.to_string());
    }
    Some(out)
}

// True if `e`'s value is safe to use as a plain integer: stored in a local,
// passed as an argument, returned, or combined arithmetically. Comparisons,
// logical ops, and bool literals are deliberately EXCLUDED here — they only
// ever appear via `cond_safe_expr` (see above).
fn int_safe_expr(expr: &Expr, locals: &std::collections::HashSet<String>, called: &mut std::collections::HashSet<String>) -> bool {
    match &expr.node {
        ExprNode::Number(n) => n.fract() == 0.0,
        ExprNode::Var(name) => locals.contains(name),
        ExprNode::Binary(op, l, r) => match op.as_str() {
            "+" | "-" | "*" | "%" | "&" | "|" | "^" | "<<" | ">>" => {
                int_safe_expr(l, locals, called) && int_safe_expr(r, locals, called)
            }
            _ => false, // "/" excluded (may produce float); comparisons/logical excluded (bool-shaped)
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
        // Everything else (strings, lists, maps, attr/index, assign-as-expr,
        // pre/post-op-as-expr, closures, new/super/await, bools, null, this)
        // is out of scope for v1.
        _ => false,
    }
}

// True if `e` is safe to use in a CONDITION position (if/while/ternary
// condition): either a genuine boolean-shaped expression built recursively
// from other condition-safe pieces, or a plain int used via C truthiness
// (matching dgn_truthy's "nonzero is truthy" rule for ints).
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

// Structural walk of a statement. Threads `locals` forward (params pre-seed
// it; ConstDecl grows it) and accumulates every top-level function name
// called (for the whole-program fixpoint). Returns false anywhere the body
// contains something this pass can't prove safe.
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
            _ => false, // bare-statement calls (e.g. print(...)) and anything else: bail
        },
        StmtNode::Return(Some(e)) => int_safe_expr(e, locals, called),
        StmtNode::Return(None) => false, // "returns null" isn't representable as int64_t
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
        // For-each (dynamic iterable), Switch, Try, nested FuncDef, ClassDef,
        // Augment, Import, Raise: all disqualify.
        _ => false,
    }
}

// CFor's init/step slots are bare Exprs (usually an Assign or a PreOp/PostOp,
// same shapes allowed at statement level) rather than full Stmts.
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

// True if every path through `stmts` (as a sequence at this block level)
// definitely reaches a `Return`. Loops are conservatively assumed NOT to
// guarantee a return by themselves (they might not execute, or exit via
// break) — that's fine as long as a later statement in the same block does.
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

// Collects (mangled_name, unmangled_param_names, body) for every top-level
// `func` — only used as input to the numeric-specialization fixpoint below.
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

// Whole-program fixpoint: which top-level functions are safe to fully
// specialize to raw int64_t? Only considers functions already known to be
// `direct_callable` (never reassigned) — specializing a function whose name
// might be rebound to something else entirely doesn't make sense anyway,
// since the boxed shell's identity is what a rebinding would replace.
// Returns (numeric_callable set, param-name map) keyed by mangled name.
fn compute_numeric_callable(
    ast: &Stmt,
    direct_callable: &std::collections::HashSet<String>,
) -> (std::collections::HashSet<String>, std::collections::HashMap<String, Vec<String>>) {
    let mut funcdefs = Vec::new();
    collect_top_level_funcdefs(ast, &mut funcdefs);

    // Structural pass: for each direct_callable function with no default/
    // rest params, check the body is int-safe and definitely returns.
    // Candidates map: mangled name -> (param names, set of mangled names it calls).
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
            called.remove(&c_name); // self-calls don't constrain the fixpoint
            candidates.insert(c_name.clone(), (pnames.clone(), called));
            param_map.insert(c_name, pnames);
        }
    }

    // Fixpoint: drop any candidate that calls something outside the
    // surviving candidate set (a function we couldn't prove safe, or one
    // that isn't even a plain top-level function call).
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

// ── Float-aware numeric specialization (additive, separate from the int-only
// path above) ────────────────────────────────────────────────────────────
//
// The int-only specializer above is left completely untouched — every
// function it already handles keeps generating byte-identical code. This
// section ONLY looks at functions the int-only pass rejected, and tries a
// broader analysis that allows float-typed parameters/locals too. If it
// can't prove a function safe, that function simply falls back to the
// ordinary boxed path, exactly as it did before this section existed.
//
// Why this needs real type inference, not just a bigger "safe op" list:
// Dragon's own dgn_div is RUNTIME-VALUE-dependent when both operands are
// int (exact division stays int, e.g. 10/2 -> int 5; 10/3 -> float 3.33),
// so "/" can only be specialized when we can PROVE at least one operand is
// float (in which case dgn_div is always float, deterministically -- see
// runtime.rs). Bitwise ops (&,|,^,<<,>>,~) read the DgnVal union's .i field
// unconditionally regardless of tag, so giving them a float operand would
// reinterpret its bits as garbage; they're only sound with Int operands.
// Getting this right requires knowing each value's type, not just "is this
// numeric".

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NumType { Int, Float }

fn num_c_type(t: NumType) -> &'static str {
    match t { NumType::Int => "int64_t", NumType::Float => "double" }
}

// ── Phase 1: parameter-type evidence ────────────────────────────────────
// A parameter's type can't be inferred from a "declaration" the way a local
// can (ConstDecl has an RHS to look at; a parameter doesn't) -- so instead
// we gather evidence for it from two places: how it's used inside the
// function body, and what literal types are passed to it at call sites
// elsewhere in the whole program. If evidence conflicts (e.g. used with a
// fractional literal AND with a bitwise op), the function is disqualified.
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

// Whole-program scan: does any call site `fn_name(...)` anywhere pass a
// fractional literal in a given argument position? That's independent
// evidence the corresponding parameter is float-typed, covering the very
// common case of a function whose body alone gives no hint (e.g.
// `func avg(a,b) { return (a+b)/2 }` called as `avg(3.5, 2.5)`).
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
        if is_float && is_int_only { return None; } // contradiction: bail, safe fallback
        out.push(if is_float { NumType::Float } else { NumType::Int });
    }
    Some(out)
}

// ── Phase 2: full type-checking walk, given resolved types for every
// candidate function's params/return (built up by the fixpoint below). ──
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

// Returns the expression's type if every subexpression type-checks under
// Dragon's real runtime promotion rules (see the module doc comment above),
// or None if anything is unprovable/unsafe. `sigs` gives the (possibly
// still-partial, mid-fixpoint) known signatures of other candidate
// functions -- a None `ret` there means "not resolved yet", which makes any
// call to it fail for this round (the fixpoint driver just retries later).
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

// Same "no implicit fall-through" requirement as the int-only path; reuses
// `block_definitely_returns`/`stmt_definitely_returns` from that section.

fn compute_numeric_callable_float(
    ast: &Stmt,
    direct_callable: &std::collections::HashSet<String>,
    already_handled: &std::collections::HashSet<String>,
) -> (std::collections::HashMap<String, FnSig>, std::collections::HashMap<String, Vec<String>>) {
    let mut funcdefs = Vec::new();
    collect_top_level_funcdefs(ast, &mut funcdefs);

    // Only consider functions the int-only pass didn't already claim.
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

    // Fixpoint over return types: round-robin retry until stable.
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
            // Try to resolve a consistent return type from every Return in
            // the body using currently-known signatures (others may still
            // be unresolved -- that's fine, we just won't make progress on
            // THIS function until they are, and we retry next round).
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

    // Final full-body verification with the now-fully-resolved signature
    // set, plus the same whole-program-fixpoint idea as the int-only path:
    // drop anything that, on full inspection, doesn't actually type-check
    // (covers cases the return-type-only pass above couldn't see, like a
    // stray operation whose result is discarded) or that calls something
    // outside the surviving set.
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

// Walks a body collecting the inferred type of every `return` reachable
// from it, using a *local copy* of `types` per branch so branches don't
// leak declarations into each other (mirrors how num_safe_stmt would see
// it during the real verification pass). Returns false if any return
// fails to type-check with current knowledge (caller treats that as "try
// again once more signatures are known", not a permanent failure).
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
        _ => true, // anything else disqualifies via num_safe_stmt's later full pass, not here
    }
}

// ── Raw emitter for float-aware specialized functions ──────────────────────
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
                    // Safety net matches the analysis: "/" only ever gets
                    // here when at least one operand is float, so plain C
                    // division already does float division (mixed int/
                    // double operands promote to double automatically);
                    // cast defensively in case both sides happen to render
                    // as int-typed subexpressions that still resolved float
                    // overall (shouldn't happen given the analysis, but the
                    // cast is free insurance against a C-level int/int trunc).
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

// Mirrors the shape of NativeTranspiler in native.rs but is deliberately
// separate: call routing here goes through _fnum_<name> for calls to other
// numeric_callable functions (proven safe by the fixpoint above), which
// NativeTranspiler has no notion of. Every expression shape this emitter
// encounters was already validated by int_safe_expr/cond_safe_expr, so
// there's no fallback path here — anything unexpected is a bug in the
// analysis above, not something this emitter tries to paper over.
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
        // Belt-and-suspenders default return: block_definitely_returns
        // already proved every real path returns, so this is unreachable,
        // but the C compiler can't see that and would otherwise warn/UB on
        // a function that (as far as it can tell) might fall off the end.
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
            _ => {} // unreachable given int_safe_stmt already validated the body
        }
    }
    // CFor's init/step slots are bare Exprs (Assign or PreOp/PostOp),
    // rendered without a trailing semicolon for use in a C `for(...)` clause.
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

    // Work out which top-level functions are safe to call directly (see
    // `direct_callable` field doc comment): defined once at module scope,
    // and never reassigned/incremented anywhere in the whole program.
    let mut top_fns = std::collections::HashSet::new();
    collect_top_level_funcs(ast, &mut top_fns);
    let mut mutated = std::collections::HashSet::new();
    collect_all_mutated_names(ast, &mut mutated);
    t.direct_callable = top_fns.difference(&mutated).cloned().collect();

    // Work out which of those are further safe to fully specialize to raw
    // int64_t (see "Integer fast-path specialization" above).
    let (numeric_callable, numeric_params) = compute_numeric_callable(ast, &t.direct_callable);
    t.numeric_callable = numeric_callable;
    t.numeric_params = numeric_params;

    // Forward-declare those functions' C signatures so direct calls type-check
    // regardless of the (arbitrary, possibly mutually-recursive) order the
    // functions are hoisted in below.
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

    // Float-aware specialization: only looks at functions the int-only
    // pass above didn't already claim (see "Float-aware numeric
    // specialization").
    let (numeric_callable_float, numeric_params_float) =
        compute_numeric_callable_float(ast, &t.direct_callable, &t.numeric_callable);
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

    // Method-call devirtualization analysis (whole-program class hierarchy).
    let mut classes = std::collections::HashMap::new();
    collect_classes(ast, &mut classes);
    t.overridable_methods = compute_overridable_methods(&classes);
    t.classes = classes;

    // Forward-declare every method of every class: a devirtualized call
    // (see resolve_devirtualized_method) can now reference a method's C
    // function from another class defined earlier in the source, which
    // otherwise wouldn't compile without a prior declaration.
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
            {
                let c_name = mangle(name);
                let fi_fn_c = format!("{}___fieldinit__", mangle(name));
                writeln!(t.out, "{}dgn_class_add_method(&{}, \"__field_init__\", _fn_{}, 1);", indent, c_name, fi_fn_c).unwrap();
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

