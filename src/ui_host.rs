use crate::engine::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const FRAMEWORK_SOURCES: &[(&str, &str)] = &[
    ("dragon/core", include_str!("../framework/core.dgn")),
    ("dragon/runtime", include_str!("../framework/runtime.dgn")),
    ("dragon/widgets", include_str!("../framework/widgets.dgn")),
    ("dragon/material", include_str!("../framework/material.dgn")),
    ("dragon/services", include_str!("../framework/services.dgn")),
    ("dragon/extras", include_str!("../framework/extras.dgn")),
    ("dragon/ui", include_str!("../framework/ui.dgn")),
];

pub struct UiHost {
    rt: tokio::runtime::Runtime,
    env: Arc<Mutex<Env>>,
}

impl UiHost {
    pub fn new(app_dir: &Path, docs_dir: &str, platform: &str, media_json: &str) -> Result<UiHost, String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .map_err(|e| format!("could not start async runtime: {}", e))?;

        let entry = app_dir.join("lib").join("main.dgn");
        if !entry.exists() {
            return Err(format!("{} not found", entry.display()));
        }

        let env = Arc::new(Mutex::new(Env::new(None, Some(entry.clone()))));
        {
            let mut e = env.lock().unwrap();
            e.store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
            e.store.insert("__app_dir__".into(), DgnValue::String(app_dir.to_string_lossy().to_string()));
            let mut bundle = HashMap::new();
            for (name, src) in FRAMEWORK_SOURCES {
                bundle.insert(name.to_string(), DgnValue::String(src.to_string()));
            }
            e.store.insert("__bundled_sources__".into(), DgnValue::Map(Arc::new(Mutex::new(bundle))));
        }
        populate_builtins(env.clone());
        {
            let mut e = env.lock().unwrap();
            e.store.insert("DRAGON_DOCS_DIR".into(), DgnValue::String(docs_dir.to_string()));
            e.store.insert("DRAGON_PLATFORM".into(), DgnValue::String(platform.to_string()));
            e.store.insert("DRAGON_MEDIA_JSON".into(), DgnValue::String(media_json.to_string()));
        }

        let host = UiHost { rt, env };
        host.run_source(
            "import \"dragon/ui\" as *\n\
             import \"main\" as DRAGON_APP\n\
             dragon_host_init(DRAGON_DOCS_DIR, DRAGON_PLATFORM, DRAGON_MEDIA_JSON)\n\
             if (DRAGON_APP.main != null) { DRAGON_APP.main() }\n",
            "bootstrap",
        )?;
        Ok(host)
    }

    fn run_source(&self, src: &str, label: &str) -> Result<(), String> {
        let tokens = lex(src).map_err(|e| format!("{}: syntax error: {}", label, e.msg))?;
        let mut parser = Parser::new(tokens);
        let ast = parser.parse().map_err(|e| format!("{}: {}", label, e))?;
        let mut chunk = Chunk::new();
        Compiler::compile_stmt(&mut chunk, &ast);
        chunk.emit(Instruction::Halt, 0);
        let env = self.env.clone();
        self.rt
            .block_on(async move {
                let mut vm = Vm::new();
                vm.run(&chunk, env).await.map(|_| ())
            })
            .map_err(|e| format!("{} (line {})", e.msg, e.line))
    }

    fn call(&self, name: &str, args: Vec<DgnValue>) -> Result<DgnValue, String> {
        let f = self
            .env
            .lock()
            .unwrap()
            .get(name)
            .ok_or_else(|| format!("framework function '{}' is not defined", name))?;
        self.rt
            .block_on(async move { call_dgn_func(f, args).await })
            .map_err(|e| format!("{} (line {})", e.msg, e.line))
    }

    fn s(v: &str) -> DgnValue {
        DgnValue::String(v.to_string())
    }

    fn as_bool(v: DgnValue) -> bool {
        matches!(v, DgnValue::Bool(true))
    }

    fn as_string(v: DgnValue) -> String {
        match v {
            DgnValue::String(s) => s,
            DgnValue::Null => String::new(),
            other => val_to_string(&other),
        }
    }

    pub fn render(&self) -> String {
        match self.call("dragon_host_render", vec![]) {
            Ok(v) => Self::as_string(v),
            Err(e) => format!("{{\"v\":1,\"error\":{}}}", serde_json::to_string(&e).unwrap_or_else(|_| "\"error\"".into())),
        }
    }

    pub fn event(&self, evt: &str) -> bool {
        self.call("dragon_host_event", vec![Self::s(evt)]).map(Self::as_bool).unwrap_or(false)
    }

    pub fn is_dirty(&self) -> bool {
        self.call("dragon_host_is_dirty", vec![]).map(Self::as_bool).unwrap_or(false)
    }

    pub fn tick(&self, now_ms: i64) -> i64 {
        match self.call("dragon_host_tick", vec![DgnValue::Int(now_ms)]) {
            Ok(v) => v.as_int().unwrap_or(-1),
            Err(_) => -1,
        }
    }

    pub fn platform_calls(&self) -> String {
        match self.call("dragon_host_platform_calls", vec![]) {
            Ok(v) => Self::as_string(v),
            Err(_) => "[]".to_string(),
        }
    }

    pub fn platform_result(&self, id: &str, result_json: &str) -> bool {
        self.call("dragon_host_platform_result", vec![Self::s(id), Self::s(result_json)])
            .map(Self::as_bool)
            .unwrap_or(false)
    }

    pub fn back(&self) -> bool {
        self.call("dragon_host_back", vec![]).map(Self::as_bool).unwrap_or(false)
    }

    pub fn set_media(&self, media_json: &str) -> bool {
        self.call("dragon_host_set_media", vec![Self::s(media_json)]).map(Self::as_bool).unwrap_or(false)
    }

    pub fn lifecycle(&self, state: &str) -> bool {
        self.call("dragon_host_lifecycle", vec![Self::s(state)]).map(Self::as_bool).unwrap_or(false)
    }

    pub fn last_error(&self) -> String {
        self.call("dragon_host_last_error", vec![]).map(Self::as_string).unwrap_or_default()
    }
}

