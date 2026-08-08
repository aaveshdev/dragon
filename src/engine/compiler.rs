use super::*;

pub struct Compiler;

impl Compiler {
    /// Helper: compile a body into its own Chunk and return (instructions, lines)
     fn compile_body(stmt: &Stmt) -> Arc<Chunk> {
        let mut c = Chunk::new();
        Self::compile_stmt(&mut c, stmt);
        c.emit(Instruction::LoadConst(DgnValue::Null), 0);
        c.emit(Instruction::Return, 0);
        Arc::new(c)
    }

    /// Like compile_stmt but for REPL use: expression statements leave their
    /// value on the stack (no trailing Pop) so the REPL can print it.
    pub fn compile_repl_stmt(chunk: &mut Chunk, stmt: &Stmt) {
        let line = stmt.line;
        match &stmt.node {
            StmtNode::Expr(e) => {
                // No Pop — leave value on stack for the REPL to print
                Self::compile_expr(chunk, e);
            }
            // Everything else compiles normally
            _ => Self::compile_stmt(chunk, stmt),
        }
    }

    pub fn compile_stmt(chunk: &mut Chunk, stmt: &Stmt) {
        let line = stmt.line;
        match &stmt.node {

            StmtNode::Block(stmts) => {
                // PASS 1: Hoist all function definitions to the top
                for s in stmts {
                    if let StmtNode::FuncDef(..) | StmtNode::ClassDef(..) = &s.node {
                        Self::compile_stmt(chunk, s);
                    }
                }
                
                // PASS 2: Compile everything else
                for s in stmts {
                    if !matches!(s.node, StmtNode::FuncDef(..) | StmtNode::ClassDef(..)) {
                        Self::compile_stmt(chunk, s);
                    }
                }
            }

            StmtNode::Multi(stmts) => {
                for s in stmts {
                    Self::compile_stmt(chunk, s);
                }
            }

            StmtNode::Expr(e) => {
                Self::compile_expr(chunk, e);
                chunk.emit(Instruction::Pop, line);
            }

            StmtNode::Return(opt_e) => {
                match opt_e {
                    Some(e) => Self::compile_expr(chunk, e),
                    None    => { chunk.emit(Instruction::LoadConst(DgnValue::Null), line); }
                };
                chunk.emit(Instruction::Return, line);
            }

            StmtNode::Break    => { chunk.emit(Instruction::Break, line); }
            StmtNode::Continue => { chunk.emit(Instruction::Continue, line); }

            StmtNode::Raise(e) => {
                Self::compile_expr(chunk, e);
                chunk.emit(Instruction::Raise, line);
            }

            StmtNode::ConstDecl(name, e) => {
                if let ExprNode::FuncExpr(params, req_params, body) = &e.node {
                   let body_chunk = Self::compile_body(body);
                    chunk.emit(Instruction::MakeClosure(
                        Some(name.clone()), params.clone(), *req_params, false,
                        body_chunk,
                    ), e.line);
                } else {
                    Self::compile_expr(chunk, e);
                }
                chunk.emit(Instruction::DefineConst(name.clone()), line);
            }

            // ── func definition ──────────────────────────────────────────────
           StmtNode::FuncDef(name, params, req_params, body, is_async) => {
                let opt_name = if name.is_empty() { None } else { Some(name.clone()) };
                let body_chunk = Self::compile_body(body);
                let instr = Instruction::MakeClosure(
                    opt_name, params.clone(), *req_params, *is_async,
                    body_chunk,
                );
                chunk.emit(instr, line);
                if !name.is_empty() {
                    chunk.emit(Instruction::StoreVar(name.clone()), line);
                }
            }

            // ── if / elif / else ──────────────────────────────────────────────
            StmtNode::IfChain(chain) => {
                let mut end_jumps = vec![];
                for (cond_opt, body) in chain {
                    if let Some(cond) = cond_opt {
                        Self::compile_expr(chunk, cond);
                        let false_jump = chunk.emit(Instruction::JumpIfFalse(0), line);
                        Self::compile_stmt(chunk, body);
                        let end_jump = chunk.emit(Instruction::Jump(0), line);
                        end_jumps.push(end_jump);
                        chunk.patch_jump(false_jump, chunk.instructions.len());
                    } else {
                        Self::compile_stmt(chunk, body);
                        let end_jump = chunk.emit(Instruction::Jump(0), line);
                        end_jumps.push(end_jump);
                    }
                }
                let end = chunk.instructions.len();
                for j in end_jumps { chunk.patch_jump(j, end); }
            }

            // ── while ─────────────────────────────────────────────────────────
            StmtNode::While(cond, body) => {
                let loop_start = chunk.instructions.len();
                Self::compile_expr(chunk, cond);
                let exit_jump = chunk.emit(Instruction::JumpIfFalse(0), line);
                // compile body; break/continue are patched below
                let body_start = chunk.instructions.len();
                Self::compile_stmt(chunk, body);
                chunk.emit(Instruction::Jump(loop_start), line);
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(exit_jump, loop_end);
                // patch break/continue inside this loop body
                Self::patch_loop_jumps(chunk, body_start, loop_end, loop_start);
            }

            // ── do-while ──────────────────────────────────────────────────────
            StmtNode::DoWhile(body, cond) => {
                let body_start = chunk.instructions.len();
                Self::compile_stmt(chunk, body);
                let cond_start = chunk.instructions.len();
                Self::compile_expr(chunk, cond);
                chunk.emit(Instruction::JumpIfFalse(0), line); // patched below
                let back_jump = chunk.instructions.len() - 1;
                chunk.emit(Instruction::Jump(body_start), line);
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(back_jump, loop_end);
                Self::patch_loop_jumps(chunk, body_start, loop_end, cond_start);
            }

            // ── for-in ────────────────────────────────────────────────────────
            StmtNode::For(var_name, value_var, iter_expr, body) => {
                Self::compile_expr(chunk, iter_expr);
                // ForIter pops the iterable, iterates: on each step pushes item
                // and sets var(s), jumps to exit when exhausted
                let for_iter_ip = chunk.emit(Instruction::ForIter(var_name.clone(), value_var.clone(), 0), line);
                let body_start = chunk.instructions.len();
                Self::compile_stmt(chunk, body);
                chunk.emit(Instruction::Jump(for_iter_ip), line); // next iteration
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(for_iter_ip, loop_end);
                Self::patch_loop_jumps(chunk, body_start, loop_end, for_iter_ip);
            }

            // ── C-style for ───────────────────────────────────────────────────
            StmtNode::CFor(init, cond, step, body) => {
                if let Some(i) = init {
                    Self::compile_expr(chunk, i);
                    chunk.emit(Instruction::Pop, line);
                }
                let loop_start = chunk.instructions.len();
                let exit_jump = if let Some(c) = cond {
                    Self::compile_expr(chunk, c);
                    Some(chunk.emit(Instruction::JumpIfFalse(0), line))
                } else { None };
                let body_start = chunk.instructions.len();
                Self::compile_stmt(chunk, body);
                let step_ip = chunk.instructions.len();
                if let Some(s) = step {
                    Self::compile_expr(chunk, s);
                    chunk.emit(Instruction::Pop, line);
                }
                chunk.emit(Instruction::Jump(loop_start), line);
                let loop_end = chunk.instructions.len();
                if let Some(j) = exit_jump { chunk.patch_jump(j, loop_end); }
                Self::patch_loop_jumps(chunk, body_start, loop_end, step_ip);
            }

            // ── switch ────────────────────────────────────────────────────────
            StmtNode::Switch(val_expr, cases, default) => {
                Self::compile_expr(chunk, val_expr);
                // Stack: [switch_val]  (stays on stack throughout)
                let mut end_jumps = vec![];
                for (case_expr, block) in cases {
                    chunk.emit(Instruction::Dup, line);
                    Self::compile_expr(chunk, case_expr);
                    chunk.emit(Instruction::BinaryOp(BinOp::Eq), line);
                    let skip = chunk.emit(Instruction::JumpIfFalse(0), line);
                    chunk.emit(Instruction::Pop, line); // pop switch_val on match
                    Self::compile_stmt(chunk, block);
                    end_jumps.push(chunk.emit(Instruction::Jump(0), line));
                    chunk.patch_jump(skip, chunk.instructions.len());
                }
                if let Some(def) = default {
                    chunk.emit(Instruction::Pop, line); // pop switch_val
                    Self::compile_stmt(chunk, def);
                } else {
                    chunk.emit(Instruction::Pop, line); // pop switch_val
                }
                let end = chunk.instructions.len();
                for j in end_jumps { chunk.patch_jump(j, end); }
            }

            // ── try / catch / finally ─────────────────────────────────────────
            StmtNode::Try(try_b, catch_var, catch_b, fin_b) => {
                let push_try_ip = chunk.emit(Instruction::PushTry(0), line);
                Self::compile_stmt(chunk, try_b);
                chunk.emit(Instruction::PopTry, line);
                let skip_catch = chunk.emit(Instruction::Jump(0), line);

                // catch block
                let catch_ip = chunk.instructions.len();
                chunk.patch_jump(push_try_ip, catch_ip);
                if let (Some(var), Some(cb)) = (catch_var, catch_b) {
                    chunk.emit(Instruction::EnterCatch(var.clone()), line);
                    Self::compile_stmt(chunk, cb);
                } else {
                    chunk.emit(Instruction::Pop, line); // discard error
                }
                chunk.patch_jump(skip_catch, chunk.instructions.len());

                // finally block runs regardless
                if let Some(fb) = fin_b {
                    Self::compile_stmt(chunk, fb);
                }
            }

            // ── class ─────────────────────────────────────────────────────────
           StmtNode::ClassDef(name, parent, body) => {
                let mut class_chunk = Chunk::new();
                Compiler::compile_stmt(&mut class_chunk, body);
                class_chunk.emit(Instruction::LoadConst(DgnValue::Null), 0);
                class_chunk.emit(Instruction::Return, 0);
                chunk.emit(Instruction::MakeClass(
                    name.clone(), parent.clone(),
                    Box::new(class_chunk.instructions), Box::new(class_chunk.source_lines),
                ), line);
                chunk.emit(Instruction::StoreVar(name.clone()), line);
            }

            // StmtNode::Augment(target_type, methods) => {
            //     for method in methods {W
            //         Self::compile_stmt(chunk, method);
            //         chunk.emit(Instruction::AddExtension(target_type.clone()), line);
            //     }
            // }

            StmtNode::Augment(target_type, methods) => {
                for method in methods {
                    if let StmtNode::FuncDef(name, params, req_params, body, is_async) = &method.node {
                        let body_chunk = Self::compile_body(body);
                        chunk.emit(Instruction::MakeClosure(
                            Some(name.clone()), params.clone(), *req_params, *is_async,
                            body_chunk,
                        ), line);
                        chunk.emit(Instruction::AddExtension(target_type.clone()), line);
                    }
                }
            }

            // ── import ────────────────────────────────────────────────────────
            StmtNode::Import(path, alias) => {
                chunk.emit(Instruction::Import(path.clone(), alias.clone()), line);
            }
        }
    }

