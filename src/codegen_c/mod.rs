//! codegen_c.rs — Dragon → C transpiler
//!
//! Translates a Dragon AST into a self-contained C99 source file that can be
//! compiled by GCC, Clang, or MSVC on any platform.
//!
//! Usage (from main.rs):
//!   dragon -c input.dgn -o app --native

#![allow(dead_code)]

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;
use std::process::Command;

use std::path::PathBuf;
use crate::engine::{Expr, ExprNode, Stmt, StmtNode, lex, Parser};

// ─────────────────────────────────────────────────────────────────────────────
//  Public entry point
// ─────────────────────────────────────────────────────────────────────────────
fn is_builtin_module(name: &str) -> bool {
    matches!(
        name,
        "math" | "file" | "os" | "path" | "time" | "json" | "base64"
        | "net" | "http" | "ffi" | "struct" | "mem" | "thread"
        | "uuid" | "crypto" | "pdf" | "xml" | "zip"
    )
}

pub fn compile_to_native_c(
    ast: &Stmt,
    src_path: &Path,          // ← new: path to the .dgn entry file
    out_path: &Path,
    keep_c: bool,
    is_lib: bool,
    is_window: bool,
) -> Result<(), String> {
    let base_dir = src_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let c_src = if is_lib {
        transpile_native_lib(ast)
    } else {
        transpile(ast, false, &base_dir)   // ← pass base_dir
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

    let status = invoke_compiler(&c_path, out_path, is_lib, is_window)?;
    if !status {
        // if !keep_c {
            let _ = fs::remove_file(&c_path);
        // }
        return Err("C compiler exited with non-zero status".into());
    }

    // if !keep_c {
        let _ = fs::remove_file(&c_path);
    // }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
//  Compiler detection + invocation
// ─────────────────────────────────────────────────────────────────────────────

fn invoke_compiler(
    c_path: &Path,
    out_path: &Path,
    is_lib: bool,
    is_window: bool,
) -> Result<bool, String> {
    let c_str = c_path.to_str().unwrap();

    // For shared libraries, ensure the output has the correct platform extension.
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
                // /LDd would be debug; /LD /MT = release DLL with static CRT
                // Static CRT (/MT) means no VCRUNTIME140.dll dependency — loads anywhere
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
    let mut args = vec![c_path, "-O2", "-o", out_path, "-lm"];
    if is_lib {
        args.push("-shared");
        args.push("-fPIC");
    }
    // On Windows with gcc/clang (MSYS2/MinGW), -mwindows sets SUBSYSTEM:WINDOWS
    // which hides the console window. Not valid on Linux/macOS.
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


mod mangle;
mod native;
mod runtime;
mod transpiler;

pub(crate) use mangle::*;
pub(crate) use native::*;
pub(crate) use runtime::*;
pub(crate) use transpiler::*;
