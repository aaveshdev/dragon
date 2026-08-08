use super::*;

#[derive(Clone, Debug)]
pub enum PromiseState {
    Pending(Arc<AsyncMutex<Option<JoinHandle<Result<DgnValue, DgnError>>>>>),
    Resolved(Result<DgnValue, DgnError>),
}

#[derive(Clone)]
pub enum DgnValue {
    Null,
    Bool(bool),
    Int(i64),
    Number(f64),
    String(String),
    List(Arc<Mutex<Vec<DgnValue>>>),
    Map(Arc<Mutex<HashMap<String, DgnValue>>>),
    ByteArray(Arc<Mutex<Vec<u8>>>),
    Func(Arc<DgnFunc>),
    Class(Arc<Mutex<Env>>),
    Instance(Arc<Mutex<HashMap<String, DgnValue>>>),
    #[cfg(not(any(target_arch = "wasm32")))]
    NativeLib(Arc<Mutex<Library>>),
    Regex(Arc<Regex>),
    Thread(Arc<thread::JoinHandle<()>>), 
   #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
    Socket(Arc<Mutex<TcpStream>>),
    Promise(Arc<AsyncMutex<PromiseState>>),
    #[allow(dead_code)]
    NativeMethod(Arc<dyn Fn(Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync>),
}
impl std::fmt::Debug for DgnValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DgnValue::NativeMethod(_) => write!(f, "<NativeMethod>"),
            other => write!(f, "{}", val_to_string(other)),
        }
    }
}