    // ── patch break/continue targets after compiling a loop body ─────────────
    // Scans [start..end) and replaces Break/Continue placeholders.
    // break  → Jump(loop_end)
    // continue → Jump(continue_target)
    fn patch_loop_jumps(chunk: &mut Chunk, start: usize, loop_end: usize, continue_target: usize) {
        for i in start..loop_end {
            match &chunk.instructions[i] {
                Instruction::Break    => { chunk.instructions[i] = Instruction::Jump(loop_end); }
                Instruction::Continue => { chunk.instructions[i] = Instruction::Jump(continue_target); }
                _ => {}
            }
        }
    }

    pub fn compile_expr(chunk: &mut Chunk, expr: &Expr) {
        let line = expr.line;
        match &expr.node {

            ExprNode::Number(n) => { 
                // If it's a whole number and fits in an i64, compile it as an Int!
                if n.fract() == 0.0 && *n >= (i64::MIN as f64) && *n <= (i64::MAX as f64) {
                    chunk.emit(Instruction::LoadConst(DgnValue::Int(*n as i64)), line);
                } else {
                    chunk.emit(Instruction::LoadConst(DgnValue::Number(*n)), line);
                }
            }
            ExprNode::String(s) => { chunk.emit(Instruction::LoadConst(DgnValue::String(s.clone())), line); }
            ExprNode::Bool(b)   => { chunk.emit(Instruction::LoadConst(DgnValue::Bool(*b)), line); }
            ExprNode::Null      => { chunk.emit(Instruction::LoadConst(DgnValue::Null), line); }
            ExprNode::This      => { chunk.emit(Instruction::This, line); }
            ExprNode::Var(n)    => { chunk.emit(Instruction::LoadVar(n.clone()), line); }

            // ── assignment ───────────────────────────────────────────────────
            ExprNode::Assign(target, val) => {
                let mut inferred_name = None;
                if let ExprNode::Var(n) = &target.node {
                    inferred_name = Some(n.clone());
                } else if let ExprNode::GetAttr(_, n) = &target.node {
                    inferred_name = Some(n.clone()); // Handles obj.myFunc = () => {}
                }

                // If the RHS is a lambda, compile it directly with the inferred name
                 if let ExprNode::FuncExpr(params, req_params, body) = &val.node {
                    let body_chunk = Self::compile_body(body);
                    chunk.emit(Instruction::MakeClosure(
                        inferred_name, params.clone(), *req_params, false,
                        body_chunk,
                    ), val.line);
                } else {
                    Self::compile_expr(chunk, val); // Normal compilation
                }
                match &target.node {
                    ExprNode::Var(n) => {
                        chunk.emit(Instruction::Dup, line);
                        chunk.emit(Instruction::StoreVar(n.clone()), line);
                    }
                    ExprNode::GetAttr(obj, n) => {
                        // stack before SetAttr: [val, obj]  — we need obj on top
                        // Dup val, then push obj, then SetAttr
                        chunk.emit(Instruction::Dup, line);       // [val, val]
                        Self::compile_expr(chunk, obj);            // [val, val, obj]
                        chunk.emit(Instruction::SetAttr(n.clone()), line); // pops obj+val, leaves val
                    }
                    ExprNode::Index(obj, idx) => {
                        chunk.emit(Instruction::Dup, line);
                        Self::compile_expr(chunk, obj);
                        Self::compile_expr(chunk, idx);
                        chunk.emit(Instruction::SetIndex, line);
                    }
                    _ => {}
                }
            }

            // ── unary ────────────────────────────────────────────────────────
            ExprNode::Unary(op, e) => {
                Self::compile_expr(chunk, e);
                chunk.emit(Instruction::UnaryOp(op.clone()), line);
            }

            // ── binary ───────────────────────────────────────────────────────
            ExprNode::Binary(op, l, r) => {
                if op == "and" {
                    Self::compile_expr(chunk, l);
                    chunk.emit(Instruction::Dup, line);
                    let fj = chunk.emit(Instruction::JumpIfFalse(0), line);
                    chunk.emit(Instruction::Pop, line);
                    Self::compile_expr(chunk, r);
                    chunk.patch_jump(fj, chunk.instructions.len());
                } else if op == "or" {
                    Self::compile_expr(chunk, l);
                    chunk.emit(Instruction::Dup, line);
                    let tj = chunk.emit(Instruction::JumpIfTrue(0), line);
                    chunk.emit(Instruction::Pop, line);
                    Self::compile_expr(chunk, r);
                    chunk.patch_jump(tj, chunk.instructions.len());
                } else if op == "??" {
                    Self::compile_expr(chunk, l);
                    chunk.emit(Instruction::Dup, line);
                    // if not null, keep; else pop and eval rhs
                    let not_null = chunk.emit(Instruction::JumpIfNotNull(0), line);
                    chunk.emit(Instruction::Pop, line);
                    Self::compile_expr(chunk, r);
                    chunk.patch_jump(not_null, chunk.instructions.len());
                } else {
                    let binop = match op.as_str() {
                        "+"  => BinOp::Add,  "-"  => BinOp::Sub,
                        "*"  => BinOp::Mul,  "/"  => BinOp::Div,  "%" => BinOp::Mod,
                        "==" => BinOp::Eq,   "!=" => BinOp::Ne,
                        "<"  => BinOp::Lt,   ">"  => BinOp::Gt,
                        "<=" => BinOp::Le,   ">=" => BinOp::Ge,
                        "&"  => BinOp::BitAnd, "|" => BinOp::BitOr, "^" => BinOp::BitXor,
                        "<<" => BinOp::Shl,  ">>" => BinOp::Shr,
                        "??" => BinOp::NullCoalesce,
                        _    => BinOp::Add,
                    };
                    Self::compile_expr(chunk, l);
                    Self::compile_expr(chunk, r);
                    chunk.emit(Instruction::BinaryOp(binop), line);
                }
            }

            // ── ternary ──────────────────────────────────────────────────────
            ExprNode::Ternary(cond, t, f) => {
                Self::compile_expr(chunk, cond);
                let fj = chunk.emit(Instruction::JumpIfFalse(0), line);
                Self::compile_expr(chunk, t);
                let ej = chunk.emit(Instruction::Jump(0), line);
                chunk.patch_jump(fj, chunk.instructions.len());
                Self::compile_expr(chunk, f);
                chunk.patch_jump(ej, chunk.instructions.len());
            }

            // ── call ─────────────────────────────────────────────────────────
            ExprNode::Call(func_expr, arg_exprs) => {
                Self::compile_expr(chunk, func_expr);
                for a in arg_exprs { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::Call(arg_exprs.len()), line);
            }

            // ── attribute / index ─────────────────────────────────────────────
            ExprNode::GetAttr(obj, name) => {
                Self::compile_expr(chunk, obj);
                chunk.emit(Instruction::GetAttr(name.clone()), line);
            }

            ExprNode::OptionalGetAttr(obj, name) => {
                Self::compile_expr(chunk, obj);
                chunk.emit(Instruction::GetOptionalAttr(name.clone()), line);
            }

            ExprNode::Index(obj, idx) => {
                Self::compile_expr(chunk, obj);
                Self::compile_expr(chunk, idx);
                chunk.emit(Instruction::GetIndex, line);
            }

            // ── collections ──────────────────────────────────────────────────
            ExprNode::List(items) => {
                for item in items { Self::compile_expr(chunk, item); }
                chunk.emit(Instruction::MakeList(items.len()), line);
            }
            ExprNode::Obj(pairs) => {
                for (k, v) in pairs {
                    chunk.emit(Instruction::LoadConst(DgnValue::String(k.clone())), line);
                     if let ExprNode::FuncExpr(params, req_params, body) = &v.node {
                        let body_chunk = Self::compile_body(body);
                        chunk.emit(Instruction::MakeClosure(
                            Some(k.clone()), params.clone(), *req_params, false,
                            body_chunk,
                        ), v.line);
                    } else {
                        Self::compile_expr(chunk, v);
                    }
                }
                chunk.emit(Instruction::MakeMap(pairs.len()), line);
            }

            // ── lambda / func expression ──────────────────────────────────────
            ExprNode::FuncExpr(params, req_params, body) => {
                let body_chunk = Self::compile_body(body);
                chunk.emit(Instruction::MakeClosure(
                    None, params.clone(), *req_params, false,
                    body_chunk,
                ), line);
            }

            // ── new ───────────────────────────────────────────────────────────
            ExprNode::New(cls_expr, args) => {
                Self::compile_expr(chunk, cls_expr);
                for a in args { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::NewObject(args.len()), line);
            }

            // ── super.method(...) ─────────────────────────────────────────────
            ExprNode::Super(method, args) => {
                for a in args { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::SuperCall(method.clone(), args.len()), line);
            }

            // ── await ─────────────────────────────────────────────────────────
            ExprNode::Await(inner) => {
                Self::compile_expr(chunk, inner);
                chunk.emit(Instruction::Await, line);
            }

            // ── pre/post inc-dec ──────────────────────────────────────────────
            ExprNode::PreOp(op, target) => {
                let is_dec = op == "DEC";
                match &target.node {
                    ExprNode::Var(n) => {
                        chunk.emit(if is_dec { Instruction::DecVar(n.clone()) } else { Instruction::IncVar(n.clone()) }, line);
                    }
                    ExprNode::GetAttr(obj, attr) => {
                        // Emit obj expression onto stack, then RMWAttr pops it
                        Self::compile_expr(chunk, obj);
                        chunk.emit(Instruction::ReadModWriteAttr(attr.clone(), true, is_dec), line);
                    }
                    ExprNode::Index(obj, idx) => {
                        // Emit obj then idx onto stack, then RMWIndex pops both
                        Self::compile_expr(chunk, obj);
                        Self::compile_expr(chunk, idx);
                        chunk.emit(Instruction::ReadModWriteIndex(true, is_dec), line);
                    }
                    _ => {}
                }
            }
            ExprNode::PostOp(op, target) => {
                let is_dec = op == "DEC";
                match &target.node {
                    ExprNode::Var(n) => {
                        chunk.emit(if is_dec { Instruction::PostDecVar(n.clone()) } else { Instruction::PostIncVar(n.clone()) }, line);
                    }
                    ExprNode::GetAttr(obj, attr) => {
                        Self::compile_expr(chunk, obj);
                        chunk.emit(Instruction::ReadModWriteAttr(attr.clone(), false, is_dec), line);
                    }
                    ExprNode::Index(obj, idx) => {
                        Self::compile_expr(chunk, obj);
                        Self::compile_expr(chunk, idx);
                        chunk.emit(Instruction::ReadModWriteIndex(false, is_dec), line);
                    }
                    _ => {}
                }
            }

            _ => { chunk.emit(Instruction::Nop, line); }
        }
    }
}

