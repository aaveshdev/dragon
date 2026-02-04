use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::path::PathBuf;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};

mod engine;
use engine::*;

#[cfg(unix)]
use libc::{Dl_info, dladdr};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref GLOBAL_ENV: Arc<Mutex<Env>> = Arc::new(Mutex::new(Env::new(None, None)));
    static ref IS_INIT: Mutex<bool> = Mutex::new(false);
}


#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn dragon_run_wasm(source_code: &str) -> String {
    console_error_panic_hook::set_once();

    let env = Arc::new(Mutex::new(Env::new(None, None)));
    env.lock().unwrap().store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
    populate_builtins(env.clone());
    let tokens = lex(source_code);
    let mut parser = Parser::new(tokens);
    
    match parser.parse() {
        Ok(ast) => {
            match eval_stmt(&ast, env) {
                Ok(_) => "Success".to_string(),
                Err(e) => format!("Runtime Error: {} at line {}", e.msg, e.line)
            }
        },
        Err(e) => format!("Parse Error: {}", e)
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


#[no_mangle]
pub extern "C" fn Dragon_Init() -> c_int {
    let path = match get_dll_path() { Some(p) => p, None => return -1 };
    

    let payload = match check_for_bundle(&path) {
        Some(s) => s,
        None => return -2 
    };

    let env = GLOBAL_ENV.clone();
    

    {
        let mut e = env.lock().unwrap();
        e.store.clear();
        e.store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
    }
    populate_builtins(env.clone());


    let sources: HashMap<String, String> = serde_json::from_str(&payload).unwrap_or_else(|_| {
        let mut m = HashMap::new(); m.insert("__entry__".to_string(), payload); m
    });

    let mut bundle_map = HashMap::new();
    for (k, v) in &sources {
        if k != "__entry__" { bundle_map.insert(k.clone(), DgnValue::String(v.clone())); }
    }
    env.lock().unwrap().store.insert("__bundled_sources__".to_string(), DgnValue::Map(Arc::new(Mutex::new(bundle_map))));


    let entry_code = sources.get("__entry__").unwrap();
    let tokens = lex(entry_code);
    let mut parser = Parser::new(tokens);
    if let Ok(ast) = parser.parse() {
        if eval_stmt(&ast, env).is_err() { return -3; }
    } else { return -4; }

    *IS_INIT.lock().unwrap() = true;
    0
}


#[no_mangle]
pub extern "C" fn Dragon_Call(func_name: *const c_char, json_args: *const c_char) -> *mut c_char {
    if !*IS_INIT.lock().unwrap() { return CString::new("Error: Not Initialized").unwrap().into_raw(); }

    let name = unsafe { CStr::from_ptr(func_name).to_str().unwrap_or("") };
    let args_str = unsafe { CStr::from_ptr(json_args).to_str().unwrap_or("[]") };
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
            Ok(res) => CString::new(val_to_string(&res)).unwrap().into_raw(),
            Err(e) => CString::new(format!("Error: {}", e.msg)).unwrap().into_raw(),
        }
    } else {
        CString::new("Error: Function not found").unwrap().into_raw()
    }
}