impl DgnValue {
    pub fn as_number(&self) -> Option<f64> {
        match self {
            DgnValue::Number(n) => Some(*n),
            DgnValue::Int(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            DgnValue::Int(n) => Some(*n),
            DgnValue::Number(n) => Some(*n as i64),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct DgnFunc {
    pub name: Option<String>,
    pub params: Vec<(String, Option<String>)>,
    pub req_params: usize,
    pub body: Option<Stmt>,
    pub closure: Arc<Mutex<Env>>,
    pub is_native: bool,
    pub is_async: bool,
    pub native_fn: Option<Arc<dyn Fn(Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync>>,
    /// Compiled bytecode for this function. When Some, the VM executes this
    /// instead of tree-walking `body`.
    pub compiled_chunk: Option<Arc<Chunk>>,
}

impl fmt::Debug for DgnFunc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DgnFunc")
         .field("name", &self.name)
         .field("params", &self.params)
         .field("is_native", &self.is_native)
         .finish()
    }
}

pub fn is_truthy(v: &DgnValue) -> bool {
    match v {
        DgnValue::Null => false,
        DgnValue::Bool(b) => *b,
        DgnValue::Int(n) => *n != 0,
        DgnValue::Number(n) => *n != 0.0,
        DgnValue::String(s) => !s.is_empty(),
        DgnValue::List(l) => !l.lock().unwrap().is_empty(),
        DgnValue::Map(m) => !m.lock().unwrap().is_empty(),
        DgnValue::ByteArray(b) => !b.lock().unwrap().is_empty(),
        _ => true,
    }
}

pub fn val_to_string(v: &DgnValue) -> String {
    match v {
        DgnValue::Null => "null".to_string(),
        DgnValue::Bool(b) => if *b { "True".to_string() } else { "False".to_string() },
        DgnValue::Int(n) => {
            let mut buf = itoa::Buffer::new();
            buf.format(*n).to_owned()
        }
        DgnValue::Number(n) => if n.fract() == 0.0 { format!("{}", *n as i64) } else { format!("{}", n) },
        DgnValue::String(s) => s.clone(),
        DgnValue::List(l) => {
            let l = l.lock().unwrap();
            let parts: Vec<String> = l.iter().map(val_to_string).collect();
            format!("[{}]", parts.join(", "))
        },
        DgnValue::Map(m) => {
            let m = m.lock().unwrap();
            let mut parts: Vec<String> = m.iter().map(|(k,v)| format!("{}: {}", k, val_to_string(v))).collect();
            parts.sort(); 
            format!("{{ {} }}", parts.join(", "))
        },
        DgnValue::Func(_) => "<function>".to_string(),
        DgnValue::Class(_) => "<class>".to_string(),
        DgnValue::Instance(_) => "<instance>".to_string(),
        DgnValue::ByteArray(b) => format!("<ByteArray len={}>", b.lock().unwrap().len()),
       #[cfg(not(any(target_arch = "wasm32")))]
        DgnValue::NativeLib(_) => "<NativeLib>".to_string(),
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        DgnValue::Socket(_) => "<Socket>".to_string(),
        DgnValue::Regex(r) => format!("<Regex {}>", r.as_str()),
        DgnValue::Thread(_) => "<Thread>".to_string(),
        DgnValue::Promise(_) => "<Promise>".to_string(),
        DgnValue::NativeMethod(_) => "<native>".to_string(),
    }
}



pub fn get_attr(obj: DgnValue, name: &str) -> Result<DgnValue, DgnError> {
    match obj {

        DgnValue::Class(env) => {
            let e = env.lock().unwrap();
            if let Some(val) = e.get(name) {
                return Ok(val);
            }
            Ok(DgnValue::Null)
        },

        DgnValue::Promise(state_arc) => {
            match name {
                "then" => {
                    let state_arc_clone = state_arc.clone();
                    Ok(native_fn(move |args| {
                        let callback = args.get(0).cloned().ok_or(DgnError::new("Expected callback for .then", 0))?;
                        let state = state_arc_clone.clone();
                        
                        // Return a NEW Promise representing the chain
                        let handle = tokio::spawn(async move {
                            let mut s = state.lock().await;
                            
                            let extracted_handle = match &mut *s {
                                PromiseState::Resolved(r) => return r.clone(),
                                PromiseState::Pending(h_arc) => {
                                    let mut h_lock = h_arc.lock().await;
                                    h_lock.take()
                                }
                            };
                            
                            let res = if let Some(h) = extracted_handle {
                                let r = h.await.map_err(|_| DgnError::new("Promise panicked", 0))?;
                                *s = PromiseState::Resolved(r.clone());
                                r
                            } else {
                                Err(DgnError::new("Promise already consumed", 0))
                            };
                            
                            match res {
                                Ok(val) => call_dgn_func(callback, vec![val]).await,
                                Err(e) => Err(e),
                            }
                        });
                        
                        Ok(DgnValue::Promise(Arc::new(AsyncMutex::new(PromiseState::Pending(Arc::new(AsyncMutex::new(Some(handle))))))))
                    }))
                },
                "catch" => {
                    let state_arc_clone = state_arc.clone();
                    Ok(native_fn(move |args| {
                        let callback = args.get(0).cloned().ok_or(DgnError::new("Expected callback for .catch", 0))?;
                        let state = state_arc_clone.clone();
                        
                        let handle = tokio::spawn(async move {
                            let mut s = state.lock().await;
                            
                            let extracted_handle = match &mut *s {
                                PromiseState::Resolved(r) => return r.clone(),
                                PromiseState::Pending(h_arc) => {
                                    let mut h_lock = h_arc.lock().await;
                                    h_lock.take()
                                }
                            };
                            
                            let res = if let Some(h) = extracted_handle {
                                let r = h.await.map_err(|_| DgnError::new("Promise panicked", 0))?;
                                *s = PromiseState::Resolved(r.clone());
                                r
                            } else {
                                Err(DgnError::new("Promise already consumed", 0))
                            };
                            
                            match res {
                                Ok(val) => Ok(val), 
                                Err(e) => call_dgn_func(callback, vec![DgnValue::String(e.msg)]).await,
                            }
                        });
                        
                        Ok(DgnValue::Promise(Arc::new(AsyncMutex::new(PromiseState::Pending(Arc::new(AsyncMutex::new(Some(handle))))))))
                    }))
                },
                _ => Ok(DgnValue::Null)
            }
        },

        DgnValue::Map(m) => {
            if let Some(val) = m.lock().unwrap().get(name) {
                return Ok(val.clone());
            }
            match name {
                "keys" => return Ok(native_method(m, |m, _| {
                    let keys: Vec<DgnValue> = m.lock().unwrap().keys().map(|k| DgnValue::String(k.clone())).collect();
                    Ok(DgnValue::List(Arc::new(Mutex::new(keys))))
                })),
                "values" => return Ok(native_method(m, |m, _| {
                    let vals: Vec<DgnValue> = m.lock().unwrap().values().cloned().collect();
                    Ok(DgnValue::List(Arc::new(Mutex::new(vals))))
                })),
                "contains" => return Ok(native_method(m, |m, args| {
                    let key = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(m.lock().unwrap().contains_key(&key)))
                })),
                "set" => return Ok(native_method(m, |m, args| {
                    if args.len() == 2 {
                        if let DgnValue::String(k) = &args[0] {
                            m.lock().unwrap().insert(k.clone(), args[1].clone());
                        }
                    }
                    Ok(DgnValue::Map(m))
                })),
                "get" => return Ok(native_method(m, |m, args| {
                    let key = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(m.lock().unwrap().get(&key).cloned().unwrap_or(DgnValue::Null))
                })),
                "remove" => return Ok(native_method(m, |m, args| {
                    if let Some(DgnValue::String(k)) = args.get(0) {
                        m.lock().unwrap().remove(k);
                    }
                    Ok(DgnValue::Map(m))
                })),
                "clear" => return Ok(native_method(m, |m, _| {
                    m.lock().unwrap().clear();
                    Ok(DgnValue::Map(m))
                })),
                "length" => return Ok(DgnValue::Number(m.lock().unwrap().len() as f64)),
                _ => {}
            }
            // Not a built-in — look up as a stored data key
            Ok(m.lock().unwrap().get(name).cloned().unwrap_or(DgnValue::Null))
        },
        DgnValue::Instance(inst) => {

            if let Some(v) = inst.lock().unwrap().get(name) {
                return Ok(v.clone());
            }
            

            let mut cls_ref = inst.lock().unwrap().get("__class__").cloned();
            while let Some(DgnValue::Class(cls_env)) = cls_ref {
                 if let Some(mem) = cls_env.lock().unwrap().get(name) {
                     
   
                    if let DgnValue::Func(f) = mem { 
                         let inst_capture = inst.clone();
                         let f_clone = f.clone();
                     
                         return Ok(DgnValue::Func(Arc::new(DgnFunc {
                             name: f.name.clone(), params: f.params.clone(), req_params: f.req_params, body: None,
                             closure: f.closure.clone(),
                             is_native: true,
                             is_async: f.is_async,
                             compiled_chunk: None,
                             native_fn: Some(Arc::new(move |args| {
                                 let rt = tokio::runtime::Handle::current();
                                    tokio::task::block_in_place(|| {
                                        rt.block_on(call_dgn_func_impl(f_clone.clone(), args, Some(DgnValue::Instance(inst_capture.clone()))))
                                    })
                             }))
                         })));
                     }
                     
                     return Ok(mem);
                 }
                 cls_ref = cls_env.lock().unwrap().get("__parent_class__");
            }
            Ok(DgnValue::Null)
        },
        DgnValue::List(l) => {
            match name {
                "push" => Ok(native_method(l, |l, args| { 
                    l.lock().unwrap().push(args.get(0).unwrap_or(&DgnValue::Null).clone()); Ok(DgnValue::List(l))
                })),
                "pop" => Ok(native_method(l, |l, _| { 
                    Ok(l.lock().unwrap().pop().unwrap_or(DgnValue::Null)) 
                })),
               "insert" => Ok(native_method(l, |l, args| {
                    let idx = if let Some(n) = args.get(0).and_then(|a| a.as_number()) { n as usize } else { 0 };
                    let val = args.get(1).cloned().unwrap_or(DgnValue::Null);
                    let mut vec = l.lock().unwrap();
                    if idx <= vec.len() { vec.insert(idx, val); }
                    Ok(DgnValue::Null)
                })),
                "remove_at" => Ok(native_method(l, |l, args| {
                    let idx = if let Some(n) = args.get(0).and_then(|a| a.as_number()) { n as usize } else { 0 };
                    let mut vec = l.lock().unwrap();
                    if idx < vec.len() { Ok(vec.remove(idx)) } else { Ok(DgnValue::Null) }
                })),
                "length" => Ok(DgnValue::Number(l.lock().unwrap().len() as f64)),
                "join" => Ok(native_method(l, |l, args| {
                    let sep = if let Some(DgnValue::String(s)) = args.get(0) { s.clone() } else { "".to_string() };
                    let strs: Vec<String> = l.lock().unwrap().iter().map(val_to_string).collect();
                    Ok(DgnValue::String(strs.join(&sep)))
                })),
                "map" => Ok(native_method(l, |l, args| {
                    let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                    let old_vec = l.lock().unwrap().clone();
                    let mut new_vec = Vec::new();
                    
                    // Grab the current runtime to block on the async function safely
                    let rt = tokio::runtime::Handle::current(); 
                    
                    for item in old_vec {
                        let val = tokio::task::block_in_place(||rt.block_on(call_dgn_func(func.clone(), vec![item])))?;
                        new_vec.push(val);
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(new_vec))))
                })),
                "filter" => Ok(native_method(l, |l, args| {
                        let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                        let old_vec = l.lock().unwrap().clone();
                        let mut new_vec = Vec::new();
                        
                        let rt = tokio::runtime::Handle::current(); // Grab runtime
                        
                        for item in old_vec {
                            // Block on the async call
                            let res = tokio::task::block_in_place(|| rt.block_on(call_dgn_func(func.clone(), vec![item.clone()])))?;
                            if is_truthy(&res) {
                                new_vec.push(item);
                            }
                        }
                        Ok(DgnValue::List(Arc::new(Mutex::new(new_vec))))
                    })),
                "sort" => Ok(native_method(l, |l, _| {
                    let mut vec = l.lock().unwrap();
                    vec.sort_by(|a, b| {
                        match (a, b) {
                            (DgnValue::Number(n1), DgnValue::Number(n2)) => n1.partial_cmp(n2).unwrap_or(std::cmp::Ordering::Equal),
                            (DgnValue::String(s1), DgnValue::String(s2)) => s1.cmp(s2),
                            _ => std::cmp::Ordering::Equal
                        }
                    });
                    Ok(DgnValue::Null)
                })),
                "reverse" => Ok(native_method(l, |l, _| {
                    l.lock().unwrap().reverse();
                    Ok(DgnValue::Null)
                })),
                "contains" => Ok(native_method(l, |l, args| {
                    let val = args.get(0).cloned().unwrap_or(DgnValue::Null);
                    let vec = l.lock().unwrap();
                    let found = vec.iter().any(|x| val_to_string(x) == val_to_string(&val));
                    Ok(DgnValue::Bool(found))
                })),

                "reduce" => Ok(native_method(l, |l, args| {
                    let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                    let vec = l.lock().unwrap().clone();
                    if vec.is_empty() {
                        return Ok(args.get(1).cloned().unwrap_or(DgnValue::Null));
                    }
                    let mut iter = vec.into_iter();
                    // Use provided initial value, or default to the first element
                    let mut acc = if args.len() > 1 { args[1].clone() } else { iter.next().unwrap() };
                    
                    let rt = tokio::runtime::Handle::current();
                    for item in iter {
                        // Instead of block_in_place which signals Tokio to spawn a new thread:
                        acc = tokio::task::block_in_place(|| rt.block_on(call_dgn_func(func.clone(), vec![acc.clone(), item])))?;
                    }
                    Ok(acc)
                })),

                "find" => Ok(native_method(l, |l, args| {
                    let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                    let vec = l.lock().unwrap().clone();
                    let rt = tokio::runtime::Handle::current();
                    for item in vec {
                        let res = tokio::task::block_in_place(|| rt.block_on(call_dgn_func(func.clone(), vec![item.clone()])))?;
                        if is_truthy(&res) { return Ok(item); }
                    }
                    Ok(DgnValue::Null)
                })),

                "any" => Ok(native_method(l, |l, args| {
                    let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                    let vec = l.lock().unwrap().clone();
                    let rt = tokio::runtime::Handle::current();
                    for item in vec {
                        let res = tokio::task::block_in_place(|| rt.block_on(call_dgn_func(func.clone(), vec![item])))?;
                        if is_truthy(&res) { return Ok(DgnValue::Bool(true)); }
                    }
                    Ok(DgnValue::Bool(false))
                })),

                "all" => Ok(native_method(l, |l, args| {
                    let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                    let vec = l.lock().unwrap().clone();
                    let rt = tokio::runtime::Handle::current();
                    for item in vec {
                        let res = tokio::task::block_in_place(|| rt.block_on(call_dgn_func(func.clone(), vec![item])))?;
                        if !is_truthy(&res) { return Ok(DgnValue::Bool(false)); }
                    }
                    Ok(DgnValue::Bool(true))
                })),

                "flat" => Ok(native_method(l, |l, _| {
                    let vec = l.lock().unwrap();
                    let mut res = Vec::new();
                    for item in vec.iter() {
                        if let DgnValue::List(sub_l) = item {
                            res.extend(sub_l.lock().unwrap().clone());
                        } else {
                            res.push(item.clone());
                        }
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(res))))
                })),

                "zip" => Ok(native_method(l, |l, args| {
                    let other = args.get(0).cloned().unwrap_or(DgnValue::Null);
                    if let DgnValue::List(other_l) = other {
                        let vec1 = l.lock().unwrap();
                        let vec2 = other_l.lock().unwrap();
                        let mut res = Vec::new();
                        let len = vec1.len().min(vec2.len()); // Zip up to the shortest list
                        for i in 0..len {
                            let pair = vec![vec1[i].clone(), vec2[i].clone()];
                            res.push(DgnValue::List(Arc::new(Mutex::new(pair))));
                        }
                        Ok(DgnValue::List(Arc::new(Mutex::new(res))))
                    } else {
                        Err(DgnError::new("zip() expects a list as an argument", 0))
                    }
                })),

                "sum" => Ok(native_method(l, |l, _| {
                    let vec = l.lock().unwrap();
                    let mut total = 0.0;
                    let mut is_float = false;
                    for item in vec.iter() {
                        if let DgnValue::Number(n) = item { total += n; is_float = true; }
                        else if let DgnValue::Int(n) = item { total += *n as f64; }
                    }
                    // Return as integer if no floats were encountered
                    if is_float { Ok(DgnValue::Number(total)) } else { Ok(DgnValue::Int(total as i64)) }
                })),

                "first" => Ok(native_method(l, |l, _| {
                    Ok(l.lock().unwrap().first().cloned().unwrap_or(DgnValue::Null))
                })),

                "last" => Ok(native_method(l, |l, _| {
                    Ok(l.lock().unwrap().last().cloned().unwrap_or(DgnValue::Null))
                })),

                "slice" => Ok(native_method(l, |l, args| {
                    let vec = l.lock().unwrap();
                    let len = vec.len() as i64;
                    let start = args.get(0).and_then(|a| a.as_int()).unwrap_or(0);
                    let end = args.get(1).and_then(|a| a.as_int()).unwrap_or(len);
                    
                    // Support negative indices (e.g., -1 for the last item)
                    let start_idx = if start < 0 { (len + start).max(0) as usize } else { start.min(len) as usize };
                    let end_idx = if end < 0 { (len + end).max(0) as usize } else { end.min(len) as usize };
                    
                    let mut res = Vec::new();
                    if start_idx <= end_idx {
                        res.extend_from_slice(&vec[start_idx..end_idx]);
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(res))))
                })),

                "unique" => Ok(native_method(l, |l, _| {
                    let vec = l.lock().unwrap();
                    let mut res = Vec::new();
                    let mut seen = std::collections::HashSet::new();
                    for item in vec.iter() {
                        // Use stringification for deep-equality matching of nested objects/lists
                        let key = val_to_string(item);
                        if seen.insert(key) {
                            res.push(item.clone());
                        }
                    }
                    Ok(DgnValue::List(Arc::new(Mutex::new(res))))
                })),

                "sort_by" => Ok(native_method(l, |l, args| {
                    let func = args.get(0).cloned().ok_or(DgnError::new("Function required", 0))?;
                    let mut vec = l.lock().unwrap().clone();
                    let rt = tokio::runtime::Handle::current();
                    
                    // Pre-compute the keys to avoid repeatedly calling the Dragon function during sort
                    let mut pairs = Vec::new();
                    for item in vec {
                        let key = rt.block_on(call_dgn_func(func.clone(), vec![item.clone()]))?;
                        pairs.push((key, item));
                    }
                    
                    pairs.sort_by(|(key_a, _), (key_b, _)| {
                        match (key_a, key_b) {
                            (DgnValue::Number(n1), DgnValue::Number(n2)) => n1.partial_cmp(n2).unwrap_or(std::cmp::Ordering::Equal),
                            (DgnValue::Int(n1), DgnValue::Int(n2)) => n1.cmp(n2),
                            (DgnValue::Int(n1), DgnValue::Number(n2)) => (*n1 as f64).partial_cmp(n2).unwrap_or(std::cmp::Ordering::Equal),
                            (DgnValue::Number(n1), DgnValue::Int(n2)) => n1.partial_cmp(&(*n2 as f64)).unwrap_or(std::cmp::Ordering::Equal),
                            (DgnValue::String(s1), DgnValue::String(s2)) => s1.cmp(s2),
                            _ => std::cmp::Ordering::Equal
                        }
                    });
                    
                    let sorted_vec: Vec<DgnValue> = pairs.into_iter().map(|(_, v)| v).collect();
                    *l.lock().unwrap() = sorted_vec;
                    Ok(DgnValue::Null)
                })),
                _ => Ok(DgnValue::Null)
            }
        },
        DgnValue::String(s) => {
             match name {
                 "length" => Ok(DgnValue::Number(s.len() as f64)),
                 "upper" => { let s=s.clone(); Ok(native_fn(move |_| Ok(DgnValue::String(s.to_uppercase())))) },
                 "lower" => { let s=s.clone(); Ok(native_fn(move |_| Ok(DgnValue::String(s.to_lowercase())))) },
                 "trim" => { let s=s.clone(); Ok(native_fn(move |_| Ok(DgnValue::String(s.trim().to_string())))) },
                 "split" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let sep = if let Some(DgnValue::String(sp)) = args.get(0) { sp.clone() } else { " ".to_string() };
                     let parts = if sep.is_empty() {
                        s.chars().map(|c| DgnValue::String(c.to_string())).collect()
                     } else {
                        s.split(&sep).map(|p| DgnValue::String(p.to_string())).collect()
                     };
                     Ok(DgnValue::List(Arc::new(Mutex::new(parts))))
                 }))},
                 "replace" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let from = if let Some(DgnValue::String(f)) = args.get(0) { f } else { "" };
                     let to = if let Some(DgnValue::String(t)) = args.get(1) { t } else { "" };
                     Ok(DgnValue::String(s.replace(from, to)))
                 }))},
                 "contains" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let sub = if let Some(DgnValue::String(sb)) = args.get(0) { sb } else { "" };
                     Ok(DgnValue::Bool(s.contains(sub)))
                 }))},
                 "starts_with" => { let s=s.clone(); Ok(native_fn(move |args| {
                    let sub = if let Some(DgnValue::String(sb)) = args.get(0) { sb } else { "" };
                    Ok(DgnValue::Bool(s.starts_with(sub)))
                }))},
                "ends_with" => { let s=s.clone(); Ok(native_fn(move |args| {
                    let sub = if let Some(DgnValue::String(sb)) = args.get(0) { sb } else { "" };
                    Ok(DgnValue::Bool(s.ends_with(sub)))
                }))},
                 "index_of" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let sub = if let Some(DgnValue::String(sb)) = args.get(0) { sb } else { "" };
                     Ok(DgnValue::Number(s.find(sub).map(|i| i as f64).unwrap_or(-1.0)))
                 }))},
                 "substring" => { let s=s.clone(); Ok(native_fn(move |args| {
                     let start = if let Some(n) = args.get(0).and_then(|a| a.as_number()) { n as usize } else { 0 };
                     let end = if let Some(n) = args.get(1).and_then(|a| a.as_number()) { n as usize } else { s.len() };
                     if start < s.len() && end <= s.len() && start <= end {
                         Ok(DgnValue::String(s[start..end].to_string()))
                     } else {
                         Ok(DgnValue::String("".to_string()))
                     }
                 }))},
                 _ => Ok(DgnValue::Null)
             }
        },
        DgnValue::ByteArray(b) => {
            match name {
                "length" => Ok(DgnValue::Number(b.lock().unwrap().len() as f64)),
                "to_string" => Ok(native_method(b, |b, _| {
                    let v = b.lock().unwrap().clone();
                    Ok(DgnValue::String(String::from_utf8_lossy(&v).to_string()))
                })),
                "push" => Ok(native_method(b, |b, args| {
                    if let Some(n) = args.get(0).and_then(|a| a.as_number()) {
                        b.lock().unwrap().push(n as u8);
                    }
                    Ok(DgnValue::Null)
                })),
                 _ => Ok(DgnValue::Null)
            }
        },
       #[cfg(not(any(target_arch = "wasm32")))]
        DgnValue::NativeLib(lib) => {
            let lib_ref = lib.clone();
            let func_name = CString::new(name).unwrap();

            Ok(native_fn(move |args| {
                let l = lib_ref.lock().unwrap();
                
                // 1. Prepare Arguments
                let mut raw_args: Vec<usize> = Vec::new();
                let mut keep_alive_strs = Vec::new(); // Keeps C-Strings valid in memory

                for arg in args {
                    match arg {
                        DgnValue::Int(n) => raw_args.push(n as usize),
                        DgnValue::Number(n) => raw_args.push(n as usize),
                        DgnValue::Bool(b) => raw_args.push(if b { 1 } else { 0 }),
                        DgnValue::String(s) => {
                            let c = CString::new(s).unwrap();
                            raw_args.push(c.as_ptr() as usize);
                            keep_alive_strs.push(c);
                        },
                        DgnValue::ByteArray(b) => {
                             let ptr = b.lock().unwrap().as_ptr() as usize;
                             raw_args.push(ptr);
                        },
                        DgnValue::Null => raw_args.push(0),
                        _ => raw_args.push(0),
                    }
                }

                // 2. Pad to 8 arguments (The Magic Fix)
                // This ensures we can call functions with 0 to 8 arguments safely
                while raw_args.len() < 8 { raw_args.push(0); }

                unsafe {
                    // Define a generic function pointer that takes 8 usize args
                    type GenericFn = unsafe extern "C" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;

                    let symbol: Symbol<GenericFn> = match l.get(func_name.as_bytes()) {
                        Ok(s) => s,
                        Err(e) => return Err(DgnError::new(&format!("FFI Error: Symbol '{}' not found", func_name.to_str().unwrap()), 0))
                    };

                    // 3. Call the C function
                    let res = symbol(
                        raw_args[0], raw_args[1], raw_args[2], raw_args[3],
                        raw_args[4], raw_args[5], raw_args[6], raw_args[7]
                    );

                    // 4. Return Result
                    // Heuristic: If it looks like a boolean (0 or 1), return Bool, otherwise Number.
                    // You can adjust this logic if needed.
                    if res == 0 || res == 1 {
                        Ok(DgnValue::Bool(res != 0))
                    } else {
                        Ok(DgnValue::Number(res as f64))
                    }
                }
            }))
        },
        DgnValue::Regex(r) => {
            match name {
                "test" => Ok(native_method(r, |r, args| {
                    let s = args.get(0).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::Bool(r.is_match(&s)))
                })),
                "match" => Ok(native_method(r, |r, args| {
                    let s = args.get(0).map(val_to_string).unwrap_or_default();
                    if let Some(m) = r.find(&s) { Ok(DgnValue::String(m.as_str().to_string())) } else { Ok(DgnValue::Null) }
                })),
                "replace" => Ok(native_method(r, |r, args| {
                    let s = args.get(0).map(val_to_string).unwrap_or_default();
                    let repl = args.get(1).map(val_to_string).unwrap_or_default();
                    Ok(DgnValue::String(r.replace_all(&s, &repl).to_string()))
                })),
                "find_all" => Ok(native_method(r, |r, args| {
                    let s = args.get(0).map(val_to_string).unwrap_or_default();
                    let matches: Vec<DgnValue> = r.find_iter(&s)
                        .map(|m| DgnValue::String(m.as_str().to_string()))
                        .collect();
                    Ok(DgnValue::List(Arc::new(Mutex::new(matches))))
                })),
                "groups" => Ok(native_method(r, |r, args| {
                    let s = args.get(0).map(val_to_string).unwrap_or_default();
                    if let Some(caps) = r.captures(&s) {
                        let groups: Vec<DgnValue> = caps.iter()
                            .map(|m| m.map(|c| DgnValue::String(c.as_str().to_string()))
                                    .unwrap_or(DgnValue::Null))
                            .collect();
                        Ok(DgnValue::List(Arc::new(Mutex::new(groups))))
                    } else {
                        Ok(DgnValue::Null)
                    }
                })),
                _ => Ok(DgnValue::Null)
            }
        }
        _ => Ok(DgnValue::Null)
    }
}

