mod engine;
use engine::*;

use std::sync::{Arc, Mutex};
use std::collections::{HashMap};
use std::fs::{self, File,OpenOptions};
use std::path::{Path, PathBuf};
use std::io::{Write, Read, Seek, SeekFrom};
use std::env;
use std::panic;

const DGN_VERSION: &str = "Dragon Programming Language v1.0.3";
const DGN_BUILD: &str = "Build 2026-02-22"; 
const DGN_AUTHOR: &str = "Aavesh Jilani";
fn main() {
    panic::set_hook(Box::new(|info| {
        println!("--------------------------------------------------");
        println!("  DRAGON CRITICAL ERROR");
        println!("--------------------------------------------------");
        if let Some(s) = info.payload().downcast_ref::<&str>() {
            println!("  Reason: {}", s);
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            println!("  Reason: {}", s);
        } else {
            println!("  An unhandled internal state error occurred.");
        }
        println!("--------------------------------------------------");
        std::process::exit(1);
    }));

    let args: Vec<String> = env::args().collect();
    let exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("dragon"));
    if let Some(payload) = check_for_bundle(&exe_path) {
        let env = Arc::new(Mutex::new(Env::new(None, Some(exe_path.clone()))));
        env.lock().unwrap().store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
        populate_builtins(env.clone());
        
        let sources: HashMap<String, String> = serde_json::from_str(&payload).unwrap_or_else(|_| {
            let mut m = HashMap::new(); m.insert("__entry__".to_string(), payload); m
        });
        
        let mut bundle_map = HashMap::new();
        for (k, v) in &sources { if k != "__entry__" { bundle_map.insert(k.clone(), DgnValue::String(v.clone())); } }
        env.lock().unwrap().store.insert("__bundled_sources__".to_string(), DgnValue::Map(Arc::new(Mutex::new(bundle_map))));
        let mut argv_list = Vec::new();
        for a in args.iter().skip(1) { argv_list.push(DgnValue::String(a.clone())); }
        let _ = env.lock().unwrap().set("argv", DgnValue::List(Arc::new(Mutex::new(argv_list))));

        let entry = sources.get("__entry__").unwrap();
        let tokens = lex(entry);
        let mut parser = Parser::new(tokens);
        match parser.parse() {
            Ok(ast) => { 
                let rt = tokio::runtime::Runtime::new().unwrap();
                if let Err(e) = rt.block_on(eval_stmt(&ast, env)) { 
                    eprintln!("Runtime Error: {} at line {}", e.msg, e.line); 
                    std::process::exit(1); 
                } 
            },   Err(e) => { eprintln!("Bundle Parse Error: {}", e); std::process::exit(1); }
        }
        return;
    }

    if args.len() < 2 {
        println!("\n==============================================");
        println!("          Dragon Programming Language");
        println!("==============================================");
        println!("Version:   {}", DGN_VERSION);
        println!("Build:     {}", DGN_BUILD);
        println!("Author:    {}", DGN_AUTHOR);
        println!("----------------------------------------------");
        println!("Platform:  {}", std::env::consts::OS);
        println!("Arch:      {}", std::env::consts::ARCH);
        println!("==============================================");
        println!("USAGE:");
        println!("  dragon <file.dgn> [args...]       Run a script file");
        println!("  dragon --repl                     Start the interactive shell");
        println!("  dragon -c <in> -o <out>           Compile script to Executable");
        println!("  dragon -c <in> -o <out> --window  Compile script to run in background (No Console)");
        println!("  dragon -c <in> -o <out> --lib     Compile script to Shared Library");
        println!("  dragon -c <in> -o <out> --web     Compile script to Web Bundle");
        println!("  dragon -v, --version              Show version information");
        println!("");
        return;
    }

    let command = &args[1];

    if command == "-v" || command == "--version" {
        println!("{}", DGN_VERSION);
        println!("{} by {}", DGN_BUILD, DGN_AUTHOR);
        return;
    }

    if command == "-build" {
       println!("Dragon Build System");
        println!("-------------------------------------");

        let input_path = Path::new("lib/main.dgn");
        
        // FIX: Target 'arm64-v8a' (Standard for Phones) inside the 'android' project folder
        let jni_dir = Path::new("app/src/main/jniLibs/x86_64");
        let output_so = jni_dir.join("libdragon_android.so");

        if !input_path.exists() {
            println!("Error: 'lib/main.dgn' not found.");
            return;
        }

        // FIX: Look for template in CURRENT DIRECTORY (where user runs command)
        let template_path = Path::new("libdragon_android.so");
        
        if !template_path.exists() {
            println!("Error: Template 'libdragon_android.so' not found in current directory.");
            println!("Please ensure the pre-compiled Android library is in: {:?}", env::current_dir().unwrap());
            return;
        }

        if !jni_dir.exists() {
            println!("Creating JNI directory: {:?}", jni_dir);
            fs::create_dir_all(jni_dir).expect("Failed to create JNI dir");
        }

        println!("Compiling Script: {}", input_path.display());

        // 2. Gather & Encrypt Sources
        let mut sources = HashMap::new();
        if let Err(e) = gather_sources(input_path, &mut sources, true) {
            eprintln!("Build Failed: {}", e);
            std::process::exit(1);
        }

        let payload = serde_json::to_string(&sources).expect("Serialization failed");
        let mut payload_bytes = payload.into_bytes();
        let payload_len = payload_bytes.len() as u64;

        let key = get_key();
        for (i, b) in payload_bytes.iter_mut().enumerate() {
            *b ^= key[i % key.len()];
        }

        // 3. Copy Template -> Destination
        println!("Installing Engine to: {:?}", output_so);
        fs::copy(&template_path, &output_so).expect("Failed to copy library template");

        // 4. Inject Code into the Binary (Append Mode)
        println!("Injecting Encrypted Bytecode...");
        
        let mut file = OpenOptions::new()
            .write(true)
            .append(true)
            .open(&output_so)
            .expect("Failed to open .so for writing");
        
        // Write Data
        file.write_all(&payload_bytes).expect("Write failed");
        // Write Length (8 bytes)
        file.write_all(&payload_len.to_le_bytes()).expect("Write len failed");
        // Write Signature (16 bytes)
        file.write_all(BUNDLE_SIGNATURE).expect("Write sig failed");

        println!("Success! App compiled and embedded into native library.");
        println!("Ready to run on Android.");
        return;
    }

    if command == "-c" {
        if args.len() < 5 || args[3] != "-o" {
            println!("Error: Invalid compilation arguments.");
            println!("Usage: dragon -c <input.dgn> -o <output.exe> [--lib]");
            return;
        }

        let input_path = Path::new(&args[2]);
        let output_path = Path::new(&args[4]);
        let is_lib_mode = args.contains(&"--lib".to_string());
        let is_web_mode = args.contains(&"--web".to_string());
        let is_window_mode = args.contains(&"--window".to_string());
        println!("Compiling {}...", input_path.display());


       let mut sources = HashMap::new();
        
        if let Err(e) = gather_sources(input_path, &mut sources, true) {
            eprintln!("Compilation Error: {}", e);
            std::process::exit(1);
        }
        
       let payload = serde_json::to_string(&sources).expect("Failed to serialize bundle");
        
        let mut payload_bytes = payload.into_bytes();
        let payload_len = payload_bytes.len() as u64;

        let key = get_key();
        println!("Encrypting {} bytes...", payload_len);
        for (i, b) in payload_bytes.iter_mut().enumerate() {
            *b ^= key[i % key.len()];
        }

        if is_lib_mode {
            let mut template_dir = exe_path.parent().unwrap().to_path_buf();

            let out_name = output_path.file_name().unwrap().to_string_lossy();
            
            let template_name = if out_name.ends_with(".so") {
                "libdragon_lib.so"
            } else if out_name.ends_with(".dylib") {
                "libdragon_lib.dylib"
            } else {
                "dragon_lib.dll" 
            };

            let template = template_dir.join(template_name);

            if !template.exists() {
                println!("Error: Library template '{}' not found.", template_name);
                println!("Copy the platform-specific template to: {:?}", template_dir);
                return;
            }
            fs::copy(&template, output_path).expect("Copy failed");
            println!("Created Shared Library: {:?}", output_path);
        } 
      else if is_web_mode {
        println!("Compiling for Web Assembly (Protected Mode)...");

        let template_dir = exe_path.parent().unwrap();
        let wasm_file = template_dir.join("dragon_lib_bg.wasm");
        let js_file = template_dir.join("dragon_lib.js");

        if !wasm_file.exists() || !js_file.exists() {
             println!("Error: WASM templates not found.");
             println!("Please build 'dragon_template' target wasm32-unknown-unknown and place .wasm and .js here.");
             return;
        }

        let out_dir = output_path.with_extension(""); 
        fs::create_dir_all(&out_dir).expect("Failed to create output dir");

        fs::copy(&wasm_file, out_dir.join("dragon_lib_bg.wasm")).unwrap();
        fs::copy(&js_file, out_dir.join("dragon.js")).unwrap();

        // 1. Gather Sources
        let mut sources = HashMap::new();
        if let Err(e) = gather_sources(input_path, &mut sources, true) {
             eprintln!("Error gathering sources: {}", e);
             return;
        }

        // 2. Serialize to JSON
        let payload_json = serde_json::to_string(&sources).expect("Failed to serialize");
        let mut payload_bytes = payload_json.into_bytes();

        // 3. Encrypt (XOR) - Same logic as Native
        let key = get_key();
        for (i, b) in payload_bytes.iter_mut().enumerate() {
            *b ^= key[i % key.len()];
        }

        // 4. Encode to Base64 (Safe for embedding in HTML string)
        use base64::{Engine as _, engine::general_purpose};
        let protected_payload = general_purpose::STANDARD.encode(&payload_bytes);

        // 5. Generate HTML with Protected Payload
        // Note: We now import `dragon_run_encrypted` instead of `dragon_run_wasm`
        let html_content = format!(r#"
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Dragon App</title>
    <style>
        body {{ font-family: monospace; background: #222; color: #eee; padding: 20px; }}
        #console {{ white-space: pre-wrap; }}
    </style>
</head>
<body>
    <h1>Dragon Output</h1>
    <div id="console">Initializing System...</div>
    <script type="module">
        import init, {{ dragon_run_wasm }} from './dragon.js';

        async function run() {{
            await init();
            const outputDiv = document.getElementById("console");
            
            const oldLog = console.log;
            console.log = (msg) => {{
                outputDiv.innerText += msg + "\n";
                oldLog(msg);
            }};

            const payload = "{}";

            outputDiv.innerText = "> System Ready. Executing...\n";
            try {{
                const result = dragon_run_wasm(payload);
                outputDiv.innerText += "\n[" + result + "]";
            }} catch(e) {{
                outputDiv.innerText += "\n[WASM Error: " + e + "]";
            }}
        }}
        run();
    </script>
</body>
</html>
"#, protected_payload);

        fs::write(out_dir.join("index.html"), html_content).unwrap();
        println!("Success! Protected Web bundle created at: {:?}", out_dir);
        return;
    }
        else {
            fs::copy(&exe_path, output_path).expect("Copy failed");
            println!("Created Executable: {:?}", output_path);

            if is_window_mode {
                if let Err(e) = hide_windows_console(output_path) {
                    println!("Warning: Could not set background execution mode: {}", e);
                } else {
                    println!("Enabled Window Mode (Background Execution).");
                }
            }
        }

        let mut outfile = fs::OpenOptions::new().append(true).open(output_path).unwrap();
        outfile.write_all(&payload_bytes).unwrap();
        outfile.write_all(&payload_len.to_le_bytes()).unwrap();
        outfile.write_all(BUNDLE_SIGNATURE).unwrap();
        
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(metadata) = fs::metadata(output_path) {
                let mut perms = metadata.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(output_path, perms).unwrap();
            }
        }

        println!("Success! Bundled {} files.", sources.len());
        return;
    }

    if command == "--repl" {
        println!("Dragon v1.0.0");
        println!("Type 'exit' to quit.");
        
        let env = Arc::new(Mutex::new(Env::new(None, None)));
        env.lock().unwrap().store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
        populate_builtins(env.clone());
        
        loop {
            print!("> ");
            std::io::stdout().flush().unwrap();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).unwrap() == 0 { break; }
            if line.trim() == "exit" { break; }
            
            let tokens = lex(&line);
            let mut parser = Parser::new(tokens);
            if let Ok(ast) = parser.parse_stmt() {
                let rt = tokio::runtime::Runtime::new().unwrap();
                match rt.block_on(eval_stmt(&ast, env.clone())) {
                    Ok(EvalResult::Val(v)) | Ok(EvalResult::Return(v)) => println!("{}", val_to_string(&v)),
                    Err(e) => println!("Error: {} at line {}", e.msg, e.line),
                    _ => {}
                }
            } else {
                println!("Syntax Error");
            }
        }
        return;
    }

    let mut filename = command.to_string();

    if command == "run" {
        let pkg_path = Path::new("package.json");
        if !pkg_path.exists() {
            eprintln!("Error: 'package.json' not found. Are you inside a Dragon project?");
            std::process::exit(1);
        }

        let pkg_data = fs::read_to_string(pkg_path).unwrap_or_else(|_| {
            eprintln!("Error: Could not read 'package.json'.");
            std::process::exit(1);
        });

        let pkg_json: serde_json::Value = serde_json::from_str(&pkg_data).unwrap_or_else(|_| {
            eprintln!("Error: 'package.json' contains invalid JSON.");
            std::process::exit(1);
        });

        if let Some(entry) = pkg_json.get("main").or_else(|| pkg_json.get("entry")).and_then(|v| v.as_str()) {
            filename = entry.to_string();
        } else {
            eprintln!("Error: 'package.json' must contain a \"main\" field (e.g., \"main\": \"src/main.dgn\").");
            std::process::exit(1);
        }
    }

    match fs::read_to_string(&filename) {
        Ok(src) => {
            let abs_path = fs::canonicalize(&filename).unwrap_or(PathBuf::from(filename));
            let env = Arc::new(Mutex::new(Env::new(None, Some(abs_path))));
            env.lock().unwrap().store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
            populate_builtins(env.clone());
            
            let mut argv_list = Vec::new();
            for a in args.iter().skip(2) { argv_list.push(DgnValue::String(a.clone())); }
            let _ = env.lock().unwrap().set("argv", DgnValue::List(Arc::new(Mutex::new(argv_list))));

            let tokens = lex(&src);
            let mut parser = Parser::new(tokens);
            match parser.parse() {
                Ok(ast) => {
                    let report = lint_ast(&ast);
                    if !report.is_clean() {
                        report.print();
                        if !report.errors.is_empty() {
                            std::process::exit(1);
                        }
                    }


                    let rt = tokio::runtime::Runtime::new().unwrap();
                    if let Err(e) = rt.block_on(eval_stmt(&ast, env)) {
                        eprintln!("Runtime Error: {} at line {}", e.msg, e.line);
                        std::process::exit(1);
                    }
                },
                Err(e) => { eprintln!("Parse Error: {}", e); std::process::exit(1); },
            }
        },
        Err(e) => {
            eprintln!("Could not read file '{}': {}", filename, e);
            std::process::exit(1);
        },
    }
}

