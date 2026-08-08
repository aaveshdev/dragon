use std::sync::{Arc, Mutex};
use std::collections::{HashMap, HashSet};
use std::thread;
use std::time::Duration;
use std::fs::{self, File};
use std::process::Command;
use std::io::{Read, Write, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::env;
use std::fmt;
use std::ffi::{CString, CStr};
use chrono::{DateTime, Local, Datelike, Timelike};
use std::os::raw::c_char;

use std::net::{TcpListener, TcpStream, Shutdown};
use std::io::{BufReader, BufRead};

use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use async_recursion::async_recursion;

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce, Key
};
use aes_gcm::aead::rand_core::RngCore;

use regex::Regex;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
use tiny_http::{Server, Response, Header, Method};
#[cfg(not(any(target_arch = "wasm32")))]
use libloading::{Library, Symbol};
use lazy_static::lazy_static;

use std::cell::RefCell;
use tokio::task_local;

#[cfg(target_arch = "wasm32")]
use web_sys;

#[cfg(not(any(target_arch = "wasm32")))]
use libffi::middle::{Cif, Type, CodePtr, Arg};
#[cfg(not(any(target_arch = "wasm32")))]
use std::ffi::c_void;

use roxmltree;
use ttf_parser;
use fontdb;
use image::GenericImageView;

lazy_static! {
    // static ref GC_STATE: Mutex<GcState> = Mutex::new(GcState::new());
    static ref FFI_SIGNATURES: Mutex<HashMap<String, (String, Vec<String>)>> = Mutex::new(HashMap::new());
static ref FFI_STRUCTS: Mutex<HashMap<String, FfiStructDef>> = Mutex::new(HashMap::new());

static ref BUILTIN_MODULE_CACHE: Mutex<HashMap<String, DgnValue>> =
        Mutex::new(HashMap::new());

    static ref TOKEN_REGEXES: Vec<(&'static str, Regex)> = vec![
        ("INC", Regex::new(r"^\+\+").unwrap()), 
        ("DEC", Regex::new(r"^--").unwrap()), 
        ("COMMENT_ML", Regex::new(r"(?s)^/\*.*?\*/").unwrap()), 
        ("COMMENT_SL", Regex::new(r"^//[^\n]*").unwrap()), 
        ("HEX", Regex::new(r"^0x[0-9a-fA-F]+").unwrap()),
        ("NUMBER", Regex::new(r"^[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?").unwrap()),
        ("STRING", Regex::new(r#"^"([^"\\]|\\.)*"|^'([^'\\]|\\.)+'"#).unwrap()), 
        ("NAME", Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*").unwrap()),
        ("OP", Regex::new(r"^(==|!=|<=|>=|=>|<<|>>|\?\?|\.\.\.|[!+\-*/%<>=?:;,&|^~(){}\[\].])").unwrap()),
        ("NEWLINE", Regex::new(r"^\n").unwrap()),
        ("SKIP", Regex::new(r"^[ \t\r\u{FEFF}\x00]+").unwrap()), 
        ("MISMATCH", Regex::new(r"^.").unwrap()),
    ];
}

task_local! {
    pub static GC_STATE: RefCell<GcState>;
}

struct FfiStructDef {
    size: usize,
    fields: Vec<(String, usize, Type)>, // Name, Offset, LibFFI Type"
    libffi_type: Type, // The constructed Type::structure
}
// We need Send/Sync to store Type in a global Mutex, forcing unsafe impl if necessary
// (libffi Types are generally thread-safe as they are read-only descriptors)
#[cfg(not(any(target_arch = "wasm32")))]
unsafe impl Send for FfiStructDef {}
#[cfg(not(any(target_arch = "wasm32")))]
unsafe impl Sync for FfiStructDef {}

pub const BUNDLE_SIGNATURE: &[u8; 16] = b"DRAGON_BUNDLE_v1";



mod ast;
mod value;
mod error;
mod gc;
mod bytecode;
mod compiler;
mod vm;
mod environment;
mod lexer;
mod parser;
mod builtins;
mod bundle;
mod linter;
mod crypto;

pub use ast::*;
pub use value::*;
pub use error::*;
pub use gc::*;
pub use bytecode::*;
pub use compiler::*;
pub use vm::*;
pub use environment::*;
pub use lexer::*;
pub use parser::*;
pub use builtins::*;
pub use bundle::*;
pub use linter::*;
pub use crypto::*;
