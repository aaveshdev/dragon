use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

mod engine;
use engine::*;
use std::panic;

#[cfg(unix)]
use libc::{Dl_info, dladdr};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_os = "android")]
use jni::sys::jstring;



#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref GLOBAL_ENV: Arc<Mutex<Env>> = Arc::new(Mutex::new(Env::new(None, None)));
    static ref IS_INIT: Mutex<bool> = Mutex::new(false);
}


#[cfg(target_os = "android")]
use jni::{JNIEnv, JavaVM, sys::{jint, JNI_VERSION_1_6}};
#[cfg(target_os = "android")]
use jni::objects::{JClass, JObject, JString, JValue, GlobalRef};

#[cfg(target_os = "android")]
use std::os::raw::c_void; //

#[cfg(target_os = "android")]
static mut ANDROID_CALLBACK: Option<GlobalRef> = None;
#[cfg(target_os = "android")]
static mut JAVA_VM: Option<JavaVM> = None;


#[cfg(target_os = "android")]
fn android_log(msg: &str) {
    use std::ffi::CString;
    use std::os::raw::{c_int, c_char};

    // Manually declaration of the Android logging function
    #[link(name = "log")]
    extern "C" {
        fn __android_log_print(prio: c_int, tag: *const c_char, fmt: *const c_char, ...) -> c_int;
    }

    // Constants from android/log.h
    const ANDROID_LOG_INFO: c_int = 4;
    // const ANDROID_LOG_ERROR: c_int = 6; // Uncomment if needed

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

#[cfg(target_os = "android")]
fn get_android_lib_path() -> Option<PathBuf> {
    let mut info: Dl_info = unsafe { std::mem::zeroed() };
    
    // Use the address of JNI_OnLoad to find the library containing it
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

// 1. Dynamic JNI Registration (Magic happens here)
// [Inside lib.rs, replace the JNI_OnLoad function with this]

#[cfg(target_os = "android")]
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

#[cfg(target_os = "android")]
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

    let mut e = env_arc.lock().unwrap();
    
    // Override I/O
    e.set("showln", native_fn(|args| {
        let s = args.iter().map(val_to_string).collect::<Vec<_>>().join(" ");
        android_log(&s);
        Ok(DgnValue::Null)
    })).unwrap();


    // Render UI Logic
    e.set("render_ui", native_fn(|args| {
        let json_ui = args.get(0).map(val_to_string).unwrap_or_default();
        unsafe {
            if let Some(vm) = &JAVA_VM {
                if let Ok(mut env) = vm.attach_current_thread() {
                    if let Some(callback_ref) = &ANDROID_CALLBACK {
                        if let Ok(jmsg) = env.new_string(json_ui) {
                            let _ = env.call_method(callback_ref, "onUpdateUI", "(Ljava/lang/String;)V", &[JValue::Object(&jmsg.into())]);
                        }
                    }
                }
            }
        }
        Ok(DgnValue::Null)
    })).unwrap();

    // --- AUTO-BOOTSTRAP (ZERO-ASSET MODE) ---
    // Look for appended bundle in our own .so file
    if let Some(lib_path) = get_android_lib_path() {
        android_log(&format!("Checking for embedded bundle in: {:?}", lib_path));
        
        // check_for_bundle logic handles reading the footer signature
        if let Some(payload) = check_for_bundle(&lib_path) {
            android_log("Bundle found! Loading...");
            
            // Parse JSON Bundle
            if let Ok(sources) = serde_json::from_str::<HashMap<String, String>>(&payload) {
                let mut bundle_map = HashMap::new();
                for (k, v) in &sources {
                    if k != "__entry__" { bundle_map.insert(k.clone(), DgnValue::String(v.clone())); }
                }
                e.store.insert("__bundled_sources__".to_string(), DgnValue::Map(Arc::new(Mutex::new(bundle_map))));

                // Run Entry Point
                if let Some(entry_code) = sources.get("__entry__") {
                    drop(e); // Release lock before execution
                    let tokens = lex(entry_code);
                    let mut parser = Parser::new(tokens);
                    if let Ok(ast) = parser.parse() {
                        if let StmtNode::Block(stmts) = ast.node {
                            for s in stmts { eval_stmt(&s, env_arc.clone()).ok(); }
                        }
                    }
                    android_log("Embedded App Executed Successfully.");
                }
            }
        } else {
            android_log("No embedded bundle found. Waiting for external script.");
        }
    }

    *IS_INIT.lock().unwrap() = true;
}

#[cfg(target_os = "android")]
unsafe extern "C" fn native_run_script(mut env: JNIEnv, _: JClass, bundle_jstr: JString) -> jstring {
    // Keep this for manual testing or fallback
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
    // ... load and run logic similar to init ...
    if let Some(entry) = sources.get("__entry__") {
        let tokens = lex(entry);
        let mut parser = Parser::new(tokens);
        if let Ok(ast) = parser.parse() {
             if let StmtNode::Block(stmts) = ast.node {
                for s in stmts { eval_stmt(&s, env_lock.clone()).ok(); }
             }
        }
    }

    env.new_string("Executed").unwrap().into_raw()
}

#[cfg(target_os = "android")]
unsafe extern "C" fn native_trigger_event(mut env: JNIEnv, _: JClass, evt_jstr: JString) {
    let evt: String = env.get_string(&evt_jstr).unwrap().into();
    let env_lock = GLOBAL_ENV.clone();
    let func = env_lock.lock().unwrap().get("on_event");
    if let Some(f) = func {
        call_dgn_func(f, vec![DgnValue::String(evt)]).ok();
    }
}
// In lib.rs

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn dragon_run_wasm(encoded_payload: &str) -> String {
    console_error_panic_hook::set_once();
    use base64::{Engine as _, engine::general_purpose};

    // 1. Decode Base64 to get encrypted bytes
    let mut payload_bytes = match general_purpose::STANDARD.decode(encoded_payload) {
        Ok(b) => b,
        Err(e) => return format!("Loader Error: Invalid Data Encoding: {}", e),
    };

    // 2. Decrypt using the shared key (XOR)
    let key = engine::get_key(); 
    for (i, b) in payload_bytes.iter_mut().enumerate() {
        *b ^= key[i % key.len()];
    }

    // 3. Convert decrypted bytes to String (JSON Bundle)
    let json_str = match String::from_utf8(payload_bytes) {
        Ok(s) => s,
        Err(e) => return format!("Loader Error: Decryption Failed: {}", e),
    };

    // 4. Parse the Bundle (Single source or JSON map)
    let sources: HashMap<String, String> = match serde_json::from_str(&json_str) {
        Ok(map) => map,
        Err(_) => {
            // Fallback if it was just a single raw file
            let mut m = HashMap::new(); 
            m.insert("__entry__".to_string(), json_str); 
            m
        }
    };

    // 5. Initialize Environment
    let env = Arc::new(Mutex::new(Env::new(None, None)));
    
    // Setup Module System
    env.lock().unwrap().store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
    populate_builtins(env.clone());

    // Load all bundled files into memory so imports work
    let mut bundle_map = HashMap::new();
    for (k, v) in &sources {
        if k != "__entry__" { 
            bundle_map.insert(k.clone(), DgnValue::String(v.clone())); 
        }
    }
    env.lock().unwrap().store.insert("__bundled_sources__".to_string(), DgnValue::Map(Arc::new(Mutex::new(bundle_map))));

    // 6. Execute Entry Point
    if let Some(entry_code) = sources.get("__entry__") {
        let tokens = lex(entry_code);
        let mut parser = Parser::new(tokens);
        
        match parser.parse() {
            Ok(ast) => {
                match eval_stmt(&ast, env) {
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

    // 1. Try to load from an embedded bundle
    let mut payload = check_for_bundle(&path);

    // 2. FALLBACK: Try to load 'mylib.dgn' from the disk if not bundled
    if payload.is_none() {
        let fallback_path = path.parent().unwrap_or(&path).join("mylib.dgn");
        if let Ok(content) = std::fs::read_to_string(&fallback_path) {
            payload = Some(content);
        } else if let Ok(content) = std::fs::read_to_string("mylib.dgn") {
            // Try current working directory as a last resort
            payload = Some(content);
        } else {
            return -2; // Initialization failed: No bundle and no mylib.dgn found
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
        let tokens = lex(entry_code);
        let mut parser = Parser::new(tokens);
        if let Ok(ast) = parser.parse() {

            if let StmtNode::Block(stmts) = ast.node {
                for s in stmts {
                    if eval_stmt(&s, env.clone()).is_err() { return -3; }
                }
            } else {
                if eval_stmt(&ast, env.clone()).is_err() { return -3; }
            }
        } else { return -4; }
    }

    *IS_INIT.lock().unwrap() = true;
    0
}


#[no_mangle]
pub extern "C" fn Dragon_Call(func_name: *const c_char, json_args: *const c_char) -> *mut c_char {
    // 1. Null Pointer Protection (Prevents panics that cause 'null' returns)
    if func_name.is_null() {
        return CString::new("Error: func_name is a null pointer").unwrap().into_raw();
    }
    
    // 2. Initialization Check
    if !*IS_INIT.lock().unwrap() { 
        return CString::new("Error: Not Initialized (Dragon_Init failed)").unwrap().into_raw(); 
    }

    let name = unsafe { CStr::from_ptr(func_name).to_str().unwrap_or("") };
    
    // Allow empty arguments by defaulting to "[]" if null is passed
    let args_str = if json_args.is_null() {
        "[]"
    } else {
        unsafe { CStr::from_ptr(json_args).to_str().unwrap_or("[]") }
    };

    let json_vals: Vec<serde_json::Value> = serde_json::from_str(args_str).unwrap_or_default();

    let mut dgn_args = Vec::new();
    for j in json_vals {
        match j {
            serde_json::Value::Number(n) => dgn_args.push(DgnValue::Number(n.as_f64().unwrap_or(0.0))),
            serde_json::Value::Bool(b) => dgn_args.push(DgnValue::Bool(b)),
            _ => dgn_args.push(DgnValue::String(j.to_string().replace("\"", ""))),
        }
    }

    let env = GLOBAL_ENV.clone();
    let func = env.lock().unwrap().get(name);

if let Some(f) = func {
        match call_dgn_func(f, dgn_args) {
            Ok(res) => CString::new(dgn_to_json_str(&res)).unwrap().into_raw(),
            Err(e) => CString::new(format!("{{\"error\": \"{}\"}}", e.msg)).unwrap().into_raw(),
        }
    } else {
        CString::new(format!("{{\"error\": \"Function '{}' not found\"}}", name)).unwrap().into_raw()
    }
}