fn get_key() -> Vec<u8> {

    let mut key = Vec::new();

    key.push(68);  
    key.push(82);  
    key.push(65);  
    key.push(71); 
    key.push(79); 
    key.push(78); 
    key.push(95); 
    key.push(50);  
    key.push(48);  
    key.push(50);  
    key.push(54); 
    
    key.push(10+20);

    key
}

fn hide_windows_console(path: &Path) -> std::io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut dos_header = [0u8; 64];
    
    if file.read_exact(&mut dos_header).is_err() {
        return Ok(()); 
    }
    
    if dos_header[0] != b'M' || dos_header[1] != b'Z' {
        return Ok(()); 
    }
    
    let pe_offset = u32::from_le_bytes(dos_header[60..64].try_into().unwrap()) as u64;
    
    file.seek(SeekFrom::Start(pe_offset))?;
    let mut pe_sig = [0u8; 4];
    file.read_exact(&mut pe_sig)?;

    if &pe_sig != b"PE\0\0" {
        return Ok(()); 
    }
    
    let subsystem_offset = pe_offset + 92;
    file.seek(SeekFrom::Start(subsystem_offset))?;
    
    let gui_subsystem: u16 = 2;
    file.write_all(&gui_subsystem.to_le_bytes())?;
    
    Ok(())
}