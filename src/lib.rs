use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

pub mod engine;
pub mod ui_host;
pub mod packages;
use engine::*;
use std::panic;

#[cfg(unix)]
use libc::{Dl_info, dladdr};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(all(target_os = "android", feature = "legacy_android"))]
use jni::sys::jstring;

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref GLOBAL_ENV: Arc<Mutex<Env>> = Arc::new(Mutex::new(Env::new(None, None)));
    static ref IS_INIT: Mutex<bool> = Mutex::new(false);
}

#[cfg(all(target_os = "android", feature = "legacy_android"))]
use jni::{JNIEnv, JavaVM, sys::{jint, JNI_VERSION_1_6}};
#[cfg(all(target_os = "android", feature = "legacy_android"))]
use jni::objects::{JClass, JObject, JString, JValue, GlobalRef};

#[cfg(all(target_os = "android", feature = "legacy_android"))]
use std::os::raw::c_void;

#[cfg(all(target_os = "android", feature = "legacy_android"))]
static mut ANDROID_CALLBACK: Option<GlobalRef> = None;
#[cfg(all(target_os = "android", feature = "legacy_android"))]
static mut JAVA_VM: Option<JavaVM> = None;

#[cfg(all(target_os = "android", feature = "legacy_android"))]
fn android_log(msg: &str) {
    use std::ffi::CString;
    use std::os::raw::{c_int, c_char};

    #[link(name = "log")]
    extern "C" {
        fn __android_log_print(prio: c_int, tag: *const c_char, fmt: *const c_char, ...) -> c_int;
    }

    const ANDROID_LOG_INFO: c_int = 4;

    let tag = CString::new("DragonNative").unwrap();
    let c_msg = CString::new(msg).unwrap_or_else(|_| CString::new("Invalid Log").unwrap());

    unsafe {
        __android_log_print(
            ANDROID_LOG_INFO,
            tag.as_ptr(),
            c_msg.as_ptr(),
        );
    }
}

#[cfg(all(target_os = "android", feature = "legacy_android"))]
fn get_android_lib_path() -> Option<PathBuf> {
    let mut info: Dl_info = unsafe { std::mem::zeroed() };

    unsafe {
        if dladdr(JNI_OnLoad as *const c_void, &mut info) != 0 {
            if !info.dli_fname.is_null() {
                let c_str = CStr::from_ptr(info.dli_fname);
                return Some(PathBuf::from(c_str.to_string_lossy().to_string()));
            }
        }
    }
    None
}

#[cfg(all(target_os = "android", feature = "legacy_android"))]
#[no_mangle]
pub extern "system" fn JNI_OnLoad(vm: JavaVM, _: *mut c_void) -> jint {
    let mut env = match vm.get_env() {
        Ok(env) => env,
        Err(_) => return JNI_VERSION_1_6,
    };

    let class_name = "io/dragon/runtime/DragonBridge";

    let bridge_class = match env.find_class(class_name) {
        Ok(c) => c,
        Err(e) => {
            android_log(&format!("CRITICAL: Class not found: {:?}", e));
            return JNI_VERSION_1_6;
        }
    };

    let methods = [
        jni::NativeMethod {
            name: "initDragon".into(),
            sig: "(Lio/dragon/runtime/DragonBridge$DragonCallback;)V".into(),
            fn_ptr: native_init_dragon as *mut c_void,
        },
        jni::NativeMethod {
            name: "runScriptBundle".into(),
            sig: "(Ljava/lang/String;)Ljava/lang/String;".into(),
            fn_ptr: native_run_script as *mut c_void,
        },
        jni::NativeMethod {
            name: "triggerEvent".into(),
            sig: "(Ljava/lang/String;)V".into(),
            fn_ptr: native_trigger_event as *mut c_void,
        },
    ];

    env.register_native_methods(bridge_class, &methods).expect("Failed to register natives");

    JNI_VERSION_1_6
}

