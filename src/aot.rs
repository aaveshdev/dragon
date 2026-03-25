//! aot.rs — Dragon AOT Compiler
//!
//! x86-64 → ELF64 (Linux) / PE64 (Windows), no external tools.
//!
//! Value representation (stack-based, 2 words per value, payload on top):
//!   push TAG then push PAYLOAD  →  stack order: [TAG][PAYLOAD(top)]
//!   TAG_NULL=0  TAG_BOOL=1  TAG_NUM=2  TAG_STR=3
//!   TAG_FUNC=6  TAG_LIST=7  TAG_MAP=8  TAG_OBJ=9
//!
//! Heap layouts:
//!   Frame:  [parent:u64][n:u64][cap:u64] then slots [hash:u64][tag:u64][pay:u64]…
//!   List:   [cap:u64][len:u64] then slots [tag:u64][pay:u64]…
//!   Map:    [cap:u64][len:u64] then slots [hash:u64][tag:u64][pay:u64]…
//!
//! r12 = current activation frame (callee-saved, preserved across calls)

#![allow(dead_code, unused_variables, unused_mut, clippy::all)]

use crate::engine::{Stmt, StmtNode, Expr, ExprNode};

pub const TAG_NULL: u64 = 0;
pub const TAG_BOOL: u64 = 1;
pub const TAG_NUM:  u64 = 2;
pub const TAG_STR:  u64 = 3;
pub const TAG_FUNC: u64 = 6;
pub const TAG_LIST: u64 = 7;
pub const TAG_MAP:  u64 = 8;
pub const TAG_OBJ:  u64 = 9;

pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 14695981039346656037;
    for b in s.bytes() { h ^= b as u64; h = h.wrapping_mul(1099511628211); }
    h
}
pub fn align_up(n: usize, a: usize) -> usize { (n + a - 1) & !(a - 1) }

#[derive(Clone, Copy, PartialEq)]
pub enum TargetOs { Linux, Windows }

// ─────────────────────────────────────────────────────────────────────────────
//  Asm  — code/rodata buffers + relocation tracking
// ─────────────────────────────────────────────────────────────────────────────
pub struct Asm {
    pub code:   Vec<u8>,
    pub rodata: Vec<u8>,
    
    code_relocs:     Vec<(usize, usize)>,
    rodata_relocs:   Vec<(usize, usize)>,
    code_abs_relocs: Vec<(usize, usize)>,
    pub labels: Vec<Option<usize>>,
    pub os: TargetOs,
    pub iat_gsh_patches: Vec<usize>,
    pub iat_wf_patches:  Vec<usize>,
    pub iat_ep_patches:  Vec<usize>,
    pub iat_va_patches:  Vec<usize>,
    pub iat_gsh: u64, pub iat_wf: u64, pub iat_ep: u64, pub iat_va: u64, pub exports: Vec<(String, usize)>,
}

impl Asm {
    pub fn new(os: TargetOs) -> Self {
        Asm {
            code: Vec::new(), rodata: Vec::new(),
            code_relocs: Vec::new(), rodata_relocs: Vec::new(),
            code_abs_relocs: Vec::new(), labels: Vec::new(), os,
            iat_gsh_patches: Vec::new(), iat_wf_patches: Vec::new(),
            iat_ep_patches: Vec::new(), iat_va_patches: Vec::new(),
            iat_gsh: 0, iat_wf: 0, iat_ep: 0, iat_va: 0, exports: Vec::new(),
        }
    }

    // ── emit ──────────────────────────────────────────────────────────────────
    #[inline] pub fn b(&mut self, v: u8)     { self.code.push(v); }
    #[inline] pub fn bs(&mut self, v: &[u8]) { self.code.extend_from_slice(v); }
    #[inline] pub fn pos(&self) -> usize      { self.code.len() }
    pub fn u32le(&mut self, v: u32) { self.bs(&v.to_le_bytes()); }
    pub fn u64le(&mut self, v: u64) { self.bs(&v.to_le_bytes()); }

    // ── labels ────────────────────────────────────────────────────────────────
    pub fn label(&mut self) -> usize { let id=self.labels.len(); self.labels.push(None); id }
    pub fn place(&mut self, id: usize) { self.labels[id] = Some(self.code.len()); }

    // ── relocations ───────────────────────────────────────────────────────────
    pub fn rel32(&mut self, lbl: usize)  { self.code_relocs.push((self.code.len(), lbl)); self.bs(&[0u8;4]); }
    pub fn rodata_ptr(&mut self, off: usize) { self.rodata_relocs.push((self.code.len(), off)); self.bs(&[0u8;8]); }
    pub fn code_abs(&mut self, lbl: usize)   { self.code_abs_relocs.push((self.code.len(), lbl)); self.bs(&[0u8;8]); }
    pub fn iat_gsh(&mut self) { self.iat_gsh_patches.push(self.pos()); self.bs(&[0u8;8]); }
    pub fn iat_wf(&mut self)  { self.iat_wf_patches.push(self.pos());  self.bs(&[0u8;8]); }
    pub fn iat_ep(&mut self)  { self.iat_ep_patches.push(self.pos());  self.bs(&[0u8;8]); }
    pub fn iat_va(&mut self)  { self.iat_va_patches.push(self.pos());  self.bs(&[0u8;8]); }

    pub fn apply_relocs(&mut self, cv: u64, rv: u64) {
        for &(p, off) in &self.rodata_relocs {
            self.code[p..p+8].copy_from_slice(&(rv+off as u64).to_le_bytes());
        }
        let cr = std::mem::take(&mut self.code_relocs);
        for (p, lid) in cr {
            let t = self.labels[lid].unwrap_or_else(|| panic!("label {} unplaced", lid));
            let rel = (t as i64) - (p as i64 + 4);
            self.code[p..p+4].copy_from_slice(&(rel as i32).to_le_bytes());
        }
        let ar = std::mem::take(&mut self.code_abs_relocs);
        for (p, lid) in ar {
            let t = self.labels[lid].unwrap_or_else(|| panic!("abs label {} unplaced", lid));
            self.code[p..p+8].copy_from_slice(&(cv+t as u64).to_le_bytes());
        }
        for &p in &self.iat_gsh_patches { let v=self.iat_gsh; self.code[p..p+8].copy_from_slice(&v.to_le_bytes()); }
        for &p in &self.iat_wf_patches  { let v=self.iat_wf;  self.code[p..p+8].copy_from_slice(&v.to_le_bytes()); }
        for &p in &self.iat_ep_patches  { let v=self.iat_ep;  self.code[p..p+8].copy_from_slice(&v.to_le_bytes()); }
        for &p in &self.iat_va_patches  { let v=self.iat_va;  self.code[p..p+8].copy_from_slice(&v.to_le_bytes()); }
    }

    pub fn add_str(&mut self, s: &str) -> usize {
        let off = self.rodata.len();
        self.rodata.extend_from_slice(s.as_bytes());
        self.rodata.push(0); off
    }

    // ── x86-64 helpers ────────────────────────────────────────────────────────
    pub fn push_rax(&mut self) { self.b(0x50); }
    pub fn push_rdx(&mut self) { self.b(0x52); }
    pub fn push_rcx(&mut self) { self.b(0x51); }
    pub fn pop_rax(&mut self)  { self.b(0x58); }
    pub fn pop_rcx(&mut self)  { self.b(0x59); }
    pub fn pop_rdx(&mut self)  { self.b(0x5A); }
    pub fn pop_rsi(&mut self)  { self.b(0x5E); }
    pub fn pop_rdi(&mut self)  { self.b(0x5F); }
    pub fn ret(&mut self)      { self.b(0xC3); }
    pub fn syscall(&mut self)  { self.bs(&[0x0F,0x05]); }

    pub fn mov_rax_u64(&mut self, v: u64) { self.bs(&[0x48,0xB8]); self.u64le(v); }
    pub fn mov_rcx_u64(&mut self, v: u64) { self.bs(&[0x48,0xB9]); self.u64le(v); }
    pub fn mov_rdx_u64(&mut self, v: u64) { self.bs(&[0x48,0xBA]); self.u64le(v); }
    pub fn mov_rsi_u64(&mut self, v: u64) { self.bs(&[0x48,0xBE]); self.u64le(v); }
    pub fn mov_rdi_u64(&mut self, v: u64) { self.bs(&[0x48,0xBF]); self.u64le(v); }
    pub fn xor_rax_rax(&mut self) { self.bs(&[0x48,0x31,0xC0]); }
    pub fn xor_rdx_rdx(&mut self) { self.bs(&[0x48,0x31,0xD2]); }
    pub fn xor_rdi_rdi(&mut self) { self.bs(&[0x48,0x31,0xFF]); }
    pub fn xor_rsi_rsi(&mut self) { self.bs(&[0x48,0x31,0xF6]); }

    pub fn call(&mut self, lbl: usize)  { self.b(0xE8); self.rel32(lbl); }
    pub fn jmp(&mut self,  lbl: usize)  { self.b(0xE9); self.rel32(lbl); }
    pub fn je(&mut self,   lbl: usize)  { self.bs(&[0x0F,0x84]); self.rel32(lbl); }
    pub fn jne(&mut self,  lbl: usize)  { self.bs(&[0x0F,0x85]); self.rel32(lbl); }
    pub fn jz(&mut self,   lbl: usize)  { self.je(lbl); }
    pub fn jnz(&mut self,  lbl: usize)  { self.jne(lbl); }
    pub fn jb(&mut self,   lbl: usize)  { self.bs(&[0x0F,0x82]); self.rel32(lbl); }
    pub fn jbe(&mut self,  lbl: usize)  { self.bs(&[0x0F,0x86]); self.rel32(lbl); }
    pub fn ja(&mut self,   lbl: usize)  { self.bs(&[0x0F,0x87]); self.rel32(lbl); }
    pub fn jae(&mut self,  lbl: usize)  { self.bs(&[0x0F,0x83]); self.rel32(lbl); }
    pub fn jp(&mut self,   lbl: usize)  { self.bs(&[0x0F,0x8A]); self.rel32(lbl); }
    pub fn jl(&mut self,   lbl: usize)  { self.bs(&[0x0F,0x8C]); self.rel32(lbl); }
    pub fn jge(&mut self,  lbl: usize)  { self.bs(&[0x0F,0x8D]); self.rel32(lbl); }
    pub fn jns(&mut self,  lbl: usize)  { self.bs(&[0x0F,0x89]); self.rel32(lbl); }

    pub fn js(&mut self,   lbl: usize)  { self.bs(&[0x0F,0x88]); self.rel32(lbl); }
    pub fn jle(&mut self,  lbl: usize)  { self.bs(&[0x0F,0x8E]); self.rel32(lbl); }

    pub fn test_rax_rax(&mut self) { self.bs(&[0x48,0x85,0xC0]); }
    pub fn test_rdi_rdi(&mut self) { self.bs(&[0x48,0x85,0xFF]); }
    pub fn test_rsi_rsi(&mut self) { self.bs(&[0x48,0x85,0xF6]); }
    pub fn test_rdx_rdx(&mut self) { self.bs(&[0x48,0x85,0xD2]); }

    pub fn add_rsp(&mut self, n: u32) {
        if n < 128 { self.bs(&[0x48,0x83,0xC4,n as u8]); }
        else { self.bs(&[0x48,0x81,0xC4]); self.u32le(n); }
    }
    pub fn sub_rsp(&mut self, n: u32) {
        if n < 128 { self.bs(&[0x48,0x83,0xEC,n as u8]); }
        else { self.bs(&[0x48,0x81,0xEC]); self.u32le(n); }
    }
    pub fn prologue(&mut self) { self.bs(&[0x55,0x48,0x89,0xE5]); }
    pub fn epilogue(&mut self) { self.bs(&[0x5D,0xC3]); }

    // Val stack: push TAG first, then PAYLOAD (payload lives on top)
    pub fn push_val(&mut self) { self.push_rax(); self.push_rdx(); }  // rax=tag, rdx=pay
    pub fn pop_val_rdi_rsi(&mut self) { self.pop_rsi(); self.pop_rdi(); } // rsi=pay, rdi=tag
    pub fn pop_two_vals(&mut self) {
        // stack top→bot: rpay, rtag, lpay, ltag
        self.pop_rcx();     // rcx = rpay
        self.pop_rdx();     // rdx = rtag
        self.pop_rsi();     // rsi = lpay
        self.pop_rdi();     // rdi = ltag
    }
    pub fn peek_val_rdi_rsi(&mut self) {
        self.bs(&[0x48,0x8B,0x7C,0x24,0x08]); // rdi=[rsp+8] tag
        self.bs(&[0x48,0x8B,0x34,0x24]);       // rsi=[rsp]   pay
    }

