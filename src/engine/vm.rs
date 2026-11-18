use super::*;

pub struct Vm {
    pub stack: Vec<DgnValue>,
    try_frames: Vec<(usize, usize)>,
    for_iter_stack: Vec<(usize, Vec<(DgnValue, DgnValue)>, usize)>,
    pub extensions: HashMap<String, HashMap<String, DgnValue>>,
    pending_snapshot: Option<Arc<Mutex<Env>>>,
}

thread_local! {
    static VM_POOL: RefCell<Vec<Vm>> = RefCell::new(Vec::with_capacity(64));
}

pub fn acquire_vm() -> Vm {
    VM_POOL.with(|p| p.borrow_mut().pop()).unwrap_or_else(|| Vm {
        stack: Vec::with_capacity(32),
        try_frames: Vec::with_capacity(8),
        for_iter_stack: Vec::with_capacity(8),
        extensions: HashMap::new(),
        pending_snapshot: None,
    })
}

pub fn release_vm(mut vm: Vm) {
    vm.stack.clear();
    vm.try_frames.clear();
    vm.for_iter_stack.clear();
    vm.pending_snapshot = None;
    VM_POOL.with(|p| p.borrow_mut().push(vm));
}

impl Vm {

        pub fn new() -> Self {
        Vm {
            stack: Vec::with_capacity(64),
            try_frames: Vec::with_capacity(8),
            for_iter_stack: Vec::with_capacity(8),
            extensions: HashMap::new(),
            pending_snapshot: None,
        }
    }

    fn pop(&mut self) -> DgnValue { self.stack.pop().unwrap_or(DgnValue::Null) }

