use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/embedded_packages.rs"));

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub root: PathBuf,
    pub manifest: Value,
}

impl Package {
    pub fn version(&self) -> String {
        self.manifest.get("version").and_then(|v| v.as_str()).unwrap_or("0.0.0").to_string()
    }

    pub fn main_file(&self) -> PathBuf {
        let m = self.manifest.get("main").and_then(|v| v.as_str()).map(|s| s.to_string())
            .unwrap_or_else(|| format!("lib/{}.dgn", self.name));
        self.root.join(m)
    }

    pub fn sub_file(&self, sub: &str) -> Option<PathBuf> {
        let lib = self.root.join("lib");
        for cand in [lib.join(format!("{}.dgn", sub)), lib.join(sub).join("index.dgn")] {
            if cand.is_file() { return Some(cand); }
        }
        None
    }

    pub fn android_str(&self, key: &str) -> Option<String> {
        self.manifest.get("android")?.get(key)?.as_str().map(|s| s.to_string())
    }
    pub fn android_list(&self, key: &str) -> Vec<String> {
        self.manifest.get("android").and_then(|a| a.get(key)).and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default()
    }
    pub fn ios_str(&self, key: &str) -> Option<String> {
        self.manifest.get("ios")?.get(key)?.as_str().map(|s| s.to_string())
    }
    pub fn windows_str(&self, key: &str) -> Option<String> {
        self.manifest.get("windows")?.get(key)?.as_str().map(|s| s.to_string())
    }
    pub fn windows_list(&self, key: &str) -> Vec<String> {
        self.manifest.get("windows").and_then(|a| a.get(key)).and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default()
    }
    pub fn dependencies(&self) -> Vec<String> {
        self.manifest.get("dependencies").and_then(|d| d.as_object()).map(|o| o.keys().cloned().collect()).unwrap_or_default()
    }
}

fn read_package(dir: &Path) -> Option<Package> {
    let text = fs::read_to_string(dir.join("package.json")).ok()?;
    let manifest: Value = serde_json::from_str(&text).ok()?;
    let name = manifest.get("name").and_then(|n| n.as_str())
        .map(|s| s.to_string())
        .or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()))?;
    Some(Package { name, root: dir.to_path_buf(), manifest })
}

fn home_dir() -> PathBuf {
    std::env::var_os("DRAGON_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".dragon")))
        .unwrap_or_else(|| std::env::temp_dir().join("dragon_home"))
}

pub fn embedded_root() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(embedded_root_uncached).clone()
}

fn embedded_root_uncached() -> PathBuf {
    let mut h: u64 = 1469598103934665603;
    for (name, bytes) in EMBEDDED_PACKAGES {
        for b in name.bytes().chain(bytes.iter().copied()) { h = (h ^ b as u64).wrapping_mul(1099511628211); }
    }
    let root = home_dir().join("packages").join(format!("{:016x}", h));
    if !root.join(".complete").exists() {
        let _ = fs::remove_dir_all(&root);
        for (rel, bytes) in EMBEDDED_PACKAGES {
            let p = root.join(rel);
            if let Some(parent) = p.parent() { let _ = fs::create_dir_all(parent); }
            let _ = fs::write(&p, bytes);
        }
        let _ = fs::create_dir_all(&root);
        let _ = fs::write(root.join(".complete"), "ok");
    }
    root
}