    pub fn movq_xmm0_rsi(&mut self) { self.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); }
    pub fn movq_xmm1_rcx(&mut self) { self.bs(&[0x66,0x48,0x0F,0x6E,0xC9]); }
    pub fn movq_rdx_xmm0(&mut self) { self.bs(&[0x66,0x48,0x0F,0x7E,0xC2]); }
    pub fn addsd(&mut self)  { self.bs(&[0xF2,0x0F,0x58,0xC1]); }
    pub fn subsd(&mut self)  { self.bs(&[0xF2,0x0F,0x5C,0xC1]); }
    pub fn mulsd(&mut self)  { self.bs(&[0xF2,0x0F,0x59,0xC1]); }
    pub fn divsd(&mut self)  { self.bs(&[0xF2,0x0F,0x5E,0xC1]); }
    pub fn ucomisd(&mut self){ self.bs(&[0x66,0x0F,0x2E,0xC1]); }

    // ── OS I/O ────────────────────────────────────────────────────────────────
    // write_stdout: entry rsi=buf, rdx=len
    pub fn emit_write_stdout(&mut self) {
        match self.os {
            TargetOs::Linux => {
                self.bs(&[0x48,0xC7,0xC0,0x01,0,0,0]); // rax=1
                self.bs(&[0x48,0xC7,0xC7,0x01,0,0,0]); // rdi=1
                self.syscall();
            }
            TargetOs::Windows => {
                self.b(0x56); self.push_rdx();           // save rsi(buf), rdx(len)
                self.bs(&[0x48,0x83,0xEC,0x20]);         // sub rsp,32
                self.bs(&[0x48,0xC7,0xC1,0xF5,0xFF,0xFF,0xFF]); // rcx=-11
                self.bs(&[0x48,0xB8]); self.iat_gsh();   // movabs rax,&GSH
                self.bs(&[0xFF,0x10]);                   // call [rax]
                self.bs(&[0x48,0x83,0xC4,0x20]);         // add rsp,32
                self.bs(&[0x41,0x58]);                   // pop r8  (len)
                self.pop_rdx();                          // rdx (buf)
                self.bs(&[0x48,0x83,0xEC,0x30]);         // sub rsp,48
                self.bs(&[0x48,0x89,0xC1]);              // rcx=rax (hFile)
                self.bs(&[0x4C,0x8D,0x4C,0x24,0x28]);   // lea r9,[rsp+40]
                self.bs(&[0x48,0xC7,0x44,0x24,0x20,0,0,0,0]); // [rsp+32]=0
                self.bs(&[0x48,0xB8]); self.iat_wf();
                self.bs(&[0xFF,0x10]);
                self.bs(&[0x48,0x83,0xC4,0x30]);         // add rsp,48
            }
        }
    }

    pub fn emit_exit(&mut self) {
        match self.os {
            TargetOs::Linux => {
                self.xor_rdi_rdi();
                self.bs(&[0x48,0xC7,0xC0,0x3C,0,0,0]);
                self.syscall();
            }
            TargetOs::Windows => {
                self.bs(&[0x48,0x83,0xEC,0x20]);
                self.bs(&[0x48,0x31,0xC9]);
                self.bs(&[0x48,0xB8]); self.iat_ep();
                self.bs(&[0xFF,0x10]);
                self.bs(&[0x48,0x83,0xC4,0x20]);
                self.b(0xF4);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Runtime labels (all public for cross-module use)
// ─────────────────────────────────────────────────────────────────────────────
#[derive(Clone, Copy)]
pub struct Rt {
    pub alloc:     usize,
    pub realloc:   usize,
    pub str_len:   usize,
    pub str_dup:   usize,
    pub str_cat:   usize,
    pub num2str:   usize,
    pub bool2str:  usize,
    pub val2str:   usize,
    pub frame_new: usize,
    pub frame_set: usize,
    pub frame_get: usize,
    pub show:      usize,
    pub truthy:    usize,
    pub add:   usize, pub sub:  usize, pub mul:  usize,
    pub div:   usize, pub modf: usize,
    pub lt:    usize, pub gt:   usize, pub le:   usize, pub ge: usize,
    pub eq:    usize, pub ne:   usize,
    pub neg:   usize, pub not:  usize,
    pub list_new:  usize, pub list_push: usize,
    pub list_get:  usize, pub list_set:  usize, pub list_len: usize,
    pub map_new:   usize, pub map_set:   usize, pub map_get:  usize,
    // Extended runtime: FFI, mem, file, string ops
    pub ffi_call:  usize,
    pub mem_read:  usize, pub mem_write: usize,
    pub file_open: usize, pub file_close: usize,
    pub file_read: usize, pub file_write: usize,
    pub str_index: usize, pub str_slice:  usize,
    pub num_floor: usize, pub num_ceil:   usize, pub num_sqrt: usize,
    pub num_parse: usize,
}

// ─────────────────────────────────────────────────────────────────────────────
//  Runtime emitter
// ─────────────────────────────────────────────────────────────────────────────
pub fn emit_runtime(a: &mut Asm) -> Rt {
    macro_rules! L { () => {{ a.label() }} }
    let alloc=L!(); let realloc=L!(); let str_len=L!(); let str_dup=L!(); let str_cat=L!();
    let num2str=L!(); let bool2str=L!(); let val2str=L!();
    let frame_new=L!(); let frame_set=L!(); let frame_get=L!();
    let show=L!(); let truthy=L!();
    let add=L!(); let sub=L!(); let mul=L!(); let div=L!(); let modf=L!();
    let lt=L!(); let gt=L!(); let le=L!(); let ge=L!(); let eq=L!(); let ne=L!();
    let neg=L!(); let not=L!();
    let list_new=L!(); let list_push=L!(); let list_get=L!(); let list_set=L!(); let list_len=L!();
    let map_new=L!(); let map_set=L!(); let map_get=L!();
    // Extended
    let ffi_call=L!();
    let mem_read=L!(); let mem_write=L!();
    let file_open=L!(); let file_close=L!(); let file_read=L!(); let file_write=L!();
    let str_index=L!(); let str_slice=L!();
    let num_floor=L!(); let num_ceil=L!(); let num_sqrt=L!();
    let num_parse=L!();

    let rt = Rt { alloc,realloc,str_len,str_dup,str_cat,num2str,bool2str,val2str,
                  frame_new,frame_set,frame_get,show,truthy,
                  add,sub,mul,div,modf,lt,gt,le,ge,eq,ne,neg,not,
                  list_new,list_push,list_get,list_set,list_len,
                  map_new,map_set,map_get,
                  ffi_call,mem_read,mem_write,
                  file_open,file_close,file_read,file_write,
                  str_index,str_slice,
                  num_floor,num_ceil,num_sqrt,num_parse };

    rt_alloc(a, rt);
    rt_realloc(a, rt);
    rt_str_len(a, rt);
    rt_str_dup(a, rt);
    rt_str_cat(a, rt);
    rt_num2str(a, rt);
    rt_bool2str(a, rt);
    rt_val2str(a, rt);
    rt_frame_new(a, rt);
    rt_frame_set(a, rt);
    rt_frame_get(a, rt);
    rt_show(a, rt);
    rt_truthy(a, rt);
    rt_add(a, rt);
    rt_arith(a, rt);
    rt_cmp(a, rt);
    rt_unary(a, rt);
    rt_list_new(a, rt);
    rt_list_push(a, rt);
    rt_list_get(a, rt);
    rt_list_set(a, rt);
    rt_list_len(a, rt);
    rt_map_new(a, rt);
    rt_map_set(a, rt);
    rt_map_get(a, rt);
    // Extended runtime
    rt_ffi_call(a, rt);
    rt_mem_read(a, rt);
    rt_mem_write(a, rt);
    rt_file_open(a, rt);
    rt_file_close(a, rt);
    rt_file_read(a, rt);
    rt_file_write(a, rt);
    rt_str_index(a, rt);
    rt_str_slice(a, rt);
    rt_num_math(a, rt);
    rt_num_parse(a, rt);
    rt
}

// ─── alloc helper: alloc `sz` bytes → rax ─────────────────────────────────
fn alloc_imm(a: &mut Asm, rt: Rt, sz: u64) {
    match a.os {
        TargetOs::Linux => {
            a.mov_rdi_u64(sz);
            a.call(rt.alloc);
        }
        TargetOs::Windows => {
            a.bs(&[0x48,0x31,0xC9]);      // xor rcx,rcx
            a.mov_rdx_u64(sz);
            a.bs(&[0x41,0xB8,0x00,0x30,0x00,0x00]); // mov r8d,0x3000
            a.bs(&[0x41,0xB9,0x04,0x00,0x00,0x00]); // mov r9d,4
            a.call(rt.alloc);
        }
    }
}

// ─── rt_alloc(size) → rax ────────────────────────────────────────────────────
//   Linux: rdi=size.  Windows: rcx=NULL rdx=size r8=0x3000 r9=4 (caller sets)
fn rt_alloc(a: &mut Asm, rt: Rt) {
    a.place(rt.alloc);
    a.prologue();
    match a.os {
        TargetOs::Linux => {
            a.bs(&[0x48,0x89,0xFE]);                        // rsi=rdi (size)
            a.xor_rdi_rdi();
            a.bs(&[0x48,0xC7,0xC2,0x03,0,0,0]);            // rdx=3 PROT_RW
            a.bs(&[0x49,0xC7,0xC2,0x22,0,0,0]);            // r10=0x22 MAP_ANON|PRIV
            a.bs(&[0x49,0xC7,0xC0,0xFF,0xFF,0xFF,0xFF]);   // r8=-1
            a.bs(&[0x4D,0x31,0xC9]);                         // r9=0
            a.bs(&[0x48,0xC7,0xC0,0x09,0,0,0]);            // rax=9 (SYS_mmap)
            a.syscall();
        }
        TargetOs::Windows => {
            a.bs(&[0x48,0x83,0xEC,0x20]);
            a.bs(&[0x48,0xB8]); a.iat_va();
            a.bs(&[0xFF,0x10]);
            a.bs(&[0x48,0x83,0xC4,0x20]);
        }
    }
    a.epilogue();
}

// ─── rt_realloc(old:rdi, old_sz:rsi, new_sz:rdx) → rax ──────────────────────
fn rt_realloc(a: &mut Asm, rt: Rt) {
    a.place(rt.realloc);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56]); // push rbx,r12,r13,r14
    a.bs(&[0x48,0x89,0xFB]);  // rbx=old
    a.bs(&[0x49,0x89,0xF4]);  // r12=old_sz
    a.bs(&[0x49,0x89,0xD5]);  // r13=new_sz
    match a.os {
        TargetOs::Linux => { a.bs(&[0x4C,0x89,0xEF]); a.call(rt.alloc); }
        TargetOs::Windows => {
            a.bs(&[0x48,0x31,0xC9]); a.bs(&[0x4C,0x89,0xEA]);
            a.bs(&[0x41,0xB8,0x00,0x30,0x00,0x00]); a.bs(&[0x41,0xB9,0x04,0x00,0x00,0x00]);
            a.call(rt.alloc);
        }
    }
    a.bs(&[0x49,0x89,0xC6]);  // r14=new_ptr
    a.bs(&[0x4C,0x89,0xF7]);  // rdi=r14
    a.bs(&[0x48,0x89,0xDE]);  // rsi=rbx
    a.bs(&[0x4C,0x89,0xE1]);  // rcx=r12
    a.bs(&[0xF3,0x48,0xA4]); // rep movsb
    a.bs(&[0x4C,0x89,0xF0]);  // rax=r14
    a.bs(&[0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

// ─── rt_str_len(ptr:rdi) → rax ───────────────────────────────────────────────
fn rt_str_len(a: &mut Asm, rt: Rt) {
    a.place(rt.str_len);
    a.prologue();
    a.bs(&[0x48,0x89,0xF8]); // rax=rdi
    let lp=a.label(); let dn=a.label();
    a.place(lp);
    a.bs(&[0x80,0x38,0x00]); a.jz(dn);
    a.bs(&[0x48,0xFF,0xC0]); a.jmp(lp);
    a.place(dn);
    a.bs(&[0x48,0x29,0xF8]);  // rax-=rdi
    a.epilogue();
}

// ─── rt_str_dup(ptr:rdi) → rax ───────────────────────────────────────────────
fn rt_str_dup(a: &mut Asm, rt: Rt) {
    a.place(rt.str_dup);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55]); // push rbx,r12,r13
    a.bs(&[0x48,0x89,0xFB]);            // rbx=src
    a.call(rt.str_len);                 // rax=len (rdi=src still)
    a.bs(&[0x49,0x89,0xC4]);            // r12=len
    match a.os {
        TargetOs::Linux => { a.bs(&[0x48,0x8D,0x78,0x01]); a.call(rt.alloc); }
        TargetOs::Windows => {
            a.bs(&[0x48,0x31,0xC9]); a.bs(&[0x48,0x8D,0x50,0x01]);
            a.bs(&[0x41,0xB8,0x00,0x30,0x00,0x00]); a.bs(&[0x41,0xB9,0x04,0x00,0x00,0x00]);
            a.call(rt.alloc);
        }
    }
    a.bs(&[0x49,0x89,0xC5]);  // r13=new
    a.bs(&[0x4C,0x89,0xEF]);  // rdi=r13
    a.bs(&[0x48,0x89,0xDE]);  // rsi=rbx
    a.bs(&[0x4C,0x89,0xE1]);  // rcx=r12
    a.bs(&[0x48,0xFF,0xC1]);  // inc rcx (+NUL)
    a.bs(&[0xF3,0x48,0xA4]); // rep movsb
    a.bs(&[0x4C,0x89,0xE8]);  // rax=r13
    a.bs(&[0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

// ─── rt_str_cat(a:rdi, b:rsi) → rax ─────────────────────────────────────────
fn rt_str_cat(a: &mut Asm, rt: Rt) {
    a.place(rt.str_cat);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57]); // push rbx,r12,r13,r14,r15
    a.bs(&[0x48,0x89,0xFB]);  // rbx=a_ptr
    a.bs(&[0x49,0x89,0xF7]);  // r15=b_ptr
    // len_a
    a.call(rt.str_len);        // rdi=rbx, rax=len_a
    a.bs(&[0x49,0x89,0xC4]);  // r12=len_a
    // len_b
    a.bs(&[0x4C,0x89,0xFF]);  // rdi=r15
    a.call(rt.str_len);
    a.bs(&[0x49,0x89,0xC5]);  // r13=len_b
    // alloc len_a+len_b+1
    a.bs(&[0x4C,0x89,0xE8]);  // rax=r13
    a.bs(&[0x4C,0x01,0xE0]);  // rax+=r12
    match a.os {
        TargetOs::Linux => { a.bs(&[0x48,0x8D,0x78,0x01]); a.call(rt.alloc); }
        TargetOs::Windows => {
            a.bs(&[0x48,0x31,0xC9]); a.bs(&[0x48,0x8D,0x50,0x01]);
            a.bs(&[0x41,0xB8,0x00,0x30,0x00,0x00]); a.bs(&[0x41,0xB9,0x04,0x00,0x00,0x00]);
            a.call(rt.alloc);
        }
    }
    a.bs(&[0x49,0x89,0xC6]);  // r14=new
    // copy a_ptr (r12 bytes), dst starts at r14 (rdi after copy = r14+r12)
    a.bs(&[0x4C,0x89,0xF7]);  // rdi=r14
    a.bs(&[0x48,0x89,0xDE]);  // rsi=rbx
    a.bs(&[0x4C,0x89,0xE1]);  // rcx=r12
    a.bs(&[0xF3,0x48,0xA4]); // rep movsb  (rdi advances)
    // copy b_ptr (r13+1 bytes including NUL)
    a.bs(&[0x4C,0x89,0xFE]);  // rsi=r15
    a.bs(&[0x4C,0x89,0xE9]);  // rcx=r13
    a.bs(&[0x48,0xFF,0xC1]);  // inc rcx
    a.bs(&[0xF3,0x48,0xA4]);
    a.bs(&[0x4C,0x89,0xF0]);  // rax=r14
    a.bs(&[0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

// ─── rt_num2str(f64_bits:rdi) → rax:ptr ──────────────────────────────────────
// Converts f64 to string. Handles integers cleanly; falls back to fixed-6-decimal for floats.
fn rt_num2str(a: &mut Asm, rt: Rt) {
    a.place(rt.num2str);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57]); // push rbx,r12-r15
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]); // movq xmm0,rdi  (f64 value)

    // Check if value is a whole number (trunc(x)==x and finite)
    a.bs(&[0xF2,0x0F,0x10,0xC8]);       // movsd xmm1,xmm0
    a.bs(&[0x66,0x0F,0x3A,0x0A,0xC0,0x03]); // roundsd xmm0,xmm0,3 (truncate)
    a.bs(&[0x66,0x0F,0x2E,0xC1]);       // ucomisd xmm0,xmm1
    let is_float = a.label();
    a.jne(is_float); a.jp(is_float);   // NaN/unordered → treat as float
    // Restore xmm0 = original value
    a.bs(&[0xF2,0x0F,0x10,0xC1]);       // movsd xmm0,xmm1

    // --- Integer path ---
    a.bs(&[0xF2,0x4C,0x0F,0x2C,0xE8]); // cvttsd2si r13,xmm0
    a.bs(&[0x4D,0x31,0xE4]);            // r12=0 (neg flag)
    a.bs(&[0x4D,0x85,0xED]);            // test r13,r13
    let pos=a.label(); a.jns(pos);
    a.bs(&[0x49,0xF7,0xDD]);            // neg r13
    a.bs(&[0x41,0xFF,0xC4]);            // inc r12
    a.place(pos);
    alloc_imm(a, rt, 32);
    a.bs(&[0x48,0x89,0xC3]);            // rbx=buf
    a.bs(&[0xC6,0x43,0x1F,0x00]);       // buf[31]=0
    a.bs(&[0x48,0x8D,0x73,0x1F]);       // rsi=buf+31
    let lp=a.label(); a.place(lp);
    a.bs(&[0x4C,0x89,0xE8]);            // rax=r13
    a.xor_rdx_rdx();
    a.bs(&[0x48,0xC7,0xC1,0x0A,0,0,0]); // rcx=10
    a.bs(&[0x48,0xF7,0xF1]);            // div rcx
    a.bs(&[0x49,0x89,0xC5]);            // r13=quot
    a.bs(&[0x48,0x83,0xC2,0x30]);       // rdx+='0'
    a.bs(&[0x48,0xFF,0xCE]);            // dec rsi
    a.bs(&[0x88,0x16]);                 // [rsi]=dl
    a.bs(&[0x4D,0x85,0xED]); a.jnz(lp);
    // prepend '-' if negative
    a.bs(&[0x4D,0x85,0xE4]);
    let nosg=a.label(); a.jz(nosg);
    a.bs(&[0x48,0xFF,0xCE]);
    a.bs(&[0xC6,0x06,0x2D]);
    a.place(nosg);
    a.bs(&[0x48,0x89,0xF0]);            // rax=rsi (ptr to number string)
    let done=a.label(); a.jmp(done);

    // --- Float path: "intpart.fracpart" with up to 6 decimal places ---
    a.place(is_float);
    a.bs(&[0xF2,0x0F,0x10,0xC1]);       // movsd xmm0,xmm1 (restore)
    alloc_imm(a, rt, 48);               // buf=rax (r12,r13,r14,r15,rbx still live)
    a.bs(&[0x49,0x89,0xC7]);            // r15=buf_start
    a.bs(&[0x4C,0x89,0xF8]);            // rax=r15 (write cursor)
    // sign
    a.bs(&[0x66,0x4C,0x0F,0x7E,0xF6]); // movq r14,xmm0 (clobbers r14)
    a.bs(&[0x49,0xC1,0xED,0x3F]);       // shr r14,63
    a.bs(&[0x4D,0x85,0xF6]);
    let fnosg=a.label(); a.jz(fnosg);
    a.bs(&[0xC6,0x00,0x2D]); a.bs(&[0x48,0xFF,0xC0]); // '-'; rax++
    // negate: xor sign bit
    a.bs(&[0x48,0xB9]); a.u64le(1u64<<63);
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC9]); // movq xmm1,rcx
    a.bs(&[0x66,0x0F,0x57,0xC1]);       // xorpd xmm0,xmm1
    a.place(fnosg);
    // integer part → r13
    a.bs(&[0xF2,0x4C,0x0F,0x2C,0xE8]); // cvttsd2si r13,xmm0
    // write integer digits using stack temp (32 bytes)
    a.bs(&[0x48,0x83,0xEC,0x20]);
    a.bs(&[0x48,0x8D,0x74,0x24,0x1F]); // rsi=rsp+31 (end of temp)
    a.bs(&[0xC6,0x44,0x24,0x1F,0x00]); // [rsp+31]=NUL
    a.bs(&[0x4C,0x89,0xEB]);            // rbx=r13
    let fdigit=a.label(); a.place(fdigit);
    a.bs(&[0x48,0x89,0xD8]); a.xor_rdx_rdx();
    a.bs(&[0x48,0xC7,0xC1,0x0A,0,0,0]); a.bs(&[0x48,0xF7,0xF1]); // div 10
    a.bs(&[0x48,0x89,0xC3]);            // rbx=quot
    a.bs(&[0x48,0x83,0xC2,0x30]);
    a.bs(&[0x48,0xFF,0xCE]); a.bs(&[0x88,0x16]); // dec rsi; [rsi]=dl
    a.bs(&[0x48,0x85,0xDB]); a.jnz(fdigit);
    a.bs(&[0x48,0xFF,0xC6]); // rsi++ → first digit
    // copy int digits to rax (write cursor)
    let fcopy=a.label(); a.place(fcopy);
    a.bs(&[0x8A,0x0E]); a.bs(&[0x84,0xC9]);
    let fcopydone=a.label(); a.jz(fcopydone);
    a.bs(&[0x88,0x08]); a.bs(&[0x48,0xFF,0xC0]); a.bs(&[0x48,0xFF,0xC6]); a.jmp(fcopy);
    a.place(fcopydone);
    a.bs(&[0x48,0x83,0xC4,0x20]); // restore rsp
    // write '.'
    a.bs(&[0xC6,0x00,0x2E]); a.bs(&[0x48,0xFF,0xC0]);
    // fractional part: frac = xmm0 - f64(r13)
    a.bs(&[0xF2,0x4C,0x0F,0x2A,0xED]); // cvtsi2sd xmm5,r13
    a.bs(&[0xF2,0x0F,0x5C,0xC5]);       // subsd xmm0,xmm5 → frac
    // multiply by 1_000_000
    a.bs(&[0x48,0xB9]); a.u64le(1_000_000.0f64.to_bits());
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC9]); // movq xmm1,rcx
    a.bs(&[0xF2,0x0F,0x59,0xC1]);       // mulsd xmm0,xmm1
    a.bs(&[0xF2,0x4C,0x0F,0x2C,0xE8]); // cvttsd2si r13,xmm0 → 6-digit frac int
    // write 6 digits backwards into temp, then copy
    a.bs(&[0x48,0x83,0xEC,0x10]);
    a.bs(&[0x48,0x8D,0x74,0x24,0x06]); // rsi=rsp+6
    a.bs(&[0xC6,0x44,0x24,0x06,0x00]); // [rsp+6]=NUL
    a.bs(&[0x4C,0x89,0xEB]);            // rbx=r13
    a.bs(&[0x41,0xB9,0x06,0,0,0]);      // r9d=6 (always write 6 digits)
    let fflp=a.label(); a.place(fflp);
    a.bs(&[0x48,0x89,0xD8]); a.xor_rdx_rdx();
    a.bs(&[0x48,0xC7,0xC1,0x0A,0,0,0]); a.bs(&[0x48,0xF7,0xF1]);
    a.bs(&[0x48,0x89,0xC3]);
    a.bs(&[0x48,0x83,0xC2,0x30]);
    a.bs(&[0x48,0xFF,0xCE]); a.bs(&[0x88,0x16]);
    a.bs(&[0x49,0xFF,0xC9]); a.jnz(fflp);
    a.bs(&[0x48,0xFF,0xC6]); // rsi++ → first frac digit
    // copy 6 frac digits to rax
    a.bs(&[0x41,0xB9,0x06,0,0,0]);
    let fcp2=a.label(); a.place(fcp2);
    a.bs(&[0x8A,0x0E]); a.bs(&[0x88,0x08]);
    a.bs(&[0x48,0xFF,0xC0]); a.bs(&[0x48,0xFF,0xC6]);
    a.bs(&[0x49,0xFF,0xC9]); a.jnz(fcp2);
    a.bs(&[0x48,0x83,0xC4,0x10]); // restore rsp
    // strip trailing '0' from frac digits
    a.bs(&[0x48,0xFF,0xC8]); // dec rax → last char
    let fstrip=a.label(); a.place(fstrip);
    a.bs(&[0x80,0x38,0x30]);
    let fstripdn=a.label(); a.jne(fstripdn);
    a.bs(&[0xC6,0x00,0x00]); a.bs(&[0x48,0xFF,0xC8]); a.jmp(fstrip);
    a.place(fstripdn);
    // if current char is '.', remove it too
    a.bs(&[0x80,0x38,0x2E]);
    let fnotdot=a.label(); a.jne(fnotdot);
    a.bs(&[0xC6,0x00,0x00]); a.jmp(done);
    a.place(fnotdot);
    a.bs(&[0x48,0xFF,0xC0]); a.bs(&[0xC6,0x00,0x00]); // inc; NUL terminate
    a.bs(&[0x4C,0x89,0xF8]); // rax=r15 (return start of buf)
    a.place(done);
    a.bs(&[0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

// ─── rt_bool2str(val:rdi) → rax:static_ptr ───────────────────────────────────
fn rt_bool2str(a: &mut Asm, rt: Rt) {
    a.place(rt.bool2str);
    a.prologue();
    let lf=a.label(); let dn=a.label();
    a.test_rdi_rdi(); a.jz(lf);
    let st=a.add_str("True");  a.bs(&[0x48,0xB8]); a.rodata_ptr(st); a.jmp(dn);
    a.place(lf);
    let sf=a.add_str("False"); a.bs(&[0x48,0xB8]); a.rodata_ptr(sf);
    a.place(dn);
    a.epilogue();
}

// ─── rt_val2str(tag:rdi, pay:rsi) → rax:heap_ptr ────────────────────────────
fn rt_val2str(a: &mut Asm, rt: Rt) {
    a.place(rt.val2str);
    a.prologue();
    a.bs(&[0x53,0x41,0x54]);  // push rbx,r12
    a.bs(&[0x48,0x89,0xFB]);  // rbx=tag
    a.bs(&[0x49,0x89,0xF4]);  // r12=pay
    let lnull=a.label(); let lbool=a.label(); let lnum=a.label(); let lstr=a.label(); let dn=a.label();
    a.test_rdi_rdi(); a.jz(lnull);
    a.bs(&[0x48,0x83,0xFF,0x01]); a.je(lbool);
    a.bs(&[0x48,0x83,0xFF,0x02]); a.je(lnum);
    a.jmp(lstr);
    a.place(lnull);
    let sn=a.add_str("null"); a.bs(&[0x48,0xBF]); a.rodata_ptr(sn);
    a.call(rt.str_dup); a.jmp(dn);
    a.place(lbool);
    a.bs(&[0x4C,0x89,0xE7]); a.call(rt.bool2str);
    a.bs(&[0x48,0x89,0xC7]); a.call(rt.str_dup); a.jmp(dn);
    a.place(lnum);
    a.bs(&[0x4C,0x89,0xE7]); a.call(rt.num2str); a.jmp(dn);
    a.place(lstr);
    a.bs(&[0x4C,0x89,0xE7]); a.call(rt.str_dup);
    a.place(dn);
    a.bs(&[0x41,0x5C,0x5B]);
    a.epilogue();
}

// Frame layout: [parent:u64][n:u64][cap:u64] + N*[hash:u64][tag:u64][pay:u64]
const FRAME_INIT_CAP: u64 = 16;
const FRAME_SLOT: u64 = 24;

fn rt_frame_new(a: &mut Asm, rt: Rt) {
    // frame_new(parent:rdi) → rax
    a.place(rt.frame_new);
    a.prologue();
    a.b(0x53);                // push rbx
    a.b(0x50);
    a.bs(&[0x48,0x89,0xFB]); // rbx=parent
    alloc_imm(a, rt, 24 + FRAME_INIT_CAP * FRAME_SLOT);
    a.bs(&[0x48,0x89,0x18]);                      // [rax]=parent
    a.bs(&[0x48,0xC7,0x40,0x08,0,0,0,0]);         // [rax+8]=0
    a.bs(&[0x48,0xC7,0x40,0x10]); a.u32le(FRAME_INIT_CAP as u32);
    a.b(0x58);
    a.b(0x5B);
    a.epilogue();
}

// frame_set(frame:rdi, hash:rsi, tag:rdx, pay:rcx)
// frame_set(frame:rdi, hash:rsi, tag:rdx, pay:rcx)
fn rt_frame_set(a: &mut Asm, rt: Rt) {
    a.place(rt.frame_set);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57]); // push rbx,r12,r13,r14,r15
    a.bs(&[0x48,0x89,0xFB]); // rbx=frame
    a.bs(&[0x49,0x89,0xF4]); // r12=hash
    a.bs(&[0x49,0x89,0xD5]); // r13=tag
    a.bs(&[0x49,0x89,0xCE]); // r14=pay
    a.bs(&[0x4D,0x31,0xC9]); // r9=0  (slot index)
    a.bs(&[0x4C,0x8B,0x43,0x08]); // r8=n (used slots)
    let srch=a.label(); let apnd=a.label(); let nxt=a.label(); let dn=a.label();
    // Search for existing slot with matching hash
    a.place(srch);
    a.bs(&[0x4D,0x39,0xC1]); a.jae(apnd);  // r9 >= r8 → append
    frame_slot_r9(a);
    a.bs(&[0x4C,0x3B,0x20]); a.jne(nxt);   // hash mismatch → next slot
    // Update existing slot
    a.bs(&[0x4C,0x89,0x68,0x08]);           // [rax+8]=r13 (tag)
    a.bs(&[0x4C,0x89,0x70,0x10]);           // [rax+16]=r14 (pay)
    a.jmp(dn);
    a.place(nxt); a.bs(&[0x49,0xFF,0xC1]); a.jmp(srch); // r9++
    // Append — check capacity first
    a.place(apnd);
    a.bs(&[0x48,0x8B,0x43,0x10]); // rax=cap=[rbx+16]
    a.bs(&[0x4C,0x39,0xC0]);       // cmp rax,r8 (cap vs n)
    let has_room=a.label(); a.ja(has_room);
    // Grow: new_cap = cap*2, realloc
    a.bs(&[0x48,0xD1,0xE0]);       // rax<<=1 (new_cap)
    a.bs(&[0x49,0x89,0xC7]);       // r15=new_cap
    a.bs(&[0x48,0x8B,0x73,0x10]); // rsi=old_cap=[rbx+16]
    a.bs(&[0x48,0x6B,0xF6,0x18]); // rsi=old_cap*24
    a.bs(&[0x48,0x83,0xC6,0x18]); // rsi+=24 (old_size = header + slots)
    a.bs(&[0x4C,0x89,0xFA]);       // rdx=r15 (new_cap)
    a.bs(&[0x48,0x6B,0xD2,0x18]); // rdx=new_cap*24
    a.bs(&[0x48,0x83,0xC2,0x18]); // rdx+=24 (new_size)
    a.bs(&[0x48,0x89,0xDF]);       // rdi=rbx (old_frame)
    a.call(rt.realloc);            // rax=new_frame_ptr
    a.bs(&[0x48,0x89,0xC3]);       // rbx=new_frame
    a.bs(&[0x4C,0x89,0x7B,0x10]); // [rbx+16]=r15 (new_cap)
    a.place(has_room);
    // Append slot at r8 (current n), using r9=r8
    a.bs(&[0x4D,0x89,0xC1]);       // r9=r8
    frame_slot_r9(a);
    a.bs(&[0x4C,0x89,0x20]);       // [rax]=r12 (hash)
    a.bs(&[0x4C,0x89,0x68,0x08]); // [rax+8]=r13 (tag)
    a.bs(&[0x4C,0x89,0x70,0x10]); // [rax+16]=r14 (pay)
    a.bs(&[0x48,0xFF,0x43,0x08]); // inc [rbx+8] (n++)
    a.place(dn);
    a.bs(&[0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]); // pop r15,r14,r13,r12,rbx
    a.epilogue();
}

