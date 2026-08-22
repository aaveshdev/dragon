mod engine;
mod codegen_c;
mod tui;
mod android_project;
// mod aot_compiler;
use engine::*;
// use engine::{Chunk, Compiler, Instruction, lint_ast, lex, Parser};
// use aot_compiler::*;

use std::sync::{Arc, Mutex};
use std::collections::{HashMap};
use std::fs::{self, File,OpenOptions};
use std::path::{Path, PathBuf};
use std::io::{Write, Read, Seek, SeekFrom};
use std::env;
use std::panic;

macro_rules! VERSION { () => { "1.0.7" } }
const DGN_VERSION: &str = concat!("Dragon Programming Language v", VERSION!());
const DGN_BUILD: &str = concat!("Build ", env!("DGN_BUILD_DATE"));
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
        let tokens = match lex(entry) {
            Ok(t) => t,
            Err(e) => { 
                eprintln!("Syntax Error: {}", e.with_source(entry).format_pretty()); 
                std::process::exit(1); 
            }
        };
        let mut parser = Parser::new(tokens);
        match parser.parse() {
            Ok(ast) => {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let mut chunk = Chunk::new();
                Compiler::compile_stmt(&mut chunk, &ast);
                chunk.emit(Instruction::Halt, 0);

                if let Err(e) = rt.block_on(async {
                    let mut vm = Vm::new();
                    vm.run(&chunk, env.clone()).await.map(|_| ())
                }) {
                    eprintln!("{}", e.with_source(entry).format_pretty());
                    std::process::exit(1);
                }
            },
            Err(e) => { eprintln!("Bundle Parse Error: {}", e); std::process::exit(1); }
        }
        return;
    }

    if args.len() >= 2 && (args[1] == "--ide" || args[1] == "-ide") {
    if let Err(e) = tui::run_tui_ide(VERSION!(), DGN_BUILD, DGN_AUTHOR) {
        eprintln!("TUI Error: {}", e);
        std::process::exit(1);
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
        println!("  dragon -c <in> -o <out> --native  Compile script to native C binary (via gcc/clang/msvc) experimental");
        println!("  dragon -c <in> -o <out> --native --lib     Native shared library experimental");
        println!("  dragon -c <in> -o <out> --native --lib --target <t1,t2,...>   Cross-compile the native library.");
        println!("        Targets: android-arm64, android-armv7, android-x86_64, android-x86, android-all,");
        println!("                 ios-arm64, ios-arm64-sim, ios-x86_64-sim, ios-all, all");
        println!("        Android → <out>/jniLibs/<abi>/lib<name>.so  (drop into a Gradle project's jniLibs/)");
        println!("        iOS     → <out>/ios/<slice>/lib<name>.a, plus <out>.xcframework with --xcframework");
        println!("        Options: --android-api <level>  (default 24)");
        println!("                 --ios-min <version>     (default 13.0)");
        println!("                 --xcframework            package iOS slices into a .xcframework");
        println!("                 --jni-package <pkg> --jni-class <Class>   emit Java_<pkg>_<Class>_<fn> JNI");
        println!("                     wrappers directly into the Android .so (both required together) --");
        println!("                     the .so becomes System.loadLibrary()-able with `external fun` as-is,");
        println!("                     no separate CMake/JNI bridge library needed.");
        println!("        Requires ANDROID_NDK_HOME for Android slices; iOS slices require building on macOS with Xcode.");
        println!("");
        println!("  dragon create <name> [--package <id>]   Scaffold a new Android app project");
        println!("        Generates package.json, lib/main.dgn + lib/ui.dgn, and a full Android Gradle project.");
        println!("        Run `dragon run`/`dragon build` inside the created directory from here on.");
        println!("  dragon run                              Run the current project");
        println!("        Inside a project made by `dragon create`: builds (native lib + Gradle), installs on a");
        println!("        connected device/emulator (via adb), and launches it.");
        println!("        Elsewhere: reads package.json's \"main\"/\"entry\" field and interprets that script.");
        println!("  dragon build [--release]                Build an APK (inside a `dragon create` project)");
        println!("        Debug APK by default; --release for a release build. Does not install/launch --");
        println!("        use `dragon run` for that.");
        println!("");
        println!("  dragon --ide                      Launch TurboDragon IDE");
        // println!("  dragon -c <in> -o <out> --web     Compile script to Web Bundle");
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
        
        let jni_dir = Path::new("app/src/main/jniLibs/x86_64");
        let output_so = jni_dir.join("libdragon_android.so");

        if !input_path.exists() {
            println!("Error: 'lib/main.dgn' not found.");
            return;
        }

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


        let mut sources = HashMap::new();
        if let Err(e) = gather_sources(input_path, &mut sources, true) {
            eprintln!("Build Failed: {}", e);
            std::process::exit(1);
        }

        let payload = serde_json::to_string(&sources).expect("Serialization failed");
        // let mut payload_bytes = payload.into_bytes();
        // let payload_len = payload_bytes.len() as u64;

        let payload_bytes = encrypt_payload(payload.as_bytes());
        let payload_len = payload_bytes.len() as u64;

  
        println!("Installing Engine to: {:?}", output_so);
        fs::copy(&template_path, &output_so).expect("Failed to copy library template");

        println!("Injecting Encrypted Bytecode...");
        
        let mut file = OpenOptions::new()
            .write(true)
            .append(true)
            .open(&output_so)
            .expect("Failed to open .so for writing");
        
        file.write_all(&payload_bytes).expect("Write failed");
        file.write_all(&payload_len.to_le_bytes()).expect("Write len failed");
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
        let is_lib_mode    = args.contains(&"--lib".to_string());
        let is_aot_mode    = args.contains(&"--aot".to_string());
        let is_web_mode    = args.contains(&"--web".to_string());
        let is_window_mode = args.contains(&"--window".to_string());
        let is_native_mode = args.contains(&"--native".to_string());
        let keep_c_mode    = args.contains(&"--keep-c".to_string());
        let make_xcframework = args.contains(&"--xcframework".to_string());

        let target_spec = args.iter().position(|a| a == "--target").and_then(|i| args.get(i + 1)).cloned();
        let android_api: u32 = args.iter().position(|a| a == "--android-api")
            .and_then(|i| args.get(i + 1))
            .and_then(|s| s.parse().ok())
            .unwrap_or(24); // NDK r27+ default floor; override with --android-api <level>
        let ios_min_version = args.iter().position(|a| a == "--ios-min")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| "13.0".to_string());
        let jni_package = args.iter().position(|a| a == "--jni-package").and_then(|i| args.get(i + 1)).cloned();
        let jni_class = args.iter().position(|a| a == "--jni-class").and_then(|i| args.get(i + 1)).cloned();
        if jni_package.is_some() != jni_class.is_some() {
            eprintln!("Error: --jni-package and --jni-class must be given together.");
            std::process::exit(1);
        }

        let cross_targets = match target_spec {
            Some(spec) => match codegen_c::CrossTarget::parse_list(&spec) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            },
            None => Vec::new(),
        };
        let cross_opts = codegen_c::CrossBuildOptions {
            targets: cross_targets,
            android_api,
            ios_min_version,
            make_xcframework,
            jni_package,
            jni_class,
        };

        println!("Compiling {}...", input_path.display());

        // ── --native path: transpile Dragon AST → C, then invoke gcc/clang/msvc ──
        if is_native_mode {
            let src = match fs::read_to_string(input_path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Error reading {}: {}", input_path.display(), e);
                    std::process::exit(1);
                }
            };

            let tokens = match lex(&src) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Syntax Error: {}", e.with_source(&src).format_pretty());
                    std::process::exit(1);
                }
            };

            let mut parser = Parser::new(tokens);
            let ast = match parser.parse() {
                Ok(a) => a,
                Err(e) => {
                    eprintln!("Parse Error: {}", e);
                    std::process::exit(1);
                }
            };

            // Lint the AST. For --native --lib, imports are resolved and
            // merged first (native has no runtime import mechanism -- it's
            // one flat C translation unit), so the linter sees the same
            // fully-merged function/global set the actual compile step
            // will use; otherwise every function pulled in from an
            // imported file would incorrectly look undefined here.
            let base_dir_for_lint = input_path.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
            let flattened_for_lint;
            let ast_for_lint: &Stmt = if is_lib_mode {
                match codegen_c::flatten_imports_for_lib(&ast, &base_dir_for_lint, &mut std::collections::HashSet::new()) {
                    Ok(a) => { flattened_for_lint = a; &flattened_for_lint }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                &ast
            };

            // --native --lib has its own bare-name builtin allowlist (see
            // codegen_c::native::builtin_call) that's intentionally
            // different from the interpreter's namespaced builtins (e.g.
            // `sqrt(x)` here vs. `math.sqrt(x)` in a normal script), so the
            // linter needs to be told about those too or it flags every
            // one of them as an undefined function.
            let native_extra_builtins = [
                "sqrt", "abs", "floor", "ceil", "pow", "min", "max",
                "substr", "index_of", "char_at", "parse_int", "parse_float",
                "file_write", "file_append", "file_read", "file_exists", "file_delete", "json_escape",
            ];
            let report = lint_ast_with_extra_builtins(ast_for_lint, &native_extra_builtins);
            if !report.is_clean() {
                report.print();
                if !report.errors.is_empty() {
                    std::process::exit(1);
                }
            }

            println!("  [native] Transpiling to C...");
            match codegen_c::compile_to_native_c(
                &ast,
                input_path,
                output_path,
                keep_c_mode,
                is_lib_mode,
                is_window_mode,
                &cross_opts,
            ) {
                Ok(()) => {
                    #[cfg(unix)]
                    {
                        // Cross-compiled artifacts (Android .so / iOS .a) are
                        // written under out_dir/jniLibs and out_dir/ios, not to
                        // output_path itself, so there's nothing at output_path
                        // to chmod in that case.
                        if cross_opts.targets.is_empty() {
                            use std::os::unix::fs::PermissionsExt;
                            if let Ok(metadata) = fs::metadata(output_path) {
                                let mut perms = metadata.permissions();
                                perms.set_mode(0o755);
                                let _ = fs::set_permissions(output_path, perms);
                            }
                        }
                    }
                    if cross_opts.targets.is_empty() {
                        println!("Success! Native output: {}", output_path.display());
                    } else {
                        println!("Success! Cross-compiled target(s) built.");
                    }
                }
                Err(e) => {
                    eprintln!("Native compilation failed: {}", e);
                    std::process::exit(1);
                }
            }
            return;
        }


       let mut sources = HashMap::new();
        
        if let Err(e) = gather_sources(input_path, &mut sources, true) {
            eprintln!("Compilation Error: {}", e);
            std::process::exit(1);
        }
        
       let payload = serde_json::to_string(&sources).expect("Failed to serialize bundle");
        
        let payload_bytes = encrypt_payload(payload.as_bytes());
        let payload_len = payload_bytes.len() as u64;