#[cfg(all(target_os = "android", feature = "legacy_android"))]
unsafe extern "C" fn native_init_dragon(mut env: JNIEnv, _: JClass, callback: JObject) {
    let vm = env.get_java_vm().unwrap();
    JAVA_VM = Some(vm);
    ANDROID_CALLBACK = Some(env.new_global_ref(callback).unwrap());

    let env_arc = GLOBAL_ENV.clone();
    {
        let mut e = env_arc.lock().unwrap();
        e.store.clear();
        e.store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
    }
    populate_builtins(env_arc.clone());

    let lib_dir_str = if let Some(lib_path) = get_android_lib_path() {
        if let Some(parent) = lib_path.parent() {
            parent.to_string_lossy().to_string()
        } else {
            "".to_string()
        }
    } else {
        "".to_string()
    };

    let mut e = env_arc.lock().unwrap();

    let _ = e.set("ANDROID_LIB_DIR", DgnValue::String(lib_dir_str.clone()));

    e.set("showln", native_fn(|args| {
        let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
        android_log(&s);
        Ok(DgnValue::Null)
    })).unwrap();

    e.set("render_ui", native_fn(|args| {
        Ok(DgnValue::Null)
    })).unwrap();

    if let Some(lib_path) = get_android_lib_path() {
        android_log(&format!("Checking for embedded bundle in: {:?}", lib_path));

        if let Some(payload) = check_for_bundle(&lib_path) {
            android_log("Bundle found! Loading...");

            if let Ok(sources) = serde_json::from_str::<HashMap<String, String>>(&payload) {
                let mut bundle_map = HashMap::new();
                for (k, v) in &sources {
                    if k != "__entry__" { bundle_map.insert(k.clone(), DgnValue::String(v.clone())); }
                }
                e.store.insert("__bundled_sources__".to_string(), DgnValue::Map(Arc::new(Mutex::new(bundle_map))));

                if let Some(entry_code) = sources.get("__entry__") {
                    drop(e);
                    let tokens = match lex(entry_code) {
                        Ok(t) => t,
                        Err(e) => return format!("Syntax Error: {} at line {}, col {}", e.msg, e.line, e.col),
                    };
                    let mut parser = Parser::new(tokens);
                    if let Ok(ast) = parser.parse() {
                        let rt = tokio::runtime::Runtime::new().unwrap();

                        let mut chunk = Chunk::new();
                        Compiler::compile_stmt(&mut chunk, &ast);
                        chunk.emit(Instruction::Halt, 0);

                        if let Err(err) = rt.block_on(async {
                            let mut vm = Vm::new();
                            vm.run(&chunk, env_arc.clone()).await
                        }) {
                            android_log(&format!("DRAGON CRASH: {} at line {}", err.msg, err.line));
                        }
                    } else {
                        android_log("DRAGON PARSE ERROR: Failed to parse bundle.");
                    }
                    android_log("Embedded App Execution Finished.");
                }
            }
        }
    }

    *IS_INIT.lock().unwrap() = true;
}

#[cfg(all(target_os = "android", feature = "legacy_android"))]
#[no_mangle]
#[export_name = "main"]
pub extern "C" fn dragon_android_main(_argc: c_int, _argv: *const *const c_char) -> c_int {
    android_log("SUCCESS: Allegro successfully found and invoked Dragon main()!");

    let env_arc = GLOBAL_ENV.clone();
    {
        let mut e = env_arc.lock().unwrap();
        e.store.clear();
        e.store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
    }
    populate_builtins(env_arc.clone());

    let lib_dir_str = if let Some(lib_path) = get_android_lib_path() {
        if let Some(parent) = lib_path.parent() {
            parent.to_string_lossy().to_string()
        } else {
            "".to_string()
        }
    } else {
        "".to_string()
    };

    {
        let mut e = env_arc.lock().unwrap();
        let _ = e.set("ANDROID_LIB_DIR", DgnValue::String(lib_dir_str.clone()));

        let _ = e.set("showln", native_fn(|args| {
            let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
            android_log(&s);
            Ok(DgnValue::Null)
        }));
    }

    if let Some(lib_path) = get_android_lib_path() {
        android_log(&format!("Checking for embedded bundle in: {:?}", lib_path));

        if let Some(payload) = check_for_bundle(&lib_path) {
            android_log("Bundle found! Loading script...");

            if let Ok(sources) = serde_json::from_str::<HashMap<String, String>>(&payload) {
                let mut bundle_map = HashMap::new();
                for (k, v) in &sources {
                    if k != "__entry__" { bundle_map.insert(k.clone(), DgnValue::String(v.clone())); }
                }
                env_arc.lock().unwrap().store.insert("__bundled_sources__".to_string(), DgnValue::Map(Arc::new(Mutex::new(bundle_map))));

                if let Some(entry_code) = sources.get("__entry__") {
                    let tokens = lex(entry_code);
                    let mut parser = Parser::new(tokens);
                    if let Ok(ast) = parser.parse() {
                        let rt = tokio::runtime::Runtime::new().unwrap();

                        let mut chunk = Chunk::new();
                        Compiler::compile_stmt(&mut chunk, &ast);
                        chunk.emit(Instruction::Halt, 0);

                        if let Err(err) = rt.block_on(async {
                            let mut vm = Vm::new();
                            vm.run(&chunk, env_arc.clone()).await
                        }) {
                            android_log(&format!("DRAGON CRASH: {} at line {}", err.msg, err.line));
                        }
                    } else {
                        android_log("DRAGON PARSE ERROR: Failed to parse bundle.");
                    }
                    android_log("Embedded App Execution Finished.");
                }
            }
        }
    }

    *IS_INIT.lock().unwrap() = true;

    0
}