// frame_get(frame:rdi, hash:rsi) → tag:rax, pay:rdx
fn rt_frame_get(a: &mut Asm, rt: Rt) {
    a.place(rt.frame_get);
    a.prologue();
    a.bs(&[0x53,0x41,0x54]);
    a.bs(&[0x48,0x89,0xFB]); // rbx=frame
    a.bs(&[0x49,0x89,0xF4]); // r12=hash
    let fl=a.label(); let inner=a.label(); let inxt=a.label();
    let miss=a.label(); let nf=a.label(); let dn=a.label();
    a.place(fl);
    a.bs(&[0x48,0x85,0xDB]); a.jz(nf);    // frame==null → not found
    a.bs(&[0x4D,0x31,0xC9]);               // r9=0
    a.bs(&[0x4C,0x8B,0x43,0x08]);          // r8=[rbx+8]=n (reload per frame)
    a.place(inner);
    a.bs(&[0x4D,0x39,0xC1]); a.jae(miss); // r9>=r8 → walk to parent
    frame_slot_r9(a);
    a.bs(&[0x4C,0x3B,0x20]); a.jne(inxt); // hash mismatch
    a.bs(&[0x48,0x8B,0x50,0x10]);          // rdx=[rax+16]=pay
    a.bs(&[0x48,0x8B,0x40,0x08]);          // rax=[rax+8]=tag
    a.jmp(dn);
    a.place(inxt); a.bs(&[0x49,0xFF,0xC1]); a.jmp(inner);
    a.place(miss);
    a.bs(&[0x48,0x8B,0x1B]); a.jmp(fl);   // rbx=[rbx+0]=parent ptr; restart
    a.place(nf); a.xor_rax_rax(); a.xor_rdx_rdx();
    a.place(dn);
    a.bs(&[0x41,0x5C,0x5B]);
    a.epilogue();
}

#[inline] fn frame_slot_r9(a: &mut Asm) {
    // rax = rbx + 24 + r9*24
    a.bs(&[0x4C,0x89,0xC8]);      // rax=r9
    a.bs(&[0x48,0x6B,0xC0,0x18]); // rax*=24
    a.bs(&[0x48,0x83,0xC0,0x18]); // rax+=24
    a.bs(&[0x48,0x01,0xD8]);      // rax+=rbx
}

// ─── rt_show(tag:rdi, pay:rsi, nl:rdx) ───────────────────────────────────────
fn rt_show(a: &mut Asm, rt: Rt) {
    a.place(rt.show);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57]); // push rbx,r12-r15
    a.bs(&[0x49,0x89,0xFC]); // r12=tag
    a.bs(&[0x49,0x89,0xF5]); // r13=pay
    a.bs(&[0x49,0x89,0xD6]); // r14=nl
    let dw=a.label(); let dn=a.label();
    let lnull=a.label(); let lbool=a.label(); let lnum=a.label();
    let lstr=a.label(); let lbf=a.label(); let llist=a.label(); let lobj=a.label();

    a.bs(&[0x4D,0x85,0xE4]); a.jz(lnull);                  // tag==0 → null
    a.bs(&[0x49,0x83,0xFC,0x01]); a.je(lbool);              // tag==1 → bool
    a.bs(&[0x49,0x83,0xFC,0x02]); a.je(lnum);               // tag==2 → num
    a.bs(&[0x49,0x83,0xFC,0x03]); a.je(lstr);               // tag==3 → str
    a.bs(&[0x49,0x83,0xFC,0x07]); a.je(llist);              // tag==7 → list
    a.jmp(lobj);                                             // else → object/map/func

    a.place(lnull); { let s=a.add_str("null");  a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x04,0,0,0]); a.jmp(dw); }
    a.place(lbool); a.bs(&[0x4D,0x85,0xED]); a.jz(lbf);
    { let s=a.add_str("True");  a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x04,0,0,0]); a.jmp(dw); }
    a.place(lbf);
    { let s=a.add_str("False"); a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x05,0,0,0]); a.jmp(dw); }
    a.place(lnum); a.bs(&[0x4C,0x89,0xEF]); a.call(rt.num2str); a.bs(&[0x48,0x89,0xC6]); inline_strlen(a); a.jmp(dw);
    a.place(lobj); { let s=a.add_str("<object>"); a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x08,0,0,0]); a.jmp(dw); }

    // ── list: print "[elem, elem, ...]" ──────────────────────────────────────
    a.place(llist);
    // r13=list_ptr, r14=nl. Save nl (we suppress it for elements), use r15=loop idx
    { let s=a.add_str("["); a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x01,0,0,0]); a.emit_write_stdout(); }
    a.bs(&[0x4D,0x31,0xFF]); // r15=0 (loop index)
    let llp=a.label(); let lend=a.label();
    a.place(llp);
    a.bs(&[0x4C,0x89,0xEF]); // rdi=r13 (list)
    a.bs(&[0x48,0x8B,0x47,0x08]); // rax=[rdi+8]=len
    a.bs(&[0x4C,0x39,0xF8]); a.jae(lend); // r15>=len → done
    // print comma-space if not first
    a.bs(&[0x4D,0x85,0xFF]);
    let noc=a.label(); a.jz(noc);
    { let s=a.add_str(", "); a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x02,0,0,0]); a.emit_write_stdout(); }
    a.place(noc);
    // get slot: list + 16 + r15*16
    a.bs(&[0x4C,0x89,0xF8]); // rax=r15
    a.bs(&[0x48,0xC1,0xE0,0x04]); a.bs(&[0x48,0x83,0xC0,0x10]);
    a.bs(&[0x4C,0x01,0xEB]); // rax+=r13
    a.bs(&[0x48,0x8B,0x78,0x08]); // rdi=[rax+8]=elem_pay → rsi for show
    a.bs(&[0x48,0x8B,0x00]);  // rax=[rax]=elem_tag
    // call show(tag, pay, nl=0) recursively
    a.bs(&[0x48,0x89,0xFE]); // rsi=rdi (pay)
    a.bs(&[0x48,0x89,0xC7]); // rdi=rax (tag)
    a.bs(&[0x48,0x31,0xD2]); // rdx=0 (no newline)
    a.call(rt.show);
    a.bs(&[0x49,0xFF,0xC7]); // r15++
    a.jmp(llp);
    a.place(lend);
    { let s=a.add_str("]"); a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x01,0,0,0]); a.emit_write_stdout(); }
    // now print newline if nl was set
    a.bs(&[0x4D,0x85,0xF6]); a.jz(dn);
    { let s=a.add_str("\n"); a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x01,0,0,0]); a.emit_write_stdout(); }
    a.jmp(dn);

    a.place(lstr); a.bs(&[0x4C,0x89,0xEE]); inline_strlen(a);
    a.place(dw); a.emit_write_stdout();
    a.bs(&[0x4D,0x85,0xF6]); a.jz(dn);
    { let s=a.add_str("\n"); a.bs(&[0x48,0xBE]); a.rodata_ptr(s); a.bs(&[0x48,0xC7,0xC2,0x01,0,0,0]); a.emit_write_stdout(); }
    a.place(dn);
    a.bs(&[0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}
fn inline_strlen(a: &mut Asm) {
    // entry rsi=ptr → rdx=len (clobbers rdi)
    let lp=a.label(); let dn=a.label();
    a.bs(&[0x48,0x89,0xF7]);
    a.place(lp);
    a.bs(&[0x80,0x3F,0x00]); a.jz(dn);
    a.bs(&[0x48,0xFF,0xC7]); a.jmp(lp);
    a.place(dn);
    a.bs(&[0x48,0x29,0xF7]);
    a.bs(&[0x48,0x89,0xFA]);
}

// ─── rt_truthy ───────────────────────────────────────────────────────────────
fn rt_truthy(a: &mut Asm, rt: Rt) {
    a.place(rt.truthy);
    a.prologue();
    let r0=a.label(); let r1=a.label(); let lbool=a.label(); let lnum=a.label();
    a.test_rdi_rdi(); a.jz(r0);
    a.bs(&[0x48,0x83,0xFF,0x01]); a.je(lbool);
    a.bs(&[0x48,0x83,0xFF,0x02]); a.je(lnum);
    a.test_rsi_rsi(); a.jz(r0); a.jmp(r1);
    a.place(lbool); a.bs(&[0x48,0x89,0xF0]); a.epilogue();
    a.place(lnum);  a.test_rsi_rsi(); a.jz(r0); a.jmp(r1);
    a.place(r1); a.bs(&[0x48,0xC7,0xC0,0x01,0,0,0]); a.epilogue();
    a.place(r0); a.xor_rax_rax(); a.epilogue();
}

// ─── rt_add: num or string concat ────────────────────────────────────────────
fn rt_add(a: &mut Asm, rt: Rt) {
    // add(ltag:rdi, lpay:rsi, rtag:rdx, rpay:rcx) → tag:rax, pay:rdx
    a.place(rt.add);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57]);
    a.bs(&[0x48,0x89,0xFB]); // rbx=ltag
    a.bs(&[0x49,0x89,0xF4]); // r12=lpay
    a.bs(&[0x49,0x89,0xD5]); // r13=rtag
    a.bs(&[0x49,0x89,0xCE]); // r14=rpay
    let lstr=a.label(); let lnum=a.label(); let lnull=a.label(); let dn=a.label();
    a.bs(&[0x48,0x83,0xFB,0x03]); a.je(lstr); // ltag==STR
    a.bs(&[0x49,0x83,0xFD,0x03]); a.je(lstr); // rtag==STR
    a.bs(&[0x48,0x83,0xFB,0x02]); a.jne(lnull);
    a.bs(&[0x49,0x83,0xFD,0x02]); a.jne(lnull);
    a.place(lnum);
    // At this point: r12=lpay (f64 bits), r14=rpay (f64 bits)
    a.bs(&[0x66,0x49,0x0F,0x6E,0xC4]); // movq xmm0,r12 (lpay)
    a.bs(&[0x66,0x49,0x0F,0x6E,0xCE]); // movq xmm1,r14 (rpay)
    a.addsd();
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); a.jmp(dn);
    // string concat
    a.place(lstr);
    a.bs(&[0x48,0x89,0xDF]); // rdi=rbx (ltag)
    a.bs(&[0x4C,0x89,0xE6]); // rsi=r12 (lpay)
    a.call(rt.val2str);       // rax=left_str
    a.bs(&[0x49,0x89,0xC7]); // r15=left_str
    a.bs(&[0x4C,0x89,0xEF]); // rdi=r13 (rtag)
    a.bs(&[0x4C,0x89,0xF6]); // rsi=r14 (rpay)
    a.call(rt.val2str);       // rax=right_str
    a.bs(&[0x48,0x89,0xC6]); // rsi=right_str
    a.bs(&[0x4C,0x89,0xFF]); // rdi=r15
    a.call(rt.str_cat);
    a.bs(&[0x48,0x89,0xC2]); // rdx=rax
    a.bs(&[0x48,0xC7,0xC0,0x03,0,0,0]); a.jmp(dn);
    a.place(lnull); a.xor_rax_rax(); a.xor_rdx_rdx();
    a.place(dn);
    a.bs(&[0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

fn rt_arith(a: &mut Asm, rt: Rt) {
    fn num_op(a: &mut Asm, lbl: usize, op: impl Fn(&mut Asm)) {
        a.place(lbl); a.prologue();
        let nu=a.label(); let dn=a.label();
        a.bs(&[0x48,0x83,0xFF,0x02]); a.jne(nu);
        a.bs(&[0x48,0x83,0xFA,0x02]); a.jne(nu);
        op(a);
        a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); a.jmp(dn);
        a.place(nu); a.xor_rax_rax(); a.xor_rdx_rdx();
        a.place(dn); a.epilogue();
    }
    num_op(a, rt.sub,  |a| { a.movq_xmm0_rsi(); a.movq_xmm1_rcx(); a.subsd(); a.movq_rdx_xmm0(); });
    num_op(a, rt.mul,  |a| { a.movq_xmm0_rsi(); a.movq_xmm1_rcx(); a.mulsd(); a.movq_rdx_xmm0(); });
    num_op(a, rt.div,  |a| { a.movq_xmm0_rsi(); a.movq_xmm1_rcx(); a.divsd(); a.movq_rdx_xmm0(); });
    num_op(a, rt.modf, |a| {
        a.movq_xmm0_rsi(); a.movq_xmm1_rcx();
        a.bs(&[0xF2,0x0F,0x10,0xD0]);
        a.divsd();
        a.bs(&[0x66,0x0F,0x3A,0x0A,0xC0,0x03]);
        a.bs(&[0xF2,0x0F,0x59,0xC1]);
        a.bs(&[0xF2,0x0F,0x5C,0xD0]);
        a.bs(&[0x66,0x48,0x0F,0x7E,0xD2]);
    });
}

fn rt_cmp(a: &mut Asm, rt: Rt) {
    fn cmp_op(a: &mut Asm, lbl: usize, br: impl Fn(&mut Asm, usize, usize)) {
        a.place(lbl); a.prologue();
        let t=a.label(); let f=a.label(); let dn=a.label();
        a.movq_xmm0_rsi(); a.movq_xmm1_rcx(); a.ucomisd();
        br(a, t, f);
        a.place(t); a.bs(&[0x48,0xC7,0xC0,0x01,0,0,0,0x48,0xC7,0xC2,0x01,0,0,0]); a.jmp(dn);
        a.place(f); a.bs(&[0x48,0xC7,0xC0,0x01,0,0,0]); a.xor_rdx_rdx();
        a.place(dn); a.epilogue();
    }
    cmp_op(a, rt.lt, |a,t,f| { a.jb(t);  a.jmp(f); });
    cmp_op(a, rt.gt, |a,t,f| { a.ja(t);  a.jmp(f); });
    cmp_op(a, rt.le, |a,t,f| { a.jbe(t); a.jmp(f); });
    cmp_op(a, rt.ge, |a,t,f| { a.jae(t); a.jmp(f); });
    // eq
    a.place(rt.eq); a.prologue();
    let t=a.label(); let f=a.label(); let dn=a.label();
    let lnum=a.label(); let loth=a.label(); let nn=a.label();
    a.test_rdi_rdi(); a.jnz(nn); a.test_rdx_rdx(); a.jnz(nn); a.jmp(t);
    a.place(nn); a.bs(&[0x48,0x39,0xD7]); a.jne(f);
    a.bs(&[0x48,0x83,0xFF,0x02]); a.je(lnum); a.jmp(loth);
    a.place(lnum); a.movq_xmm0_rsi(); a.movq_xmm1_rcx(); a.ucomisd(); a.jp(f); a.jne(f); a.jmp(t);
    a.place(loth); a.bs(&[0x48,0x39,0xCE]); a.jne(f);
    a.place(t); a.bs(&[0x48,0xC7,0xC0,0x01,0,0,0,0x48,0xC7,0xC2,0x01,0,0,0]); a.jmp(dn);
    a.place(f); a.bs(&[0x48,0xC7,0xC0,0x01,0,0,0]); a.xor_rdx_rdx();
    a.place(dn); a.epilogue();
    // ne
    a.place(rt.ne); a.prologue(); a.call(rt.eq); a.bs(&[0x48,0x83,0xF2,0x01]); a.epilogue();
}

fn rt_unary(a: &mut Asm, rt: Rt) {
    a.place(rt.neg); a.prologue();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // rax=TAG_NUM
    a.bs(&[0x48,0x89,0xF2]);             // rdx=rsi (pay)
    a.mov_rcx_u64(1u64<<63);
    a.bs(&[0x48,0x31,0xCA]);             // rdx^=sign
    a.epilogue();

    a.place(rt.not); a.prologue();
    a.call(rt.truthy);
    a.bs(&[0x48,0x83,0xF0,0x01]);       // xor rax,1
    a.bs(&[0x48,0x89,0xC2]);
    a.bs(&[0x48,0xC7,0xC0,0x01,0,0,0]);
    a.epilogue();
}

// List: [cap:u64][len:u64] + N*[tag:u64][pay:u64]
fn rt_list_new(a: &mut Asm, rt: Rt) {
    a.place(rt.list_new);
    a.prologue();
    alloc_imm(a, rt, 16 + 8*16);  // cap=8 initial
    a.bs(&[0x48,0xC7,0x00,0x08,0,0,0]);  // [rax]=8 (cap)
    a.bs(&[0x48,0xC7,0x40,0x08,0,0,0,0]); // [rax+8]=0 (len)
    a.epilogue();
}

fn rt_list_push(a: &mut Asm, rt: Rt) {
    // list_push(list:rdi, tag:rsi, pay:rdx)
    a.place(rt.list_push);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56]);
    a.bs(&[0x48,0x89,0xFB]); // rbx=list
    a.bs(&[0x49,0x89,0xF4]); // r12=tag
    a.bs(&[0x49,0x89,0xD5]); // r13=pay
    a.bs(&[0x48,0x8B,0x43,0x08]); // rax=len
    a.bs(&[0x48,0x8B,0x0B]);      // rcx=cap
    a.bs(&[0x48,0x39,0xC8]);      // cmp rcx,rax
    let ng=a.label(); a.ja(ng);
    // grow
    a.bs(&[0x48,0x89,0xCE]);       // rsi=old_cap
    a.bs(&[0x48,0xD1,0xE1]);       // rcx<<=1
    a.bs(&[0x49,0x89,0xCE]);       // r14=new_cap
    a.bs(&[0x48,0x6B,0xF6,0x10]); // rsi=old_cap*16
    a.bs(&[0x48,0x83,0xC6,0x10]); // rsi+=16 (old_sz)
    a.bs(&[0x4C,0x89,0xF1]);       // rcx=r14
    a.bs(&[0x48,0x6B,0xC9,0x10]); // rcx*=16
    a.bs(&[0x48,0x83,0xC1,0x10]); // rcx+=16 (new_sz)
    a.bs(&[0x48,0x89,0xDF]);       // rdi=rbx
    a.bs(&[0x48,0x89,0xCA]);       // rdx=rcx
    a.call(rt.realloc);
    a.bs(&[0x48,0x89,0xC3]);       // rbx=new_ptr
    a.bs(&[0x4C,0x89,0xF1]);       // rcx=r14
    a.bs(&[0x48,0x89,0x0B]);       // [rbx]=new_cap
    a.place(ng);
    a.bs(&[0x48,0x8B,0x43,0x08]); // rax=len
    a.bs(&[0x48,0xC1,0xE0,0x04]); // rax<<=4
    a.bs(&[0x48,0x83,0xC0,0x10]); // rax+=16
    a.bs(&[0x48,0x01,0xD8]);       // rax+=rbx (slot)
    a.bs(&[0x4C,0x89,0x20]);       // [rax]=tag
    a.bs(&[0x4C,0x89,0x68,0x08]); // [rax+8]=pay
    a.bs(&[0x48,0xFF,0x43,0x08]); // inc [rbx+8]
    a.bs(&[0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

fn rt_list_get(a: &mut Asm, rt: Rt) {
    // list_get(list:rdi, idx_f64:rsi) → tag:rax, pay:rdx
    a.place(rt.list_get);
    a.prologue();
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); // movq xmm0,rsi
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xC8]); // cvttsd2si rcx,xmm0
    a.bs(&[0x48,0xC1,0xE1,0x04]);       // rcx<<=4
    a.bs(&[0x48,0x83,0xC1,0x10]);       // rcx+=16
    a.bs(&[0x48,0x01,0xF9]);            // rcx+=rdi (slot)
    a.bs(&[0x48,0x8B,0x51,0x08]);       // rdx=[rcx+8] (pay first)
    a.bs(&[0x48,0x8B,0x01]);            // rax=[rcx]   (tag)
    a.epilogue();
}

