//! ELF64 writer for Linux AOT output.
use crate::aot::{Asm, align_up};

pub fn emit_elf64(buf: Asm, is_lib: bool, base: u64, cv: u64, rv: u64) -> Vec<u8> {
    const PAGE: usize = 0x1000;
    const EH: usize = 64;
    const PH: usize = 56;
    let hdr = EH + PH * 2;
    let cf = hdr;
    let ce = cf + align_up(buf.code.len(), PAGE);
    let rf = ce;
    let entry = if is_lib { 0 } else { cv };
    let mut b = Vec::with_capacity(rf + buf.rodata.len());
    let mut eh = [0u8; EH];
    eh[0..4].copy_from_slice(b"\x7fELF");
    eh[4]=2; eh[5]=1; eh[6]=1;
    eh[16..18].copy_from_slice(&(if is_lib{3u16}else{2u16}).to_le_bytes());
    eh[18..20].copy_from_slice(&0x3Eu16.to_le_bytes());
    eh[20..24].copy_from_slice(&1u32.to_le_bytes());
    eh[24..32].copy_from_slice(&entry.to_le_bytes());
    eh[32..40].copy_from_slice(&(EH as u64).to_le_bytes());
    eh[52..54].copy_from_slice(&(EH as u16).to_le_bytes());
    eh[54..56].copy_from_slice(&(PH as u16).to_le_bytes());
    eh[56..58].copy_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&eh);
    b.extend_from_slice(&ph(1,0x5,cf as u64,cv,buf.code.len() as u64,(ce-cf) as u64,PAGE as u64));
    b.extend_from_slice(&ph(1,0x4,rf as u64,rv,buf.rodata.len() as u64,align_up(buf.rodata.len(),PAGE) as u64,PAGE as u64));
    while b.len() < cf { b.push(0x90); }
    b.extend_from_slice(&buf.code);
    while b.len() < ce { b.push(0); }
    b.extend_from_slice(&buf.rodata);
    while b.len() < rf + align_up(buf.rodata.len(), PAGE) { b.push(0); }
    b
}

fn ph(pt:u32,pf:u32,off:u64,va:u64,fs:u64,ms:u64,al:u64)->[u8;56]{
    let mut p=[0u8;56];
    p[0..4].copy_from_slice(&pt.to_le_bytes()); p[4..8].copy_from_slice(&pf.to_le_bytes());
    p[8..16].copy_from_slice(&off.to_le_bytes()); p[16..24].copy_from_slice(&va.to_le_bytes());
    p[24..32].copy_from_slice(&va.to_le_bytes()); p[32..40].copy_from_slice(&fs.to_le_bytes());
    p[40..48].copy_from_slice(&ms.to_le_bytes()); p[48..56].copy_from_slice(&al.to_le_bytes());
    p
}