pub fn default_media_json() -> String {
    "{\"width\":411,\"height\":891,\"pixelRatio\":2.625,\"top\":24,\"bottom\":0,\"left\":0,\"right\":0,\"keyboard\":0,\"brightness\":\"light\",\"textScale\":1.0,\"locale\":\"en_US\"}".to_string()
}

pub fn newest_mtime(dir: &Path) -> u64 {
    fn walk(dir: &Path, best: &mut u64) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, best);
                } else if p.extension().map(|x| x == "dgn").unwrap_or(false) {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(m) = meta.modified() {
                            if let Ok(d) = m.duration_since(std::time::UNIX_EPOCH) {
                                let t = d.as_millis() as u64;
                                if t > *best {
                                    *best = t;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut best = 0;
    walk(dir, &mut best);
    best
}

pub fn find_tap_id(frame: &serde_json::Value, label: &str) -> Option<String> {
    fn matches_label(node: &serde_json::Value, label: &str) -> bool {
        if let Some(obj) = node.as_object() {
            for key in ["text", "label", "hint"] {
                if obj.get(key).and_then(|v| v.as_str()) == Some(label) {
                    return true;
                }
            }
        }
        false
    }
    fn handler_of(node: &serde_json::Value) -> Option<String> {
        if let Some(obj) = node.as_object() {
            for key in ["onTap", "onPressed", "onChanged", "onSelected", "onLongPress"] {
                if let Some(id) = obj.get(key).and_then(|v| v.as_str()) {
                    return Some(id.to_string());
                }
            }
        }
        None
    }
    fn walk(node: &serde_json::Value, label: &str, stack: &mut Vec<Option<String>>, owner: Option<(String, usize)>) -> Option<String> {
        let h = match &owner {
            Some((id, i)) => Some(format!("{}:{}", id, i)),
            None => handler_of(node),
        };
        stack.push(h.clone());
        if matches_label(node, label) {
            for cand in stack.iter().rev() {
                if let Some(id) = cand {
                    return Some(id.clone());
                }
            }
        }
        match node {
            serde_json::Value::Object(map) => {
                let my_handler = handler_of(node);
                for (k, v) in map {
                    if (k == "items" || k == "labels") && my_handler.is_some() {
                        if let Some(arr) = v.as_array() {
                            for (i, item) in arr.iter().enumerate() {
                                if let Some(found) = walk(item, label, stack, Some((my_handler.clone().unwrap(), i))) {
                                    return Some(found);
                                }
                            }
                            continue;
                        }
                    }
                    if let Some(found) = walk(v, label, stack, None) {
                        return Some(found);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for v in items {
                    if let Some(found) = walk(v, label, stack, None) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }
        stack.pop();
        None
    }
    walk(frame, label, &mut Vec::new(), None)
}

pub fn app_dir_from_arg(arg: Option<&str>) -> PathBuf {
    let p = PathBuf::from(arg.unwrap_or("."));
    if p.join("lib").join("main.dgn").exists() {
        p
    } else if p.file_name().map(|n| n == "lib").unwrap_or(false) && p.join("main.dgn").exists() {
        p.parent().map(|x| x.to_path_buf()).unwrap_or(p)
    } else {
        p
    }
}

#[cfg(any(target_os = "android", feature = "jni-glue"))]
pub mod jni_glue {
    use super::*;
    use jni::objects::{JObject, JString};
    use jni::sys::{jboolean, jlong, jstring, JNI_FALSE, JNI_TRUE};
    use jni::JNIEnv;

    static HOST: Mutex<Option<UiHost>> = Mutex::new(None);

    fn jstr(env: &mut JNIEnv, s: &JString) -> String {
        env.get_string(s).map(|j| j.into()).unwrap_or_default()
    }

    fn out(env: &mut JNIEnv, s: &str) -> jstring {
        env.new_string(s).map(|j| j.into_raw()).unwrap_or(std::ptr::null_mut())
    }

    fn b(v: bool) -> jboolean {
        if v { JNI_TRUE } else { JNI_FALSE }
    }

    fn with_host<R>(default: R, f: impl FnOnce(&UiHost) -> R) -> R {
        match HOST.lock() {
            Ok(guard) => match guard.as_ref() {
                Some(h) => f(h),
                None => default,
            },
            Err(_) => default,
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_init(
        mut env: JNIEnv,
        _this: JObject,
        app_dir: JString,
        docs_dir: JString,
        media_json: JString,
    ) -> jstring {
        let app_dir = jstr(&mut env, &app_dir);
        let docs_dir = jstr(&mut env, &docs_dir);
        let media = jstr(&mut env, &media_json);
        match UiHost::new(Path::new(&app_dir), &docs_dir, "android", &media) {
            Ok(h) => {
                *HOST.lock().unwrap() = Some(h);
                out(&mut env, "")
            }
            Err(e) => out(&mut env, &e),
        }
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_render(env: JNIEnv, _this: JObject) -> jstring {
        let mut env = env;
        let s = with_host("{\"v\":1,\"error\":\"Dragon runtime is not initialised\"}".to_string(), |h| h.render());
        out(&mut env, &s)
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_event(
        mut env: JNIEnv,
        _this: JObject,
        evt: JString,
    ) -> jboolean {
        let evt = jstr(&mut env, &evt);
        b(with_host(false, |h| h.event(&evt)))
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_isDirty(_env: JNIEnv, _this: JObject) -> jboolean {
        b(with_host(false, |h| h.is_dirty()))
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_tick(
        _env: JNIEnv,
        _this: JObject,
        now_ms: jlong,
    ) -> jlong {
        with_host(-1, |h| h.tick(now_ms))
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_platformCalls(env: JNIEnv, _this: JObject) -> jstring {
        let mut env = env;
        let s = with_host("[]".to_string(), |h| h.platform_calls());
        out(&mut env, &s)
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_platformResult(
        mut env: JNIEnv,
        _this: JObject,
        id: JString,
        result_json: JString,
    ) -> jboolean {
        let id = jstr(&mut env, &id);
        let res = jstr(&mut env, &result_json);
        b(with_host(false, |h| h.platform_result(&id, &res)))
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_back(_env: JNIEnv, _this: JObject) -> jboolean {
        b(with_host(false, |h| h.back()))
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_setMedia(
        mut env: JNIEnv,
        _this: JObject,
        media_json: JString,
    ) -> jboolean {
        let m = jstr(&mut env, &media_json);
        b(with_host(false, |h| h.set_media(&m)))
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_lifecycle(
        mut env: JNIEnv,
        _this: JObject,
        state: JString,
    ) -> jboolean {
        let s = jstr(&mut env, &state);
        b(with_host(false, |h| h.lifecycle(&s)))
    }

    #[no_mangle]
    pub extern "system" fn Java_org_dragonlang_runtime_DragonNative_lastError(env: JNIEnv, _this: JObject) -> jstring {
        let mut env = env;
        let s = with_host(String::new(), |h| h.last_error());
        out(&mut env, &s)
    }
}