fn rt_list_set(a: &mut Asm, rt: Rt) {
    // list_set(list:rdi, idx_f64:rsi, tag:rdx, pay:rcx)
    a.place(rt.list_set);
    a.prologue();
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); // movq xmm0,rsi
    a.bs(&[0xF2,0x4C,0x0F,0x2C,0xC0]); // cvttsd2si r8,xmm0
    a.bs(&[0x4C,0xC1,0xE0,0x04]);       // r8<<=4
    a.bs(&[0x49,0x83,0xC0,0x10]);       // r8+=16
    a.bs(&[0x4C,0x01,0xC7]);            // rdi+=r8
    a.bs(&[0x48,0x89,0x17]);            // [rdi]=rdx (tag)
    a.bs(&[0x48,0x89,0x4F,0x08]);       // [rdi+8]=rcx (pay)
    a.epilogue();
}

fn rt_list_len(a: &mut Asm, rt: Rt) {
    // list_len(list:rdi) → rax:f64_bits_of_len
    a.place(rt.list_len);
    a.prologue();
    a.bs(&[0x48,0x8B,0x47,0x08]);       // rax=[rdi+8]
    a.bs(&[0xF2,0x48,0x0F,0x2A,0xC0]); // cvtsi2sd xmm0,rax
    a.bs(&[0x66,0x48,0x0F,0x7E,0xC0]); // movq rax,xmm0
    a.epilogue();
}

// Map: [cap:u64][len:u64] + N*[hash:u64][tag:u64][pay:u64]
fn rt_map_new(a: &mut Asm, rt: Rt) {
    a.place(rt.map_new);
    a.prologue();
    alloc_imm(a, rt, 16 + 8*24);
    a.bs(&[0x48,0xC7,0x00,0x08,0,0,0]);
    a.bs(&[0x48,0xC7,0x40,0x08,0,0,0,0]);
    a.epilogue();
}

fn rt_map_set(a: &mut Asm, rt: Rt) {
    // map_set(map:rdi, hash:rsi, tag:rdx, pay:rcx)
    // Map layout: [cap:u64][len:u64] + N*[hash:u64][tag:u64][pay:u64]
    a.place(rt.map_set);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57]); // push rbx,r12,r13,r14,r15
    a.bs(&[0x48,0x89,0xFB]); // rbx=map
    a.bs(&[0x49,0x89,0xF4]); // r12=hash
    a.bs(&[0x49,0x89,0xD5]); // r13=tag
    a.bs(&[0x49,0x89,0xCE]); // r14=pay
    a.bs(&[0x4D,0x31,0xC9]); // r9=0
    a.bs(&[0x4C,0x8B,0x43,0x08]); // r8=[rbx+8]=len
    let sr=a.label(); let ap=a.label(); let nx=a.label(); let dn=a.label();
    // Search existing slot
    a.place(sr); a.bs(&[0x4D,0x39,0xC1]); a.jae(ap); // r9>=r8 → append
    map_slot_r9(a);
    a.bs(&[0x4C,0x3B,0x20]); a.jne(nx);   // hash mismatch
    // Update
    a.bs(&[0x4C,0x89,0x68,0x08]);          // [rax+8]=r13 (tag)
    a.bs(&[0x4C,0x89,0x70,0x10]);          // [rax+16]=r14 (pay)
    a.jmp(dn);
    a.place(nx); a.bs(&[0x49,0xFF,0xC1]); a.jmp(sr);
    // Append — capacity check
    a.place(ap);
    a.bs(&[0x48,0x8B,0x03]);       // rax=cap=[rbx]
    a.bs(&[0x4C,0x39,0xC0]);       // cmp rax,r8
    let has_room=a.label(); a.ja(has_room);
    // Grow: new_cap=cap*2
    a.bs(&[0x48,0xD1,0xE0]);       // rax<<=1
    a.bs(&[0x49,0x89,0xC7]);       // r15=new_cap
    a.bs(&[0x48,0x8B,0x33]);       // rsi=old_cap=[rbx]
    a.bs(&[0x48,0x6B,0xF6,0x18]); // rsi=old_cap*24
    a.bs(&[0x48,0x83,0xC6,0x10]); // rsi+=16 (header)
    a.bs(&[0x4C,0x89,0xFA]);       // rdx=r15
    a.bs(&[0x48,0x6B,0xD2,0x18]); // rdx=new_cap*24
    a.bs(&[0x48,0x83,0xC2,0x10]); // rdx+=16 (header)
    a.bs(&[0x48,0x89,0xDF]);       // rdi=rbx
    a.call(rt.realloc);
    a.bs(&[0x48,0x89,0xC3]);       // rbx=new_ptr
    a.bs(&[0x4C,0x89,0x3B]);       // [rbx]=r15 (new_cap)
    a.place(has_room);
    a.bs(&[0x4D,0x89,0xC1]);       // r9=r8 (append at current len)
    map_slot_r9(a);
    a.bs(&[0x4C,0x89,0x20]);       // [rax]=r12 (hash)
    a.bs(&[0x4C,0x89,0x68,0x08]); // [rax+8]=r13 (tag)
    a.bs(&[0x4C,0x89,0x70,0x10]); // [rax+16]=r14 (pay)
    a.bs(&[0x48,0xFF,0x43,0x08]); // inc [rbx+8] (len++)
    a.place(dn);
    a.bs(&[0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

fn rt_map_get(a: &mut Asm, rt: Rt) {
    // map_get(map:rdi, hash:rsi) → tag:rax, pay:rdx
    // Returns TAG_NULL(0)/0 if not found.
    a.place(rt.map_get);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55]);    // push rbx,r12,r13
    a.bs(&[0x48,0x89,0xFB]);              // rbx=map
    a.bs(&[0x49,0x89,0xF4]);              // r12=hash
    a.bs(&[0x4D,0x31,0xC9]);              // r9=0
    a.bs(&[0x4C,0x8B,0x6B,0x08]);        // r13=[rbx+8]=len (search only used slots)
    let sr=a.label(); let nx=a.label(); let nf=a.label(); let dn=a.label();
    a.place(sr); a.bs(&[0x4D,0x39,0xCD]); a.jae(nf); // r9>=r13 → not found
    map_slot_r9(a);
    a.bs(&[0x4C,0x3B,0x20]); a.jne(nx);  // hash mismatch
    a.bs(&[0x48,0x8B,0x50,0x10]);         // rdx=[rax+16]=pay
    a.bs(&[0x48,0x8B,0x40,0x08]);         // rax=[rax+8]=tag
    a.jmp(dn);
    a.place(nx); a.bs(&[0x49,0xFF,0xC1]); a.jmp(sr);
    a.place(nf); a.xor_rax_rax(); a.xor_rdx_rdx();
    a.place(dn);
    a.bs(&[0x41,0x5D,0x41,0x5C,0x5B]);   // pop r13,r12,rbx
    a.epilogue();
}

#[inline] fn map_slot_r9(a: &mut Asm) {
    // rax = rbx + 16 + r9*24
    a.bs(&[0x4C,0x89,0xC8]);
    a.bs(&[0x48,0x6B,0xC0,0x18]);
    a.bs(&[0x48,0x83,0xC0,0x10]);
    a.bs(&[0x48,0x01,0xD8]);
}

// ─────────────────────────────────────────────────────────────────────────────
//  Extended Runtime: FFI, Mem, File, String ops, Math
// ─────────────────────────────────────────────────────────────────────────────

// ffi_call(func_ptr:rdi, arg_ptr:rsi, nargs:rdx) → tag:rax, pay:rdx
// Calls an arbitrary native function pointer with up to 6 integer/ptr args (Linux) or 4 (Windows).
// Arguments are taken from a DgnValue array: arg_ptr[i*16]=tag, arg_ptr[i*16+8]=pay
// Only numeric and string pays are forwarded (strings as raw ptr, nums as i64/ptr cast).
// Returns the numeric result as TAG_NUM(f64(i64(rax))).
fn rt_ffi_call(a: &mut Asm, rt: Rt) {
    a.place(rt.ffi_call);
    a.prologue();
    // Save: rbx=func_ptr, r12=arg_ptr, r13=nargs
    a.bs(&[0x53,0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57]);
    a.bs(&[0x48,0x89,0xFB]);  // rbx=func_ptr (rdi)
    a.bs(&[0x49,0x89,0xF4]);  // r12=arg_ptr  (rsi)
    a.bs(&[0x49,0x89,0xD5]);  // r13=nargs    (rdx)
    // Load up to 6 args from the array into registers per OS ABI
    // Linux  arg order: rdi,rsi,rdx,rcx,r8,r9
    // Windows arg order: rcx,rdx,r8,r9 (+ shadow space)
    // We load them backwards into regs. Use r14 as temp, r15 as index.
    a.bs(&[0x4D,0x31,0xFF]); // r15=0 (index)
    // helper: load arg[r15] pay → r14
    macro_rules! ld { ($a:expr, $dst_rex:expr, $dst_lo:expr) => {{
        $a.bs(&[0x4D,0x89,0xF8]); // rax=r15
        $a.bs(&[0x48,0xC1,0xE0,0x04]); $a.bs(&[0x48,0x83,0xC0,0x08]); // rax=rax*16+8
        $a.bs(&[0x4C,0x01,0xE0]); // rax+=r12
        $a.bs(&[$dst_rex,0x8B,$dst_lo,0x00]); // mov reg,[rax]
    }} }
    // We always load all 6 slots (unused ones will contain garbage but nargs guards the call).
    // For simplicity emit loads for all 6, caller passes exact nargs count.
    // Load arg0→rdi/rcx, arg1→rsi/rdx, arg2→rdx/r8, arg3→rcx/r9, arg4→r8, arg5→r9
    // We load in reverse so lower args end up correct after potential clobbering.
    // Strategy: load into r8-r13 first, then copy to arg regs.
    match a.os {
        TargetOs::Linux => {
            // Load into staging regs first (r8d..r13), then move to rdi,rsi,rdx,rcx,r8,r9
            // We stage in rbx(func_ptr saved), use stack scratch instead
            a.bs(&[0x48,0x83,0xEC,0x30]); // sub rsp,48 (6 slots * 8)
            for i in 0usize..6 {
                let byte_off = (i * 16 + 8) as i32;
                // load arg[i].pay → rax, then store on scratch stack
                a.bs(&[0x4C,0x89,0xE0]); // rax=r12
                if byte_off < 128 {
                    a.bs(&[0x48,0x83,0xC0]); a.b(byte_off as u8);
                } else {
                    a.bs(&[0x48,0x05]); a.u32le(byte_off as u32);
                }
                a.bs(&[0x48,0x8B,0x00]); // rax=[rax] (pay)
                let stk_off = (i * 8) as u8;
                a.bs(&[0x48,0x89,0x44,0x24]); a.b(stk_off); // [rsp+stk_off]=rax
            }
            // Now move from scratch into correct registers
            a.bs(&[0x48,0x8B,0x3C,0x24]);         // rdi=[rsp+0] (arg0)
            a.bs(&[0x48,0x8B,0x74,0x24,0x08]);    // rsi=[rsp+8] (arg1)
            a.bs(&[0x48,0x8B,0x54,0x24,0x10]);    // rdx=[rsp+16] (arg2)
            a.bs(&[0x48,0x8B,0x4C,0x24,0x18]);    // rcx=[rsp+24] (arg3)
            a.bs(&[0x4C,0x8B,0x44,0x24,0x20]);    // r8=[rsp+32] (arg4)
            a.bs(&[0x4C,0x8B,0x4C,0x24,0x28]);    // r9=[rsp+40] (arg5)
            a.bs(&[0x48,0x83,0xC4,0x30]);          // add rsp,48
            a.bs(&[0xFF,0xD3]);                    // call rbx
        }
        TargetOs::Windows => {
            // Windows: shadow space + 4 reg args: rcx,rdx,r8,r9
            a.bs(&[0x48,0x83,0xEC,0x48]); // sub rsp,72 (shadow 32 + 4 args spill 32 + align 8)
            for i in 0usize..4 {
                let byte_off = (i * 16 + 8) as i32;
                a.bs(&[0x4C,0x89,0xE0]); // rax=r12
                if byte_off < 128 {
                    a.bs(&[0x48,0x83,0xC0]); a.b(byte_off as u8);
                } else {
                    a.bs(&[0x48,0x05]); a.u32le(byte_off as u32);
                }
                a.bs(&[0x48,0x8B,0x00]);
                let stk_off = (32 + i * 8) as u8;
                a.bs(&[0x48,0x89,0x44,0x24]); a.b(stk_off);
            }
            a.bs(&[0x48,0x8B,0x4C,0x24,0x20]); // rcx=[rsp+32]
            a.bs(&[0x48,0x8B,0x54,0x24,0x28]); // rdx=[rsp+40]
            a.bs(&[0x4C,0x8B,0x44,0x24,0x30]); // r8=[rsp+48]
            a.bs(&[0x4C,0x8B,0x4C,0x24,0x38]); // r9=[rsp+56]
            a.bs(&[0xFF,0xD3]);                 // call rbx
            a.bs(&[0x48,0x83,0xC4,0x48]);      // add rsp,72
        }
    }
    // Result in rax → convert i64 to f64 → TAG_NUM
    a.bs(&[0xF2,0x48,0x0F,0x2A,0xC0]); // cvtsi2sd xmm0,rax
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // rax=TAG_NUM
    a.bs(&[0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

// mem_read(addr:rdi, size:rsi) → TAG_NUM(value)
// Reads 1/2/4/8 bytes from address; size=1,2,4,8
fn rt_mem_read(a: &mut Asm, _rt: Rt) {
    a.place(_rt.mem_read);
    a.prologue();
    // rdi=addr, rsi=size (f64 bits)
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); // movq xmm0,rsi
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xC0]); // cvttsd2si rax,xmm0 → size as int
    // Switch on size
    let s1=a.label(); let s2=a.label(); let s4=a.label(); let s8=a.label(); let dn=a.label();
    a.bs(&[0x48,0x83,0xF8,0x01]); a.je(s1);
    a.bs(&[0x48,0x83,0xF8,0x02]); a.je(s2);
    a.bs(&[0x48,0x83,0xF8,0x04]); a.je(s4);
    a.jmp(s8);
    a.place(s1); a.bs(&[0x0F,0xB6,0x07]); a.jmp(dn); // movzx eax,byte[rdi]
    a.place(s2); a.bs(&[0x0F,0xB7,0x07]); a.jmp(dn); // movzx eax,word[rdi]
    a.place(s4); a.bs(&[0x8B,0x07]);       a.jmp(dn); // mov eax,[rdi]
    a.place(s8); a.bs(&[0x48,0x8B,0x07]);             // mov rax,[rdi]
    a.place(dn);
    a.bs(&[0xF2,0x48,0x0F,0x2A,0xC0]); // cvtsi2sd xmm0,rax
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // TAG_NUM
    a.epilogue();
}

// mem_write(addr:rdi, value:rsi, size:rdx) → null
fn rt_mem_write(a: &mut Asm, _rt: Rt) {
    a.place(_rt.mem_write);
    a.prologue();
    // rdi=addr, rsi=value(f64 bits), rdx=size(f64 bits)
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); // movq xmm0,rsi
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xC8]); // cvttsd2si rcx,xmm0 → value as i64
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC2]); // movq xmm0,rdx
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xC0]); // cvttsd2si rax,xmm0 → size as int
    let s1=a.label(); let s2=a.label(); let s4=a.label(); let dn=a.label();
    a.bs(&[0x48,0x83,0xF8,0x01]); a.je(s1);
    a.bs(&[0x48,0x83,0xF8,0x02]); a.je(s2);
    a.bs(&[0x48,0x83,0xF8,0x04]); a.je(s4);
    a.bs(&[0x48,0x89,0x0F]); a.jmp(dn); // mov [rdi],rcx  (8 bytes)
    a.place(s4); a.bs(&[0x89,0x0F]); a.jmp(dn); // mov [rdi],ecx
    a.place(s2); a.bs(&[0x66,0x89,0x0F]); a.jmp(dn); // mov [rdi],cx
    a.place(s1); a.bs(&[0x88,0x0F]);
    a.place(dn);
    a.xor_rax_rax(); a.xor_rdx_rdx(); // return null
    a.epilogue();
}

// ── File I/O ──────────────────────────────────────────────────────────────────
// file_open(path_ptr:rdi, mode_ptr:rsi) → TAG_NUM(fd/handle as f64)
// mode is a C string: "r","w","a","rb","wb" etc.
// Returns -1 (as f64) on error.
fn rt_file_open(a: &mut Asm, rt: Rt) {
    a.place(rt.file_open);
    a.prologue();
    a.bs(&[0x53,0x41,0x54]); // push rbx,r12
    a.bs(&[0x48,0x89,0xFB]); // rbx=path_ptr
    a.bs(&[0x49,0x89,0xF4]); // r12=mode_ptr
    match a.os {
        TargetOs::Linux => {
            // sys_open(path:rdi, flags:rsi, mode:rdx) → fd
            // Parse mode string: 'r'=O_RDONLY(0), 'w'=O_WRONLY|O_CREAT|O_TRUNC(577), 'a'=O_WRONLY|O_CREAT|O_APPEND(1089)
            a.bs(&[0x41,0x0F,0xB6,0x04]); a.b(0x24); // movzx eax,byte[r12]  (first mode char)
            let mw=a.label(); let ma=a.label(); let mdn=a.label();
            a.bs(&[0x3C,0x77]); a.je(mw); // 'w'
            a.bs(&[0x3C,0x61]); a.je(ma); // 'a'
            // 'r' (default) — O_RDONLY=0
            a.bs(&[0x48,0x31,0xF6]); a.jmp(mdn); // rsi=0
            a.place(mw); a.mov_rsi_u64(0x241); a.jmp(mdn); // O_WRONLY|O_CREAT|O_TRUNC
            a.place(ma); a.mov_rsi_u64(0x441); // O_WRONLY|O_CREAT|O_APPEND
            a.place(mdn);
            a.bs(&[0x48,0x89,0xDF]); // rdi=rbx (path)
            a.bs(&[0x48,0xC7,0xC2,0xB4,0x01,0,0]); // rdx=0644 octal = 0x1A4... use 0644=420
            a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // rax=SYS_open(2)
            a.syscall();
            // rax = fd (negative on error → already correct as signed)
        }
        TargetOs::Windows => {
            // Use CreateFileA(path, access, share, sec, disp, flags, tmpl)
            // Simplified: use _open via msvcrt or just store 0/-1
            // For full support we'd need to import CreateFileA; for now use a sentinel
            // that can be improved. Store -1 to signal "use Rust layer".
            a.bs(&[0x48,0xC7,0xC0,0xFF,0xFF,0xFF,0xFF]); // rax=-1 (not implemented stub)
        }
    }
    // Convert rax (fd) to f64 TAG_NUM
    a.bs(&[0xF2,0x48,0x0F,0x2A,0xC0]); // cvtsi2sd xmm0,rax
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // TAG_NUM
    a.bs(&[0x41,0x5C,0x5B]);
    a.epilogue();
}

// file_close(fd_pay:rdi) → null
fn rt_file_close(a: &mut Asm, _rt: Rt) {
    a.place(_rt.file_close);
    a.prologue();
    // rdi = fd as f64 bits
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]); // movq xmm0,rdi
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xC7]); // cvttsd2si rdi,xmm0
    match a.os {
        TargetOs::Linux => {
            a.bs(&[0x48,0xC7,0xC0,0x03,0,0,0]); // rax=SYS_close(3)
            a.syscall();
        }
        TargetOs::Windows => { /* stub */ }
    }
    a.xor_rax_rax(); a.xor_rdx_rdx();
    a.epilogue();
}

// file_read(fd_pay:rdi, buf_ptr:rsi, len:rdx) → TAG_NUM(bytes_read)
fn rt_file_read(a: &mut Asm, _rt: Rt) {
    a.place(_rt.file_read);
    a.prologue();
    // rdi=fd(f64bits), rsi=buf_ptr, rdx=len(f64bits)
    a.bs(&[0x53]); // push rbx
    a.bs(&[0x48,0x89,0xF3]); // rbx=buf_ptr
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]); // movq xmm0,rdi
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xFF]); // cvttsd2si rdi,xmm0 → fd
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC2]); // movq xmm0,rdx
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xD2]); // cvttsd2si rdx,xmm0 → len
    match a.os {
        TargetOs::Linux => {
            a.bs(&[0x48,0x89,0xDE]); // rsi=rbx (buf)
            a.bs(&[0x48,0xC7,0xC0,0x00,0,0,0]); // rax=SYS_read(0)
            a.syscall();
        }
        TargetOs::Windows => { a.xor_rax_rax(); }
    }
    a.bs(&[0xF2,0x48,0x0F,0x2A,0xC0]); // cvtsi2sd xmm0,rax
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]);
    a.b(0x5B);
    a.epilogue();
}

// file_write(fd_pay:rdi, buf_ptr:rsi, len:rdx) → TAG_NUM(bytes_written)
fn rt_file_write(a: &mut Asm, _rt: Rt) {
    a.place(_rt.file_write);
    a.prologue();
    a.bs(&[0x53]); // push rbx
    a.bs(&[0x48,0x89,0xF3]); // rbx=buf_ptr
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]); // movq xmm0,rdi
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xFF]); // cvttsd2si rdi,xmm0 → fd
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC2]); // movq xmm0,rdx
    a.bs(&[0xF2,0x48,0x0F,0x2C,0xD2]); // cvttsd2si rdx,xmm0 → len
    match a.os {
        TargetOs::Linux => {
            a.bs(&[0x48,0x89,0xDE]); // rsi=rbx (buf)
            a.bs(&[0x48,0xC7,0xC0,0x01,0,0,0]); // rax=SYS_write(1)
            a.syscall();
        }
        TargetOs::Windows => { a.xor_rax_rax(); }
    }
    a.bs(&[0xF2,0x48,0x0F,0x2A,0xC0]);
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]);
    a.b(0x5B);
    a.epilogue();
}

