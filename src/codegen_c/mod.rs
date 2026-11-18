#![allow(dead_code)]

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;
use std::process::Command;

use std::path::PathBuf;
use crate::engine::{Expr, ExprNode, Stmt, StmtNode, lex, Parser};

pub mod target;
pub use target::CrossTarget;
use target::{ndk_clang_path, ios_clang_and_sysroot};

fn is_builtin_module(name: &str) -> bool {
    matches!(
        name,
        "math" | "file" | "os" | "path" | "time" | "json" | "base64"
        | "net" | "http" | "ffi" | "struct" | "mem" | "thread"
        | "uuid" | "crypto" | "pdf" | "xml" | "zip" | "hw" | "task" | "gfx" | "input" | "con" | "disk" | "proc" | "blob"
    )
}

#[derive(Default, Clone)]
pub struct CrossBuildOptions {
    pub targets: Vec<CrossTarget>,
    pub android_api: u32,
    pub ios_min_version: String,
    pub make_xcframework: bool,
    pub jni_package: Option<String>,
    pub jni_class: Option<String>,
}

pub fn compile_to_native_c(
    ast: &Stmt,
    src_path: &Path,
    out_path: &Path,
    keep_c: bool,
    is_lib: bool,
    is_window: bool,
    cross: &CrossBuildOptions,
) -> Result<(), String> {
    if !cross.targets.is_empty() && !is_lib {
        return Err(
            "--target is only supported together with --lib. Cross-compiling a \
             standalone executable via --native isn't wired up yet; build a \
             shared/static library and drive it from a native Android/iOS host app instead."
                .into(),
        );
    }

    let base_dir = src_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let flattened_lib_ast;
    let flattened_exe_ast;
    let ast: &Stmt = if is_lib {
        flattened_lib_ast = flatten_imports_for_lib(ast, &base_dir, &mut std::collections::HashSet::new())?;
        &flattened_lib_ast
    } else if let Some(f) = app::flatten_for_executable(ast, src_path)? {
        flattened_exe_ast = f;
        &flattened_exe_ast
    } else {
        ast
    };

    let c_src = if is_lib {
        transpile_native_lib(ast).map_err(|e| e)?
    } else {
        transpile(ast, false, &base_dir)
    };

    let c_path = {
        let stem = out_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let dir = std::env::temp_dir();
        dir.join(format!("dragon_{}.c", stem))
    };
    fs::write(&c_path, &c_src).map_err(|e| format!("Failed to write C source: {}", e))?;
    println!("  [native] C source → {}", c_path.display());

    if cross.targets.is_empty() {
        let status = invoke_compiler(&c_path, out_path, is_lib, is_window)?;
        if !status {
            if !keep_c {
                let _ = fs::remove_file(&c_path);
            }
            return Err("C compiler exited with non-zero status".into());
        }
    } else {
        let stem = out_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let header_text = generate_native_lib_header(ast, &stem).ok();

        let mut android_jni_path: Option<PathBuf> = None;
        if is_lib {
            if let (Some(pkg), Some(cls)) = (&cross.jni_package, &cross.jni_class) {
                let jni_wrappers = generate_jni_wrappers(ast, pkg, cls).map_err(|e| {
                    if !keep_c { let _ = fs::remove_file(&c_path); }
                    e
                })?;
                let augmented = format!(
                    "{}\n\n/* ---- JNI wrappers (Android) ---- */\n#include <jni.h>\n\n{}",
                    c_src, jni_wrappers
                );
                let jni_path = std::env::temp_dir().join(format!("dragon_{}_jni.c", stem));
                fs::write(&jni_path, &augmented)
                    .map_err(|e| format!("Failed to write JNI-augmented C source: {}", e))?;
                println!("  [native] Android C source (+JNI) → {}", jni_path.display());
                android_jni_path = Some(jni_path);
            }
        }

        if let Err(e) = invoke_cross_compiler(&c_path, android_jni_path.as_deref(), out_path, &stem, cross, header_text.as_deref()) {
            if !keep_c {
                let _ = fs::remove_file(&c_path);
                if let Some(p) = &android_jni_path { let _ = fs::remove_file(p); }
            }
            return Err(e);
        }
        if !keep_c {
            if let Some(p) = &android_jni_path { let _ = fs::remove_file(p); }
        }
    }

    if !keep_c {
        let _ = fs::remove_file(&c_path);
    }

    if is_lib {
        let stem = out_path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        match generate_native_lib_header(ast, &stem) {
            Ok(header) => {
                let h_dir = out_path.parent().unwrap_or(Path::new("."));
                let h_path = h_dir.join(format!("{}.h", stem));
                if let Err(e) = fs::write(&h_path, &header) {
                    println!("  [native] warning: failed to write header {}: {}", h_path.display(), e);
                } else {
                    println!("  [native] header    → {}", h_path.display());
                }
            }
            Err(e) => {
                println!("  [native] warning: could not generate header: {}", e);
            }
        }
    }

    Ok(())
}