pub fn package_roots(app_dir: &Path) -> BTreeMap<String, Package> {
    let mut out: BTreeMap<String, Package> = BTreeMap::new();
    let mut add_dir = |out: &mut BTreeMap<String, Package>, dir: &Path, name_hint: Option<&str>| {
        if let Some(mut p) = read_package(dir) {
            if let Some(n) = name_hint { p.name = n.to_string(); }
            out.entry(p.name.clone()).or_insert(p);
        }
    };
    if let Ok(entries) = fs::read_dir(app_dir.join("packages")) {
        let mut v: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        v.sort();
        for d in v { add_dir(&mut out, &d, None); }
    }
    if let Ok(text) = fs::read_to_string(app_dir.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(deps) = v.get("dependencies").and_then(|d| d.as_object()) {
                for (name, spec) in deps {
                    let path = match spec {
                        Value::String(s) => s.strip_prefix("path:").map(|p| p.to_string()),
                        Value::Object(o) => o.get("path").and_then(|p| p.as_str()).map(|s| s.to_string()),
                        _ => None,
                    };
                    if let Some(p) = path { add_dir(&mut out, &app_dir.join(p), Some(name)); }
                }
            }
        }
    }
    if let Some(var) = std::env::var_os("DRAGON_PACKAGES") {
        for base in std::env::split_paths(&var) {
            if let Ok(entries) = fs::read_dir(&base) {
                let mut v: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
                v.sort();
                for d in v { add_dir(&mut out, &d, None); }
            }
        }
    }
    let emb = embedded_root();
    if let Ok(entries) = fs::read_dir(&emb) {
        let mut v: Vec<_> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        v.sort();
        for d in v { add_dir(&mut out, &d, None); }
    }
    out
}

pub fn resolve_package_import(spec: &str, roots: &BTreeMap<String, Package>) -> Result<PathBuf, String> {
    let rest = spec.strip_prefix("package:").ok_or_else(|| format!("'{}' is not a package import", spec))?;
    let (name, sub) = match rest.split_once('/') { Some((n, s)) => (n, Some(s)), None => (rest, None) };
    let pkg = roots.get(name).ok_or_else(|| {
        let known: Vec<&String> = roots.keys().collect();
        format!("package '{}' not found (known packages: {}). Put it in packages/{} or list it under \"dependencies\" in package.json.",
            name, if known.is_empty() { "none".to_string() } else { known.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ") }, name)
    })?;
    match sub {
        None => {
            let f = pkg.main_file();
            if f.is_file() { Ok(f) } else { Err(format!("package '{}' has no main file {}", name, f.display())) }
        }
        Some(s) => pkg.sub_file(s).ok_or_else(|| format!("package '{}' has no module '{}'", name, s)),
    }
}

pub fn app_root_of(entry: &Path) -> PathBuf {
    let start = entry.parent().unwrap_or(Path::new("."));
    let start = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    let mut cur: Option<&Path> = Some(&start);
    while let Some(d) = cur {
        if d.join("package.json").is_file() { return d.to_path_buf(); }
        cur = d.parent();
    }
    if start.file_name().map(|n| n == "lib").unwrap_or(false) {
        if let Some(p) = start.parent() { return p.to_path_buf(); }
    }
    start
}

fn scan_imports(dir: &Path, out: &mut BTreeSet<String>) {
    let entries = match fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() { scan_imports(&p, out); }
        else if p.extension().map(|x| x == "dgn").unwrap_or(false) {
            if let Ok(src) = fs::read_to_string(&p) {
                for line in src.lines() {
                    let t = line.trim_start();
                    if let Some(rest) = t.strip_prefix("import") {
                        let rest = rest.trim_start();
                        if let Some(r) = rest.strip_prefix("\"package:") {
                            if let Some(end) = r.find('"') {
                                let name = r[..end].split('/').next().unwrap_or("").to_string();
                                if !name.is_empty() { out.insert(name); }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub fn used_packages(app_dir: &Path) -> Vec<Package> {
    let roots = package_roots(app_dir);
    let mut names: BTreeSet<String> = BTreeSet::new();
    scan_imports(&app_dir.join("lib"), &mut names);
    if let Ok(text) = fs::read_to_string(app_dir.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(d) = v.get("dependencies").and_then(|d| d.as_object()) { for k in d.keys() { names.insert(k.clone()); } }
        }
    }
    let mut result: Vec<Package> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = names.into_iter().collect();
    while let Some(n) = queue.pop() {
        if !seen.insert(n.clone()) { continue; }
        if let Some(p) = roots.get(&n) {
            let mut more = BTreeSet::new();
            scan_imports(&p.root.join("lib"), &mut more);
            for d in p.dependencies() { more.insert(d); }
            for m in more { queue.push(m); }
            result.push(p.clone());
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    result
}
