use super::*;

pub struct Compiler;

thread_local! {
    static LOOP_VARS: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());

    static TRY_STACK: std::cell::RefCell<Vec<(Option<Stmt>, usize, bool)>> = std::cell::RefCell::new(Vec::new());
    static LOOP_DEPTH: std::cell::Cell<usize> = std::cell::Cell::new(0);
}

impl Compiler {
    fn emit_closure(chunk: &mut Chunk, instr: Instruction, line: usize) -> usize {
        let vars: Vec<String> = LOOP_VARS.with(|v| v.borrow().clone());
        if !vars.is_empty() {
            chunk.emit(Instruction::SnapshotNext(vars), line);
        }
        chunk.emit(instr, line)
    }

     fn compile_body(stmt: &Stmt) -> Arc<Chunk> {
        let mut c = Chunk::new();
        let saved = TRY_STACK.with(|t| std::mem::take(&mut *t.borrow_mut()));
        let sd = LOOP_DEPTH.with(|d| d.replace(0));
        Self::compile_stmt(&mut c, stmt);
        TRY_STACK.with(|t| *t.borrow_mut() = saved);
        LOOP_DEPTH.with(|d| d.set(sd));
        c.emit(Instruction::LoadConst(DgnValue::Null), 0);
        c.emit(Instruction::Return, 0);
        Arc::new(c)
    }

    fn unwind_trys(chunk: &mut Chunk, line: usize, only_depth: Option<usize>) {
        let all = TRY_STACK.with(|t| t.borrow().clone());
        let mut i = all.len();
        while i > 0 {
            i -= 1;
            let (fin, depth, active) = &all[i];
            if let Some(d) = only_depth { if *depth != d { break; } }
            if *active { chunk.emit(Instruction::PopTry, line); }
            if let Some(f) = fin {
                let saved = TRY_STACK.with(|t| { let mut b = t.borrow_mut(); let sv = b.clone(); b.truncate(i); sv });
                Self::compile_stmt(chunk, f);
                TRY_STACK.with(|t| *t.borrow_mut() = saved);
            }
        }
    }

    fn loop_enter() { LOOP_DEPTH.with(|d| d.set(d.get() + 1)); }
    fn loop_leave() { LOOP_DEPTH.with(|d| d.set(d.get() - 1)); }

    pub fn compile_repl_stmt(chunk: &mut Chunk, stmt: &Stmt) {
        let line = stmt.line;
        match &stmt.node {
            StmtNode::Expr(e) => {
                Self::compile_expr(chunk, e);
            }
            _ => Self::compile_stmt(chunk, stmt),
        }
    }