// str_index(str_ptr:rdi, idx(f64):rsi) → TAG_NUM(char_code) or TAG_NULL if OOB
fn rt_str_index(a: &mut Asm, rt: Rt) {
    a.place(rt.str_index);
    a.prologue();
    a.bs(&[0x53,0x41,0x54]); // push rbx,r12
    a.bs(&[0x48,0x89,0xFB]); // rbx=str_ptr
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); // movq xmm0,rsi
    a.bs(&[0xF2,0x4C,0x0F,0x2C,0xE0]); // cvttsd2si r12,xmm0 → idx (i64)
    // get len
    a.bs(&[0x48,0x89,0xDF]); a.call(rt.str_len); // rax=len, rdi still=rbx
    // bounds check: if r12 < 0 or r12 >= len → null
    a.bs(&[0x4D,0x85,0xE4]); let oob1=a.label(); a.js(oob1); // idx<0
    a.bs(&[0x4C,0x39,0xE0]); let oob2=a.label(); a.jae(oob2); // idx>=len
    // load char
    a.bs(&[0x4A,0x0F,0xB6,0x04,0x23]); // movzx eax,byte[rbx+r12]
    a.bs(&[0xF2,0x48,0x0F,0x2A,0xC0]); // cvtsi2sd xmm0,rax
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // TAG_NUM
    let dn=a.label(); a.jmp(dn);
    a.place(oob1); a.place(oob2);
    a.xor_rax_rax(); a.xor_rdx_rdx(); // TAG_NULL
    a.place(dn);
    a.bs(&[0x41,0x5C,0x5B]);
    a.epilogue();
}

fn rt_str_slice(a: &mut Asm, rt: Rt) {
    // str_slice(str_ptr:rdi, start(f64):rsi, end(f64):rdx) → TAG_STR(new_ptr)
    a.place(rt.str_slice);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55]); // push rbx,r12,r13
    a.bs(&[0x48,0x89,0xFB]); // rbx=str_ptr
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); // movq xmm0,rsi
    a.bs(&[0xF2,0x4C,0x0F,0x2C,0xE0]); // cvttsd2si r12,xmm0 → start
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC2]); // movq xmm0,rdx
    a.bs(&[0xF2,0x4C,0x0F,0x2C,0xE8]); // cvttsd2si r13,xmm0 → end
    // clamp
    a.bs(&[0x4D,0x85,0xE4]); let ok_s=a.label(); a.jns(ok_s); a.bs(&[0x4D,0x31,0xE4]); a.place(ok_s);
    // get len
    a.bs(&[0x48,0x89,0xDF]); a.call(rt.str_len); // rax=len
    // if end > len: end=len
    a.bs(&[0x4C,0x39,0xE8]); let ok_e=a.label(); a.jle(ok_e); a.bs(&[0x49,0x89,0xC5]); a.place(ok_e);
    // if start >= end: return ""
    a.bs(&[0x4D,0x39,0xEC]); let nonempty=a.label(); a.jl(nonempty);
    let empty_s = a.add_str(""); a.bs(&[0x48,0xBF]); a.rodata_ptr(empty_s);
    a.call(rt.str_dup);
    a.bs(&[0x48,0x89,0xC2]); a.bs(&[0x48,0xC7,0xC0,0x03,0,0,0]);
    let sdone=a.label(); a.jmp(sdone);
    a.place(nonempty);
    // alloc (end-start+1), copy bytes
    a.bs(&[0x4C,0x89,0xE8]); // rax=r13
    a.bs(&[0x4C,0x29,0xE0]); // rax-=r12  (len of slice)
    match a.os {
        TargetOs::Linux => { a.bs(&[0x48,0x8D,0x78,0x01]); a.call(rt.alloc); }
        TargetOs::Windows => {
            a.bs(&[0x48,0x31,0xC9]); a.bs(&[0x48,0x8D,0x50,0x01]);
            a.bs(&[0x41,0xB8,0x00,0x30,0x00,0x00]); a.bs(&[0x41,0xB9,0x04,0x00,0x00,0x00]);
            a.call(rt.alloc);
        }
    }
    a.bs(&[0x49,0x89,0xC6]); // r14=new_buf (temporarily use r14... push/pop)
    a.bs(&[0x41,0x56]); // push r14
    a.bs(&[0x4C,0x89,0xF7]); // rdi=r14 (dst)
    a.bs(&[0x48,0x89,0xDE]); // rsi=rbx (src base)
    a.bs(&[0x4C,0x01,0xE6]); // rsi+=r12 (src+start)
    a.bs(&[0x4C,0x89,0xE9]); // rcx=r13
    a.bs(&[0x4C,0x29,0xE1]); // rcx-=r12 (count)
    a.bs(&[0xF3,0x48,0xA4]); // rep movsb
    a.bs(&[0xC6,0x07,0x00]); // [rdi]=0 (NUL terminate)
    a.bs(&[0x4C,0x89,0xF0]); // rax=r14 (new_buf ptr)
    a.bs(&[0x41,0x5E]); // pop r14 (restore)
    a.bs(&[0x48,0x89,0xC2]); // rdx=rax
    a.bs(&[0x48,0xC7,0xC0,0x03,0,0,0]); // TAG_STR
    a.place(sdone);
    a.bs(&[0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

// num_floor/ceil/sqrt  and  num_parse
fn rt_num_math(a: &mut Asm, rt: Rt) {
    // floor(x:rdi f64bits) → TAG_NUM
    a.place(rt.num_floor);
    a.prologue();
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]); // movq xmm0,rdi
    a.bs(&[0x66,0x0F,0x3A,0x0A,0xC0,0x01]); // roundsd xmm0,xmm0,1 (floor)
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]);
    a.epilogue();

    a.place(rt.num_ceil);
    a.prologue();
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]);
    a.bs(&[0x66,0x0F,0x3A,0x0A,0xC0,0x02]); // roundsd round=2 (ceil)
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]);
    a.epilogue();

    a.place(rt.num_sqrt);
    a.prologue();
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]);
    a.bs(&[0xF2,0x0F,0x51,0xC0]); // sqrtsd xmm0,xmm0
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]);
    a.epilogue();
}

// num_parse(str_ptr:rdi) → TAG_NUM(f64) or TAG_NULL on failure
fn rt_num_parse(a: &mut Asm, rt: Rt) {
    a.place(rt.num_parse);
    a.prologue();
    a.bs(&[0x53,0x41,0x54,0x41,0x55]); // push rbx,r12,r13
    a.bs(&[0x48,0x89,0xFB]); // rbx=ptr
    a.xor_rax_rax();          // rax=integer accumulator
    a.bs(&[0x4D,0x31,0xE4]); // r12=0 (neg flag)
    // skip leading spaces
    let sk=a.label(); a.place(sk);
    a.bs(&[0x0F,0xB6,0x0B]);             // movzx ecx,byte[rbx]
    a.bs(&[0x80,0xF9,0x20]); let nsp=a.label(); a.jne(nsp);
    a.bs(&[0x48,0xFF,0xC3]); a.jmp(sk); a.place(nsp);
    // check '-'
    a.bs(&[0x80,0xF9,0x2D]); let nosg=a.label(); a.jne(nosg);
    a.bs(&[0x41,0xFF,0xC4]); a.bs(&[0x48,0xFF,0xC3]);
    a.bs(&[0x0F,0xB6,0x0B]);
    a.place(nosg);
    // read integer digits
    let dlp=a.label(); a.place(dlp);
    a.bs(&[0x80,0xF9,0x30]); let dlt=a.label(); a.jb(dlt);
    a.bs(&[0x80,0xF9,0x39]); let dgt=a.label(); a.ja(dgt);
    a.bs(&[0x48,0x6B,0xC0,0x0A]);
    a.bs(&[0x80,0xE9,0x30]); a.bs(&[0x48,0x0F,0xB6,0xC9]); a.bs(&[0x48,0x01,0xC8]);
    a.bs(&[0x48,0xFF,0xC3]); a.bs(&[0x0F,0xB6,0x0B]); a.jmp(dlp);
    a.place(dlt); a.place(dgt);
    // convert int part to f64
    a.bs(&[0xF2,0x48,0x0F,0x2A,0xC0]); // cvtsi2sd xmm0,rax (int part)
    // r13 = frac_int accumulator; r8 = frac power (10^n divisor)
    // check for '.'
    a.bs(&[0x80,0xF9,0x2E]); let nodot=a.label(); a.jne(nodot);
    a.bs(&[0x48,0xFF,0xC3]); a.bs(&[0x0F,0xB6,0x0B]); // skip '.'
    a.bs(&[0x4D,0x31,0xED]); // r13=0 (frac_int)
    a.bs(&[0x49,0xC7,0xC0,0x01,0,0,0]); // r8=1 (power)
    // read fractional digits
    let flp=a.label(); a.place(flp);
    a.bs(&[0x80,0xF9,0x30]); let flt2=a.label(); a.jb(flt2);
    a.bs(&[0x80,0xF9,0x39]); let fgt2=a.label(); a.ja(fgt2);
    a.bs(&[0x4D,0x6B,0xED,0x0A]); // r13*=10
    a.bs(&[0x80,0xE9,0x30]); a.bs(&[0x48,0x0F,0xB6,0xC9]); a.bs(&[0x4C,0x01,0xE9]); // r13+=digit
    a.bs(&[0x4D,0x6B,0xC0,0x0A]); // r8*=10
    a.bs(&[0x48,0xFF,0xC3]); a.bs(&[0x0F,0xB6,0x0B]); a.jmp(flp);
    a.place(flt2); a.place(fgt2);
    // xmm0 += f64(r13)/f64(r8)
    a.bs(&[0xF2,0x4C,0x0F,0x2A,0xCD]); // cvtsi2sd xmm1,r13
    a.bs(&[0xF2,0x4C,0x0F,0x2A,0xD0]); // cvtsi2sd xmm2,r8
    a.bs(&[0xF2,0x0F,0x5E,0xCA]);       // divsd xmm1,xmm2
    a.bs(&[0xF2,0x0F,0x58,0xC1]);       // addsd xmm0,xmm1
    a.place(nodot);
    // apply sign
    a.bs(&[0x4D,0x85,0xE4]); let noneg=a.label(); a.jz(noneg);
    a.bs(&[0x48,0xB9]); a.u64le(1u64<<63);
    a.bs(&[0x66,0x48,0x0F,0x6E,0xC9]); a.bs(&[0x66,0x0F,0x57,0xC1]);
    a.place(noneg);
    a.movq_rdx_xmm0();
    a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]);
    a.bs(&[0x41,0x5D,0x41,0x5C,0x5B]);
    a.epilogue();
}

// ─────────────────────────────────────────────────────────────────────────────
//  AOT Compiler
// ─────────────────────────────────────────────────────────────────────────────
pub struct Aot {
    pub a: Asm,
    pub rt: Rt,
    loops:   Vec<(usize, usize)>, // (break_lbl, continue_lbl)
    returns: Vec<usize>,
    program_main: usize,
}

impl Aot {
    pub fn new(os: TargetOs) -> Self {
        let mut a = Asm::new(os);
        a.rodata.extend_from_slice(&[0u8; 8]);
        let program_main = a.label();
        a.jmp(program_main);             // 5 bytes: E9 xx xx xx xx
        let rt = emit_runtime(&mut a);
        Aot { a, rt, loops: vec![], returns: vec![], program_main }
    }