pub fn native_method<T: Send + Sync + 'static>(target: Arc<T>, f: impl Fn(Arc<T>, Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync + 'static) -> DgnValue {
    DgnValue::NativeMethod(Arc::new(move |args| f(target.clone(), args)))
}

pub fn native_fn(f: impl Fn(Vec<DgnValue>) -> Result<DgnValue, DgnError> + Send + Sync + 'static) -> DgnValue {
    DgnValue::Func(Arc::new(DgnFunc {
        name: Some("native".into()), params: vec![], req_params: 0, body: None,
        closure: Arc::new(Mutex::new(Env::new(None, None))), is_native: true, is_async: false,
        compiled_chunk: None,
        native_fn: Some(Arc::new(f))
    }))
}

// pub async fn call_dgn_func(func: DgnValue, args: Vec<DgnValue>) -> Result<DgnValue, DgnError> {
//     if let DgnValue::Func(f) = func {
//         if f.is_native {
//              return (f.native_fn.unwrap())(args);
//         }
//         call_dgn_func_impl(f, args, None).await
//     } else {
//         Err(DgnError::new("Not a function", 0))
//     }
// }

pub async fn call_dgn_func(func: DgnValue, args: Vec<DgnValue>) -> Result<DgnValue, DgnError> {
    if let DgnValue::Func(f) = func {
        if f.is_native {
             return (f.native_fn.as_ref().unwrap())(args);
        }
        call_dgn_func_impl(f, args, None).await
    } else {
        Err(DgnError::new("Not a function", 0))
    }
}

