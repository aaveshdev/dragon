//! PE64 writer — imports: GetStdHandle, WriteFile, ExitProcess, VirtualAlloc

use crate::aot::{Asm, align_up};

const FA: usize = 0x200;
const MA: usize = 0x1000;

pub fn emit_pe64(
    mut buf: Asm, is_lib: bool, window: bool,
    base: u64, code_va: u64, rdata_va: u64,
) -> Vec<u8> {
    let hdr_raw   = 0x200usize;
    let text_rva  = (code_va  - base) as usize;
    let rdata_rva = (rdata_va - base) as usize;

    let text_rva  = (code_va  - base) as usize;
    let rdata_rva = (rdata_va - base) as usize;

    // --- ADD THIS EXPORT BUILDER BLOCK ---
    let mut export_rva = 0u32;
    let mut export_size = 0u32;
    
    if is_lib && !buf.exports.is_empty() {
        // Resolve labels to RVAs and sort alphabetically (PE Requirement)
        let mut exp_resolved: Vec<(String, u32)> = buf.exports.iter().map(|(name, lbl)| {
            let offset = buf.labels[*lbl].unwrap();
            let rva = text_rva as u32 + offset as u32;
            (name.clone(), rva)
        }).collect();
        exp_resolved.sort_by(|a, b| a.0.cmp(&b.0)); 

        let num_exports = exp_resolved.len() as u32;
        let edata_rva = rdata_rva as u32 + buf.rodata.len() as u32;
        export_rva = edata_rva;

        let eat_rva = edata_rva + 40;
        let npt_rva = eat_rva + num_exports * 4;
        let ord_rva = npt_rva + num_exports * 4;
        let strings_rva = ord_rva + num_exports * 2;

        let mut edata = Vec::new();
        let dll_name = b"dragon_lib.dll\0";
        
        // 1. IMAGE_EXPORT_DIRECTORY (40 bytes)
        edata.extend_from_slice(&[0u8; 12]);                  // Flags, TimeDateStamp, Versions
        edata.extend_from_slice(&strings_rva.to_le_bytes());  // NameRVA
        edata.extend_from_slice(&1u32.to_le_bytes());         // OrdinalBase
        edata.extend_from_slice(&num_exports.to_le_bytes());  // AddressTableEntries
        edata.extend_from_slice(&num_exports.to_le_bytes());  // NumberOfNamePointers
        edata.extend_from_slice(&eat_rva.to_le_bytes());      // EAT RVA
        edata.extend_from_slice(&npt_rva.to_le_bytes());      // NPT RVA
        edata.extend_from_slice(&ord_rva.to_le_bytes());      // Ordinal RVA

        // 2. Export Address Table (EAT)
        for (_, rva) in &exp_resolved { edata.extend_from_slice(&rva.to_le_bytes()); }

        // 3. Name Pointer Table (NPT) and Strings
        let mut current_str_rva = strings_rva + dll_name.len() as u32;
        let mut npt = Vec::new();
        let mut str_buf = Vec::new();
        str_buf.extend_from_slice(dll_name);

        for (name, _) in &exp_resolved {
            npt.extend_from_slice(&current_str_rva.to_le_bytes());
            str_buf.extend_from_slice(name.as_bytes());
            str_buf.push(0);
            current_str_rva += name.len() as u32 + 1;
        }

        edata.extend_from_slice(&npt);
        // 4. Ordinal Table
        for i in 0..num_exports { edata.extend_from_slice(&(i as u16).to_le_bytes()); } 
        // 5. Raw Strings
        edata.extend_from_slice(&str_buf); 
        
        export_size = edata.len() as u32;
        buf.rodata.extend_from_slice(&edata); // Embed into .rdata
    }
    let text_raw  = align_up(buf.code.len(),   FA);
    let rdata_raw = align_up(buf.rodata.len(), FA);

    // .idata placed after .rdata
    let idata_rva  = rdata_rva + align_up(buf.rodata.len(), MA);
    let idata_foff = hdr_raw + text_raw + rdata_raw;

    // Imports: GetStdHandle, WriteFile, ExitProcess, VirtualAlloc
    let iat_off   = 40usize;       // after 2 descriptors (20 bytes each)
    let int_off   = iat_off + 5*8; // 4 slots + null sentinel
    let names_off = int_off  + 5*8;
    let n_gsh = names_off;
    let n_wf  = n_gsh + 2 + "GetStdHandle\0".len();
    let n_ep  = n_wf  + 2 + "WriteFile\0".len();
    let n_va  = n_ep  + 2 + "ExitProcess\0".len();
    let dll_off   = n_va  + 2 + "VirtualAlloc\0".len();
    let idata_size   = dll_off + "KERNEL32.dll\0".len();
    let idata_size_a = align_up(idata_size, FA);

    let gsh_va = base + (idata_rva + iat_off) as u64;
    let wf_va  = gsh_va + 8;
    let ep_va  = gsh_va + 16;
    let va_va  = gsh_va + 24;

    buf.iat_gsh = gsh_va;
    buf.iat_wf  = wf_va;
    buf.iat_ep  = ep_va;
    buf.iat_va  = va_va;
    buf.apply_relocs(code_va, rdata_va);

    let n_secs = 3usize;
    let sub: u16 = if window { 2 } else { 3 };
    let ch:  u16 = if is_lib { 0x2022 } else { 0x0022 };
    let image_size = idata_rva + align_up(idata_size, MA);
    let text_foff  = hdr_raw;
    let rdata_foff = text_foff + text_raw;

    let mut b = Vec::with_capacity(idata_foff + idata_size_a);

    // DOS stub
    let mut dos = [0u8; 0x40]; dos[0]=b'M'; dos[1]=b'Z';
    dos[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
    b.extend_from_slice(&dos);
    b.extend_from_slice(b"PE\0\0");

    // COFF header
    b.extend_from_slice(&0x8664u16.to_le_bytes());
    b.extend_from_slice(&(n_secs as u16).to_le_bytes());
    b.extend_from_slice(&[0u8; 12]);
    b.extend_from_slice(&240u16.to_le_bytes());
    b.extend_from_slice(&ch.to_le_bytes());

    // Optional header (240 bytes)
    let os_off = b.len();
    b.extend_from_slice(&0x020Bu16.to_le_bytes()); b.push(14); b.push(0);
    b.extend_from_slice(&(text_raw as u32).to_le_bytes());
    b.extend_from_slice(&(rdata_raw as u32 + idata_size_a as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(text_rva as u32).to_le_bytes()); // EntryPoint
    b.extend_from_slice(&(text_rva as u32).to_le_bytes()); // BaseOfCode
    b.extend_from_slice(&base.to_le_bytes());
    b.extend_from_slice(&(MA as u32).to_le_bytes());
    b.extend_from_slice(&(FA as u32).to_le_bytes());
    b.extend_from_slice(&[6,0, 0,0, 6,0, 0,0, 6,0, 0,0]);
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(image_size as u32).to_le_bytes());
    b.extend_from_slice(&(hdr_raw as u32).to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&sub.to_le_bytes());
    b.extend_from_slice(&0x0000u16.to_le_bytes()); // DllCharacteristics: no ASLR
    b.extend_from_slice(&0x0010_0000u64.to_le_bytes());
    b.extend_from_slice(&0x0001_0000u64.to_le_bytes());
    b.extend_from_slice(&0x0010_0000u64.to_le_bytes());
    b.extend_from_slice(&0x0001_0000u64.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&export_rva.to_le_bytes());
    b.extend_from_slice(&export_size.to_le_bytes());
    b.extend_from_slice(&(idata_rva as u32).to_le_bytes());
    b.extend_from_slice(&(idata_size as u32).to_le_bytes());
    b.extend(&[0u8; 128 - 16]);
    assert_eq!(b.len() - os_off, 240, "opt header size mismatch");

    // Section headers
    b.extend_from_slice(&sec(b".text\0\0\0",  buf.code.len() as u32,   text_rva as u32,  text_raw as u32,  text_foff as u32,  0x6000_0020));
    b.extend_from_slice(&sec(b".rdata\0\0",   buf.rodata.len() as u32, rdata_rva as u32, rdata_raw as u32, rdata_foff as u32, 0xC000_0040));
    b.extend_from_slice(&sec(b".idata\0\0",   idata_size as u32,       idata_rva as u32, idata_size_a as u32, idata_foff as u32, 0xC000_0040));

    while b.len() < hdr_raw { b.push(0); }
    b.extend_from_slice(&buf.code);
    while b.len() < text_foff + text_raw { b.push(0); }
    b.extend_from_slice(&buf.rodata);
    while b.len() < rdata_foff + rdata_raw { b.push(0); }

    // .idata
    let mut idata = vec![0u8; idata_size_a];
    let int_rva = (idata_rva + int_off) as u32;
    idata[0..4].copy_from_slice(&int_rva.to_le_bytes());
    idata[12..16].copy_from_slice(&((idata_rva + dll_off) as u32).to_le_bytes());
    idata[16..20].copy_from_slice(&((idata_rva + iat_off) as u32).to_le_bytes());
    for (i, name_off) in [n_gsh, n_wf, n_ep, n_va].iter().enumerate() {
        let rva = (idata_rva + name_off) as u64;
        idata[iat_off+i*8..iat_off+i*8+8].copy_from_slice(&rva.to_le_bytes());
        idata[int_off+i*8..int_off+i*8+8].copy_from_slice(&rva.to_le_bytes());
    }
    for (off, name) in [
        (n_gsh, "GetStdHandle\0"), (n_wf, "WriteFile\0"),
        (n_ep,  "ExitProcess\0"),  (n_va, "VirtualAlloc\0"),
    ] {
        idata[off+2..off+2+name.len()].copy_from_slice(name.as_bytes());
    }
    idata[dll_off..dll_off+13].copy_from_slice(b"KERNEL32.dll\0");
    b.extend_from_slice(&idata);
    b
}

fn sec(n:&[u8;8],vs:u32,rva:u32,raw:u32,fo:u32,ch:u32)->[u8;40]{
    let mut h=[0u8;40];
    h[0..8].copy_from_slice(n);
    h[8..12].copy_from_slice(&vs.to_le_bytes()); h[12..16].copy_from_slice(&rva.to_le_bytes());
    h[16..20].copy_from_slice(&raw.to_le_bytes()); h[20..24].copy_from_slice(&fo.to_le_bytes());
    h[36..40].copy_from_slice(&ch.to_le_bytes()); h
}