fn invoke_compiler(
    c_path: &Path,
    out_path: &Path,
    is_lib: bool,
    is_window: bool,
) -> Result<bool, String> {
    let c_str = c_path.to_str().unwrap();

    let lib_path_buf;
    let o_str = if is_lib {
        let stem = out_path.file_stem().unwrap_or_default().to_string_lossy();
        let dir  = out_path.parent().unwrap_or(Path::new("."));
        let (prefix, ext) = if cfg!(target_os = "windows") {
            ("", "dll")
        } else if cfg!(target_os = "macos") {
            ("lib", "dylib")
        } else {
            ("lib", "so")
        };
        let lib_name = format!("{}{}.{}", prefix, stem, ext);
        lib_path_buf = dir.join(lib_name);
        lib_path_buf.to_str().unwrap()
    } else {
        lib_path_buf = out_path.to_path_buf();
        out_path.to_str().unwrap()
    };

    if cfg!(target_os = "windows") {
        if which_exists("cl") {
            let fe_arg = format!("/Fe:{}", o_str);
            let mut args: Vec<&str> = vec![c_str, &fe_arg, "/O2", "/nologo"];

            args.push("/DDGN_NO_GC");

            if is_lib {
                args.push("/LD");
                args.push("/MT");
            } else {
                args.push("/MT");
            }
            if is_window { args.extend(&["/link", "/SUBSYSTEM:WINDOWS"]); }

            let ok = Command::new("cl").args(&args).status()
                .map(|s| s.success())
                .unwrap_or(false);
            if ok { return Ok(true); }
        }
        if which_exists("gcc") {
            return run_gcc("gcc", c_str, o_str, is_lib, is_window);
        }
        if which_exists("clang") {
            return run_gcc("clang", c_str, o_str, is_lib, is_window);
        }
        Err("No C compiler found. Install GCC (e.g. via MSYS2) or MSVC Build Tools, \
             then ensure it is on PATH."
            .into())
    } else if cfg!(target_os = "macos") {
        let cc = if which_exists("clang") {
            "clang"
        } else if which_exists("gcc") {
            "gcc"
        } else {
            return Err("No C compiler found. Run: xcode-select --install".into());
        };
        run_gcc(cc, c_str, o_str, is_lib, is_window)
    } else {
        let cc = if which_exists("gcc") {
            "gcc"
        } else if which_exists("cc") {
            "cc"
        } else if which_exists("clang") {
            "clang"
        } else {
            return Err("No C compiler found. Install with: apt install gcc".into());
        };
        run_gcc(cc, c_str, o_str, is_lib, is_window)
    }
}

fn run_gcc(
    cc: &str,
    c_path: &str,
    out_path: &str,
    is_lib: bool,
    is_window: bool,
) -> Result<bool, String> {
    let opt_flags: &[&str] = if is_lib {
        &["-O3"]
    } else if cc == "gcc" {
        &["-O3", "-fno-inline-functions", "-fomit-frame-pointer"]
    } else {
        &["-O2"]
    };
    let mut args = vec![c_path];
    args.extend_from_slice(opt_flags);
    args.extend_from_slice(&["-o", out_path, "-lm"]);
    if is_lib {
        args.push("-shared");
        args.push("-fPIC");
    }
    if is_window && cfg!(target_os = "windows") {
        args.push("-mwindows");
    }
    args.push("-s");

    let status = Command::new(cc)
        .args(&args)
        .status()
        .map_err(|e| format!("Failed to launch {}: {}", cc, e))?;
    Ok(status.success())
}