    pub fn compile_stmt(chunk: &mut Chunk, stmt: &Stmt) {
        let line = stmt.line;
        match &stmt.node {

            StmtNode::Block(stmts) => {
                for s in stmts {
                    if let StmtNode::Import(..) = &s.node {
                        Self::compile_stmt(chunk, s);
                    }
                }

                for s in stmts {
                    if let StmtNode::FuncDef(..) | StmtNode::ClassDef(..) = &s.node {
                        Self::compile_stmt(chunk, s);
                    }
                }

                for s in stmts {
                    if !matches!(s.node, StmtNode::FuncDef(..) | StmtNode::ClassDef(..) | StmtNode::Import(..)) {
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
                Self::unwind_trys(chunk, line, None);
                chunk.emit(Instruction::Return, line);
            }

            StmtNode::Break    => {
                let d = LOOP_DEPTH.with(|d| d.get());
                Self::unwind_trys(chunk, line, Some(d));
                chunk.emit(Instruction::Break, line);
            }
            StmtNode::Continue => {
                let d = LOOP_DEPTH.with(|d| d.get());
                Self::unwind_trys(chunk, line, Some(d));
                chunk.emit(Instruction::Continue, line);
            }

            StmtNode::Raise(e) => {
                Self::compile_expr(chunk, e);
                chunk.emit(Instruction::Raise, line);
            }

            StmtNode::ConstDecl(name, e) => {
                if let ExprNode::FuncExpr(params, req_params, body) = &e.node {
                   let body_chunk = Self::compile_body(body);
                    Self::emit_closure(chunk, Instruction::MakeClosure(
                        Some(name.clone()), params.clone(), *req_params, false,
                        body_chunk,
                    ), e.line);
                } else {
                    Self::compile_expr(chunk, e);
                }
                chunk.emit(Instruction::DefineConst(name.clone()), line);
            }

           StmtNode::FuncDef(name, params, req_params, body, is_async) => {
                let opt_name = if name.is_empty() { None } else { Some(name.clone()) };
                let body_chunk = Self::compile_body(body);
                let instr = Instruction::MakeClosure(
                    opt_name, params.clone(), *req_params, *is_async,
                    body_chunk,
                );
                Self::emit_closure(chunk, instr, line);
                if !name.is_empty() {
                    chunk.emit(Instruction::StoreVar(name.clone()), line);
                }
            }

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

            StmtNode::While(cond, body) => {
                let loop_start = chunk.instructions.len();
                Self::compile_expr(chunk, cond);
                let exit_jump = chunk.emit(Instruction::JumpIfFalse(0), line);
                let body_start = chunk.instructions.len();
                Self::loop_enter();
                Self::compile_stmt(chunk, body);
                Self::loop_leave();
                chunk.emit(Instruction::Jump(loop_start), line);
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(exit_jump, loop_end);
                Self::patch_loop_jumps(chunk, body_start, loop_end, loop_start);
            }

            StmtNode::DoWhile(body, cond) => {
                let body_start = chunk.instructions.len();
                Self::loop_enter();
                Self::compile_stmt(chunk, body);
                Self::loop_leave();
                let cond_start = chunk.instructions.len();
                Self::compile_expr(chunk, cond);
                chunk.emit(Instruction::JumpIfFalse(0), line);
                let back_jump = chunk.instructions.len() - 1;
                chunk.emit(Instruction::Jump(body_start), line);
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(back_jump, loop_end);
                Self::patch_loop_jumps(chunk, body_start, loop_end, cond_start);
            }

            StmtNode::For(var_name, value_var, iter_expr, body) => {
                Self::compile_expr(chunk, iter_expr);
                let for_iter_ip = chunk.emit(Instruction::ForIter(var_name.clone(), value_var.clone(), 0), line);
                let body_start = chunk.instructions.len();
                let pushed = 1 + if value_var.is_some() { 1 } else { 0 };
                LOOP_VARS.with(|v| {
                    v.borrow_mut().push(var_name.clone());
                    if let Some(vv) = value_var { v.borrow_mut().push(vv.clone()); }
                });
                Self::loop_enter();
                Self::compile_stmt(chunk, body);
                Self::loop_leave();
                LOOP_VARS.with(|v| { for _ in 0..pushed { v.borrow_mut().pop(); } });
                chunk.emit(Instruction::Jump(for_iter_ip), line);
                let loop_end = chunk.instructions.len();
                chunk.patch_jump(for_iter_ip, loop_end);
                Self::patch_loop_jumps(chunk, body_start, loop_end, for_iter_ip);
            }

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
                Self::loop_enter();
                Self::compile_stmt(chunk, body);
                Self::loop_leave();
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

            StmtNode::Switch(val_expr, cases, default) => {
                Self::compile_expr(chunk, val_expr);
                let mut end_jumps = vec![];
                for (case_expr, block) in cases {
                    chunk.emit(Instruction::Dup, line);
                    Self::compile_expr(chunk, case_expr);
                    chunk.emit(Instruction::BinaryOp(BinOp::Eq), line);
                    let skip = chunk.emit(Instruction::JumpIfFalse(0), line);
                    chunk.emit(Instruction::Pop, line);
                    Self::compile_stmt(chunk, block);
                    end_jumps.push(chunk.emit(Instruction::Jump(0), line));
                    chunk.patch_jump(skip, chunk.instructions.len());
                }
                if let Some(def) = default {
                    chunk.emit(Instruction::Pop, line);
                    Self::compile_stmt(chunk, def);
                } else {
                    chunk.emit(Instruction::Pop, line);
                }
                let end = chunk.instructions.len();
                for j in end_jumps { chunk.patch_jump(j, end); }
            }

            StmtNode::Try(try_b, catch_var, catch_b, fin_b) => {
                let depth = LOOP_DEPTH.with(|d| d.get());
                let push_try_ip = chunk.emit(Instruction::PushTry(0), line);
                TRY_STACK.with(|t| t.borrow_mut().push((fin_b.as_ref().map(|b| (**b).clone()), depth, true)));
                Self::compile_stmt(chunk, try_b);
                TRY_STACK.with(|t| { t.borrow_mut().pop(); });
                chunk.emit(Instruction::PopTry, line);
                let skip_catch = chunk.emit(Instruction::Jump(0), line);

                let catch_ip = chunk.instructions.len();
                chunk.patch_jump(push_try_ip, catch_ip);
                let mut reraise = false;
                if let (Some(var), Some(cb)) = (catch_var, catch_b) {
                    chunk.emit(Instruction::EnterCatch(var.clone()), line);
                    if let Some(fb) = fin_b {

                        let h = chunk.emit(Instruction::PushTry(0), line);
                        TRY_STACK.with(|t| t.borrow_mut().push((Some((**fb).clone()), depth, true)));
                        Self::compile_stmt(chunk, cb);
                        TRY_STACK.with(|t| { t.borrow_mut().pop(); });
                        chunk.emit(Instruction::PopTry, line);
                        let sk = chunk.emit(Instruction::Jump(0), line);
                        chunk.patch_jump(h, chunk.instructions.len());
                        chunk.emit(Instruction::StoreVar("__pending_error__".to_string()), line);
                        Self::compile_stmt(chunk, fb);
                        chunk.emit(Instruction::LoadVar("__pending_error__".to_string()), line);
                        chunk.emit(Instruction::Raise, line);
                        chunk.patch_jump(sk, chunk.instructions.len());
                    } else {
                        Self::compile_stmt(chunk, cb);
                    }
                } else if fin_b.is_some() {

                    chunk.emit(Instruction::StoreVar("__pending_error__".to_string()), line);
                    Self::compile_stmt(chunk, fin_b.as_ref().unwrap());
                    chunk.emit(Instruction::LoadVar("__pending_error__".to_string()), line);
                    chunk.emit(Instruction::Raise, line);
                    reraise = true;
                } else {
                    chunk.emit(Instruction::Pop, line);
                }
                chunk.patch_jump(skip_catch, chunk.instructions.len());

                if let Some(fb) = fin_b {
                    let _ = reraise;
                    Self::compile_stmt(chunk, fb);
                }
            }

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

            StmtNode::Augment(target_type, methods) => {
                for method in methods {
                    if let StmtNode::FuncDef(name, params, req_params, body, is_async) = &method.node {
                        let body_chunk = Self::compile_body(body);
                        Self::emit_closure(chunk, Instruction::MakeClosure(
                            Some(name.clone()), params.clone(), *req_params, *is_async,
                            body_chunk,
                        ), line);
                        chunk.emit(Instruction::AddExtension(target_type.clone()), line);
                    }
                }
            }

            StmtNode::Import(path, alias) => {
                chunk.emit(Instruction::Import(path.clone(), alias.clone()), line);
            }
        }
    }

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

            ExprNode::Assign(target, val) => {
                let mut inferred_name = None;
                if let ExprNode::Var(n) = &target.node {
                    inferred_name = Some(n.clone());
                } else if let ExprNode::GetAttr(_, n) = &target.node {
                    inferred_name = Some(n.clone());
                }

                 if let ExprNode::FuncExpr(params, req_params, body) = &val.node {
                    let body_chunk = Self::compile_body(body);
                    Self::emit_closure(chunk, Instruction::MakeClosure(
                        inferred_name, params.clone(), *req_params, false,
                        body_chunk,
                    ), val.line);
                } else {
                    Self::compile_expr(chunk, val);
                }
                match &target.node {
                    ExprNode::Var(n) => {
                        chunk.emit(Instruction::Dup, line);
                        chunk.emit(Instruction::StoreVar(n.clone()), line);
                    }
                    ExprNode::GetAttr(obj, n) => {
                        chunk.emit(Instruction::Dup, line);
                        Self::compile_expr(chunk, obj);
                        chunk.emit(Instruction::SetAttr(n.clone()), line);
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

            ExprNode::Unary(op, e) => {
                Self::compile_expr(chunk, e);
                chunk.emit(Instruction::UnaryOp(op.clone()), line);
            }

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

            ExprNode::Ternary(cond, t, f) => {
                Self::compile_expr(chunk, cond);
                let fj = chunk.emit(Instruction::JumpIfFalse(0), line);
                Self::compile_expr(chunk, t);
                let ej = chunk.emit(Instruction::Jump(0), line);
                chunk.patch_jump(fj, chunk.instructions.len());
                Self::compile_expr(chunk, f);
                chunk.patch_jump(ej, chunk.instructions.len());
            }

            ExprNode::Call(func_expr, arg_exprs) => {
                Self::compile_expr(chunk, func_expr);
                for a in arg_exprs { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::Call(arg_exprs.len()), line);
            }

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

            ExprNode::List(items) => {
                for item in items { Self::compile_expr(chunk, item); }
                chunk.emit(Instruction::MakeList(items.len()), line);
            }
            ExprNode::Obj(pairs) => {
                for (k, v) in pairs {
                    chunk.emit(Instruction::LoadConst(DgnValue::String(k.clone())), line);
                     if let ExprNode::FuncExpr(params, req_params, body) = &v.node {
                        let body_chunk = Self::compile_body(body);
                        Self::emit_closure(chunk, Instruction::MakeClosure(
                            Some(k.clone()), params.clone(), *req_params, false,
                            body_chunk,
                        ), v.line);
                    } else {
                        Self::compile_expr(chunk, v);
                    }
                }
                chunk.emit(Instruction::MakeMap(pairs.len()), line);
            }

            ExprNode::FuncExpr(params, req_params, body) => {
                let body_chunk = Self::compile_body(body);
                Self::emit_closure(chunk, Instruction::MakeClosure(
                    None, params.clone(), *req_params, false,
                    body_chunk,
                ), line);
            }

            ExprNode::New(cls_expr, args) => {
                Self::compile_expr(chunk, cls_expr);
                for a in args { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::NewObject(args.len()), line);
            }

            ExprNode::Super(method, args) => {
                for a in args { Self::compile_expr(chunk, a); }
                chunk.emit(Instruction::SuperCall(method.clone(), args.len()), line);
            }

            ExprNode::Await(inner) => {
                Self::compile_expr(chunk, inner);
                chunk.emit(Instruction::Await, line);
            }

            ExprNode::PreOp(op, target) => {
                let is_dec = op == "DEC";
                match &target.node {
                    ExprNode::Var(n) => {
                        chunk.emit(if is_dec { Instruction::DecVar(n.clone()) } else { Instruction::IncVar(n.clone()) }, line);
                    }
                    ExprNode::GetAttr(obj, attr) => {
                        Self::compile_expr(chunk, obj);
                        chunk.emit(Instruction::ReadModWriteAttr(attr.clone(), true, is_dec), line);
                    }
                    ExprNode::Index(obj, idx) => {
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