    pub fn compile(mut self, ast: &Stmt, is_lib: bool) -> Asm {
        self.a.place(self.program_main);
        let run_init = self.a.label();
        if is_lib && self.a.os == TargetOs::Windows {
            self.a.bs(&[0x48, 0x83, 0xFA, 0x01]); // cmp rdx, 1 (DLL_PROCESS_ATTACH)
            self.a.je(run_init);
            self.a.bs(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]); // mov rax, 1
            self.a.ret();
        }
        self.a.place(run_init);
        self.a.prologue();
        // push r12,r13,r14,r15,rbx  (callee-saved; r12=frame)
        self.a.bs(&[0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57,0x53]); // push r12..rbx
        self.a.bs(&[0x48,0x83,0xEC,0x08]); // sub rsp,8 (align)
        // frame_new(NULL) → r12
        match self.a.os {
            TargetOs::Linux   => { self.a.xor_rdi_rdi(); }
            TargetOs::Windows => { self.a.bs(&[0x48,0x31,0xC9]); }
        }
        self.a.call(self.rt.frame_new);
        self.a.bs(&[0x49,0x89,0xC4]); // r12=frame
        if is_lib && self.a.os == TargetOs::Windows {
            self.a.bs(&[0x48, 0xB8]); self.a.rodata_ptr(0); // mov rax, &rodata[0]
            self.a.bs(&[0x4C, 0x89, 0x20]);                 // mov [rax], r12
        }
        self.compile_stmt(ast);
        // FIX: undo the sub rsp,8 alignment before restoring callee-saved registers
        self.a.bs(&[0x48,0x83,0xC4,0x08]); // add rsp,8
        self.a.bs(&[0x5B,0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C]); // pop rbx,r15,r14,r13,r12
        self.a.b(0x5D); // pop rbp
       if is_lib && self.a.os == TargetOs::Windows {
            self.a.bs(&[0x48, 0xC7, 0xC0, 0x01, 0x00, 0x00, 0x00]); // mov rax, 1
            self.a.ret();
        } else {
            self.a.emit_exit();
        }
        self.a
    }

    // ── statement ────────────────────────────────────────────────────────────
    fn compile_stmt(&mut self, s: &Stmt) {
        match &s.node {
            StmtNode::Block(v) => {
                for s in v { if matches!(s.node, StmtNode::FuncDef(..) | StmtNode::ClassDef(..)) { self.compile_stmt(s); } }
                for s in v { if !matches!(s.node, StmtNode::FuncDef(..) | StmtNode::ClassDef(..)) { self.compile_stmt(s); } }
            }
            StmtNode::Expr(e) => { self.compile_expr(e); self.a.add_rsp(16); }
            StmtNode::ConstDecl(n, e) => {
                self.compile_expr(e);
                self.emit_store(n);
                self.a.add_rsp(16); // discard the re-pushed value (assignment is a statement here)
            }
            StmtNode::Return(opt) => {
                match opt {
                    Some(e) => {
                        self.compile_expr(e);
                        self.a.pop_rsi(); self.a.pop_rdi(); // rsi=pay, rdi=tag
                        self.a.bs(&[0x48,0x89,0xF8,0x48,0x89,0xF2]); // rax=tag, rdx=pay
                    }
                    None => { self.a.xor_rax_rax(); self.a.xor_rdx_rdx(); }
                }
                if let Some(&rl) = self.returns.last() {
                    self.a.jmp(rl);
                } else {
                    // Top-level return (outside a function) — undo alignment and exit
                    self.a.bs(&[0x48,0x83,0xC4,0x08]); // add rsp,8
                    self.a.bs(&[0x5B,0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C]);
                    self.a.b(0x5D);
                    self.a.emit_exit();
                }
            }
            StmtNode::IfChain(chain) => { self.compile_if(chain); }
            StmtNode::While(cond, body) => {
                let top=self.a.label(); let end=self.a.label();
                self.a.place(top);
                self.compile_expr(cond); self.a.pop_val_rdi_rsi();
                self.a.call(self.rt.truthy); self.a.test_rax_rax(); self.a.jz(end);
                self.loops.push((end, top));
                self.compile_stmt(body);
                self.loops.pop();
                self.a.jmp(top); self.a.place(end);
            }
            StmtNode::DoWhile(body, cond) => {
                let top=self.a.label(); let end=self.a.label();
                self.a.place(top);
                self.loops.push((end, top));
                self.compile_stmt(body);
                self.loops.pop();
                self.compile_expr(cond); self.a.pop_val_rdi_rsi();
                self.a.call(self.rt.truthy); self.a.test_rax_rax(); self.a.jnz(top);
                self.a.place(end);
            }
            StmtNode::For(var, iter, body) => { self.compile_for(var, iter, body); }
            StmtNode::CFor(init, cond, step, body) => { self.compile_cfor(init.as_ref(), cond.as_ref(), step.as_ref(), body); }
            StmtNode::Break    => { if let Some(&(b,_))=self.loops.last() { self.a.jmp(b); } }
            StmtNode::Continue => { if let Some(&(_,c))=self.loops.last() { self.a.jmp(c); } }
            StmtNode::FuncDef(name, params, body, _)   => { self.compile_func(name, params, body); }
            StmtNode::ClassDef(name, parent, body)      => { self.compile_class(name, parent.as_deref(), body); }
            StmtNode::Switch(val, cases, default)        => { self.compile_switch(val, cases, default.as_deref()); }
            StmtNode::Try(tb, _, cb, fb) => {
                self.compile_stmt(tb);
                if let Some(f) = fb { self.compile_stmt(f); }
            }
            StmtNode::Raise(_) => {
                let msg=self.a.add_str("raise\n");
                self.a.bs(&[0x48,0xBE]); self.a.rodata_ptr(msg);
                self.a.bs(&[0x48,0xC7,0xC2,0x06,0,0,0]);
                self.a.emit_write_stdout();
                self.a.emit_exit();
            }
            _ => {}
        }
    }

    fn compile_if(&mut self, chain: &[(Option<Expr>, Stmt)]) {
        let end=self.a.label();
        let mut nexts: Vec<usize> = Vec::new();
        for (i, (copt, body)) in chain.iter().enumerate() {
            if i > 0 { if let Some(&nl)=nexts.last() { if self.a.labels[nl].is_none() { self.a.place(nl); } } }
            if let Some(cond) = copt {
                self.compile_expr(cond);
                self.a.pop_val_rdi_rsi(); self.a.call(self.rt.truthy);
                self.a.test_rax_rax();
                let nl=self.a.label(); nexts.push(nl);
                self.a.jz(nl);
                self.compile_stmt(body); self.a.jmp(end);
            } else {
                nexts.push(self.a.label());
                self.compile_stmt(body); self.a.jmp(end);
            }
        }
        if let Some(&last)=nexts.last() { if self.a.labels[last].is_none() { self.a.place(last); } }
        self.a.place(end);
    }

    fn compile_switch(&mut self, val: &Expr, cases: &[(Expr, Stmt)], default: Option<&Stmt>) {
        let end=self.a.label();
        self.compile_expr(val); // [tag][pay] on stack
        for (cv, body) in cases {
            let next=self.a.label();
            self.a.peek_val_rdi_rsi(); // rdi=tag, rsi=pay (switch val)
            self.compile_expr(cv);
            self.a.pop_rcx(); self.pop_rdx_safe(); // rcx=case_pay, rdx=case_tag
            self.a.call(self.rt.eq);
            self.a.test_rdx_rdx(); self.a.jz(next);
            self.a.add_rsp(16);
            self.compile_stmt(body); self.a.jmp(end);
            self.a.place(next);
        }
        self.a.add_rsp(16);
        if let Some(d) = default { self.compile_stmt(d); }
        self.a.place(end);
    }

    fn pop_rdx_safe(&mut self) { self.a.pop_rdx(); }

    fn compile_for(&mut self, var: &str, iter: &Expr, body: &Stmt) {
        self.compile_expr(iter);
        // stack: [tag][pay(top=f64 bound)]
        self.a.bs(&[0x41,0x5F]); // pop r15=pay (bound as f64 bits)
        self.a.add_rsp(8);        // discard tag
        self.a.bs(&[0x66,0x49,0x0F,0x6E,0xC7]); // movq xmm0,r15
        self.a.bs(&[0xF2,0x4C,0x0F,0x2C,0xC0]); // cvttsd2si r8,xmm0  (r8=bound as int)
        self.a.bs(&[0x48,0x31,0xDB]);             // rbx=0  (counter)
        let top=self.a.label(); let cont=self.a.label(); let end=self.a.label();
        self.a.place(top);
        self.a.bs(&[0x4C,0x39,0xC3]); self.a.jae(end); // cmp rbx,r8; jae end
        // convert rbx to f64 and store in var
        self.a.bs(&[0xF2,0x48,0x0F,0x2A,0xC3]); // cvtsi2sd xmm0,rbx
        self.a.bs(&[0x66,0x48,0x0F,0x7E,0xC1]); // movq rcx,xmm0  (pay)
        self.a.bs(&[0x4C,0x89,0xE7]);            // rdi=r12
        self.a.mov_rsi_u64(fnv1a(var));
        self.a.bs(&[0x48,0xC7,0xC2,0x02,0,0,0]); // rdx=TAG_NUM
        self.a.call(self.rt.frame_set);
        // cont label = increment point (continue jumps here, not to top)
        self.loops.push((end, cont));
        self.compile_stmt(body);
        self.loops.pop();
        self.a.place(cont);
        self.a.bs(&[0x48,0xFF,0xC3]); // inc rbx
        self.a.jmp(top);
        self.a.place(end);
    }

    fn compile_cfor(&mut self, init: Option<&Expr>, cond: Option<&Expr>, step: Option<&Expr>, body: &Stmt) {
        if let Some(i) = init { self.compile_expr(i); self.a.add_rsp(16); }
        let top=self.a.label(); let end=self.a.label(); let stp=self.a.label();
        self.a.place(top);
        if let Some(c) = cond {
            self.compile_expr(c); self.a.pop_val_rdi_rsi();
            self.a.call(self.rt.truthy); self.a.test_rax_rax(); self.a.jz(end);
        }
        self.loops.push((end, stp));
        self.compile_stmt(body);
        self.loops.pop();
        self.a.place(stp);
        if let Some(s) = step { self.compile_expr(s); self.a.add_rsp(16); }
        self.a.jmp(top); self.a.place(end);
    }

    // ── expression ───────────────────────────────────────────────────────────
    fn compile_expr(&mut self, e: &Expr) {
        match &e.node {
            ExprNode::Null   => { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
            ExprNode::This   => { self.emit_load("this"); }
            ExprNode::Bool(b)=> {
                self.a.bs(&[0x48,0xC7,0xC0,0x01,0,0,0]); self.a.push_rax();
                self.a.mov_rax_u64(if *b{1}else{0}); self.a.push_rax();
            }
            ExprNode::Number(n) => {
                self.a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); self.a.push_rax();
                self.a.mov_rax_u64(n.to_bits()); self.a.push_rax();
            }
            ExprNode::String(s) => {
                let off=self.a.add_str(s);
                self.a.bs(&[0x48,0xC7,0xC0,0x03,0,0,0]); self.a.push_rax();
                self.a.bs(&[0x48,0xB8]); self.a.rodata_ptr(off); self.a.push_rax();
            }
            ExprNode::Var(n)    => { self.emit_load(n); }
            ExprNode::Assign(t, v) => { self.compile_assign(t, v); }
            ExprNode::Binary(op, l, r) => { self.compile_binary(op, l, r); }
            ExprNode::Unary(op, e) => {
                self.compile_expr(e); self.a.pop_val_rdi_rsi();
                let lbl = if op=="-" { self.rt.neg } else { self.rt.not };
                self.a.call(lbl); self.a.push_val();
            }
            ExprNode::Ternary(c,t,f) => {
                let el=self.a.label(); let en=self.a.label();
                self.compile_expr(c); self.a.pop_val_rdi_rsi();
                self.a.call(self.rt.truthy); self.a.test_rax_rax(); self.a.jz(el);
                self.compile_expr(t); self.a.jmp(en);
                self.a.place(el); self.compile_expr(f);
                self.a.place(en);
            }
            ExprNode::Call(f, args) => { self.compile_call(f, args); }
            ExprNode::List(items)   => { self.compile_list_literal(items); }
            ExprNode::Obj(pairs)    => { self.compile_obj_literal(pairs); }
            ExprNode::Index(obj, idx) => { self.compile_index(obj, idx); }
            ExprNode::GetAttr(obj, attr) => { self.compile_getattr(obj, attr); }
            ExprNode::FuncExpr(params, body) => {
                let name = format!("__lam_{}__", self.a.pos());
                self.compile_func(&name, params, body);
                self.emit_load(&name);
            }
            ExprNode::New(cls, args) => { self.compile_new(cls, args); }
            ExprNode::PreOp(op, tgt) => { self.compile_preop(op, tgt); }
            ExprNode::PostOp(op, tgt) => { self.compile_postop(op, tgt); }
            ExprNode::Await(inner)  => { self.compile_expr(inner); }
            ExprNode::Super(method, args) => { self.compile_super(method, args); }
            _ => { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
        }
    }

    fn compile_assign(&mut self, tgt: &Expr, val: &Expr) {
        match &tgt.node {
            ExprNode::Var(n) => {
                self.compile_expr(val);
                self.emit_store(n);
            }
            ExprNode::Index(obj, idx) => {
                self.compile_expr(obj);
                self.compile_expr(idx);
                self.compile_expr(val);
                // stack top→bot: val_pay, val_tag, idx_pay, idx_tag, obj_pay, obj_tag
                self.a.pop_rcx();  // rcx=val_pay
                self.a.pop_rdx();  // rdx=val_tag
                self.a.pop_rsi();  // rsi=idx_pay (f64 index)
                self.a.add_rsp(8); // discard idx_tag
                self.a.pop_rdi();  // rdi=obj_pay (list ptr)
                self.a.add_rsp(8); // discard obj_tag
                self.a.call(self.rt.list_set);
                // list_set returns nothing useful; push null as expression result
                self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
            }
            ExprNode::GetAttr(obj, attr) => {
                self.compile_expr(obj);
                self.compile_expr(val);
                // stack top→bot: val_pay, val_tag, obj_pay, obj_tag
                self.a.pop_rcx();  // rcx=val_pay
                self.a.pop_rdx();  // rdx=val_tag
                self.a.pop_rsi();  // rsi=obj_pay (map ptr)
                self.a.add_rsp(8); // discard obj_tag
                self.a.bs(&[0x48,0x89,0xF7]); // rdi=rsi (obj map ptr)
                self.a.mov_rsi_u64(fnv1a(attr));
                self.a.call(self.rt.map_set);
                self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
            }
            _ => { self.compile_expr(val); }
        }
    }

    fn compile_compound_assign(&mut self, tgt: &Expr, op: &str, val: &Expr) {
        // Load LHS, compute LHS op RHS, store back
        match &tgt.node {
            ExprNode::Var(n) => {
                self.emit_load(n);          // push [tag][pay]
                self.compile_expr(val);     // push [tag][pay]
                self.a.pop_two_vals();      // rdi=ltag,rsi=lpay,rdx=rtag,rcx=rpay
                let lbl = match op {
                    "+=" => self.rt.add,  "-=" => self.rt.sub,
                    "*=" => self.rt.mul,  "/=" => self.rt.div,
                    "%=" => self.rt.modf, _   => self.rt.add,
                };
                self.a.call(lbl);
                self.a.push_val();          // push result [tag][pay]
                self.emit_store(n);
            }
            ExprNode::Index(obj, idx) => {
                // read-modify-write: load old value, apply op, store back
                self.compile_expr(obj);
                self.compile_expr(idx);
                // peek obj_pay and idx_pay for the get
                self.a.bs(&[0x48,0x8B,0x7C,0x24,0x18]); // rdi=[rsp+24]=obj_pay
                self.a.bs(&[0x48,0x8B,0x74,0x24,0x08]); // rsi=[rsp+8]=idx_pay
                self.a.call(self.rt.list_get);            // rax=tag, rdx=pay (old)
                self.a.push_val();
                self.compile_expr(val);
                self.a.pop_two_vals();
                let lbl = match op {
                    "+=" => self.rt.add, "-=" => self.rt.sub,
                    "*=" => self.rt.mul, "/=" => self.rt.div,
                    "%=" => self.rt.modf, _ => self.rt.add,
                };
                self.a.call(lbl);           // rax=new_tag, rdx=new_pay
                // stack still has [idx_pay][idx_tag][obj_pay][obj_tag]
                self.a.bs(&[0x48,0x89,0xD1]); // rcx=rdx (new_pay)
                self.a.bs(&[0x48,0x89,0xC2]); // rdx=rax (new_tag)
                self.a.pop_rsi();              // rsi=idx_pay
                self.a.add_rsp(8);
                self.a.pop_rdi();              // rdi=obj_pay
                self.a.add_rsp(8);
                self.a.call(self.rt.list_set);
                self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
            }
            ExprNode::GetAttr(obj, attr) => {
                self.compile_expr(obj);
                // peek obj_pay for get
                self.a.bs(&[0x48,0x8B,0x3C,0x24]); // rdi=[rsp]=obj_pay
                self.a.mov_rsi_u64(fnv1a(attr));
                self.a.call(self.rt.map_get);        // rax=tag, rdx=old_pay
                self.a.push_val();
                self.compile_expr(val);
                self.a.pop_two_vals();
                let lbl = match op {
                    "+=" => self.rt.add, "-=" => self.rt.sub,
                    "*=" => self.rt.mul, "/=" => self.rt.div,
                    "%=" => self.rt.modf, _ => self.rt.add,
                };
                self.a.call(lbl);
                // rax=new_tag, rdx=new_pay; stack has [obj_pay][obj_tag]
                self.a.bs(&[0x48,0x89,0xD1]); // rcx=rdx (new_pay)
                self.a.bs(&[0x48,0x89,0xC2]); // rdx=rax (new_tag)
                self.a.pop_rdi();              // rdi=obj_pay
                self.a.add_rsp(8);             // discard obj_tag
                self.a.bs(&[0x48,0x89,0xF7]); // rdi already set
                self.a.mov_rsi_u64(fnv1a(attr));
                self.a.call(self.rt.map_set);
                self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
            }
            _ => { self.compile_expr(val); }
        }
    }

    fn compile_binary(&mut self, op: &str, l: &Expr, r: &Expr) {
        // Compound assignment operators
        match op {
            "+=" | "-=" | "*=" | "/=" | "%=" => {
                self.compile_compound_assign(l, op, r);
                return;
            }
            _ => {}
        }
        match op {
            "and" => {
                let dn=self.a.label();
                self.compile_expr(l);
                self.a.peek_val_rdi_rsi(); self.a.call(self.rt.truthy);
                self.a.test_rax_rax(); self.a.jz(dn);
                self.a.add_rsp(16); self.compile_expr(r);
                self.a.place(dn); return;
            }
            "or" => {
                let dn=self.a.label();
                self.compile_expr(l);
                self.a.peek_val_rdi_rsi(); self.a.call(self.rt.truthy);
                self.a.test_rax_rax(); self.a.jnz(dn);
                self.a.add_rsp(16); self.compile_expr(r);
                self.a.place(dn); return;
            }
            "??" => {
                let dn=self.a.label();
                self.compile_expr(l);
                self.a.peek_val_rdi_rsi();
                self.a.test_rdi_rdi(); self.a.jnz(dn);
                self.a.add_rsp(16); self.compile_expr(r);
                self.a.place(dn); return;
            }
            _ => {}
        }
        self.compile_expr(l); self.compile_expr(r);
        self.a.pop_two_vals(); // rdi=ltag,rsi=lpay,rdx=rtag,rcx=rpay
        let lbl = match op {
            "+"=>self.rt.add, "-"=>self.rt.sub, "*"=>self.rt.mul,
            "/"=>self.rt.div, "%"=>self.rt.modf,
            "=="=>self.rt.eq, "!="=>self.rt.ne,
            "<"=>self.rt.lt,  ">"=>self.rt.gt,
            "<="=>self.rt.le, ">="=>self.rt.ge,
            _=>self.rt.add,
        };
        self.a.call(lbl);
        self.a.push_val();
    }

    fn compile_call(&mut self, func: &Expr, args: &[Expr]) {
        // Built-ins
        if let ExprNode::Var(name) = &func.node {
            match name.as_str() {
                "show"|"showln" => {
                    let nl = if name=="showln" {1u32} else {0u32};
                    for arg in args {
                        self.compile_expr(arg);
                        self.a.pop_val_rdi_rsi();
                        self.a.bs(&[0x48,0xC7,0xC2]); self.a.u32le(nl);
                        self.a.call(self.rt.show);
                    }
                    self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                    return;
                }
                "exit" => {
                    if let Some(arg)=args.first() {
                        self.compile_expr(arg);
                        self.a.pop_rsi(); self.a.add_rsp(8);
                        self.a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]);
                        match self.a.os {
                            TargetOs::Linux   => { self.a.bs(&[0xF2,0x48,0x0F,0x2C,0xF8]); }
                            TargetOs::Windows => { self.a.bs(&[0xF2,0x48,0x0F,0x2C,0xC8]); }
                        }
                    } else {
                        match self.a.os {
                            TargetOs::Linux   => { self.a.xor_rdi_rdi(); }
                            TargetOs::Windows => { self.a.bs(&[0x48,0x31,0xC9]); }
                        }
                    }
                    self.a.emit_exit();
                    self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                    return;
                }
                "len" => {
                    if let Some(arg)=args.first() {
                        self.compile_expr(arg);
                        // stack: [tag][pay(top)]
                        self.a.pop_rdi();  // rdi=pay (list/map ptr)
                        self.a.add_rsp(8); // discard tag
                        self.a.call(self.rt.list_len); // rax=f64_bits_of_len
                        self.a.bs(&[0x49,0x89,0xC7]); // r15=f64_bits (save)
                        self.a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); self.a.push_rax(); // push TAG_NUM
                        self.a.bs(&[0x4C,0x89,0xF8]); self.a.push_rax();             // push f64 pay
                    } else {
                        self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                    }
                    return;
                }
                // ── Math builtins ───────────────────────────────────────────
                "floor" | "math_floor" => {
                    if let Some(a) = args.first() {
                        self.compile_expr(a);
                        self.a.pop_rdi(); self.a.add_rsp(8); // rdi=pay(f64)
                        self.a.call(self.rt.num_floor);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                "ceil" | "math_ceil" => {
                    if let Some(a) = args.first() {
                        self.compile_expr(a);
                        self.a.pop_rdi(); self.a.add_rsp(8);
                        self.a.call(self.rt.num_ceil);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                "sqrt" | "math_sqrt" => {
                    if let Some(a) = args.first() {
                        self.compile_expr(a);
                        self.a.pop_rdi(); self.a.add_rsp(8);
                        self.a.call(self.rt.num_sqrt);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                "parseInt" | "parseFloat" | "parse_num" => {
                    if let Some(a) = args.first() {
                        self.compile_expr(a);
                        self.a.pop_rdi(); self.a.add_rsp(8); // rdi=str_ptr
                        self.a.call(self.rt.num_parse);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                // ── String builtins ─────────────────────────────────────────
                "charCodeAt" | "char_code" => {
                    // charCodeAt(str, idx)
                    if args.len() >= 2 {
                        self.compile_expr(&args[0]);
                        self.compile_expr(&args[1]);
                        self.a.pop_rsi(); self.a.add_rsp(8); // rsi=idx(f64)
                        self.a.pop_rdi(); self.a.add_rsp(8); // rdi=str_ptr
                        self.a.call(self.rt.str_index);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                "substr" | "slice" | "substring" => {
                    // slice(str, start, end)
                    if args.len() >= 3 {
                        self.compile_expr(&args[0]);
                        self.compile_expr(&args[1]);
                        self.compile_expr(&args[2]);
                        self.a.pop_rdx(); self.a.add_rsp(8); // rdx=end(f64)
                        self.a.pop_rsi(); self.a.add_rsp(8); // rsi=start(f64)
                        self.a.pop_rdi(); self.a.add_rsp(8); // rdi=str_ptr
                        self.a.call(self.rt.str_slice);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                // ── FFI builtins ─────────────────────────────────────────────
                // ffi_call(func_ptr_num, arg1, arg2, ...)
                "ffi_call" | "__ffi" => {
                    if args.is_empty() {
                        self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                        return;
                    }
                    // Compile all arguments first (they end up as DgnValues on stack)
                    // Compile func_ptr (first arg)
                    self.compile_expr(&args[0]);
                    self.a.pop_rsi(); self.a.add_rsp(8); // rsi=func_ptr as f64 bits
                    self.a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); // movq xmm0,rsi
                    self.a.bs(&[0xF2,0x49,0x0F,0x2C,0xFF]); // cvttsd2si r15,xmm0 → func_ptr int
                    // Build arg DgnValue array on the stack
                    let nargs = args.len() - 1;
                    if nargs > 0 {
                        self.a.sub_rsp((nargs * 16) as u32);
                        // compile each arg and write tag+pay into the array
                        for i in 0..nargs {
                            self.compile_expr(&args[i + 1]);
                            // stack top: pay, then tag
                            self.a.pop_rax(); // pay
                            let pay_off = (i * 16) as u32;
                            let tag_off = pay_off + 8;
                            if pay_off < 128 {
                                self.a.bs(&[0x48,0x89,0x44,0x24]); self.a.b(pay_off as u8);
                            } else {
                                self.a.bs(&[0x48,0x89,0x84,0x24]); self.a.u32le(pay_off);
                            }
                            self.a.pop_rax(); // tag
                            if tag_off < 128 {
                                self.a.bs(&[0x48,0x89,0x44,0x24]); self.a.b(tag_off as u8);
                            } else {
                                self.a.bs(&[0x48,0x89,0x84,0x24]); self.a.u32le(tag_off);
                            }
                        }
                        self.a.bs(&[0x4C,0x89,0xFF]);          // rdi=r15 (func_ptr)
                        self.a.bs(&[0x48,0x89,0xE6]);           // rsi=rsp (arg array)
                        self.a.mov_rdx_u64(nargs as u64);       // rdx=nargs
                        self.a.call(self.rt.ffi_call);
                        self.a.add_rsp((nargs * 16) as u32);
                    } else {
                        self.a.bs(&[0x4C,0x89,0xFF]);  // rdi=r15
                        self.a.xor_rsi_rsi();
                        self.a.xor_rdx_rdx();
                        self.a.call(self.rt.ffi_call);
                    }
                    self.a.push_val();
                    return;
                }
                // ── Memory builtins ──────────────────────────────────────────
                "mem_read" | "__mem_read" => {
                    if args.len() >= 2 {
                        self.compile_expr(&args[0]); // addr
                        self.compile_expr(&args[1]); // size
                        self.a.pop_rsi(); self.a.add_rsp(8); // rsi=size(f64)
                        self.a.pop_rdi(); self.a.add_rsp(8); // rdi=addr(f64→ptr)
                        // convert addr f64 → i64
                        self.a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]); // movq xmm0,rdi
                        self.a.bs(&[0xF2,0x48,0x0F,0x2C,0xFF]); // cvttsd2si rdi,xmm0
                        self.a.call(self.rt.mem_read);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                "mem_write" | "__mem_write" => {
                    if args.len() >= 3 {
                        self.compile_expr(&args[0]); // addr
                        self.compile_expr(&args[1]); // value
                        self.compile_expr(&args[2]); // size
                        self.a.pop_rdx(); self.a.add_rsp(8); // rdx=size(f64)
                        self.a.pop_rsi(); self.a.add_rsp(8); // rsi=value(f64)
                        self.a.pop_rdi(); self.a.add_rsp(8); // rdi=addr(f64)
                        self.a.bs(&[0x66,0x48,0x0F,0x6E,0xC7]);
                        self.a.bs(&[0xF2,0x48,0x0F,0x2C,0xFF]); // rdi=addr as ptr
                        self.a.call(self.rt.mem_write);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                // ── File builtins ────────────────────────────────────────────
                "file_open" | "open" => {
                    if args.len() >= 2 {
                        self.compile_expr(&args[0]); // path (str)
                        self.compile_expr(&args[1]); // mode (str)
                        self.a.pop_rsi(); self.a.add_rsp(8); // rsi=mode_ptr
                        self.a.pop_rdi(); self.a.add_rsp(8); // rdi=path_ptr
                        self.a.call(self.rt.file_open);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                "file_close" | "close" => {
                    if let Some(a) = args.first() {
                        self.compile_expr(a);
                        self.a.pop_rdi(); self.a.add_rsp(8); // rdi=fd pay
                        self.a.call(self.rt.file_close);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                "file_read" | "fread" => {
                    if args.len() >= 3 {
                        self.compile_expr(&args[0]); // fd
                        self.compile_expr(&args[1]); // buf (ptr as num)
                        self.compile_expr(&args[2]); // len
                        self.a.pop_rdx(); self.a.add_rsp(8);
                        self.a.pop_rsi(); self.a.add_rsp(8);
                        self.a.pop_rdi(); self.a.add_rsp(8);
                        self.a.call(self.rt.file_read);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                "file_write" | "fwrite" => {
                    if args.len() >= 3 {
                        self.compile_expr(&args[0]);
                        self.compile_expr(&args[1]);
                        self.compile_expr(&args[2]);
                        self.a.pop_rdx(); self.a.add_rsp(8);
                        self.a.pop_rsi(); self.a.add_rsp(8);
                        self.a.pop_rdi(); self.a.add_rsp(8);
                        self.a.call(self.rt.file_write);
                        self.a.push_val();
                    } else { self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); }
                    return;
                }
                _ => {}
            }
        }

        // Method call obj.method(args)
        if let ExprNode::GetAttr(obj_e, mname) = &func.node {
            self.compile_method(obj_e, mname, args);
            return;
        }

        // User function call
        for arg in args { self.compile_expr(arg); }
        let nargs = args.len();
        self.compile_expr(func);
        self.a.pop_rsi(); self.a.add_rsp(8); // rsi=func_ptr (pay), discard tag
        self.a.bs(&[0xFF,0xD6]); // call rsi
        if nargs > 0 { self.a.add_rsp((nargs * 16) as u32); }
        self.a.push_val();
    }

    fn compile_method(&mut self, obj_e: &Expr, method: &str, args: &[Expr]) {
        self.compile_expr(obj_e); // stack: [obj_tag][obj_pay(top)]
        match method {
            // ── list mutators ────────────────────────────────────────────────
            "push"|"append" => {
                // obj_pay = list ptr, stays on stack while we push each arg
                for arg in args {
                    self.compile_expr(arg);             // [arg_tag][arg_pay(top)]
                    self.a.pop_rdx();                   // rdx = arg_pay
                    self.a.pop_rsi();                   // rsi = arg_tag
                    self.a.bs(&[0x48,0x8B,0x3C,0x24]); // rdi = [rsp] = obj_pay (list)
                    self.a.call(self.rt.list_push);     // list_push(list, tag, pay)
                }
                self.a.add_rsp(16);                     // discard obj
                self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); // null
            }
            "pop" => {
                self.a.pop_rdi(); self.a.add_rsp(8);    // rdi=list, drop tag
                self.a.bs(&[0x48,0x8B,0x47,0x08]);     // rax=[rdi+8]=len
                let ok=self.a.label(); let done=self.a.label();
                self.a.test_rax_rax(); self.a.jnz(ok);
                self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                self.a.jmp(done);
                self.a.place(ok);
                self.a.bs(&[0x48,0xFF,0x4F,0x08]);     // dec [rdi+8]
                self.a.bs(&[0x48,0xFF,0xC8]);           // dec rax → last idx
                self.a.bs(&[0x48,0xC1,0xE0,0x04]);     // rax<<=4
                self.a.bs(&[0x48,0x83,0xC0,0x10]);     // rax+=16
                self.a.bs(&[0x48,0x01,0xF8]);           // rax+=rdi → slot
                self.a.bs(&[0x48,0x8B,0x50,0x08]);     // rdx=[rax+8]=pay
                self.a.bs(&[0x48,0x8B,0x00]);           // rax=[rax]=tag
                self.a.push_val();
                self.a.place(done);
            }
            // ── queries ─────────────────────────────────────────────────────
            "len"|"length" => {
                self.a.pop_rdi(); self.a.add_rsp(8);
                self.a.call(self.rt.list_len);          // rax=f64 bits of len
                self.a.bs(&[0x48,0x89,0xC2]);           // rdx=rax (pay)
                self.a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // rax=TAG_NUM
                self.a.push_val();
            }
            "toString"|"to_str"|"str" => {
                // stack: [tag][pay(top)] — pop pay then tag
                self.a.pop_rsi(); self.a.pop_rdi();     // rsi=pay, rdi=tag
                self.a.call(self.rt.val2str);           // rax=str_ptr
                self.a.bs(&[0x48,0x89,0xC2]);           // rdx=rax (str ptr = pay)
                self.a.bs(&[0x48,0xC7,0xC0,0x03,0,0,0]); // rax=TAG_STR
                self.a.push_val();
            }
            "keys" => {
                self.a.add_rsp(16); // discard obj — stub: return empty list
                self.a.call(self.rt.list_new);
                self.a.bs(&[0x48,0x89,0xC2]);           // rdx=rax (list ptr=pay)
                self.a.bs(&[0x48,0xC7,0xC0]); self.a.u32le(TAG_LIST as u32);
                self.a.push_val();
            }
            // ── Higher-order functions ───────────────────────────────────────
            // Register plan: r15=src_list, r14=cb_ptr, r13=result_list (for filter/map)
            // rbx=loop counter, r12 used by list_push internally (callee-saved, OK)
            // After any call: rax/rdx=return values; r12-r15,rbx preserved by callee.
            "filter" => {
                if args.is_empty() {
                    self.a.add_rsp(16);
                    self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                    return;
                }
                // pop obj (src list)
                self.a.pop_rdi(); self.a.add_rsp(8);    // rdi=src_list, drop tag
                self.a.bs(&[0x49,0x89,0xFF]);           // r15=src_list
                // compile callback, pop its func ptr
                self.compile_expr(&args[0]);
                self.a.pop_rsi(); self.a.add_rsp(8);    // rsi=cb_ptr, drop tag
                self.a.bs(&[0x49,0x89,0xF6]);           // r14=cb_ptr
                // allocate result list → r13
                self.a.call(self.rt.list_new);
                self.a.bs(&[0x49,0x89,0xC5]);           // r13=result_list
                // loop: rbx = 0 .. src_len
                self.a.bs(&[0x48,0x31,0xDB]);           // rbx=0
                let lp=self.a.label(); let end=self.a.label();
                self.a.place(lp);
                // load src_len fresh each iteration (src doesn't change, but good habit)
                self.a.bs(&[0x4C,0x89,0xFF]);           // rdi=r15
                self.a.bs(&[0x48,0x8B,0x47,0x08]);     // rax=[rdi+8]=src_len
                self.a.bs(&[0x48,0x39,0xD8]); self.a.jae(end); // rbx>=len → done
                // compute slot ptr: src + 16 + rbx*16
                self.a.bs(&[0x48,0x89,0xD8]);           // rax=rbx
                self.a.bs(&[0x48,0xC1,0xE0,0x04]);     // rax<<=4 (*16)
                self.a.bs(&[0x48,0x83,0xC0,0x10]);     // rax+=16
                self.a.bs(&[0x4C,0x01,0xF8]);           // rax+=r15 → slot ptr
                // push elem as single arg: [elem_tag][elem_pay(top)]
                self.a.bs(&[0x48,0x8B,0x08]);           // rcx=[rax]=elem_tag (use rcx as tmp)
                self.a.bs(&[0x48,0x8B,0x50,0x08]);     // rdx=[rax+8]=elem_pay
                self.a.push_rcx();                      // push elem_tag (use push_rcx=0x51)
                self.a.push_rdx();                      // push elem_pay
                self.a.bs(&[0x41,0xFF,0xD6]);           // call r14 (cb)
                self.a.add_rsp(16);                     // clean up 1 arg (16 bytes)
                // cb returned rax=ret_tag, rdx=ret_pay — call truthy
                self.a.bs(&[0x48,0x89,0xC7]);           // rdi=rax (tag)
                self.a.bs(&[0x48,0x89,0xD6]);           // rsi=rdx (pay)
                self.a.call(self.rt.truthy);
                self.a.test_rax_rax();
                let skip=self.a.label(); self.a.jz(skip);
                // truthy: reload slot and push elem to result
                self.a.bs(&[0x48,0x89,0xD8]);           // rax=rbx
                self.a.bs(&[0x48,0xC1,0xE0,0x04]);
                self.a.bs(&[0x48,0x83,0xC0,0x10]);
                self.a.bs(&[0x4C,0x01,0xF8]);           // rax=slot ptr
                // list_push(rdi=list, rsi=tag, rdx=pay)
                // elem_tag at [rax], elem_pay at [rax+8]
                self.a.bs(&[0x48,0x8B,0x30]);           // rsi=[rax]=elem_tag
                self.a.bs(&[0x48,0x8B,0x50,0x08]);     // rdx=[rax+8]=elem_pay
                self.a.bs(&[0x4C,0x89,0xEF]);           // rdi=r13 (result)
                self.a.call(self.rt.list_push);
                self.a.place(skip);
                self.a.bs(&[0x48,0xFF,0xC3]); self.a.jmp(lp); // rbx++; loop
                self.a.place(end);
                // return result list
                self.a.bs(&[0x48,0xC7,0xC0]); self.a.u32le(TAG_LIST as u32);
                self.a.bs(&[0x4C,0x89,0xEA]); // rdx=r13 (result list ptr=pay)
                self.a.push_val();
            }
            "map" => {
                if args.is_empty() {
                    self.a.add_rsp(16);
                    self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                    return;
                }
                self.a.pop_rdi(); self.a.add_rsp(8);
                self.a.bs(&[0x49,0x89,0xFF]);           // r15=src_list
                self.compile_expr(&args[0]);
                self.a.pop_rsi(); self.a.add_rsp(8);
                self.a.bs(&[0x49,0x89,0xF6]);           // r14=cb_ptr
                self.a.call(self.rt.list_new);
                self.a.bs(&[0x49,0x89,0xC5]);           // r13=result_list
                self.a.bs(&[0x48,0x31,0xDB]);           // rbx=0
                let lp=self.a.label(); let end=self.a.label();
                self.a.place(lp);
                self.a.bs(&[0x4C,0x89,0xFF]);
                self.a.bs(&[0x48,0x8B,0x47,0x08]);
                self.a.bs(&[0x48,0x39,0xD8]); self.a.jae(end);
                // slot ptr
                self.a.bs(&[0x48,0x89,0xD8]);
                self.a.bs(&[0x48,0xC1,0xE0,0x04]);
                self.a.bs(&[0x48,0x83,0xC0,0x10]);
                self.a.bs(&[0x4C,0x01,0xF8]);           // rax=slot
                // push elem arg
                self.a.bs(&[0x48,0x8B,0x08]);           // rcx=[rax]=elem_tag
                self.a.bs(&[0x48,0x8B,0x50,0x08]);     // rdx=[rax+8]=elem_pay
                self.a.push_rcx();
                self.a.push_rdx();
                self.a.bs(&[0x41,0xFF,0xD6]);           // call r14
                self.a.add_rsp(16);
                // rax=ret_tag, rdx=ret_pay → push to result
                // list_push(rdi=r13, rsi=ret_tag=rax, rdx=ret_pay)
                self.a.bs(&[0x48,0x89,0xC6]);           // rsi=rax (ret_tag)
                // rdx already = ret_pay
                self.a.bs(&[0x4C,0x89,0xEF]);           // rdi=r13 (result)
                self.a.call(self.rt.list_push);
                self.a.bs(&[0x48,0xFF,0xC3]); self.a.jmp(lp);
                self.a.place(end);
                self.a.bs(&[0x48,0xC7,0xC0]); self.a.u32le(TAG_LIST as u32);
                self.a.bs(&[0x4C,0x89,0xEA]); // rdx=r13
                self.a.push_val();
            }
            "forEach" => {
                if args.is_empty() {
                    self.a.add_rsp(16);
                    self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                    return;
                }
                self.a.pop_rdi(); self.a.add_rsp(8);
                self.a.bs(&[0x49,0x89,0xFF]);           // r15=src_list
                self.compile_expr(&args[0]);
                self.a.pop_rsi(); self.a.add_rsp(8);
                self.a.bs(&[0x49,0x89,0xF6]);           // r14=cb_ptr
                self.a.bs(&[0x48,0x31,0xDB]);           // rbx=0
                let lp=self.a.label(); let end=self.a.label();
                self.a.place(lp);
                self.a.bs(&[0x4C,0x89,0xFF]);
                self.a.bs(&[0x48,0x8B,0x47,0x08]);
                self.a.bs(&[0x48,0x39,0xD8]); self.a.jae(end);
                self.a.bs(&[0x48,0x89,0xD8]);
                self.a.bs(&[0x48,0xC1,0xE0,0x04]);
                self.a.bs(&[0x48,0x83,0xC0,0x10]);
                self.a.bs(&[0x4C,0x01,0xF8]);
                self.a.bs(&[0x48,0x8B,0x08]);
                self.a.bs(&[0x48,0x8B,0x50,0x08]);
                self.a.push_rcx(); self.a.push_rdx();
                self.a.bs(&[0x41,0xFF,0xD6]);
                self.a.add_rsp(16);
                self.a.bs(&[0x48,0xFF,0xC3]); self.a.jmp(lp);
                self.a.place(end);
                self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); // null
            }
            "reduce" => {
                // list.reduce(fn, init?) — fn(acc, elem) → new acc
                if args.is_empty() {
                    self.a.add_rsp(16);
                    self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                    return;
                }
                self.a.pop_rdi(); self.a.add_rsp(8);
                self.a.bs(&[0x49,0x89,0xFF]);           // r15=src
                self.compile_expr(&args[0]);
                self.a.pop_rsi(); self.a.add_rsp(8);
                self.a.bs(&[0x49,0x89,0xF6]);           // r14=cb
                // initial accumulator: args[1] or null
                if args.len() >= 2 {
                    self.compile_expr(&args[1]);
                } else {
                    self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                }
                // acc is now on stack as [acc_tag][acc_pay(top)]
                self.a.bs(&[0x48,0x31,0xDB]);           // rbx=0
                let lp=self.a.label(); let end=self.a.label();
                self.a.place(lp);
                self.a.bs(&[0x4C,0x89,0xFF]);
                self.a.bs(&[0x48,0x8B,0x47,0x08]);
                self.a.bs(&[0x48,0x39,0xD8]); self.a.jae(end);
                // slot
                self.a.bs(&[0x48,0x89,0xD8]);
                self.a.bs(&[0x48,0xC1,0xE0,0x04]);
                self.a.bs(&[0x48,0x83,0xC0,0x10]);
                self.a.bs(&[0x4C,0x01,0xF8]);           // rax=slot
                // push elem
                self.a.bs(&[0x48,0x8B,0x08]);
                self.a.bs(&[0x48,0x8B,0x50,0x08]);
                self.a.push_rcx(); self.a.push_rdx();   // [elem_tag][elem_pay(top)]
                // acc is below elem on stack: ...[acc_tag][acc_pay][elem_tag][elem_pay(top)]
                // cb(acc_tag, acc_pay, elem_tag, elem_pay) — but we push them as 2 args:
                // push acc, then push elem, call cb(acc, elem)
                // Actually just push both 2-word values → cb sees 2 args on stack
                // cb signature is fn(acc, elem): acc at deeper, elem at shallower
                // Stack now: [acc_tag][acc_pay][elem_tag][elem_pay(top)] — 4 words
                self.a.bs(&[0x41,0xFF,0xD6]);           // call r14 (cb gets acc+elem on stack)
                self.a.add_rsp(32);                     // clean up 2 args
                // rax=new_acc_tag, rdx=new_acc_pay — push as new acc
                self.a.push_val();
                self.a.bs(&[0x48,0xFF,0xC3]); self.a.jmp(lp);
                self.a.place(end);
                // acc is on stack — leave it as the expression result (already [tag][pay])
                // (nothing to do — it's already pushed)
            }
            "find" => {
                // list.find(fn) → first element where fn(elem) is truthy, else null
                if args.is_empty() {
                    self.a.add_rsp(16);
                    self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
                    return;
                }
                self.a.pop_rdi(); self.a.add_rsp(8);
                self.a.bs(&[0x49,0x89,0xFF]);
                self.compile_expr(&args[0]);
                self.a.pop_rsi(); self.a.add_rsp(8);
                self.a.bs(&[0x49,0x89,0xF6]);
                self.a.bs(&[0x48,0x31,0xDB]);
                let lp=self.a.label(); let found=self.a.label(); let end=self.a.label();
                self.a.place(lp);
                self.a.bs(&[0x4C,0x89,0xFF]);
                self.a.bs(&[0x48,0x8B,0x47,0x08]);
                self.a.bs(&[0x48,0x39,0xD8]); self.a.jae(end);
                self.a.bs(&[0x48,0x89,0xD8]);
                self.a.bs(&[0x48,0xC1,0xE0,0x04]);
                self.a.bs(&[0x48,0x83,0xC0,0x10]);
                self.a.bs(&[0x4C,0x01,0xF8]);           // rax=slot
                self.a.bs(&[0x48,0x8B,0x08]);
                self.a.bs(&[0x48,0x8B,0x50,0x08]);
                self.a.push_rcx(); self.a.push_rdx();
                self.a.bs(&[0x41,0xFF,0xD6]);
                self.a.add_rsp(16);
                self.a.bs(&[0x48,0x89,0xC7]); self.a.bs(&[0x48,0x89,0xD6]);
                self.a.call(self.rt.truthy);
                self.a.test_rax_rax(); self.a.jnz(found);
                self.a.bs(&[0x48,0xFF,0xC3]); self.a.jmp(lp);
                self.a.place(found);
                // reload elem at rbx
                self.a.bs(&[0x48,0x89,0xD8]);
                self.a.bs(&[0x48,0xC1,0xE0,0x04]);
                self.a.bs(&[0x48,0x83,0xC0,0x10]);
                self.a.bs(&[0x4C,0x01,0xF8]);           // rax=slot ptr
                // [rax]=elem_tag, [rax+8]=elem_pay
                self.a.bs(&[0x48,0x8B,0x50,0x08]);     // rdx=[rax+8]=elem_pay
                self.a.bs(&[0x48,0x8B,0x00]);           // rax=[rax]=elem_tag
                self.a.push_val();
                let done=self.a.label(); self.a.jmp(done);
                self.a.place(end);
                self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax(); // null
                self.a.place(done);
            }
            // ── generic user-defined method lookup ───────────────────────────
            _ => {
                let hash=fnv1a(method);
                // obj is on stack: [obj_tag][obj_pay(top)]
                // peek obj_pay without popping — it stays as 'this' arg
                self.a.bs(&[0x48,0x8B,0x3C,0x24]);     // rdi=[rsp]=obj_pay
                self.a.mov_rsi_u64(hash);
                self.a.call(self.rt.map_get);           // rax=tag, rdx=func_ptr
                self.a.bs(&[0x49,0x89,0xD7]);           // r15=func_ptr
                // push user args on top of obj (which stays as 'this')
                for arg in args { self.compile_expr(arg); }
                self.a.bs(&[0x41,0xFF,0xD7]);           // call r15
                // clean up: obj (this=1) + user args
                self.a.add_rsp(((args.len() + 1) * 16) as u32);
                self.a.push_val();
            }
        }
    }

    fn compile_list_literal(&mut self, items: &[Expr]) {
        self.a.call(self.rt.list_new);
        self.a.bs(&[0x49,0x89,0xC7]); // r15=list ptr
        for item in items {
            self.compile_expr(item);
            // stack top: pay, then tag
            self.a.pop_rdx(); // pay
            self.a.pop_rsi(); // tag
            self.a.bs(&[0x4C,0x89,0xFF]); // rdi=r15 (list)
            self.a.call(self.rt.list_push);
        }
        // push TAG_LIST as tag, r15 (list ptr) as pay
        self.a.bs(&[0x48,0xC7,0xC0]); self.a.u32le(TAG_LIST as u32);
        self.a.push_rax();             // push tag=TAG_LIST
        self.a.bs(&[0x4C,0x89,0xF8]); // rax=r15
        self.a.push_rax();             // push pay=list_ptr
    }

    fn compile_obj_literal(&mut self, pairs: &[(String, Expr)]) {
        self.a.call(self.rt.map_new);
        self.a.bs(&[0x49,0x89,0xC7]); // r15=map
        for (k, v) in pairs {
            self.compile_expr(v);
            // stack top: pay, then tag
            self.a.pop_rcx(); // rcx=pay
            self.a.pop_rdx(); // rdx=tag
            self.a.bs(&[0x4C,0x89,0xFF]); // rdi=r15 (map)
            self.a.mov_rsi_u64(fnv1a(k));
            self.a.call(self.rt.map_set);
        }
        // push TAG_OBJ as tag, r15 (map ptr) as pay
        self.a.bs(&[0x48,0xC7,0xC0]); self.a.u32le(TAG_OBJ as u32);
        self.a.push_rax();             // push tag=TAG_OBJ
        self.a.bs(&[0x4C,0x89,0xF8]); // rax=r15
        self.a.push_rax();             // push pay=map_ptr
    }

    fn compile_index(&mut self, obj: &Expr, idx: &Expr) {
        self.compile_expr(obj);   // [obj_tag][obj_pay(top)]
        self.compile_expr(idx);   // [obj_tag][obj_pay][idx_tag][idx_pay(top)]
        self.a.pop_rsi();          // rsi=idx_pay (f64 bits)
        self.a.add_rsp(8);         // discard idx_tag
        self.a.pop_rdi();          // rdi=obj_pay (list/map ptr)
        self.a.add_rsp(8);         // discard obj_tag
        self.a.call(self.rt.list_get); // rax=tag, rdx=pay
        self.a.push_val();
    }

    fn compile_getattr(&mut self, obj: &Expr, attr: &str) {
        self.compile_expr(obj);
        // stack: [tag][pay(top)]
        self.a.pop_rsi(); self.a.add_rsp(8); // rsi=obj_pay (map ptr), discard tag
        self.a.bs(&[0x48,0x89,0xF7]);        // rdi=rsi (map ptr)
        self.a.mov_rsi_u64(fnv1a(attr));
        self.a.call(self.rt.map_get);        // rax=tag, rdx=pay
        // If not found (rax==u64::MAX), try __parent__ chain
        // For simplicity in AOT, we trust map_get returns TAG_NULL on miss (u64::MAX means truly not found)
        // Push whatever we got
        self.a.push_val(); // push rax=tag, rdx=pay
    }

    fn compile_new(&mut self, cls_e: &Expr, args: &[Expr]) {
        self.compile_expr(cls_e);
        // stack top: pay=class_ptr, tag
        self.a.pop_rsi(); self.a.add_rsp(8); // rsi=class_ptr, discard tag
        self.a.bs(&[0x49,0x89,0xF7]);        // r15=class_ptr
        // Create instance map
        self.a.call(self.rt.map_new);
        self.a.bs(&[0x49,0x89,0xC6]);        // r14=instance (map ptr)
        // Store __class__ field pointing to class map
        let class_h=fnv1a("__class__");
        self.a.bs(&[0x4C,0x89,0xF7]);        // rdi=r14 (instance)
        self.a.mov_rsi_u64(class_h);
        self.a.bs(&[0x48,0xC7,0xC2]); self.a.u32le(TAG_OBJ as u32); // rdx=TAG_OBJ
        self.a.bs(&[0x4C,0x89,0xF9]);        // rcx=r15 (class ptr)
        self.a.call(self.rt.map_set);
        // Look up __init__ method in class map
        let init_h=fnv1a("init");
        self.a.bs(&[0x4C,0x89,0xFF]);        // rdi=r15 (class map)
        self.a.mov_rsi_u64(init_h);
        self.a.call(self.rt.map_get);        // rax=tag, rdx=func_ptr
        let skip=self.a.label();
        self.a.bs(&[0x48,0x83,0xF8]); self.a.b(TAG_FUNC as u8); // cmp rax,TAG_FUNC
        self.a.jne(skip);
        self.a.bs(&[0x49,0x89,0xD5]);        // r13=func_ptr
        // Push instance (this) as first arg
        self.a.bs(&[0x48,0xC7,0xC0]); self.a.u32le(TAG_OBJ as u32);
        self.a.push_rax();                   // push TAG_OBJ (tag)
        self.a.bs(&[0x4C,0x89,0xF0]);        // rax=r14 (instance)
        self.a.push_rax();                   // push instance ptr (pay)
        // Push user args
        for arg in args { self.compile_expr(arg); }
        self.a.bs(&[0x41,0xFF,0xD5]);        // call r13 (init)
        let n1 = (1 + args.len()) * 16;
        self.a.add_rsp(n1 as u32);           // clean up this + args
        self.a.place(skip);
        // Return the instance as TAG_OBJ
        self.a.bs(&[0x48,0xC7,0xC0]); self.a.u32le(TAG_OBJ as u32);
        self.a.push_rax();                   // push tag=TAG_OBJ
        self.a.bs(&[0x4C,0x89,0xF0]);        // rax=r14 (instance)
        self.a.push_rax();                   // push pay=instance ptr
    }

    /// super.method(args): load 'this' from frame, look up method in __parent__ class map,
    /// then call it with 'this' as first argument.
    fn compile_super(&mut self, method: &str, args: &[Expr]) {
        // 1. Load 'this' from current frame
        self.emit_load("this"); // [tag][pay(top)] — pay = instance map ptr

        // 2. Load instance map ptr to look up __class__.__parent__
        self.a.bs(&[0x48,0x8B,0x3C,0x24]); // rdi=[rsp] = this_pay (map ptr), don't pop
        // Get __class__ from instance
        self.a.mov_rsi_u64(fnv1a("__class__"));
        self.a.call(self.rt.map_get); // rax=tag, rdx=class_ptr
        // Get __parent__ from class
        self.a.bs(&[0x48,0x89,0xD7]); // rdi=rdx (class_ptr)
        self.a.mov_rsi_u64(fnv1a("__parent__"));
        self.a.call(self.rt.map_get); // rax=tag, rdx=parent_map_ptr
        self.a.bs(&[0x49,0x89,0xD7]); // r15=parent_map_ptr

        // 3. Look up method in parent map
        self.a.bs(&[0x48,0x89,0xD7]); // rdi=rdx (parent_map_ptr)
        self.a.mov_rsi_u64(fnv1a(method));
        self.a.call(self.rt.map_get); // rax=tag, rdx=func_ptr
        self.a.bs(&[0x49,0x89,0xD6]); // r14=func_ptr

        // 4. 'this' is already on stack (deepest). Push user args on top.
        for arg in args { self.compile_expr(arg); }

        // 5. Call the parent method
        self.a.bs(&[0x41,0xFF,0xD6]); // call r14

        // 6. Cleanup: this (1) + user args
        let total = (1 + args.len()) * 16;
        self.a.add_rsp(total as u32);

        self.a.push_val();
    }

    fn compile_preop(&mut self, op: &str, tgt: &Expr) {
        if let ExprNode::Var(name) = &tgt.node {
            // Load current value
            self.emit_load(name);
            // Stack: [tag][pay(top)]
            self.a.pop_rsi(); self.a.add_rsp(8); // rsi=old_pay, discard tag
            let one = 1.0f64.to_bits();
            self.a.bs(&[0x66,0x48,0x0F,0x6E,0xC6]); // movq xmm0,rsi
            self.a.mov_rcx_u64(one);
            self.a.movq_xmm1_rcx();
            if op == "INC" { self.a.addsd(); } else { self.a.subsd(); }
            self.a.movq_rdx_xmm0(); // rdx=new_pay
            // Push [TAG_NUM][new_pay] for emit_store (and as expression result)
            self.a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // rax=TAG_NUM
            self.a.push_rax();  // push tag
            self.a.push_rdx();  // push new_pay
            // emit_store pops 2 words, stores, repushes 2 words — result stays on stack
            self.emit_store(name);
        } else {
            self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
        }
    }

    fn compile_postop(&mut self, op: &str, tgt: &Expr) {
        if let ExprNode::Var(name) = &tgt.node {
            // Load current (old) value: stack=[tag][pay(top)]
            self.emit_load(name);
            // Save old tag and pay in callee-saved regs (r14=old_tag, r15=old_pay)
            self.a.bs(&[0x4D,0x8B,0x7C,0x24,0x00]); // r15=[rsp+0] = old_pay  (top)
            self.a.bs(&[0x4D,0x8B,0x74,0x24,0x08]); // r14=[rsp+8] = old_tag
            // Compute new value (old_pay ± 1.0) using xmm0
            // FIX: was 0x4C (REX.R+REX.B) which encoded movq xmm8,r15; correct is 0x49 (REX.B only)
            let one = 1.0f64.to_bits();
            self.a.bs(&[0x66,0x49,0x0F,0x6E,0xC7]); // movq xmm0,r15  (REX.B to access r15, no REX.R)
            self.a.mov_rcx_u64(one);
            self.a.movq_xmm1_rcx();
            if op == "INC" { self.a.addsd(); } else { self.a.subsd(); }
            self.a.movq_rdx_xmm0(); // rdx=new_pay
            // Overwrite top-of-stack with new value for emit_store
            self.a.bs(&[0x48,0xC7,0xC0,0x02,0,0,0]); // rax=TAG_NUM
            self.a.bs(&[0x48,0x89,0x44,0x24,0x08]);   // [rsp+8]=rax (new tag, replaces old tag)
            self.a.bs(&[0x48,0x89,0x14,0x24]);         // [rsp+0]=rdx (new pay, replaces old pay)
            // Now pop and store new value
            self.emit_store(name);
            // emit_store re-pushes the new value — discard it
            self.a.add_rsp(16);
            // Push OLD value as the expression result (post-increment returns old value)
            self.a.bs(&[0x4C,0x89,0xF0]); // rax=r14 (old_tag)
            self.a.push_rax();
            self.a.bs(&[0x4C,0x89,0xF8]); // rax=r15 (old_pay)
            self.a.push_rax();
        } else {
            self.a.xor_rax_rax(); self.a.push_rax(); self.a.push_rax();
        }
    }

    // ── function definition ───────────────────────────────────────────────────
    fn compile_func(&mut self, name: &str, params: &[String], body: &Stmt) {
        let skip=self.a.label(); let func_lbl=self.a.label();

        let tramp_lbl = self.a.label();
        let is_c_export = name.starts_with("c_");

        if !name.starts_with("__lam_") {
            if is_c_export {
                self.a.exports.push((name.to_string(), tramp_lbl));
            } else {
                self.a.exports.push((name.to_string(), func_lbl));
            }
        }

        self.a.jmp(skip);

        // --- GENERATE C-ABI TRAMPOLINE IF PREFIXED WITH "c_" ---
        if is_c_export {
            self.a.place(tramp_lbl);
            self.a.prologue();
            
            // 1. SAVE PYTHON'S NON-VOLATILE REGISTERS + ALIGN STACK
            self.a.bs(&[0x41, 0x54]); // push r12 (Saves Python's internal state)
            self.a.bs(&[0x50]);       // push rax (Stack padding for 16-byte alignment)
            
            // 2. PUSH ARG 1 (From RCX) FIRST -> It goes deeper into the stack
            self.a.bs(&[0xF2, 0x48, 0x0F, 0x2A, 0xC1]); // cvtsi2sd xmm0, rcx
            self.a.movq_rdx_xmm0();
            self.a.bs(&[0x48, 0xC7, 0xC0, 0x02, 0x00, 0x00, 0x00]); // rax = TAG_NUM
            self.a.push_rax(); // tag
            self.a.push_rdx(); // pay

            // 3. PUSH ARG 2 (From RDX) SECOND -> It sits at the top of the stack
            self.a.bs(&[0xF2, 0x48, 0x0F, 0x2A, 0xC2]); // cvtsi2sd xmm0, rdx
            self.a.movq_rdx_xmm0();
            self.a.bs(&[0x48, 0xC7, 0xC0, 0x02, 0x00, 0x00, 0x00]); // rax = TAG_NUM
            self.a.push_rax(); // tag
            self.a.push_rdx(); // pay
            
            // 4. LOAD DRAGON'S GLOBAL FRAME STATE
            self.a.bs(&[0x48, 0xB8]); self.a.rodata_ptr(0); // mov rax, &rodata[0]
            self.a.bs(&[0x4C, 0x8B, 0x20]);                 // mov r12, [rax]
            
            self.a.call(func_lbl); // Call actual Dragon function
            
            // 5. DYNAMIC RETURN
            self.a.bs(&[0x48, 0x83, 0xF8, 0x02]);       // cmp rax, 2 (TAG_NUM)
            self.a.bs(&[0x75, 0x0C]);                   // jne return_ptr
            
            self.a.bs(&[0x66, 0x48, 0x0F, 0x6E, 0xC2]); // movq xmm0, rdx
            self.a.bs(&[0xF2, 0x48, 0x0F, 0x2C, 0xC0]); // cvttsd2si rax, xmm0
            self.a.bs(&[0xEB, 0x03]);                   // jmp done
            
            // return_ptr:
            self.a.bs(&[0x48, 0x89, 0xD0]);             // mov rax, rdx
            // done:
            
            // 6. CLEANUP & RESTORE PYTHON'S STATE
            self.a.add_rsp(32);       // Pop the 32 bytes of arguments we pushed
            self.a.bs(&[0x58]);       // pop rax (Remove stack padding)
            self.a.bs(&[0x41, 0x5C]); // pop r12 (Restores Python's internal state)
            
            self.a.epilogue();
        }
        // --------------------------------------------------------
        self.a.place(func_lbl);
        self.a.prologue();
        self.a.bs(&[0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57,0x53]); // push r12..rbx
        self.a.bs(&[0x48,0x83,0xEC,0x08]); // sub rsp,8 (align) // push r12,r13,r14,r15,rbx
        // frame_new(r12) → r12
        match self.a.os {
            TargetOs::Linux   => { self.a.bs(&[0x4C,0x89,0xE7]); } // rdi=r12
            TargetOs::Windows => { self.a.bs(&[0x4C,0x89,0xE1]); } // rcx=r12
        }
        self.a.call(self.rt.frame_new);
        self.a.bs(&[0x49,0x89,0xC4]); // r12=new_frame
        // Bind params from stack (args pushed L→R before call; after prologue+5pushes:
        //   [rbp+0]=saved_rbp, [rbp+8]=ret_addr(no — rbp is rbp)
        // Standard x86-64: CALL pushes retaddr, then prologue does push rbp; mov rbp,rsp
        //   → [rbp]   = old_rbp
        //   → [rbp+8] = retaddr... NO: rsp points to old_rbp after push rbp;
        //     rbp=rsp (after mov rbp,rsp); so [rbp]=old_rbp, [rbp+8] = what was at [rsp+8]
        //     before push rbp = retaddr
        //   wait: call pushes retaddr, then push rbp  → stack: ...args retaddr old_rbp
        //     rbp=rsp (after mov rbp,rsp) → [rbp]=old_rbp, [rbp+8]=retaddr
        //     but that's wrong: rbp=rsp after push rbp, and rsp points to old_rbp.
        //     so [rbp+0]=old_rbp, [rbp+8]=retaddr... no retaddr is BELOW old_rbp.
        //   Let's be precise:
        //     Before call: rsp=X, stack=[args]
        //     After call (pushes retaddr): rsp=X-8, [rsp]=retaddr
        //     push rbp: rsp=X-16, [rsp]=old_rbp; [rsp+8]=retaddr
        //     mov rbp,rsp → rbp=X-16
        //     So: [rbp]=old_rbp, [rbp+8]=retaddr, [rbp+16]=arg_last, [rbp+24]=arg_last_tag,...
        //     Args were pushed L→R: arg0 first (deepest), argN-1 last (shallowest, near top).
        //     arg_last (argN-1) tag   at [rbp+16]
        //     arg_last (argN-1) pay   at [rbp+24]
        //     arg_last-1 tag          at [rbp+32]
        //     ...
        //     arg0 tag                at [rbp+16+(n-1)*16]
        //     arg0 pay                at [rbp+24+(n-1)*16]
        //   Then we pushed r12,r13,r14,r15,rbx (5*8=40 bytes) after prologue.
        //   But those pushes happen AFTER prologue (push rbp; mov rbp,rsp), so they
        //   don't change rbp. rbp still points to old_rbp. ✓
        let n = params.len();
        for (i, param) in params.iter().enumerate() {
            let off = 16i32 + ((n - 1 - i) as i32) * 16;
            let off_tag = off + 8; // tag is 8 bytes above pay
            // mov rbx,[rbp+off_tag]  (64-bit load, 32-bit disp if needed)
            if off_tag < 128 {
                self.a.bs(&[0x48,0x8B,0x5D]); self.a.b(off_tag as u8);
            } else {
                self.a.bs(&[0x48,0x8B,0x9D]); self.a.u32le(off_tag as u32);
            }
            // mov r13,[rbp+off]
            if off < 128 {
                self.a.bs(&[0x4C,0x8B,0x6D]); self.a.b(off as u8);
            } else {
                self.a.bs(&[0x4C,0x8B,0xAD]); self.a.u32le(off as u32);
            }
            self.a.bs(&[0x4C,0x89,0xE7]);  // rdi=r12
            self.a.mov_rsi_u64(fnv1a(param));
            self.a.bs(&[0x48,0x89,0xDA]);  // rdx=rbx (tag)
            self.a.bs(&[0x4C,0x89,0xE9]);  // rcx=r13 (pay)
            self.a.call(self.rt.frame_set);
        }
        // Compile body
        let rl=self.a.label();
        self.returns.push(rl);
        self.compile_stmt(body);
        self.returns.pop();
        // Fall-through (no explicit return) → return null
        self.a.xor_rax_rax(); self.a.xor_rdx_rdx();
        // Return label — both explicit 'return' jumps and fall-through arrive here
        self.a.place(rl);
        // FIX: undo the sub rsp,8 alignment padding before restoring callee-saved regs
        self.a.bs(&[0x48,0x83,0xC4,0x08]); // add rsp,8
        self.a.bs(&[0x5B,0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C]); // pop rbx,r15,r14,r13,r12
        self.a.epilogue();
        self.a.place(skip);
        // Store func ptr in current frame
        let hash=fnv1a(name);
        self.a.bs(&[0x48,0xB8]); self.a.code_abs(func_lbl); // movabs rax,func_va
        self.a.bs(&[0x48,0x89,0xC1]);   // rcx=rax (func_ptr)
        self.a.bs(&[0x4C,0x89,0xE7]);   // rdi=r12
        self.a.mov_rsi_u64(hash);
        self.a.bs(&[0x48,0xC7,0xC2]); self.a.u32le(TAG_FUNC as u32);
        self.a.call(self.rt.frame_set);
    }

    fn compile_class(&mut self, name: &str, parent: Option<&str>, body: &Stmt) {
        // Class is a map of method_name → func_ptr
        self.a.call(self.rt.map_new);
        self.a.bs(&[0x49,0x89,0xC7]); // r15=class_map

        // Compile methods (parent __parent__ link added after, below)
        if let StmtNode::Block(stmts) = &body.node {
            for stmt in stmts {
                if let StmtNode::FuncDef(mname, params, mbody, _) = &stmt.node {
                    let skip = self.a.label();
                    let fl   = self.a.label();
                    self.a.jmp(skip);
                    self.a.place(fl);
                    self.a.prologue();
                    self.a.bs(&[0x41,0x54,0x41,0x55,0x41,0x56,0x41,0x57,0x53]); // push r12-r15,rbx
                    self.a.bs(&[0x48,0x83,0xEC,0x08]); // sub rsp,8 (align)
                    // frame_new(r12) → r12
                    match self.a.os {
                        TargetOs::Linux   => { self.a.bs(&[0x4C,0x89,0xE7]); } // rdi=r12
                        TargetOs::Windows => { self.a.bs(&[0x4C,0x89,0xE1]); } // rcx=r12
                    }
                    self.a.call(self.rt.frame_new);
                    self.a.bs(&[0x49,0x89,0xC4]); // r12=new_frame
                    // Bind params: 'this' is first (deepest), then rest
                    let all_params: Vec<String> = std::iter::once("this".to_string())
                        .chain(params.iter().cloned()).collect();
                    let n = all_params.len();
                    for (i, param) in all_params.iter().enumerate() {
                        let off = 16i32 + ((n - 1 - i) as i32) * 16;
                        let off_tag = off + 8;
                        if off_tag < 128 {
                            self.a.bs(&[0x48,0x8B,0x5D]); self.a.b(off_tag as u8);
                        } else {
                            self.a.bs(&[0x48,0x8B,0x9D]); self.a.u32le(off_tag as u32);
                        }
                        if off < 128 {
                            self.a.bs(&[0x4C,0x8B,0x6D]); self.a.b(off as u8);
                        } else {
                            self.a.bs(&[0x4C,0x8B,0xAD]); self.a.u32le(off as u32);
                        }
                        self.a.bs(&[0x4C,0x89,0xE7]); // rdi=r12
                        self.a.mov_rsi_u64(fnv1a(param));
                        self.a.bs(&[0x48,0x89,0xDA]); // rdx=rbx (tag)
                        self.a.bs(&[0x4C,0x89,0xE9]); // rcx=r13 (pay)
                        self.a.call(self.rt.frame_set);
                    }
                    let rl = self.a.label();
                    self.returns.push(rl);
                    self.compile_stmt(mbody);
                    self.returns.pop();
                    self.a.xor_rax_rax(); self.a.xor_rdx_rdx();
                    self.a.place(rl);
                    self.a.bs(&[0x48,0x83,0xC4,0x08]); // add rsp,8 (undo alignment)
                    self.a.bs(&[0x5B,0x41,0x5F,0x41,0x5E,0x41,0x5D,0x41,0x5C]); // pop rbx,r15-r12
                    self.a.epilogue();
                    self.a.place(skip);
                    // Store method in class map (r15)
                    self.a.bs(&[0x48,0xB8]); self.a.code_abs(fl); // movabs rax,func_va
                    self.a.bs(&[0x48,0x89,0xC1]);   // rcx=rax (func_ptr = pay)
                    self.a.bs(&[0x4C,0x89,0xFF]);   // rdi=r15 (class map)
                    self.a.mov_rsi_u64(fnv1a(mname));
                    self.a.bs(&[0x48,0xC7,0xC2]); self.a.u32le(TAG_FUNC as u32); // rdx=TAG_FUNC
                    self.a.call(self.rt.map_set);
                }
            }
        }

        // Inherit parent methods: after compiling our own, copy parent's entries
        // that are not already overridden. Use __parent__ link at runtime via map_get fallback.
        // For AOT simplicity, store the parent ref in the class map now (if parent given).
        if let Some(p) = parent {
            let p_hash = fnv1a(p);
            // Load parent class ptr from current frame
            self.a.bs(&[0x4C,0x89,0xE7]); // rdi=r12
            self.a.mov_rsi_u64(p_hash);
            self.a.call(self.rt.frame_get); // rax=tag, rdx=parent_map_ptr
            self.a.bs(&[0x49,0x89,0xD6]); // r14=parent_ptr (rdx)
            // Store __parent__ in class map
            let par_h = fnv1a("__parent__");
            self.a.bs(&[0x4C,0x89,0xFF]);  // rdi=r15
            self.a.mov_rsi_u64(par_h);
            self.a.bs(&[0x48,0xC7,0xC2]); self.a.u32le(TAG_MAP as u32); // rdx=TAG_MAP
            self.a.bs(&[0x4C,0x89,0xF1]); // rcx=r14 (parent_ptr)
            self.a.call(self.rt.map_set);
        }

        // Store class in current frame as TAG_MAP (it's a map of methods)
        let hash = fnv1a(name);
        self.a.bs(&[0x4C,0x89,0xFF]);   // rdi=r15 (class map ptr as pay... wait)
        // frame_set(frame:rdi, hash:rsi, tag:rdx, pay:rcx)
        self.a.bs(&[0x4C,0x89,0xE7]);   // rdi=r12
        self.a.mov_rsi_u64(hash);
        self.a.bs(&[0x48,0xC7,0xC2]); self.a.u32le(TAG_MAP as u32); // rdx=TAG_MAP
        self.a.bs(&[0x4C,0x89,0xF9]);   // rcx=r15 (class map ptr)
        self.a.call(self.rt.frame_set);
    }

    fn emit_load(&mut self, name: &str) {
        let hash=fnv1a(name);
        self.a.bs(&[0x4C,0x89,0xE7]); // rdi=r12
        self.a.mov_rsi_u64(hash);
        self.a.call(self.rt.frame_get);
        self.a.push_val(); // rax=tag, rdx=pay → push tag then pay
    }

    fn emit_store(&mut self, name: &str) {
        let hash=fnv1a(name);
        // Stack top→bot: pay, tag
        self.a.pop_rsi(); self.a.pop_rdi(); // rsi=pay, rdi=tag
        self.a.bs(&[0x48,0x89,0xFB]);       // rbx=tag  (callee-saved)
        self.a.bs(&[0x49,0x89,0xF5]);       // r13=pay  (callee-saved)
        self.a.bs(&[0x4C,0x89,0xE7]);       // rdi=r12 (frame)
        self.a.mov_rsi_u64(hash);
        self.a.bs(&[0x48,0x89,0xDA]);       // rdx=rbx (tag)
        self.a.bs(&[0x4C,0x89,0xE9]);       // rcx=r13 (pay)
        self.a.call(self.rt.frame_set);
        // Re-push value: tag then pay (pay on top)
        self.a.bs(&[0x48,0x89,0xD8]); // rax=rbx (tag)
        self.a.push_rax();             // push tag
        self.a.bs(&[0x4C,0x89,0xE8]); // rax=r13 (pay)
        self.a.push_rax();             // push pay
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Public entry point
// ─────────────────────────────────────────────────────────────────────────────
pub fn compile_to_native(ast: &Stmt, is_lib: bool, is_window: bool, os: TargetOs) -> Vec<u8> {
    let mut asm = Aot::new(os).compile(ast, is_lib);
    const PAGE: usize = 0x1000;
    match os {
        TargetOs::Linux => {
            let cv = 0x0040_0000u64 + 0x1000;
            let rv = cv + align_up(asm.code.len(), PAGE) as u64;
            asm.apply_relocs(cv, rv);
            // For .so: use ET_DYN (type=3) and PIC-friendly base 0
            // is_lib=true → ELF shared object (.so), is_lib=false → ELF executable
            crate::codegen_elf::emit_elf64(asm, is_lib, 0x0040_0000, cv, rv)
        }
        TargetOs::Windows => {
            let base: u64 = if is_lib {
                0x0000_0001_8000_0000 // preferred DLL base (above 2GB for large address)
            } else {
                0x0000_0001_4000_0000 // exe base
            };
            let cv = base + 0x1000;
            let rv = cv + align_up(asm.code.len(), PAGE) as u64;
            // is_lib=true → PE DLL (.dll), is_lib=false → PE executable (.exe)
            crate::codegen_pe::emit_pe64(asm, is_lib, is_window, base, cv, rv)
        }
    }
}