fn which_exists(name: &str) -> bool {
    Command::new(if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    })
    .arg(name)
    .output()
    .map(|o| o.status.success())
    .unwrap_or(false)
}

fn invoke_cross_compiler(
    c_path: &Path,
    android_c_path: Option<&Path>,
    out_path: &Path,
    stem: &str,
    cross: &CrossBuildOptions,
    header_text: Option<&str>,
) -> Result<(), String> {
    let out_dir = out_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let c_str = c_path.to_str().ok_or("Non-UTF8 temp C path")?;

    let android_c_str = android_c_path.and_then(|p| p.to_str()).unwrap_or(c_str);

    let mut errors: Vec<String> = Vec::new();

    let android_targets: Vec<CrossTarget> =
        cross.targets.iter().copied().filter(|t| t.is_android()).collect();
    if !android_targets.is_empty() {
        let jni_libs_dir = out_dir.join("jniLibs");
        for t in android_targets {
            match ndk_clang_path(t, cross.android_api) {
                Ok(clang) => {
                    let abi_dir = jni_libs_dir.join(t.android_abi_dir());
                    if let Err(e) = fs::create_dir_all(&abi_dir) {
                        errors.push(format!("[{}] could not create {}: {}", t.label(), abi_dir.display(), e));
                        continue;
                    }
                    let so_name = format!("lib{}.so", stem);
                    let so_path = abi_dir.join(&so_name);

                    let soname_flag = format!("-Wl,-soname,{}", so_name);
                    let args = [
                        android_c_str, "-shared", "-fPIC", "-O3",
                        "-o", so_path.to_str().unwrap(),
                        "-lm",
                        soname_flag.as_str(),
                        "-Wl,-z,max-page-size=16384",
                    ];
                    println!("  [native] [{}] compiling with {}", t.label(), clang.display());
                    match Command::new(&clang).args(args).status() {
                        Ok(s) if s.success() => println!("  [native] [{}] → {}", t.label(), so_path.display()),
                        Ok(s) => errors.push(format!("[{}] clang exited with {}", t.label(), s)),
                        Err(e) => errors.push(format!("[{}] failed to launch {}: {}", t.label(), clang.display(), e)),
                    }
                }
                Err(e) => errors.push(format!("[{}] {}", t.label(), e)),
            }
        }
    }

    let ios_targets: Vec<CrossTarget> =
        cross.targets.iter().copied().filter(|t| t.is_ios()).collect();
    let mut built_slices: Vec<(CrossTarget, PathBuf)> = Vec::new();
    if !ios_targets.is_empty() {
        let ios_dir = out_dir.join("ios");
        for t in ios_targets {
            match build_ios_slice(c_str, &ios_dir, stem, t, &cross.ios_min_version, &[]) {
                Ok(a_path) => built_slices.push((t, a_path)),
                Err(e) => errors.push(format!("[{}] {}", t.label(), e)),
            }
        }

        if cross.make_xcframework && !built_slices.is_empty() {
            match package_xcframework(&out_dir, stem, &built_slices, header_text) {
                Ok(xc_path) => println!("  [native] xcframework → {}", xc_path.display()),
                Err(e) => errors.push(format!("[xcframework] {}", e)),
            }
        } else if !cross.make_xcframework && !built_slices.is_empty() {
            println!(
                "  [native] iOS static libs built under {} — pass --xcframework to package them.",
                ios_dir.display()
            );
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{} of {} target(s) failed:\n  - {}",
            errors.len(), cross.targets.len(), errors.join("\n  - ")))
    }
}

