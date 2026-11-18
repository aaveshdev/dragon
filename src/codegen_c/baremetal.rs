use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::engine::Stmt;

const FILES: &[(&str, &str)] = &[
    ("console.c", include_str!("../../baremetal/console.c")),
    ("cursor.c", include_str!("../../baremetal/cursor.c")),
    ("input_hl.c", include_str!("../../baremetal/input_hl.c")),
    ("sched.c", include_str!("../../baremetal/sched.c")),
    ("font8x13.h", include_str!("../../baremetal/font8x13.h")),
    ("gfx.c", include_str!("../../baremetal/gfx.c")),
    ("include/assert.h", include_str!("../../baremetal/include/assert.h")),
    ("include/dgn_pcpu.h", include_str!("../../baremetal/include/dgn_pcpu.h")),
    ("include/inttypes.h", include_str!("../../baremetal/include/inttypes.h")),
    ("include/arpa/inet.h", include_str!("../../baremetal/include/arpa/inet.h")),
    ("include/ctype.h", include_str!("../../baremetal/include/ctype.h")),
    ("include/dirent.h", include_str!("../../baremetal/include/dirent.h")),
    ("include/dlfcn.h", include_str!("../../baremetal/include/dlfcn.h")),
    ("include/errno.h", include_str!("../../baremetal/include/errno.h")),
    ("include/math.h", include_str!("../../baremetal/include/math.h")),
    ("include/netdb.h", include_str!("../../baremetal/include/netdb.h")),
    ("include/netinet/in.h", include_str!("../../baremetal/include/netinet/in.h")),
    ("include/pthread.h", include_str!("../../baremetal/include/pthread.h")),
    ("include/regex.h", include_str!("../../baremetal/include/regex.h")),
    ("include/setjmp.h", include_str!("../../baremetal/include/setjmp.h")),
    ("include/stdio.h", include_str!("../../baremetal/include/stdio.h")),
    ("include/stdlib.h", include_str!("../../baremetal/include/stdlib.h")),
    ("include/string.h", include_str!("../../baremetal/include/string.h")),
    ("include/sys/socket.h", include_str!("../../baremetal/include/sys/socket.h")),
    ("include/sys/stat.h", include_str!("../../baremetal/include/sys/stat.h")),
    ("include/time.h", include_str!("../../baremetal/include/time.h")),
    ("include/unistd.h", include_str!("../../baremetal/include/unistd.h")),
    ("libc.c", include_str!("../../baremetal/libc.c")),
    ("x86_64/board.c", include_str!("../../baremetal/x86_64/board.c")),
    ("x86_64/disk.c", include_str!("../../baremetal/x86_64/disk.c")),
    ("x86_64/fb.c", include_str!("../../baremetal/x86_64/fb.c")),
    ("x86_64/input.c", include_str!("../../baremetal/x86_64/input.c")),
    ("x86_64/proc.c", include_str!("../../baremetal/x86_64/proc.c")),
    ("x86_64/kernel.c", include_str!("../../baremetal/x86_64/kernel.c")),
    ("x86_64/smp.c", include_str!("../../baremetal/x86_64/smp.c")),
    ("x86_64/ata.c", include_str!("../../baremetal/x86_64/ata.c")),
    ("x86_64/link.ld", include_str!("../../baremetal/x86_64/link.ld")),
    ("x86_64/start.S", include_str!("../../baremetal/x86_64/start.S")),
    ("user/crt0.S", include_str!("../../baremetal/user/crt0.S")),
    ("user/user.c", include_str!("../../baremetal/user/user.c")),
    ("user/link.ld", include_str!("../../baremetal/user/link.ld")),
    ("riscv64/board.c", include_str!("../../baremetal/riscv64/board.c")),
    ("aarch64/board.c", include_str!("../../baremetal/aarch64/board.c")),
    ("riscv64/disk.c", include_str!("../../baremetal/riscv64/disk.c")),
    ("aarch64/disk.c", include_str!("../../baremetal/aarch64/disk.c")),
    ("riscv64/fb.c", include_str!("../../baremetal/riscv64/fb.c")),
    ("aarch64/fb.c", include_str!("../../baremetal/aarch64/fb.c")),
    ("riscv64/input.c", include_str!("../../baremetal/riscv64/input.c")),
    ("aarch64/input.c", include_str!("../../baremetal/aarch64/input.c")),
    ("riscv64/proc.c", include_str!("../../baremetal/riscv64/proc.c")),
    ("aarch64/proc.c", include_str!("../../baremetal/aarch64/proc.c")),
    ("riscv64/kernel.c", include_str!("../../baremetal/riscv64/kernel.c")),
    ("aarch64/kernel.c", include_str!("../../baremetal/aarch64/kernel.c")),
    ("riscv64/smp.c", include_str!("../../baremetal/riscv64/smp.c")),
    ("aarch64/smp.c", include_str!("../../baremetal/aarch64/smp.c")),
    ("riscv64/ata.c", include_str!("../../baremetal/riscv64/ata.c")),
    ("aarch64/ata.c", include_str!("../../baremetal/aarch64/ata.c")),
    ("riscv64/link.ld", include_str!("../../baremetal/riscv64/link.ld")),
    ("aarch64/link.ld", include_str!("../../baremetal/aarch64/link.ld")),
    ("aarch64/link_pie.ld", include_str!("../../baremetal/aarch64/link_pie.ld")),
    ("riscv64/start.S", include_str!("../../baremetal/riscv64/start.S")),
    ("aarch64/start.S", include_str!("../../baremetal/aarch64/start.S")),
];

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for d in std::env::split_paths(&path) {
        let p = d.join(name);
        if p.is_file() { return Some(p); }
        if cfg!(windows) {
            let e = d.join(format!("{}.exe", name));
            if e.is_file() { return Some(e); }
        }
    }
    None
}