//         if is_aot_mode {
//             // ── AOT path ──────────────────────────────────────────────────
//             println!("Compiling {} (AOT mode)...", input_path.display());

//             let src = match fs::read_to_string(input_path) {
//                 Ok(s) => s,
//                 Err(e) => { eprintln!("Error reading {}: {}", input_path.display(), e); std::process::exit(1); }
//             };

//             let tokens = lex(&src);
//             let mut parser = Parser::new(tokens);
//             let ast = match parser.parse() {
//                 Ok(a) => a,
//                 Err(e) => { eprintln!("Parse Error: {}", e); std::process::exit(1); }
//             };

//             // Lint first
//             let report = lint_ast(&ast);
//             if !report.is_clean() {
//                 report.print();
//                 if !report.errors.is_empty() { std::process::exit(1); }
//             }

//             // Compile AST → bytecode Chunk (reuse existing compiler)
//             let mut chunk = Chunk::new();
//             Compiler::compile_stmt(&mut chunk, &ast);
//             chunk.emit(Instruction::Halt, 0);

//             // Compile Chunk → .o via Cranelift
//             let obj_path = output_path.with_extension("o");
//             if let Err(e) = aot_compiler::compile_to_object(&chunk, &obj_path) {
//                 eprintln!("AOT Compile Error: {}", e);
//                 std::process::exit(1);
//             }