#[cfg(all(target_os = "android", feature = "legacy_android"))]
unsafe extern "C" fn native_run_script(mut env: JNIEnv, _: JClass, bundle_jstr: JString) -> jstring {
    let bundle_b64: String = env.get_string(&bundle_jstr).unwrap().into();

    use base64::{Engine as _, engine::general_purpose};
    let mut payload_bytes = match general_purpose::STANDARD.decode(&bundle_b64) {
        Ok(b) => b,
        Err(e) => return env.new_string(format!("Error: {}", e)).unwrap().into_raw(),
    };

    let key = engine::get_key();
    for (i, b) in payload_bytes.iter_mut().enumerate() { *b ^= key[i % key.len()]; }

    let json_str = String::from_utf8(payload_bytes).unwrap_or_default();
    let sources: HashMap<String, String> = serde_json::from_str(&json_str).unwrap_or_default();

    let env_lock = GLOBAL_ENV.clone();
    if let Some(entry) = sources.get("__entry__") {
        let tokens = lex(entry);
        let mut parser = Parser::new(tokens);
        if let Ok(ast) = parser.parse() {
             let rt = tokio::runtime::Runtime::new().unwrap();
             let mut chunk = Chunk::new();
             Compiler::compile_stmt(&mut chunk, &ast);
             chunk.emit(Instruction::Halt, 0);

             let _ = rt.block_on(async {
                 let mut vm = Vm::new();
                 vm.run(&chunk, env_lock.clone()).await
             });
        }
    }

    env.new_string("Executed").unwrap().into_raw()
}

