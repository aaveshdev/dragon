use super::*;

pub struct GcState {
    objects: Vec<WeakContainer>,
    allocations: usize,
    threshold: usize,
}

enum WeakContainer {
    List(std::sync::Weak<Mutex<Vec<DgnValue>>>),
    Map(std::sync::Weak<Mutex<HashMap<String, DgnValue>>>),
}

impl GcState {
    fn new() -> Self {
        GcState { objects: Vec::new(), allocations: 0, threshold: 1000 }
    }
}

pub fn gc_register(val: &DgnValue) {
    // try_with safely checks if we are running inside an agent's Tokio task
    let _ = GC_STATE.try_with(|state_cell| {
        let mut state = state_cell.borrow_mut(); // No Mutex lock! Fast!
        match val {
            DgnValue::List(l) => {
                state.objects.push(WeakContainer::List(Arc::downgrade(l)));
                state.allocations += 1;
            },
            DgnValue::Map(m) | DgnValue::Instance(m) => {
                state.objects.push(WeakContainer::Map(Arc::downgrade(m)));
                state.allocations += 1;
            },
            _ => {}
        }
    });
}

pub fn gc_check_auto(env: Arc<Mutex<Env>>) {
    let should_collect = GC_STATE.try_with(|state_cell| {
        let state = state_cell.borrow();
        state.allocations >= state.threshold
    }).unwrap_or(false);
    
    if should_collect {
        gc_collect(env);
    }
}

pub fn gc_collect(root_env: Arc<Mutex<Env>>) -> usize {
    let mut visited_objs = HashSet::new(); 
    let mut visited_envs = HashSet::new(); 
    
    mark_env(root_env, &mut visited_objs, &mut visited_envs);
    
    GC_STATE.try_with(|state_cell| {
        let mut state = state_cell.borrow_mut();
        let mut reclaimed = 0;
        
        state.objects.retain(|weak| {
            match weak {
                WeakContainer::List(w) => {
                    if let Some(arc) = w.upgrade() {
                        let ptr = Arc::as_ptr(&arc) as usize;
                        if !visited_objs.contains(&ptr) {
                            arc.lock().unwrap().clear();
                            reclaimed += 1;
                            return false; 
                        }
                        true 
                    } else { false }
                },
                WeakContainer::Map(w) => {
                    if let Some(arc) = w.upgrade() {
                        let ptr = Arc::as_ptr(&arc) as usize;
                        if !visited_objs.contains(&ptr) {
                            arc.lock().unwrap().clear();
                            reclaimed += 1;
                            return false; 
                        }
                        true
                    } else { false }
                }
            }
        });
        
        state.allocations = 0;
        reclaimed
    }).unwrap_or(0)
}

fn mark_env(env: Arc<Mutex<Env>>, visited_objs: &mut HashSet<usize>, visited_envs: &mut HashSet<usize>) {
    let mut curr = Some(env);
    
    while let Some(e_arc) = curr {
        let ptr = Arc::as_ptr(&e_arc) as usize;
        
        if !visited_envs.insert(ptr) {
            break;
        }

        let e = e_arc.lock().unwrap();
        for val in e.store.values() {
            mark_val(val, visited_objs, visited_envs);
        }
        
        curr = e.parent.clone();
    }
}

fn mark_val(val: &DgnValue, visited_objs: &mut HashSet<usize>, visited_envs: &mut HashSet<usize>) {
    match val {
        DgnValue::List(l) => {
            let ptr = Arc::as_ptr(l) as usize;
            if visited_objs.insert(ptr) {
                let vec = l.lock().unwrap();
                for v in vec.iter() { mark_val(v, visited_objs, visited_envs); }
            }
        },
        DgnValue::Map(m) | DgnValue::Instance(m) => {
            let ptr = Arc::as_ptr(m) as usize;
            if visited_objs.insert(ptr) {
                let map = m.lock().unwrap();
                for v in map.values() { mark_val(v, visited_objs, visited_envs); }
            }
        },
        DgnValue::Func(f) => {
            mark_env(f.closure.clone(), visited_objs, visited_envs);
        },
        DgnValue::Class(env) => {
            mark_env(env.clone(), visited_objs, visited_envs);
        },
        _ => {}
    }
}