#[async_recursion]
pub async fn call_dgn_func_impl(f: Arc<DgnFunc>, args: Vec<DgnValue>, this_ctx: Option<DgnValue>) -> Result<DgnValue, DgnError> {
    

    let has_rest = f.params.last().map(|(n, _)| n.starts_with("...")).unwrap_or(false);
    if !has_rest && args.len() < f.req_params {
        let fn_name = f.name.as_deref().unwrap_or("<anonymous>");
        return Err(DgnError::new(&format!("Function '{}' expects at least {} argument(s), but got {}", fn_name, f.req_params, args.len()), 0));
    }

    // Build the call scope directly — no Arc/Mutex needed until we hand it to run()
    let current_path = f.closure.lock().unwrap().current_path.clone();
    let param_count = f.params.len() + if this_ctx.is_some() { 1 } else { 0 } + 1;
    let mut scope_store = HashMap::with_capacity(param_count);

    // Bind parameters directly into the store
    for (i, (param_name, param_type)) in f.params.iter().enumerate() {
        if let Some(rest_name) = param_name.strip_prefix("...") {
            let rest: Vec<DgnValue> = args.get(i..).unwrap_or(&[]).to_vec();
            scope_store.insert(
                rest_name.to_string(),
                DgnValue::List(Arc::new(Mutex::new(rest))),
            );
            break;
        }

        let v = args.get(i).cloned().unwrap_or(DgnValue::Null);

        if let Some(expected) = param_type {
            let actual = match &v {
                DgnValue::Null => "null",
                DgnValue::Bool(_) => "bool",
                DgnValue::Number(_) => "number",
                DgnValue::Int(_) => "int",
                DgnValue::String(_) => "string",
                DgnValue::List(_) => "list",
                DgnValue::Map(_) => "map",
                DgnValue::ByteArray(_) => "bytes",
                DgnValue::Func(_) => "function",
                DgnValue::Class(_) => "class",
                DgnValue::Instance(_) => "instance",
                _ => "any",
            };
            let is_compatible = (expected == actual)
                || (expected == "any")
                || (expected == "number" && actual == "int");
            if !is_compatible {
                let fn_name = f.name.as_deref().unwrap_or("<anonymous>");
                return Err(DgnError::new(
                    &format!("TypeError in '{}': Parameter '{}' expects '{}', got '{}'",
                        fn_name, param_name, expected, actual), 0));
            }
        }

        scope_store.insert(param_name.clone(), v);
    }

    if let Some(th) = this_ctx {
        scope_store.insert("this".to_string(), th);
    }

    // Wrap once — single Arc+Mutex construction, no double-lock
    let scope = Arc::new(Mutex::new(Env {
        store: scope_store,
        constants: HashSet::new(),
        parent: Some(f.closure.clone()),
        current_path,
    }));

    if let Some(chunk_arc) = &f.compiled_chunk {
        let mut vm = acquire_vm();
        let result = vm.run(chunk_arc, scope).await;
        release_vm(vm);
        return result;
    }
    Ok(DgnValue::Null)
}

