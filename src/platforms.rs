use serde_json::Value;
use std::fs;
use std::path::Path;

pub const ALL: [&str; 3] = ["android", "ios", "windows"];

pub fn normalize(s: &str) -> Option<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "android" => Some("android"),
        "ios" => Some("ios"),
        "windows" | "win" | "win64" | "windows-x64" => Some("windows"),
        _ => None,
    }
}

pub fn parse_list(s: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for part in s.split(',').map(|p| p.trim()).filter(|p| !p.is_empty()) {
        if part.eq_ignore_ascii_case("all") {
            for a in ALL { if !out.iter().any(|x| x == a) { out.push(a.to_string()); } }
            continue;
        }
        match normalize(part) {
            Some(p) => { if !out.iter().any(|x| x == p) { out.push(p.to_string()); } }
            None => return Err(format!("unknown platform '{}' (choose from: android, ios, windows, all)", part)),
        }
    }
    if out.is_empty() { return Err("no platforms given".into()); }
    Ok(out)
}

pub fn declared_in(json: &Value) -> Vec<String> {
    if let Some(arr) = json.get("platforms").and_then(|v| v.as_array()) {
        let v: Vec<String> = arr.iter().filter_map(|x| x.as_str()).filter_map(normalize).map(|s| s.to_string()).collect();
        if !v.is_empty() { return v; }
    }
    if let Some(s) = json.get("platform").and_then(|v| v.as_str()) {
        if let Some(p) = normalize(s) { return vec![p.to_string()]; }
    }
    vec!["android".to_string()]
}

pub fn declared(pkg_path: &Path) -> Vec<String> {
    fs::read_to_string(pkg_path).ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .map(|j| declared_in(&j))
        .unwrap_or_else(|| vec!["android".to_string()])
}

pub fn host() -> &'static str {
    if cfg!(target_os = "windows") { "windows" } else if cfg!(target_os = "macos") { "ios" } else { "android" }
}

pub fn default_target(declared: &[String]) -> String {
    let h = host();
    if declared.iter().any(|d| d == h) { h.to_string() } else { declared[0].clone() }
}

pub fn flag_value(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{}=", name);
    for (i, a) in args.iter().enumerate() {
        if a == name { return args.get(i + 1).cloned(); }
        if let Some(v) = a.strip_prefix(&eq) { return Some(v.to_string()); }
    }
    None
}

fn check_flags(args: &[String]) -> Result<(), String> {
    for a in args.iter().skip(2) {
        if let Some(f) = a.strip_prefix("--") {
            let name = f.split('=').next().unwrap_or("");
            if name.starts_with("pla") && name != "platform" && name != "platforms" {
                return Err(format!("unknown option '--{}' (did you mean --platform?)", name));
            }
        }
    }
    Ok(())
}

pub fn requested(args: &[String], declared: &[String]) -> Result<Vec<String>, String> {
    check_flags(args)?;
    let v = match flag_value(args, "--platform").or_else(|| flag_value(args, "--platforms")) {
        Some(s) => parse_list(&s)?,
        None => return Ok(vec![default_target(declared)]),
    };
    for p in &v {
        if !declared.contains(p) {
            return Err(format!("this project doesn't list '{}' in package.json \"platforms\" (it has: {})", p, declared.join(", ")));
        }
    }
    Ok(v)
}

pub fn requested_build(args: &[String], declared: &[String]) -> Result<Vec<String>, String> {
    check_flags(args)?;
    if flag_value(args, "--platform").or_else(|| flag_value(args, "--platforms")).is_none() { return Ok(declared.to_vec()); }
    requested(args, declared)
}

pub fn package_json(name: &str, package: &str, platforms: &[String]) -> String {
    let list = platforms.iter().map(|p| format!("\"{}\"", p)).collect::<Vec<_>>().join(", ");
    let mut s = format!("{{\n  \"name\": \"{}\",\n  \"package\": \"{}\",\n  \"platforms\": [{}],\n  \"runtime\": \"app\",\n  \"main\": \"lib/main.dgn\"", name, package, list);
    if platforms.iter().any(|p| p == "android") { s.push_str(",\n  \"androidDir\": \"android\",\n  \"abis\": [\"arm64-v8a\", \"x86_64\"]"); }
    if platforms.iter().any(|p| p == "ios") { s.push_str(&format!(",\n  \"ios\": {{ \"bundleId\": \"{}\", \"minVersion\": \"16.0\" }}", package)); }
    if platforms.iter().any(|p| p == "windows") { s.push_str(&format!(",\n  \"windows\": {{ \"title\": \"{}\" }}", name)); }
    s.push_str("\n}\n");
    s
}