#[cfg(all(target_os = "android", feature = "legacy_android"))]
unsafe extern "C" fn native_trigger_event(mut env: JNIEnv, _: JClass, evt_jstr: JString) {
    let evt: String = env.get_string(&evt_jstr).unwrap().into();
    let env_lock = GLOBAL_ENV.clone();
    let func = env_lock.lock().unwrap().get("on_event");
    if let Some(f) = func {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(call_dgn_func(f, vec![DgnValue::String(evt)]));
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn dragon_run_wasm(encoded_payload: &str) -> String {
    console_error_panic_hook::set_once();
    use base64::{Engine as _, engine::general_purpose};

    let mut payload_bytes = match general_purpose::STANDARD.decode(encoded_payload) {
        Ok(b) => b,
        Err(e) => return format!("Loader Error: Invalid Data Encoding: {}", e),
    };

    let key = engine::get_key();
    for (i, b) in payload_bytes.iter_mut().enumerate() {
        *b ^= key[i % key.len()];
    }

    let json_str = match String::from_utf8(payload_bytes) {
        Ok(s) => s,
        Err(e) => return format!("Loader Error: Decryption Failed: {}", e),
    };

    let sources: HashMap<String, String> = match serde_json::from_str(&json_str) {
        Ok(map) => map,
        Err(_) => {
            let mut m = HashMap::new();
            m.insert("__entry__".to_string(), json_str);
            m
        }
    };

    let env = Arc::new(Mutex::new(Env::new(None, None)));

    env.lock().unwrap().store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
    populate_builtins(env.clone());

    let mut bundle_map = HashMap::new();
    for (k, v) in &sources {
        if k != "__entry__" {
            bundle_map.insert(k.clone(), DgnValue::String(v.clone()));
        }
    }
    env.lock().unwrap().store.insert("__bundled_sources__".to_string(), DgnValue::Map(Arc::new(Mutex::new(bundle_map))));

    if let Some(entry_code) = sources.get("__entry__") {
        let tokens = lex(entry_code);
        let mut parser = Parser::new(tokens);

        match parser.parse() {
            Ok(ast) => {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let mut chunk = Chunk::new();
                Compiler::compile_stmt(&mut chunk, &ast);
                chunk.emit(Instruction::Halt, 0);

                match rt.block_on(async {
                    let mut vm = Vm::new();
                    vm.run(&chunk, env).await
                }) {
                    Ok(_) => "Program Finished".to_string(),
                    Err(e) => format!("Runtime Error: {} at line {}", e.msg, e.line)
                }
            },
            Err(e) => format!("Parse Error: {}", e)
        }
    } else {
        "Error: No entry point found in bundle.".to_string()
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn dragon_run_source(source_code: &str) -> String {
    console_error_panic_hook::set_once();

    let output_buf: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let env = Arc::new(Mutex::new(Env::new(None, None)));
    env.lock().unwrap().store.insert(
        "__modules__".into(),
        DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))),
    );
    populate_builtins(env.clone());

    let buf_clone = output_buf.clone();
    env.lock().unwrap().set("showln", native_fn(move |args| {
        let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
        buf_clone.lock().unwrap().push(s);
        Ok(DgnValue::Null)
    })).unwrap();

    let buf_clone2 = output_buf.clone();
    env.lock().unwrap().set("show", native_fn(move |args| {
        let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
        buf_clone2.lock().unwrap().push(s);
        Ok(DgnValue::Null)
    })).unwrap();

    let tokens = match lex(source_code) {
        Ok(t) => t,
        Err(e) => return format!("Syntax Error: {}", e.with_source(source_code).format_pretty()),
    };

    let mut parser = Parser::new(tokens);
    match parser.parse() {
        Ok(ast) => {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let mut chunk = Chunk::new();
            Compiler::compile_stmt(&mut chunk, &ast);
            chunk.emit(Instruction::Halt, 0);

            let result = rt.block_on(async {
                let mut vm = Vm::new();
                vm.run(&chunk, env).await
            });

            let captured = output_buf.lock().unwrap().join("\n");
            match result {
                Ok(_) => {
                    if captured.is_empty() {
                        "(no output)".to_string()
                    } else {
                        captured
                    }
                }
                Err(e) => {
                    let err = format!("Runtime Error: {} at line {}", e.msg, e.line);
                    if captured.is_empty() {
                        err
                    } else {
                        format!("{}\n{}", captured, err)
                    }
                }
            }
        }
        Err(e) => format!("Parse Error: {}", e),
    }
}

fn get_dll_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    unsafe {
        use std::ptr;
        extern "system" {
            fn GetModuleHandleExA(f: u32, n: *const u8, h: *mut *mut std::ffi::c_void) -> i32;
            fn GetModuleFileNameA(h: *mut std::ffi::c_void, b: *mut u8, s: u32) -> u32;
        }
        let mut handle = ptr::null_mut();

        if GetModuleHandleExA(4, Dragon_Init as *const u8, &mut handle) != 0 {
            let mut buf = [0u8; 1024];
            let len = GetModuleFileNameA(handle, buf.as_mut_ptr(), 1024);
            return Some(PathBuf::from(String::from_utf8_lossy(&buf[..len as usize]).to_string()));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unsafe {
        let mut info: Dl_info = std::mem::zeroed();
        if dladdr(Dragon_Init as *const std::ffi::c_void, &mut info) != 0 {
            if !info.dli_fname.is_null() {
                let path_cstr = CStr::from_ptr(info.dli_fname);
                return Some(PathBuf::from(path_cstr.to_string_lossy().into_owned()));
            }
        }
    }
    None
}

fn dgn_to_json_str(v: &DgnValue) -> String {
    match v {
        DgnValue::Null => "null".to_string(),
        DgnValue::Bool(b) => if *b { "true".to_string() } else { "false".to_string() },
        DgnValue::Number(n) => n.to_string(),
        DgnValue::String(s) => format!("\"{}\"", s.replace("\"", "\\\"")),
        DgnValue::List(l) => {
            let items: Vec<String> = l.lock().unwrap().iter().map(dgn_to_json_str).collect();
            format!("[{}]", items.join(","))
        },
        DgnValue::Map(m) | DgnValue::Instance(m) => {
            let items: Vec<String> = m.lock().unwrap().iter()
                .map(|(k, v)| format!("\"{}\":{}", k, dgn_to_json_str(v))).collect();
            format!("{{{}}}", items.join(","))
        },
        _ => format!("\"{}\"", val_to_string(v).replace("\"", "\\\""))
    }
}

#[no_mangle]
pub extern "C" fn Dragon_Init() -> c_int {
    let path = match get_dll_path() {
        Some(p) => p,
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };

    let env = GLOBAL_ENV.clone();

    {
        let mut e = env.lock().unwrap();
        e.store.clear();
        e.store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
    }
    populate_builtins(env.clone());

    let mut payload = check_for_bundle(&path);

    if payload.is_none() {
        let fallback_path = path.parent().unwrap_or(&path).join("mylib.dgn");
        if let Ok(content) = std::fs::read_to_string(&fallback_path) {
            payload = Some(content);
        } else if let Ok(content) = std::fs::read_to_string("mylib.dgn") {
            payload = Some(content);
        } else {
            return -2;
        }
    }

    let payload_str = payload.unwrap();

    let sources: HashMap<String, String> = serde_json::from_str(&payload_str).unwrap_or_else(|_| {
        let mut m = HashMap::new();
        m.insert("__entry__".to_string(), payload_str);
        m
    });

    let mut bundle_map = HashMap::new();
    for (k, v) in &sources {
        if k != "__entry__" { bundle_map.insert(k.clone(), DgnValue::String(v.clone())); }
    }
    env.lock().unwrap().store.insert("__bundled_sources__".to_string(), DgnValue::Map(Arc::new(Mutex::new(bundle_map))));

    if let Some(entry_code) = sources.get("__entry__") {
        let tokens = match lex(entry_code) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Syntax Error: {}", e.with_source(entry_code).format_pretty());
                return -1;
            }
        };
        let mut parser = Parser::new(tokens);
        if let Ok(ast) = parser.parse() {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let mut chunk = Chunk::new();
            Compiler::compile_stmt(&mut chunk, &ast);
            chunk.emit(Instruction::Halt, 0);

            if rt.block_on(async {
                let mut vm = Vm::new();
                vm.run(&chunk, env.clone()).await
            }).is_err() {
                return -3;
            }
        } else { return -4; }
    }

    *IS_INIT.lock().unwrap() = true;
    0
}