//             // Link: .o + dragon_runtime.a → final binary
//             println!("Linking {}...", output_path.display());
//          let link_status = if cfg!(target_os = "windows") {
//     // ── Compile the C shim if the .obj is not already present ─────────
//     // (Do this once; skip if already built)
//     let shim_obj = std::path::Path::new("aot_main_shim.obj");
//     if !shim_obj.exists() {
//         let shim_c = std::path::Path::new("aot_main_shim.c");
//         if shim_c.exists() {
//             let status = std::process::Command::new("cl")
//                 .args(["/c", "/MT", "aot_main_shim.c", "/Fo:aot_main_shim.obj"])
//                 .status();
//             if !matches!(status, Ok(s) if s.success()) {
//                 eprintln!("AOT: Failed to compile aot_main_shim.c");
//                 std::process::exit(1);
//             }
//         } else {
//             eprintln!("AOT: aot_main_shim.c not found — cannot build entry point");
//             eprintln!("     Place aot_main_shim.c in the same directory as dragon.exe");
//             std::process::exit(1);
//         }
//     }

//     if is_lib_mode {
//         // ── Windows DLL ───────────────────────────────────────────────
//         std::process::Command::new("cl")
//             .args([
//                 "/LD",                               // build a DLL
//                 "/MT",                               // static CRT (no DLL dependency)
//                 obj_path.to_str().unwrap(),          // user's .o from Cranelift
//                 "dragon_runtime.lib",
//                 &format!("/Fe:{}", output_path.to_str().unwrap()),
//                 "/link",
//                 "ws2_32.lib", "user32.lib", "advapi32.lib",
//                 "bcrypt.lib", "ntdll.lib", "kernel32.lib",
//                 "shell32.lib", "ole32.lib",
//             ])
//             .status()
//     } else {
//         // ── Windows EXE ───────────────────────────────────────────────
//         std::process::Command::new("cl")
//             .args([
//                 "/MT",                               // static CRT — provides:
//                                                      //   mainCRTStartup, memcpy,
//                                                      //   memmove, memset, fmod,
//                                                      //   __chkstk, __CxxFrameHandler3
//                 obj_path.to_str().unwrap(),          // user's Cranelift .o
//                 "aot_main_shim.obj",                 // C entry point (not Rust)
//                 "dragon_runtime.lib",                // static runtime
//                 &format!("/Fe:{}", output_path.to_str().unwrap()),
//                 "/link",
//                 // Windows system libs (cover all __imp_* symbols)
//                 "ws2_32.lib",    // sockets
//                 "user32.lib",    // UI / console
//                 "advapi32.lib",  // registry / security
//                 "bcrypt.lib",    // crypto
//                 "ntdll.lib",     // NT internals
//                 "kernel32.lib",  // HeapAlloc/Free/ReAlloc, GetProcess*, etc.
//                 "shell32.lib",
//                 "ole32.lib",
//                 "/SUBSYSTEM:CONSOLE",
//                 "/ENTRY:mainCRTStartup",
//             ])
//             .status()
//     }
// } else if cfg!(target_os = "macos") {
//     // ── macOS ─────────────────────────────────────────────────────────
//     if is_lib_mode {
//         std::process::Command::new("cc")
//             .args([
//                 "-dynamiclib",
//                 obj_path.to_str().unwrap(),
//                 "aot_main_shim.o",
//                 "-L.", "-ldragon_runtime",
//                 "-lpthread", "-ldl", "-lm",
//                 "-o", output_path.to_str().unwrap(),
//             ])
//             .status()
//     } else {
//         std::process::Command::new("cc")
//             .args([
//                 obj_path.to_str().unwrap(),
//                 "aot_main_shim.o",
//                 "-L.", "-ldragon_runtime",
//                 "-lpthread", "-ldl", "-lm",
//                 "-o", output_path.to_str().unwrap(),
//             ])
//             .status()
//     }
// } else {
//     // ── Linux ─────────────────────────────────────────────────────────
//     if is_lib_mode {
//         std::process::Command::new("cc")
//             .args([
//                 "-shared", "-fPIC",
//                 obj_path.to_str().unwrap(),
//                 "-L.", "-ldragon_runtime",
//                 "-lpthread", "-ldl", "-lm",
//                 "-o", output_path.to_str().unwrap(),
//             ])
//             .status()
//     } else {
//         std::process::Command::new("cc")
//             .args([
//                 obj_path.to_str().unwrap(),
//                 "aot_main_shim.o",
//                 "-L.", "-ldragon_runtime",
//                 "-lpthread", "-ldl", "-lm",
//                 "-o", output_path.to_str().unwrap(),
//             ])
//             .status()
//     }
// };