pub(crate) fn build_ios_slice(
    c_str: &str,
    ios_dir: &Path,
    stem: &str,
    target: CrossTarget,
    min_version: &str,
    extra: &[&str],
) -> Result<PathBuf, String> {
    let (clang, sysroot) = ios_clang_and_sysroot(target)?;
    let slice_dir = ios_dir.join(target.label());
    fs::create_dir_all(&slice_dir).map_err(|e| format!("could not create {}: {}", slice_dir.display(), e))?;

    let obj_path = slice_dir.join(format!("{}.o", stem));
    let triple = target.ios_clang_target(min_version);
    let compile_args = [
        "-c", c_str,
        "-arch", target.ios_arch(),
        "-target", &triple,
        "-isysroot", sysroot.to_str().unwrap(),
        "-O2", "-fPIC",
        "-o", obj_path.to_str().unwrap(),
    ];
    let compile_args: Vec<&str> = compile_args.iter().copied().chain(extra.iter().copied()).collect();
    println!("  [native] [{}] compiling with {} (-target {})", target.label(), clang.display(), triple);
    let status = Command::new(&clang).args(compile_args).status()
        .map_err(|e| format!("failed to launch {}: {}", clang.display(), e))?;
    if !status.success() {
        return Err(format!("clang exited with {}", status));
    }

    let a_path = slice_dir.join(format!("lib{}.a", stem));
    let ar_status = Command::new("xcrun")
        .args(["ar", "-rcs", a_path.to_str().unwrap(), obj_path.to_str().unwrap()])
        .status()
        .map_err(|e| format!("failed to launch xcrun ar: {}", e))?;
    if !ar_status.success() {
        return Err(format!("ar exited with {}", ar_status));
    }
    println!("  [native] [{}] → {}", target.label(), a_path.display());
    Ok(a_path)
}

pub(crate) fn package_xcframework(
    out_dir: &Path,
    stem: &str,
    slices: &[(CrossTarget, PathBuf)],
    header_text: Option<&str>,
) -> Result<PathBuf, String> {
    let headers_dir = out_dir.join("ios").join("include");
    fs::create_dir_all(&headers_dir).map_err(|e| format!("could not create {}: {}", headers_dir.display(), e))?;
    if let Some(h) = header_text {
        fs::write(headers_dir.join(format!("{}.h", stem)), h)
            .map_err(|e| format!("could not write header: {}", e))?;
    }

    let device_lib = slices.iter().find(|(t, _)| *t == CrossTarget::IosArm64).map(|(_, p)| p.clone());
    let sim_libs: Vec<PathBuf> = slices.iter().filter(|(t, _)| t.is_ios_simulator()).map(|(_, p)| p.clone()).collect();

    let sim_fat_lib = if sim_libs.len() > 1 {
        let fat_path = out_dir.join("ios").join(format!("lib{}-simulator-fat.a", stem));
        let mut args = vec!["lipo", "-create"];
        let sim_strs: Vec<&str> = sim_libs.iter().map(|p| p.to_str().unwrap()).collect();
        args.extend(sim_strs.iter().copied());
        args.push("-output");
        let fat_str = fat_path.to_str().unwrap();
        args.push(fat_str);
        let status = Command::new("xcrun").args(&args).status()
            .map_err(|e| format!("failed to launch xcrun lipo: {}", e))?;
        if !status.success() {
            return Err(format!("lipo exited with {}", status));
        }
        Some(fat_path)
    } else {
        sim_libs.into_iter().next()
    };

    if device_lib.is_none() && sim_fat_lib.is_none() {
        return Err("no iOS slices available to package".into());
    }

    let xc_path = out_dir.join(format!("{}.xcframework", stem));
    let _ = fs::remove_dir_all(&xc_path);
    let mut args = vec!["xcodebuild".to_string(), "-create-xcframework".to_string()];
    if let Some(ref d) = device_lib {
        args.push("-library".into());
        args.push(d.to_str().unwrap().to_string());
        args.push("-headers".into());
        args.push(headers_dir.to_str().unwrap().to_string());
    }
    if let Some(ref s) = sim_fat_lib {
        args.push("-library".into());
        args.push(s.to_str().unwrap().to_string());
        args.push("-headers".into());
        args.push(headers_dir.to_str().unwrap().to_string());
    }
    args.push("-output".into());
    args.push(xc_path.to_str().unwrap().to_string());

    let status = Command::new("xcrun").args(&args).status()
        .map_err(|e| format!("failed to launch xcrun xcodebuild: {}", e))?;
    if !status.success() {
        return Err(format!("xcodebuild -create-xcframework exited with {}", status));
    }
    Ok(xc_path)
}

mod app;
pub mod baremetal;
mod modpass;
mod mangle;
mod native;
mod runtime;
mod transpiler;

pub use app::compile_app;
pub(crate) use mangle::*;
pub(crate) use native::*;
pub(crate) use runtime::*;
pub(crate) use transpiler::*;