#[no_mangle]
pub extern "C" fn Dragon_Call(func_name: *const c_char, json_args: *const c_char) -> *mut c_char {
    if func_name.is_null() {
        return CString::new("Error: func_name is a null pointer").unwrap().into_raw();
    }

    if !*IS_INIT.lock().unwrap() {
        return CString::new("Error: Not Initialized (Dragon_Init failed)").unwrap().into_raw();
    }

    let name = unsafe { CStr::from_ptr(func_name).to_str().unwrap_or("") };

    let args_str = if json_args.is_null() {
        "[]"
    } else {
        unsafe { CStr::from_ptr(json_args).to_str().unwrap_or("[]") }
    };

    let json_vals: Vec<serde_json::Value> = serde_json::from_str(args_str).unwrap_or_default();

    let mut dgn_args = Vec::new();
    for j in json_vals {
        match j {
            serde_json::Value::Null       => dgn_args.push(DgnValue::Null),
            serde_json::Value::Bool(b)    => dgn_args.push(DgnValue::Bool(b)),
            serde_json::Value::Number(n)  => dgn_args.push(DgnValue::Number(n.as_f64().unwrap_or(0.0))),
            serde_json::Value::String(s)  => dgn_args.push(DgnValue::String(s)),
            serde_json::Value::Array(arr) => {
                let items: Vec<DgnValue> = arr.into_iter().map(|v| match v {
                    serde_json::Value::String(s) => DgnValue::String(s),
                    serde_json::Value::Number(n) => DgnValue::Number(n.as_f64().unwrap_or(0.0)),
                    serde_json::Value::Bool(b)   => DgnValue::Bool(b),
                    serde_json::Value::Null      => DgnValue::Null,
                    other => DgnValue::String(other.to_string()),
                }).collect();
                dgn_args.push(DgnValue::List(Arc::new(Mutex::new(items))));
            },
            serde_json::Value::Object(obj) => {
                let mut map = std::collections::HashMap::new();
                for (k, v) in obj {
                    let dv = match v {
                        serde_json::Value::String(s) => DgnValue::String(s),
                        serde_json::Value::Number(n) => DgnValue::Number(n.as_f64().unwrap_or(0.0)),
                        serde_json::Value::Bool(b)   => DgnValue::Bool(b),
                        serde_json::Value::Null      => DgnValue::Null,
                        other => DgnValue::String(other.to_string()),
                    };
                    map.insert(k, dv);
                }
                dgn_args.push(DgnValue::Map(Arc::new(Mutex::new(map))));
            },
        }
    }

    let env = GLOBAL_ENV.clone();
    let func = env.lock().unwrap().get(name);

    if let Some(f) = func {
       let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(call_dgn_func(f, dgn_args)) {
            Ok(res) => CString::new(dgn_to_json_str(&res)).unwrap().into_raw(),
            Err(e) => CString::new(format!("{{\"error\": \"{}\"}}", e.msg)).unwrap().into_raw(),
        }
    } else {
        CString::new(format!("{{\"error\": \"Function '{}' not found\"}}", name)).unwrap().into_raw()
    }
}