    #[async_recursion]
    pub async fn run(
        &mut self,
        chunk: &Chunk,
        env: Arc<Mutex<Env>>,
    ) -> Result<DgnValue, DgnError> {
        let mut ip = 0;

        while ip < chunk.instructions.len() {
            let line = chunk.source_lines[ip];
            match &chunk.instructions[ip] {
                Instruction::Halt => break,
                Instruction::Nop  => {}

                Instruction::LoadConst(v) => self.stack.push(v.clone()),
                Instruction::LoadVar(ref n) => {
                    let v = env.lock().unwrap().get(n)
                        .ok_or_else(|| DgnError::new(&format!("Var '{}' not defined", n), line))?;
                    self.stack.push(v);
                }

                Instruction::StoreVar(ref n) => {
                    let v = self.pop();
                    let mut env_guard = env.lock().unwrap();
                    if env_guard.store.contains_key("__class_name__") {
                        env_guard.store.insert(n.clone(), v);
                    } else if env_guard.store.contains_key(n.as_str()) {
                        if !env_guard.constants.contains(n.as_str()) {
                            env_guard.store.insert(n.clone(), v);
                        } else {
                            return Err(DgnError::new(&format!("Cannot reassign constant '{}'", n), line));
                        }
                    } else {
                        drop(env_guard);
                        env.lock().unwrap().set(n.clone(), v)
                        .map_err(|e| DgnError::new(&e, line))?;
                    }
                }

                Instruction::DefineConst(ref n) => {
                    let v = self.pop();
                    env.lock().unwrap().define_const(n.clone(), v)
                        .map_err(|e| DgnError::new(&e, line))?;
                }
                Instruction::Pop => { self.pop(); }
                Instruction::Dup => {
                    let v = self.stack.last().cloned().unwrap_or(DgnValue::Null);
                    self.stack.push(v);
                }
                Instruction::This => {
                    let v = env.lock().unwrap().get("this")
                        .ok_or(DgnError::new("'this' not defined", line))?;
                    self.stack.push(v);
                }

                Instruction::Jump(t)           => { ip = *t; continue; }
                Instruction::JumpIfFalse(t)    => { let v = self.pop(); if !is_truthy(&v) { ip = *t; continue; } }
                Instruction::JumpIfTrue(t)     => { let v = self.pop(); if  is_truthy(&v) { ip = *t; continue; } }
                Instruction::JumpIfNotNull(t)  => { let v = self.pop(); if !matches!(v, DgnValue::Null) { ip = *t; continue; } }

                Instruction::UnaryOp(ref op) => {
                    let v = self.pop();
                    let r = match (op.as_str(), v) {
                        ("-",   DgnValue::Int(n))    => DgnValue::Int(-n),
                        ("+",   DgnValue::Int(n))    => DgnValue::Int(n),
                        ("~",   DgnValue::Int(n))    => DgnValue::Int(!n),

                        ("-",   DgnValue::Number(n)) => DgnValue::Number(-n),
                        ("+",   DgnValue::Number(n)) => DgnValue::Number(n),
                        ("not", x)                   => DgnValue::Bool(!is_truthy(&x)),
                        ("~",   DgnValue::Number(n)) => DgnValue::Number(!(n as i64) as f64),
                        _ => return Err(DgnError::new("Invalid unary op", line)),
                    };
                    self.stack.push(r);
                }
                Instruction::BinaryOp(ref op) => {
                    let r = self.pop();
                    let l = self.pop();
                    self.stack.push(vm_binary_op(op, l, r, line)?);
                }

                Instruction::MakeList(n) => {
                    let start = self.stack.len().saturating_sub(*n);
                    let items: Vec<DgnValue> = self.stack.drain(start..).collect();
                    let val = DgnValue::List(Arc::new(Mutex::new(items)));
                    gc_register(&val);
                    self.stack.push(val);
                }
                Instruction::MakeMap(n) => {
                    let start = self.stack.len().saturating_sub(n * 2);
                    let pairs: Vec<DgnValue> = self.stack.drain(start..).collect();
                    let mut map = HashMap::new();
                    let mut i = 0;
                    while i + 1 < pairs.len() {
                        if let DgnValue::String(k) = &pairs[i] {
                            map.insert(k.clone(), pairs[i + 1].clone());
                        }
                        i += 2;
                    }
                    let val = DgnValue::Map(Arc::new(Mutex::new(map)));
                    gc_register(&val);
                    self.stack.push(val);
                }

                    Instruction::MakeClosureRef(..) | Instruction::MakeClassRef(..) => {
                            self.stack.push(DgnValue::Null);
                        }

                Instruction::GetAttr(ref name) | Instruction::GetOptionalAttr(ref name) => {
                    let obj = self.pop();

                    if matches!(chunk.instructions[ip], Instruction::GetOptionalAttr(_)) && matches!(obj, DgnValue::Null) {
                        self.stack.push(DgnValue::Null);
                        ip += 1;
                        continue;
                    }

                    let mut val = get_attr(obj.clone(), name).map_err(|mut e| { e.line = line; e })?;

                    if let DgnValue::Null = val {
                        let type_name = match obj {
                            DgnValue::String(_) => "string",
                            DgnValue::Int(_) => "int",
                            DgnValue::Number(_) => "number",
                            DgnValue::List(_) => "list",
                            DgnValue::Map(_) => "map",
                            DgnValue::ByteArray(_) => "bytes",
                            _ => "unknown",
                        };

                        if let Some(exts) = self.extensions.get(type_name) {
                            if let Some(DgnValue::Func(f)) = exts.get(name) {
                                let f_clone = f.clone();
                                let obj_capture = obj.clone();
                                val = DgnValue::Func(Arc::new(DgnFunc {
                                    name: f.name.clone(), params: f.params.clone(), req_params: f.req_params, body: None,
                                    closure: f.closure.clone(),
                                    is_native: true,
                                    is_async: f.is_async,
                                    compiled_chunk: None,
                                    native_fn: Some(Arc::new(move |args| {
                                        let rt = tokio::runtime::Handle::current();
                                        tokio::task::block_in_place(|| {
                                            rt.block_on(call_dgn_func_impl(f_clone.clone(), args, Some(obj_capture.clone())))
                                        })
                                    }))
                                }));
                            }
                        }
                    }
                    self.stack.push(val);
                }

                Instruction::AddExtension(ref target_type) => {
                    let func_val = self.pop();
                    if let DgnValue::Func(f) = &func_val {
                        let name = f.name.clone().unwrap_or_default();
                        self.extensions
                            .entry(target_type.clone())
                            .or_insert_with(HashMap::new)
                            .insert(name, func_val);
                    }
                }
                Instruction::SetAttr(ref name) => {
                    let obj = self.pop();
                    let val = self.stack.last().cloned().unwrap_or(DgnValue::Null);
                    match obj {
                        DgnValue::Map(m) | DgnValue::Instance(m) => {
                            m.lock().unwrap().insert(name.clone(), val);
                        }
                        DgnValue::Class(c) => {
                            c.lock().unwrap().store.insert(name.clone(), val);
                        }
                        _ => {}
                    }
                }
                Instruction::GetIndex => {
                    let idx = self.pop();
                    let obj = self.pop();
                    let r = match obj {
                        DgnValue::List(l) => {
                            if let Some(i) = idx.as_number() {
                                l.lock().unwrap().get(i as usize).cloned().unwrap_or(DgnValue::Null)
                            } else { DgnValue::Null }
                        },
                        DgnValue::ByteArray(b) => {
                            if let Some(i) = idx.as_number() {
                                b.lock().unwrap().get(i as usize).map(|x| DgnValue::Number(*x as f64)).unwrap_or(DgnValue::Null)
                            } else { DgnValue::Null }
                        },
                        DgnValue::Map(m) | DgnValue::Instance(m) => {
                            match idx {
                                DgnValue::String(k) => m.lock().unwrap().get(&k).cloned().unwrap_or(DgnValue::Null),
                                DgnValue::Int(_) | DgnValue::Number(_) => {
                                    m.lock().unwrap().get(&val_to_string(&idx)).cloned().unwrap_or(DgnValue::Null)
                                }
                                _ => DgnValue::Null,
                            }
                        },
                        DgnValue::String(s) => {
                            if let Some(i) = idx.as_number() {
                                let i = i as usize;
                                let result = if s.is_ascii() {
                                    s.as_bytes().get(i).map(|&b| DgnValue::String((b as char).to_string()))
                                } else {
                                    s.chars().nth(i).map(|c| DgnValue::String(c.to_string()))
                                };
                                result.unwrap_or(DgnValue::Null)
                            } else { DgnValue::Null }
                        },
                        _ => DgnValue::Null,
                    };
                    self.stack.push(r);
                }
                Instruction::SetIndex => {
                    let idx = self.pop();
                    let obj = self.pop();
                    let val = self.stack.last().cloned().unwrap_or(DgnValue::Null);
                    match obj {
                        DgnValue::List(l) => {
                            if let Some(n) = idx.as_number() {
                                let mut v = l.lock().unwrap();
                                let i = n as usize;
                                if i < v.len() { v[i] = val; }
                            }
                        }
                        DgnValue::Map(m) | DgnValue::Instance(m) => {
                            match idx {
                                DgnValue::String(k) => { m.lock().unwrap().insert(k, val); }
                                DgnValue::Int(_) | DgnValue::Number(_) => {
                                    m.lock().unwrap().insert(val_to_string(&idx), val);
                                }
                                _ => {}
                            }
                        }
                        DgnValue::ByteArray(b) => {
                            if let Some(n) = idx.as_number() {
                                let mut v = b.lock().unwrap();
                                let i = n as usize;
                                if i < v.len() {
                                    if let Some(bv) = val.as_number() { v[i] = bv as u8; }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                Instruction::Call(argc) => {
                    let start = self.stack.len().saturating_sub(*argc);
                    let args: Vec<DgnValue> = self.stack.drain(start..).collect();
                    let func = self.pop();

                    if let DgnValue::Class(cls_env) = func {
                        match instantiate_class(cls_env, args).await {
                            Ok(inst) => { self.stack.push(inst); ip += 1; continue; }
                            Err(mut e) => {
                                if e.line == 0 { e.line = line; }
                                if let Some((catch_ip, depth)) = self.try_frames.pop() {
                                    self.stack.truncate(depth);
                                    self.stack.push(DgnValue::String(e.msg));
                                    ip = catch_ip;
                                    continue;
                                }
                                return Err(e);
                            }
                        }
                    }

                    if let DgnValue::Func(ref f) = func {
                        if f.is_async && !f.is_native {
                            let f_clone = f.clone();
                            let h = tokio::spawn(async move {
                                call_dgn_func_impl(f_clone, args, None).await
                            });
                            let ha = Arc::new(AsyncMutex::new(Some(h)));
                            self.stack.push(DgnValue::Promise(Arc::new(AsyncMutex::new(
                                PromiseState::Pending(ha)
                            ))));
                            ip += 1;
                            continue;
                        }
                    }

                    if let DgnValue::NativeMethod(ref f) = func {
                        match f(args) {
                            Ok(r) => { self.stack.push(r); ip += 1; continue; }
                            Err(mut e) => {
                                if e.line == 0 { e.line = line; }
                                if let Some((catch_ip, depth)) = self.try_frames.pop() {
                                    self.stack.truncate(depth);
                                    self.stack.push(DgnValue::String(e.msg));
                                    ip = catch_ip;
                                    continue;
                                }
                                return Err(e);
                            }
                        }
                    }

                    match call_dgn_func(func, args).await {
                        Ok(r) => { self.stack.push(r); }
                        Err(mut e) => {
                            if e.line == 0 { e.line = line; }
                            if let Some((catch_ip, depth)) = self.try_frames.pop() {
                                self.stack.truncate(depth);
                                self.stack.push(DgnValue::String(e.msg));
                                ip = catch_ip;
                                continue;
                            }
                            return Err(e);
                        }
                    }
                }

                Instruction::Return => { return Ok(self.pop()); }

                Instruction::SnapshotNext(ref names) => {
                    let snap = Arc::new(Mutex::new(Env::new(Some(env.clone()), None)));
                    {
                        let cur = env.lock().unwrap();
                        let mut sg = snap.lock().unwrap();
                        for n in names {
                            if let Some(v) = cur.get(n) { sg.store.insert(n.clone(), v); }
                        }
                    }
                    self.pending_snapshot = Some(snap);
                }

                Instruction::MakeClosure(ref opt_name, ref params, req_params, is_async, ref chunk_arc) => {
                    let chunk_arc = chunk_arc.clone();
                    let captured_env = self.pending_snapshot.take().unwrap_or_else(|| env.clone());
                    let func = DgnFunc {
                        name: opt_name.clone(),
                        params: params.clone(),
                        req_params: *req_params,
                        body: None,
                        closure: captured_env,
                        is_native: false,
                        is_async: *is_async,
                        native_fn: None,
                        compiled_chunk: Some(chunk_arc),
                    };
                    self.stack.push(DgnValue::Func(Arc::new(func)));
                }

                Instruction::MakeClass(ref name, ref parent_name, ref body_instrs, ref body_lines) => {
                    let mut parent_env: Option<Arc<Mutex<Env>>> = None;
                    if let Some(ref pname) = parent_name {
                        match env.lock().unwrap().get(pname) {
                            Some(DgnValue::Class(penv)) => parent_env = Some(penv),
                            _ => return Err(DgnError::new("Parent class not found", line)),
                        }
                    }

                    let class_env = Arc::new(Mutex::new(Env::new(Some(env.clone()), None)));
                    class_env.lock().unwrap().store.insert(
                        "__class_name__".into(), DgnValue::String(name.clone())
                    );
                    if let Some(pe) = parent_env {
                        class_env.lock().unwrap().store.insert(
                            "__parent_class__".into(), DgnValue::Class(pe)
                        );
                    }

                    let body_chunk = Chunk {
                        instructions: *body_instrs.clone(),
                        source_lines:  *body_lines.clone(),
                    };
                    let mut class_vm = Vm::new();
                    class_vm.run(&body_chunk, class_env.clone()).await?;

                    let class_val = DgnValue::Class(class_env);
                    self.stack.push(class_val);
                }

                Instruction::NewObject(argc) => {
                    let start = self.stack.len().saturating_sub(*argc);
                    let args: Vec<DgnValue> = self.stack.drain(start..).collect();
                    let cls_val = self.pop();

                    if let DgnValue::Class(cls_env) = cls_val {
                        let inst_map = Arc::new(Mutex::new(HashMap::new()));
                        inst_map.lock().unwrap().insert(
                            "__class__".into(), DgnValue::Class(cls_env)
                        );
                        let instance = DgnValue::Instance(inst_map);
                        gc_register(&instance);

                        if let Ok(init) = get_attr(instance.clone(), "init") {
                            if let DgnValue::Func(_) = init {
                                match call_dgn_func(init, args).await {
                                    Ok(_) => {}
                                    Err(mut e) => {
                                        e.line = line;
                                        if let Some((catch_ip, depth)) = self.try_frames.pop() {
                                            self.stack.truncate(depth);
                                            self.stack.push(DgnValue::String(e.msg));
                                            ip = catch_ip;
                                            continue;
                                        }
                                        return Err(e);
                                    }
                                }
                            }
                        }
                        self.stack.push(instance);
                    } else {
                        return Err(DgnError::new("Operand is not a class", line));
                    }
                }

                Instruction::SuperCall(ref method_name, argc) => {
                    let start = self.stack.len().saturating_sub(*argc);
                    let args: Vec<DgnValue> = self.stack.drain(start..).collect();

                    let this_val = env.lock().unwrap().get("this")
                        .ok_or(DgnError::new("super used outside class method", line))?;

                    if !matches!(this_val, DgnValue::Instance(_)) {
                        return Err(DgnError::new("super requires instance context", line));
                    }

                    let parent_cls = env.lock().unwrap().get("__parent_class__")
                        .ok_or(DgnError::new("No parent class for super", line))?;

                    let result = if let DgnValue::Class(penv) = parent_cls {
                        let method = penv.lock().unwrap().get(method_name)
                            .ok_or(DgnError::new(&format!("Method '{}' not in parent", method_name), line))?;

                        if let DgnValue::Func(f) = method {
                            match call_dgn_func_impl(f, args, Some(this_val)).await {
                                Ok(v) => v,
                                Err(mut e) => {
                                    if e.line == 0 { e.line = line; }
                                    if let Some((catch_ip, depth)) = self.try_frames.pop() {
                                        self.stack.truncate(depth);
                                        self.stack.push(DgnValue::String(e.msg));
                                        ip = catch_ip;
                                        continue;
                                    }
                                    return Err(e);
                                }
                            }
                        } else {
                            DgnValue::Null
                        }
                    } else {
                        return Err(DgnError::new("Parent is not a class", line));
                    };

                    self.stack.push(result);
                }

                Instruction::ForIter(ref var_name, ref value_var, exit_ip) => {
                    let is_active = self.for_iter_stack.last().map(|(k,_,_)| *k == ip).unwrap_or(false);
                    if !is_active {
                        let iterable = self.pop();
                        let items: Vec<(DgnValue, DgnValue)> = match iterable {
                            DgnValue::List(l) => l.lock().unwrap().iter().enumerate()
                                .map(|(i, v)| (DgnValue::Int(i as i64), v.clone())).collect(),
                            DgnValue::String(s) => s.chars().enumerate()
                                .map(|(i, c)| (DgnValue::Int(i as i64), DgnValue::String(c.to_string()))).collect(),
                            DgnValue::Int(n) => (0..n)
                                .map(|i| (DgnValue::Int(i), DgnValue::Int(i))).collect(),
                            DgnValue::Number(n) => (0..n as i64)
                                .map(|i| (DgnValue::Int(i), DgnValue::Number(i as f64))).collect(),
                            DgnValue::ByteArray(b) => b.lock().unwrap().iter().enumerate()
                                .map(|(i, b)| (DgnValue::Int(i as i64), DgnValue::Number(*b as f64))).collect(),
                            DgnValue::Map(m) | DgnValue::Instance(m) => m.lock().unwrap().iter()
                                .map(|(k, v)| (DgnValue::String(k.clone()), v.clone())).collect(),
                            _ => return Err(DgnError::new("Not iterable", line)),
                        };
                        self.for_iter_stack.push((ip, items, 0));
                    }

                    let (_, items, idx) = self.for_iter_stack.last_mut().unwrap();
                    if *idx >= items.len() {
                        self.for_iter_stack.pop();
                        ip = *exit_ip;
                        continue;
                    }
                    let (k, v) = items[*idx].clone();
                    *idx += 1;
                    let mut env_guard = env.lock().unwrap();
                    if let Some(vname) = value_var {
                        env_guard.store.insert(var_name.clone(), k);
                        env_guard.store.insert(vname.clone(), v);
                    } else {
                        env_guard.store.insert(var_name.clone(), v);
                    }
                }

                Instruction::Break | Instruction::Continue | Instruction::DoWhileStart => {}

                Instruction::PushTry(catch_ip) => {
                    self.try_frames.push((*catch_ip, self.stack.len()));
                }
                Instruction::PopTry => { self.try_frames.pop(); }
                Instruction::Raise => {
                    let msg = val_to_string(&self.pop());
                    if let Some((catch_ip, depth)) = self.try_frames.pop() {
                        self.stack.truncate(depth);
                        self.stack.push(DgnValue::String(msg));
                        ip = catch_ip;
                        continue;
                    }
                    return Err(DgnError::new(&msg, line));
                }
                Instruction::EnterCatch(ref var_name) => {
                    let err = self.pop();
                    env.lock().unwrap().set(var_name.clone(), err)
                        .map_err(|e| DgnError::new(&e, line))?;
                }

                Instruction::Await => {
                    let val = self.pop();
                    if let DgnValue::Promise(ref sa) = val {
                        let mut state = sa.lock().await;
                        let extracted = match &mut *state {
                            PromiseState::Resolved(r) => {
                                let r = r.clone();
                                drop(state);
                                self.stack.push(r?);
                                ip += 1;
                                continue;
                            }
                            PromiseState::Pending(ha) => ha.lock().await.take(),
                        };
                        if let Some(h) = extracted {
                            let res = h.await.map_err(|_| DgnError::new("Promise panicked", line))?;
                            *state = PromiseState::Resolved(res.clone());
                            self.stack.push(res?);
                        } else {
                            return Err(DgnError::new("Promise already consumed", line));
                        }
                    } else {
                        self.stack.push(val);
                    }
                }

                Instruction::Import(ref path, ref alias) => {
                    vm_do_import(path, alias, env.clone(), line).await?;
                }

                Instruction::IncVar(ref n) => {
                    let mut env_guard = env.lock().unwrap();
                    let cur = env_guard.get(n).unwrap_or(DgnValue::Int(0));
                    match cur {
                    DgnValue::Int(v) => {
                        let new_val = DgnValue::Int(v + 1);
                        env_guard.store.insert(n.clone(), new_val.clone());
                        drop(env_guard);
                        self.stack.push(new_val);
                    }
                    DgnValue::Number(v) => {
                        let new_val = DgnValue::Number(v + 1.0);
                        env_guard.store.insert(n.clone(), new_val.clone());
                        drop(env_guard);
                        self.stack.push(new_val);
                    }
                    _ => return Err(DgnError::new("Cannot ++ non-number", line)),
                    }
                }

                Instruction::DecVar(ref n) => {
                    let mut env_guard = env.lock().unwrap();
                    let cur = env_guard.get(n).unwrap_or(DgnValue::Int(0));
                    match cur {
                    DgnValue::Int(v) => {
                        let new_val = DgnValue::Int(v - 1);
                        env_guard.store.insert(n.clone(), new_val.clone());
                        drop(env_guard);
                        self.stack.push(new_val);
                    }
                    DgnValue::Number(v) => {
                        let new_val = DgnValue::Number(v - 1.0);
                        env_guard.store.insert(n.clone(), new_val.clone());
                        drop(env_guard);
                        self.stack.push(new_val);
                    }
                    _ => return Err(DgnError::new("Cannot -- non-number", line)),
                    }
                }

                Instruction::PostIncVar(ref n) => {
                    let mut env_guard = env.lock().unwrap();
                    let cur = env_guard.get(n).unwrap_or(DgnValue::Int(0));
                    match cur {
                        DgnValue::Int(v) => {
                            env_guard.store.insert(n.clone(), DgnValue::Int(v + 1));
                            drop(env_guard);
                            self.stack.push(DgnValue::Int(v));
                        }
                        DgnValue::Number(v) => {
                            env_guard.store.insert(n.clone(), DgnValue::Number(v + 1.0));
                            drop(env_guard);
                            self.stack.push(DgnValue::Number(v));
                        }
                        _ => return Err(DgnError::new("Cannot ++ non-number", line)),
                    }
                }

                Instruction::PostDecVar(ref n) => {
                    let mut env_guard = env.lock().unwrap();
                    let cur = env_guard.get(n).unwrap_or(DgnValue::Int(0));
                    match cur {
                    DgnValue::Int(v) => {
                        env_guard.store.insert(n.clone(), DgnValue::Int(v - 1));
                        drop(env_guard);
                        self.stack.push(DgnValue::Int(v));
                    }
                    DgnValue::Number(v) => {
                        env_guard.store.insert(n.clone(), DgnValue::Number(v - 1.0));
                        drop(env_guard);
                        self.stack.push(DgnValue::Number(v));
                    }
                    _ => return Err(DgnError::new("Cannot -- non-number", line)),
                    }
                }

                Instruction::ReadModWriteAttr(ref attr, is_pre, is_dec) => {
                    let obj = self.pop();
                    let cur = get_attr(obj.clone(), attr).unwrap_or(DgnValue::Null);
                    match cur {
                        DgnValue::Int(n) => {
                            let nv = if *is_dec { n - 1 } else { n + 1 };
                            if let DgnValue::Map(m) | DgnValue::Instance(m) = &obj {
                                m.lock().unwrap().insert(attr.clone(), DgnValue::Int(nv));
                            }
                            self.stack.push(DgnValue::Int(if *is_pre { nv } else { n }));
                        }
                        DgnValue::Number(n) => {
                            let nv = if *is_dec { n - 1.0 } else { n + 1.0 };
                            if let DgnValue::Map(m) | DgnValue::Instance(m) = &obj {
                                m.lock().unwrap().insert(attr.clone(), DgnValue::Number(nv));
                            }
                            self.stack.push(DgnValue::Number(if *is_pre { nv } else { n }));
                        }
                        _ => return Err(DgnError::new("Cannot ++/-- non-number attr", line)),
                    }
                }
                Instruction::ReadModWriteIndex(is_pre, is_dec) => {
                    let idx = self.pop();
                    let obj = self.pop();

                    let cur = match &obj {
                        DgnValue::List(l) => {
                            if let Some(i) = idx.as_number() {
                                l.lock().unwrap().get(i as usize).cloned().unwrap_or(DgnValue::Null)
                            } else { DgnValue::Null }
                        },
                        DgnValue::Map(m) | DgnValue::Instance(m) => {
                            if let DgnValue::String(k) = &idx {
                                m.lock().unwrap().get(k).cloned().unwrap_or(DgnValue::Null)
                            } else { DgnValue::Null }
                        },
                        _ => DgnValue::Null,
                    };

                    if let Some(n) = cur.as_number() {
                        let is_int = matches!(cur, DgnValue::Int(_));
                        let nv = if *is_dec { n - 1.0 } else { n + 1.0 };
                        let nv_val = if is_int { DgnValue::Int(nv as i64) } else { DgnValue::Number(nv) };
                        match &obj {
                            DgnValue::List(l) => {
                                if let Some(i) = idx.as_number() {
                                    let mut v = l.lock().unwrap();
                                    let i = i as usize;
                                    if i < v.len() { v[i] = nv_val.clone(); }
                                }
                            }
                            DgnValue::Map(m) | DgnValue::Instance(m) => {
                                if let DgnValue::String(k) = &idx {
                                    m.lock().unwrap().insert(k.clone(), nv_val.clone());
                                }
                            }
                            _ => {}
                        }
                        let result = if *is_pre { nv_val } else { if is_int { DgnValue::Int(n as i64) } else { DgnValue::Number(n) } };
                        self.stack.push(result);
                    } else { return Err(DgnError::new("Cannot ++/-- non-number index", line)); }
                }
            }
            ip += 1;
        }

        Ok(self.stack.pop().unwrap_or(DgnValue::Null))
    }
}

pub async fn vm_do_import(
    path: &str,
    alias: &Option<String>,
    env: Arc<Mutex<Env>>,
    line: usize,
) -> Result<(), DgnError> {
    let default_name = |p: &str| -> String {
        let q = p.strip_prefix("package:").unwrap_or(p);
        std::path::Path::new(q).file_stem().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| q.to_string())
    };
    let bind = match alias.as_deref() {
        Some("*") | None => default_name(path),
        Some(a) => a.to_string(),
    };

    if let Some(module_val) = get_builtin_module(path) {
        env.lock().unwrap().set(bind, module_val).map_err(|e| DgnError::new(&e, line))?;
        return Ok(());
    }

    let root = Env::get_root(env.clone());

    let bundled_src = {
        let r = root.lock().unwrap();
        if let Some(DgnValue::Map(map)) = r.store.get("__bundled_sources__") {
            let m = map.lock().unwrap();
            let key2 = format!("{}.dgn", path);
            if let Some(DgnValue::String(s)) = m.get(path).or_else(|| m.get(&key2)) {
                Some(s.clone())
            } else { None }
        } else { None }
    };

    let (code, new_env_path) = if let Some(src) = bundled_src {
        (src, PathBuf::from(format!("bundle:///{}", path)))
    } else {
        let current_dir = env.lock().unwrap().current_path
            .parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut search_dirs = vec![current_dir.clone(), current_dir.join("dgn_modules")];
        if let Ok(dp) = std::env::var("DRAGON_PATH") {
            for p in std::env::split_paths(&dp) { search_dirs.push(p); }
        }

        let path_obj = Path::new(path);
        let mut target_files = Vec::new();
        if path_obj.extension().is_some() {
            target_files.push(path_obj.to_path_buf());
        } else {
            target_files.push(PathBuf::from(format!("{}.dgn", path)));
            if let Some(name) = path_obj.file_name() {
                let n = name.to_string_lossy();
                target_files.push(PathBuf::from(path).join(format!("{}.dgn", n)));
                target_files.push(PathBuf::from(path).join("index.dgn"));
            }
        }

        let mut found_path = None;
        if path.starts_with("package:") {
            let app_dir = {
                let r = root.lock().unwrap();
                match r.store.get("__app_dir__") {
                    Some(DgnValue::String(d)) => PathBuf::from(d),
                    _ => crate::packages::app_root_of(&r.current_path),
                }
            };
            let roots = crate::packages::package_roots(&app_dir);
            let f = crate::packages::resolve_package_import(path, &roots).map_err(|e| DgnError::new(&e, line))?;
            found_path = Some(f.canonicalize().unwrap_or(f));
        }
        'outer: for dir in search_dirs.iter().filter(|_| found_path.is_none()) {
            for target in &target_files {
                let candidate = dir.join(target);
                if candidate.exists() && candidate.is_file() {
                    found_path = Some(candidate.canonicalize().unwrap_or(candidate));
                    break 'outer;
                }
            }
        }

        let canon = found_path
            .ok_or_else(|| DgnError::new(&format!("Import not found: '{}'", path), line))?;
        let c = fs::read_to_string(&canon)
            .map_err(|e| DgnError::new(&e.to_string(), line))?;
        (c, canon)
    };

    let cache_key = new_env_path.to_string_lossy().to_string();

    let cached_mod = {
        let r = root.lock().unwrap();
        if let Some(DgnValue::Map(modules)) = r.store.get("__modules__") {
            modules.lock().unwrap().get(&cache_key).cloned()
        } else { None }
    };

    let mod_val = if let Some(m) = cached_mod {
        if let DgnValue::Class(mod_env) = &m {
            if mod_env.lock().unwrap().store.contains_key("__loading__") {
                return Err(DgnError::new(
                    &format!("Cyclic import detected: '{}' is already loading.", path), line
                ));
            }
        }
        m
    } else {
        let tokens = lex(&code)?;
        let mut parser = Parser::new(tokens);
        let module_body = parser.parse().map_err(|e| DgnError::new(&e, line))?;

        let mod_env = Arc::new(Mutex::new(Env::new(None, Some(new_env_path))));
        populate_builtins(mod_env.clone());
        mod_env.lock().unwrap().store.insert("__loading__".to_string(), DgnValue::Bool(true));
        {
            let r = root.lock().unwrap();
            let mut me = mod_env.lock().unwrap();
            for k in ["__modules__", "__bundled_sources__", "__app_dir__"] {
                if let Some(v) = r.store.get(k) { me.store.insert(k.to_string(), v.clone()); }
            }
        }

        let placeholder = DgnValue::Class(mod_env.clone());
        {
            let r = root.lock().unwrap();
            if let Some(DgnValue::Map(modules)) = r.store.get("__modules__") {
                modules.lock().unwrap().insert(cache_key, placeholder.clone());
            }
        }

        let mut mod_chunk = Chunk::new();
        Compiler::compile_stmt(&mut mod_chunk, &module_body);
        mod_chunk.emit(Instruction::Halt, 0);

        let mut mod_vm = Vm::new();
        mod_vm.run(&mod_chunk, mod_env.clone()).await?;

        mod_env.lock().unwrap().store.remove("__loading__");
        placeholder
    };

    if alias.as_deref() == Some("*") {
        if let DgnValue::Class(mod_env) = &mod_val {
            let skip = builtin_name_set();
            let exports: Vec<(String, DgnValue)> = mod_env.lock().unwrap().store.iter()
                .filter(|(k, _)| !k.starts_with('_') && !skip.contains(k.as_str()))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let mut target = env.lock().unwrap();
            for (k, v) in exports {
                target.store.insert(k, v);
            }
        }
        return Ok(());
    }
    let final_bind = alias.clone().unwrap_or_else(|| default_name(path));
    env.lock().unwrap().set(final_bind, mod_val).map_err(|e| DgnError::new(&e, line))?;
    Ok(())
}

pub fn vm_binary_op(op: &BinOp, l: DgnValue, r: DgnValue, line: usize) -> Result<DgnValue, DgnError> {
    fn to_f(v: &DgnValue) -> Option<f64> { match v { DgnValue::Number(n) => Some(*n), DgnValue::Int(n) => Some(*n as f64), _ => None } }
    fn both_int(l: &DgnValue, r: &DgnValue) -> Option<(i64, i64)> {
        match (l, r) { (DgnValue::Int(a), DgnValue::Int(b)) => Some((*a, *b)), _ => None }
    }
    match op {
        BinOp::Add => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Int(a.wrapping_add(b))),
            (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Number(a + b)),
            (DgnValue::Int(a), DgnValue::Number(b)) => Ok(DgnValue::Number(a as f64 + b)),
            (DgnValue::Number(a), DgnValue::Int(b)) => Ok(DgnValue::Number(a + b as f64)),
            (DgnValue::String(mut a), b) => { a.push_str(&val_to_string(&b)); Ok(DgnValue::String(a)) },
            (a, DgnValue::String(b)) => Ok(DgnValue::String(val_to_string(&a) + &b)),
            (DgnValue::List(a), DgnValue::List(b)) => {
                let mut combined = a.lock().unwrap().clone();
                combined.extend(b.lock().unwrap().iter().cloned());
                Ok(DgnValue::List(Arc::new(Mutex::new(combined))))
            },
            _ => Ok(DgnValue::Null),
        },
        BinOp::Sub => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Int(a.wrapping_sub(b))),
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number(a-b)) } else { Ok(DgnValue::Null) }
        },
        BinOp::Mul => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Int(a.wrapping_mul(b))),
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number(a*b)) } else { Ok(DgnValue::Null) }
        },
        BinOp::Div => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => {
                if b == 0 { return Err(DgnError::new("Division by zero", line)); }
                if a % b == 0 { Ok(DgnValue::Int(a / b)) } else { Ok(DgnValue::Number(a as f64 / b as f64)) }
            },
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number(a/b)) } else { Ok(DgnValue::Null) }
        },
        BinOp::Mod => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => {
                if b == 0 { return Err(DgnError::new("Modulo by zero", line)); }
                Ok(DgnValue::Int(a % b))
            },
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number(a%b)) } else { Ok(DgnValue::Null) }
        },
        BinOp::Eq  => match (l,r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Bool(a==b)),
            (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Bool((a-b).abs() < f64::EPSILON)),
            (DgnValue::Int(a), DgnValue::Number(b)) => Ok(DgnValue::Bool((a as f64 - b).abs() < f64::EPSILON)),
            (DgnValue::Number(a), DgnValue::Int(b)) => Ok(DgnValue::Bool((a - b as f64).abs() < f64::EPSILON)),
            (DgnValue::String(a), DgnValue::String(b)) => Ok(DgnValue::Bool(a==b)),
            (DgnValue::Bool(a),   DgnValue::Bool(b))   => Ok(DgnValue::Bool(a==b)),
            (DgnValue::Null,      DgnValue::Null)       => Ok(DgnValue::Bool(true)),
            (DgnValue::ByteArray(a), DgnValue::ByteArray(b)) => Ok(DgnValue::Bool(*a.lock().unwrap() == *b.lock().unwrap())),
            (DgnValue::Map(a), DgnValue::Map(b)) => Ok(DgnValue::Bool(std::sync::Arc::ptr_eq(&a, &b))),
            (DgnValue::List(a), DgnValue::List(b)) => Ok(DgnValue::Bool(std::sync::Arc::ptr_eq(&a, &b))),
            _ => Ok(DgnValue::Bool(false)),
        },
        BinOp::Ne  => match (l,r) {
            (DgnValue::Int(a), DgnValue::Int(b))       => Ok(DgnValue::Bool(a != b)),
            (DgnValue::Number(a), DgnValue::Number(b)) => Ok(DgnValue::Bool((a - b).abs() >= f64::EPSILON)),
            (DgnValue::Int(a), DgnValue::Number(b))    => Ok(DgnValue::Bool((a as f64 - b).abs() >= f64::EPSILON)),
            (DgnValue::Number(a), DgnValue::Int(b))    => Ok(DgnValue::Bool((a - b as f64).abs() >= f64::EPSILON)),
            (DgnValue::String(a), DgnValue::String(b)) => Ok(DgnValue::Bool(a != b)),
            (DgnValue::Bool(a),   DgnValue::Bool(b))   => Ok(DgnValue::Bool(a != b)),
            (DgnValue::Null,      DgnValue::Null)       => Ok(DgnValue::Bool(false)),
            (DgnValue::ByteArray(a), DgnValue::ByteArray(b)) => Ok(DgnValue::Bool(*a.lock().unwrap() != *b.lock().unwrap())),
            (DgnValue::Map(a), DgnValue::Map(b)) => Ok(DgnValue::Bool(!std::sync::Arc::ptr_eq(&a, &b))),
            (DgnValue::List(a), DgnValue::List(b)) => Ok(DgnValue::Bool(!std::sync::Arc::ptr_eq(&a, &b))),
            _ => Ok(DgnValue::Bool(true)),
        },
        BinOp::Lt  => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Bool(a<b)) } else { Ok(DgnValue::Bool(false)) },
        BinOp::Gt  => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Bool(a>b)) } else { Ok(DgnValue::Bool(false)) },
        BinOp::Le  => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Bool(a<=b)) } else { Ok(DgnValue::Bool(false)) },
        BinOp::Ge  => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Bool(a>=b)) } else { Ok(DgnValue::Bool(false)) },
        BinOp::BitAnd => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Int(a & b)),
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number((a as i64 & b as i64) as f64)) } else { Ok(DgnValue::Null) }
        },
        BinOp::BitOr  => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Int(a | b)),
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number((a as i64 | b as i64) as f64)) } else { Ok(DgnValue::Null) }
        },
        BinOp::BitXor => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Int(a ^ b)),
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number((a as i64 ^ b as i64) as f64)) } else { Ok(DgnValue::Null) }
        },
        BinOp::Shl   => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Int(a << (b & 63))),
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number(((a as i64) << (b as i64)) as f64)) } else { Ok(DgnValue::Null) }
        },
        BinOp::Shr   => match (l, r) {
            (DgnValue::Int(a), DgnValue::Int(b)) => Ok(DgnValue::Int(a >> (b & 63))),
            (l, r) => if let (Some(a), Some(b)) = (to_f(&l), to_f(&r)) { Ok(DgnValue::Number(((a as i64) >> (b as i64)) as f64)) } else { Ok(DgnValue::Null) }
        },
        BinOp::NullCoalesce => if let DgnValue::Null = l { Ok(r) } else { Ok(l) },
        BinOp::And | BinOp::Or => Ok(DgnValue::Null),
    }
}

pub async fn instantiate_class(cls_env: Arc<Mutex<Env>>, args: Vec<DgnValue>) -> Result<DgnValue, DgnError> {
    let inst_map = Arc::new(Mutex::new(HashMap::new()));
    inst_map.lock().unwrap().insert("__class__".into(), DgnValue::Class(cls_env));
    let instance = DgnValue::Instance(inst_map);
    gc_register(&instance);
    if let Ok(init) = get_attr(instance.clone(), "init") {
        if let DgnValue::Func(_) = init {
            call_dgn_func(init, args).await?;
        }
    }
    Ok(instance)
}

pub fn builtin_name_set() -> &'static HashSet<String> {
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        let e = Arc::new(Mutex::new(Env::new(None, None)));
        populate_builtins(e.clone());
        let keys: HashSet<String> = e.lock().unwrap().store.keys().cloned().collect();
        keys
    })
}
