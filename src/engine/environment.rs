use super::*;

#[derive(Debug)]
pub struct Env {
    pub store: HashMap<String, DgnValue>,
    pub constants: HashSet<String>,
    pub parent: Option<Arc<Mutex<Env>>>,
    pub current_path: PathBuf,
}

impl Env {

    pub fn new(parent: Option<Arc<Mutex<Env>>>, path: Option<PathBuf>) -> Self {
        let p = path.or_else(|| {
            parent.as_ref().map(|par| par.lock().unwrap().current_path.clone())
        }).unwrap_or_else(|| PathBuf::from("."));

        Env { store: HashMap::new(), constants: HashSet::new(), parent, current_path: p }
    }

    pub fn get(&self, name: &str) -> Option<DgnValue> {
        if let Some(v) = self.store.get(name) {
          return Some(v.clone());
        }
        let mut current_parent = self.parent.clone();
        while let Some(parent_arc) = current_parent {
            let parent_guard = parent_arc.lock().unwrap();
            if let Some(v) = parent_guard.store.get(name) {
            return Some(v.clone());
        }
        current_parent = parent_guard.parent.clone();
        }
        None
    }

    pub fn define(&mut self, name: String, val: DgnValue) {
        self.store.insert(name, val);
    }

    pub fn set(&mut self, name: impl Into<String>, val: DgnValue) -> Result<(), String> {
        let name = name.into();

        if self.constants.contains(&name) {
            return Err(format!("Cannot reassign constant '{}'", name));
        }

         if self.update_existing(&name, &val)? {
            return Ok(());
        }
        self.store.insert(name, val);
        Ok(())
    }

    pub fn define_const(&mut self, name: String, val: DgnValue) -> Result<(), String> {
        if self.store.contains_key(&name) {
            return Err(format!("Variable '{}' already declared", name));
        }
        self.store.insert(name.clone(), val);
        self.constants.insert(name);
        Ok(())
    }

    pub fn update_existing(&mut self, name: &str, val: &DgnValue) -> Result<bool, String> {
        if self.store.contains_key(name) {
        if self.constants.contains(name) {
            return Err(format!("Cannot reassign constant '{}'", name));
        }
        self.store.insert(name.to_string(), val.clone());
            return Ok(true);
        }
        let mut current_parent = self.parent.clone();
        while let Some(parent_arc) = current_parent {
            let mut parent_guard = parent_arc.lock().unwrap();
            if parent_guard.store.contains_key(name) {
                if parent_guard.constants.contains(name) {
                    return Err(format!("Cannot reassign constant '{}'", name));
                }
                 parent_guard.store.insert(name.to_string(), val.clone());
                    return Ok(true);
            }
        current_parent = parent_guard.parent.clone();
        }
        Ok(false)
    }

    pub fn get_root(env: Arc<Mutex<Env>>) -> Arc<Mutex<Env>> {
        let mut curr = env;
        loop {
            let parent = curr.lock().unwrap().parent.clone();
            match parent {
                Some(p) => curr = p,
                None => return curr,
            }
        }
    }
}
