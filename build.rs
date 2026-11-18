use std::io::Write;
use std::path::Path;

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = match std::fs::read_dir(dir) { Ok(e) => e.flatten().collect(), Err(_) => return };
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() { walk(&p, base, out); }
        else if let Ok(rel) = p.strip_prefix(base) { out.push(rel.to_string_lossy().replace('\\', "/")); }
    }
}

fn embed_packages() {
    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("packages");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("embedded_packages.rs");
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    let mut f = std::fs::File::create(out).unwrap();
    writeln!(f, "pub const EMBEDDED_PACKAGES: &[(&str, &[u8])] = &[").unwrap();
    for rel in &files {
        writeln!(f, "    ({:?}, include_bytes!({:?})),", rel, root.join(rel).to_string_lossy()).unwrap();
    }
    writeln!(f, "];").unwrap();
    println!("cargo:rerun-if-changed=packages");
}

fn main() {
    embed_packages();
    let date = chrono::Utc::now().format("%Y-%m-%d");
    println!("cargo:rustc-env=DGN_BUILD_DATE={}", date);

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=NONEXISTENT_ALWAYS_REBUILD");
}