fn find_tool(names: &[&str]) -> Option<String> {
    for n in names { if let Some(p) = which(n) { return Some(p.to_string_lossy().to_string()); } }
    None
}

pub fn compile_baremetal(ast: &Stmt, src_path: &Path, out_path: &Path, keep_c: bool, arch: &str, run_qemu: bool, embeds: &[String], sdk_dir: Option<&str>) -> Result<(), String> {
    struct Arch { target: &'static str, flags: &'static [&'static str], dir: &'static str, link: &'static str, qemu: &'static str, qargs: &'static [&'static str], title: &'static str }
    let a = match arch {
        "riscv64" => Arch { target: "riscv64-unknown-elf", flags: &["-march=rv64imafdc", "-mabi=lp64d", "-mcmodel=medany"], dir: "riscv64", link: "riscv64/link.ld",
            qemu: "qemu-system-riscv64", qargs: &["-machine", "virt", "-bios", "none", "-nographic", "-m", "64M"], title: "RISC-V 64" },
        "aarch64" => Arch { target: "aarch64-unknown-none-elf", flags: &["-march=armv8-a", "-mno-outline-atomics", "-fno-pic", "-fno-pie"], dir: "aarch64", link: "aarch64/link.ld",
            qemu: "qemu-system-aarch64", qargs: &["-machine", "virt", "-cpu", "cortex-a72", "-nographic", "-m", "128M"], title: "AArch64" },
        "aarch64-uefi" => Arch { target: "aarch64-unknown-none-elf", flags: &["-march=armv8-a", "-mno-outline-atomics", "-fPIE"], dir: "aarch64", link: "aarch64/link_pie.ld",
            qemu: "qemu-system-aarch64", qargs: &[], title: "AArch64 (UEFI, position independent)" },
        "x86_64" => Arch { target: "x86_64-unknown-none-elf", flags: &["-mno-red-zone", "-mcmodel=kernel", "-mno-avx", "-mno-avx2", "-fno-pic", "-fno-pie", "-fno-math-errno"], dir: "x86_64", link: "x86_64/link.ld",
            qemu: "qemu-system-x86_64", qargs: &["-machine", "pc", "-nographic", "-m", "128M", "-no-reboot", "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04"], title: "x86-64" },
        "dxe" => Arch { target: "x86_64-unknown-none-elf", flags: &["-mno-red-zone", "-mcmodel=large", "-mno-avx", "-mno-avx2", "-fno-pic", "-fno-pie", "-fno-math-errno", "-DDGN_USER"], dir: "user", link: "user/link.ld",
            qemu: "qemu-system-x86_64", qargs: &[], title: "x86-64 user program (.dxe)" },
        _ => return Err(format!("unknown --baremetal target '{}'. Available: riscv64, aarch64, aarch64-uefi, x86_64, dxe", arch)),
    };
    let clang = find_tool(&["clang", "clang-18", "clang-17", "clang-16", "clang-19", "clang-20"])
        .ok_or("bare-metal builds need clang (with the RISC-V / AArch64 / x86-64 target) on PATH")?;
    let lld = find_tool(&["ld.lld", "ld.lld-18", "ld.lld-17", "ld.lld-16", "ld.lld-19", "ld.lld-20", "lld"])
        .ok_or("bare-metal builds need ld.lld on PATH")?;
    let res_out = Command::new(&clang).arg("-print-resource-dir").output().map_err(|e| format!("cannot run {}: {}", clang, e))?;
    let res = String::from_utf8_lossy(&res_out.stdout).trim().to_string();

    let base_dir = src_path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let flattened;
    let ast: &Stmt = match super::app::flatten_for_executable(ast, src_path)? {
        Some(f) => { flattened = f; &flattened }
        None => ast,
    };
    let c_src = super::transpile(ast, false, &base_dir);

    let work = std::env::temp_dir().join(format!("dragon_baremetal_{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    for (rel, text) in FILES.iter() {
        let p = work.join("sdk").join(rel);
        fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        fs::write(&p, text).map_err(|e| e.to_string())?;
    }
    let sdk = work.join("sdk");
    if let Some(dir) = sdk_dir {
        fn overlay(from: &Path, to: &Path) -> Result<(), String> {
            fs::create_dir_all(to).map_err(|e| e.to_string())?;
            for e in fs::read_dir(from).map_err(|e| format!("--sdk {}: {}", from.display(), e))?.flatten() {
                let p = e.path();
                let t = to.join(e.file_name());
                if p.is_dir() { overlay(&p, &t)?; } else { fs::copy(&p, &t).map_err(|e| e.to_string())?; }
            }
            Ok(())
        }
        overlay(Path::new(dir), &sdk)?;
    }
    let c_path = work.join("program.c");
    fs::write(&c_path, &c_src).map_err(|e| format!("Failed to write C source: {}", e))?;
    println!("  [baremetal] C source -> {}", c_path.display());

    let inc = sdk.join("include");
    let cflags: Vec<String> = vec![
        format!("--target={}", a.target),
        "-ffreestanding".into(), "-fno-builtin".into(), "-fno-stack-protector".into(), "-nostdinc".into(),
        "-isystem".into(), format!("{}/include", res), "-I".into(), inc.to_string_lossy().into(), "-O2".into(), "-w".into(), "-DDGN_BAREMETAL".into(),
    ];
    let mut cflags = cflags;
    for f in a.flags { cflags.push((*f).to_string()); }
    let compile = |src: &Path, obj: &Path| -> Result<(), String> {
        let st = Command::new(&clang).args(&cflags).arg("-c").arg(src).arg("-o").arg(obj).status()
            .map_err(|e| format!("Failed to launch {}: {}", clang, e))?;
        if st.success() { Ok(()) } else { Err(format!("compiling {} failed", src.display())) }
    };

    let is_dxe = arch == "dxe";
    let mut sources: Vec<String> = if is_dxe { vec!["crt0.S".to_string(), "user.c".to_string()] } else { ["start.S", "board.c", "kernel.c", "fb.c", "input.c", "disk.c", "proc.c", "smp.c", "ata.c"].iter().map(|f| f.to_string()).collect() }
        .into_iter().map(|f| format!("sdk/{}/{}", a.dir, f)).collect();
    let common: &[&str] = if is_dxe { &["libc.c", "gfx.c"] } else { &["sched.c", "input_hl.c", "gfx.c", "console.c", "cursor.c", "libc.c", "imgdec.c", "webpdec.c", "auddec.c"] };
    for f in common {
        if matches!(*f, "imgdec.c" | "webpdec.c" | "auddec.c") && !sdk.join(f).exists() { continue; }
        sources.push(format!("sdk/{}", f));
    }
    sources.push("program.c".to_string());
    if !is_dxe {

        let mut b = String::from("#include <stddef.h>\n");
        let mut names = Vec::new();
        for (i, f) in embeds.iter().enumerate() {
            let data_len = fs::metadata(f).map_err(|e| format!("--embed {}: {}", f, e))?.len() as usize;
            let abs = fs::canonicalize(f).map_err(|e| format!("--embed {}: {}", f, e))?;
            let ap = abs.to_string_lossy().replace('\\', "/").replace('"', "");
            b.push_str(&format!("__asm__(\".pushsection .rodata\\n.balign 4096\\nblob_{}_start:\\n.incbin \\\"{}\\\"\\n.byte 0\\n.popsection\\n\");\nextern const unsigned char blob_{}_start[];\n#define blob_{} blob_{}_start\n", i, ap, i, i, i));
            let base = Path::new(f).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            names.push((base, data_len));
        }
        b.push_str(&format!("int dgn_blob_count(void) {{ return {}; }}\n", names.len()));
        b.push_str("const char* dgn_blob_name(int i) { switch (i) {");
        for (i, (n, _)) in names.iter().enumerate() { b.push_str(&format!("case {}: return \"{}\"; ", i, n.replace('\\', "").replace('"', ""))); }
        b.push_str("} return 0; }\n");
        b.push_str("const void* dgn_blob_addr(int i) { switch (i) {");
        for i in 0..names.len() { b.push_str(&format!("case {}: return blob_{}; ", i, i)); }
        b.push_str("} return 0; }\n");
        b.push_str("size_t dgn_blob_size(int i) { switch (i) {");
        for (i, (_, n)) in names.iter().enumerate() { b.push_str(&format!("case {}: return {}; ", i, n)); }
        b.push_str("} return 0; }\n");
        fs::write(work.join("blobs.c"), b).map_err(|e| e.to_string())?;
        sources.push("blobs.c".to_string());
    }
    let mut o_files = Vec::new();
    if !is_dxe {
        let ext = sdk.join(a.dir).join("ext");
        if let Ok(rd) = fs::read_dir(&ext) {
            let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            v.sort();
            for p in v {
                match p.extension().and_then(|x| x.to_str()) {
                    Some("o") => o_files.push(p),
                    Some("c") => { let o = work.join(format!("ext_{}.o", o_files.len())); compile(&p, &o)?; o_files.push(o); }
                    _ => {}
                }
            }
        }
    }
    for (i, s) in sources.iter().enumerate() {
        let o = work.join(format!("obj{}.o", i));
        compile(&work.join(s), &o)?;
        o_files.push(o);
    }
    let mut ld = Command::new(&lld);
    let ld_out = if is_dxe { work.join("program.bin") } else { out_path.to_path_buf() };
    ld.arg("-T").arg(sdk.join(a.link)).arg("-o").arg(&ld_out);
    if is_dxe { ld.arg("--oformat=binary"); }
    if arch == "aarch64-uefi" { ld.arg("-pie").arg("-Bsymbolic").arg("--no-dynamic-linker").arg("-z").arg("norelro").arg("--hash-style=gnu"); }
    for o in &o_files { ld.arg(o); }
    let st = ld.status().map_err(|e| format!("Failed to launch {}: {}", lld, e))?;
    if !st.success() { return Err("linking failed".into()); }
    if is_dxe && keep_c {

        let mut ld2 = Command::new(&lld);
        ld2.arg("-T").arg(sdk.join(a.link)).arg("-o").arg(out_path.with_extension("elf"));
        for o in &o_files { ld2.arg(o); }
        let _ = ld2.status();
    }
    if is_dxe {
        let image = fs::read(&ld_out).map_err(|e| e.to_string())?;
        let name = out_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        fs::write(out_path, dxe_pack(&image, &name)).map_err(|e| e.to_string())?;
    }

    if keep_c {
        let kept = out_path.with_extension("c");
        let _ = fs::copy(&c_path, &kept);
        println!("  [baremetal] kept C source -> {}", kept.display());
    }
    let _ = fs::remove_dir_all(&work);
    let size = fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
    if !is_dxe { println!("Success! Bare-metal {} image: {} ({} bytes)", a.title, out_path.display(), size); }
    if is_dxe {
        println!("Success! .dxe program: {} ({} bytes); copy it into an operating system that loads .dxe files", out_path.display(), size);
        let _ = fs::remove_dir_all(&work);
        return Ok(());
    }
    let qemu_cmd = format!("{} {} -kernel {}", a.qemu, a.qargs.join(" "), out_path.display());
    if run_qemu {
        let q = find_tool(&[a.qemu]).ok_or(format!("--qemu needs {} on PATH", a.qemu))?;
        println!("  [baremetal] running under QEMU (Ctrl-A then X quits)");
        let _ = Command::new(q).args(a.qargs).arg("-kernel").arg(out_path).status();
    } else {
        println!("  run it with: {}", qemu_cmd);
    }
    Ok(())
}

pub const DXE_BASE: u64 = 0x80_0000_0000;
pub const DXE_MEM: u64 = 8 << 20;
pub const DXE_STACK: u32 = 1 << 20;

pub fn dxe_pack(image: &[u8], name: &str) -> Vec<u8> {
    let mut h = Vec::with_capacity(64 + image.len());
    h.extend_from_slice(b"DXE1");
    h.extend_from_slice(&1u32.to_le_bytes());
    h.extend_from_slice(&DXE_BASE.to_le_bytes());
    h.extend_from_slice(&DXE_BASE.to_le_bytes());
    h.extend_from_slice(&(image.len() as u64).to_le_bytes());
    h.extend_from_slice(&DXE_MEM.to_le_bytes());
    h.extend_from_slice(&DXE_STACK.to_le_bytes());
    h.extend_from_slice(&0u32.to_le_bytes());
    let mut n = [0u8; 16];
    for (i, b) in name.bytes().take(15).enumerate() { n[i] = b; }
    h.extend_from_slice(&n);
    h.extend_from_slice(image);
    h
}