//             match link_status {
//                 Ok(s) if s.success() => {
//                     println!("Success! AOT binary: {}", output_path.display());
//                     // Clean up intermediate .o
//                     let _ = fs::remove_file(&obj_path);
//                 }
//                 Ok(s) => {
//                     eprintln!("Linker exited with status {}", s);
//                     std::process::exit(1);
//                 }
//                 Err(e) => {
//                     eprintln!("Linker error: {}", e);
//                     std::process::exit(1);
//                 }
//             }

//             #[cfg(unix)]
//             {
//                 use std::os::unix::fs::PermissionsExt;
//                 if let Ok(meta) = fs::metadata(output_path) {
//                     let mut perms = meta.permissions();
//                     perms.set_mode(0o755);
//                     let _ = fs::set_permissions(output_path, perms);
//                 }
//             }
//             return;
//         }

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
        println!("Dragon version {} REPL", VERSION!());
        println!("Type 'exit' to quit.");

        let env = Arc::new(Mutex::new(Env::new(None, None)));
        env.lock().unwrap().store.insert("__modules__".into(), DgnValue::Map(Arc::new(Mutex::new(HashMap::new()))));
        populate_builtins(env.clone());

        // The REPL shares one persistent VM so variables defined in one line
        // are visible in the next — the env acts as the persistent scope.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let mut repl_vm = Vm::new();

        loop {
            print!("> ");
            std::io::stdout().flush().unwrap();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).unwrap() == 0 { break; }
            if line.trim() == "exit" { break; }

            let tokens = match lex(&line) {
                Ok(t) => t,
                Err(e) => {
                    println!("Syntax Error: {} at line {}, col {}", e.msg, e.line, e.col);
                    continue;
                }
            };
            let mut parser = Parser::new(tokens);
            match parser.parse_stmt() {
                Ok(ast) => {
                    let mut chunk = Chunk::new();
                    Compiler::compile_repl_stmt(&mut chunk, &ast);
                    chunk.emit(Instruction::Halt, 0);

                    let result = rt.block_on(async {
                        repl_vm.run(&chunk, env.clone()).await
                    });

                    match result {
                        Ok(v) => {
                            if !matches!(v, DgnValue::Null) {
                                println!("{}", val_to_string(&v));
                            }
                        }
                        Err(e) => println!("Error: {} at line {}", e.msg, e.line),
                    }
                }
                Err(_) => println!("Syntax Error"),
            }
        }
        return;
    }

    let mut filename = command.to_string();

    if command == "create" {
        if args.len() < 3 {
            eprintln!("Usage: dragon create <name> [--package <reverse.dns.name>]");
            std::process::exit(1);
        }
        let name = &args[2];
        let package = args.iter().position(|a| a == "--package")
            .and_then(|i| args.get(i + 1))
            .cloned()
            .unwrap_or_else(|| format!("com.example.{}", name.replace(['-', ' '], "_")));
        if let Err(e) = android_project::create_project(name, &package) {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
        return;
    }

    if command == "build" {
        let release = args.iter().any(|a| a == "--release");
        let dragon_exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("dragon"));
        match android_project::build_android(&dragon_exe, release) {
            Ok(_) => {}
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
        return;
    }

    if command == "run" {
        let pkg_path = Path::new("package.json");
        if pkg_path.exists() && android_project::is_android_project(pkg_path) {
            let dragon_exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("dragon"));
            match android_project::run_android(&dragon_exe) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
            return;
        }
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

            let tokens = match lex(&src) {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("Syntax Error: {}", e.with_source(&src).format_pretty());
                    std::process::exit(1);
                }
            };
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

                    let mut chunk = Chunk::new();
                    Compiler::compile_stmt(&mut chunk, &ast);
                    chunk.emit(Instruction::Halt, 0);

                    let rt = tokio::runtime::Runtime::new().unwrap();
                    if let Err(e) = rt.block_on(async {
                        let mut vm = Vm::new();
                        vm.run(&chunk, env.clone()).await.map(|_| ())
                    }) {
                        eprintln!("{}", e.with_source(&src).format_pretty());
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