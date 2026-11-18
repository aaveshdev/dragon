use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Table {
    pub names: HashMap<String, String>,
    pub args: HashMap<String, String>,
}

pub fn parse_table(text: &str) -> Table {
    let mut names = HashMap::new();
    let mut args = HashMap::new();
    let mut in_args = false;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') { continue; }
        if l == "[args]" { in_args = true; continue; }
        if let Some((a, b)) = l.split_once('=') {
            let (a, b) = (a.trim().to_string(), b.trim().to_string());
            if in_args { args.insert(a, b); } else { names.insert(a, b); }
        }
    }
    Table { names, args }
}

pub fn builtin_table() -> Table { parse_table(include_str!("renames.txt")) }

fn is_ident_start(c: char) -> bool { c.is_ascii_alphabetic() || c == '_' }
fn is_ident(c: char) -> bool { c.is_ascii_alphanumeric() || c == '_' }

pub fn migrate_source(src: &str, t: &Table, rename_strings: bool) -> (String, usize) {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut n = 0usize;
    let mut i = 0usize;
    let mut stack: Vec<char> = Vec::new();
    let mut prev_sig: char = '\0';
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' { out.push(chars[i]); i += 1; }
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            out.push_str("/*"); i += 2;
            while i < chars.len() && !(chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '/') { out.push(chars[i]); i += 1; }
            if i < chars.len() { out.push_str("*/"); i += 2; }
            continue;
        }
        if c == '"' || c == '\'' || (c == 'r' && i + 1 < chars.len() && (chars[i + 1] == '"' || chars[i + 1] == '\'') && (i == 0 || !is_ident(chars[i - 1]))) {
            let raw = c == 'r';
            if raw { out.push('r'); i += 1; }
            let q = chars[i];
            let start = i;
            out.push(q); i += 1;
            let mut body = String::new();
            while i < chars.len() && chars[i] != q {
                if chars[i] == '\\' && !raw && i + 1 < chars.len() { body.push(chars[i]); body.push(chars[i + 1]); i += 2; continue; }
                body.push(chars[i]); i += 1;
            }
            let _ = start;
            if rename_strings {
                if let Some(new) = t.names.get(&body) { out.push_str(new); n += 1; } else { out.push_str(&body); }
            } else {
                out.push_str(&body);
            }
            if i < chars.len() { out.push(q); i += 1; }
            prev_sig = q;
            continue;
        }
        if c == '`' {
            out.push('`'); i += 1;
            while i < chars.len() && chars[i] != '`' {
                if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
                    let mut depth = 0; let mut j = i + 1; let s = j + 1;
                    while j < chars.len() {
                        if chars[j] == '{' { depth += 1; } else if chars[j] == '}' { depth -= 1; if depth == 0 { break; } }
                        j += 1;
                    }
                    let inner: String = chars[s..j.min(chars.len())].iter().collect();
                    let (m, k) = migrate_source(&inner, t, rename_strings);
                    n += k;
                    out.push_str("${"); out.push_str(&m); out.push('}');
                    i = (j + 1).min(chars.len());
                    continue;
                }
                if chars[i] == '\\' && i + 1 < chars.len() { out.push(chars[i]); out.push(chars[i + 1]); i += 2; continue; }
                out.push(chars[i]); i += 1;
            }
            if i < chars.len() { out.push('`'); i += 1; }
            prev_sig = '`';
            continue;
        }
        if is_ident_start(c) {
            let s = i;
            while i < chars.len() && is_ident(chars[i]) { i += 1; }
            let word: String = chars[s..i].iter().collect();
            let mut k = i;
            while k < chars.len() && (chars[k] == ' ' || chars[k] == '\t') { k += 1; }
            let next = if k < chars.len() { chars[k] } else { '\0' };
            let enclosing = stack.last().copied().unwrap_or('\0');
            let after_dot = prev_sig == '.';
            if let Some(new) = t.names.get(&word) {
                out.push_str(new); n += 1;
            } else if let Some(new) = t.args.get(&word) {
                let map_key = enclosing == '{' && next == ':' && (prev_sig == '{' || prev_sig == ',') && !after_dot;
                if map_key { out.push_str(&word); } else { out.push_str(new); n += 1; }
            } else {
                out.push_str(&word);
            }
            prev_sig = 'a';
            continue;
        }
        match c {
            '(' | '[' | '{' => stack.push(c),
            ')' | ']' | '}' => { stack.pop(); }
            _ => {}
        }
        out.push(c);
        if !c.is_whitespace() { prev_sig = c; }
        i += 1;
    }
    (out, n)
}

pub fn declared_names(src: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for line in src.lines() {
        let t = line.trim_start();
        for kw in ["class ", "func "] {
            if let Some(rest) = t.strip_prefix(kw) {
                let name: String = rest.chars().take_while(|c| is_ident(*c)).collect();
                if !name.is_empty() { out.insert(name); }
            }
        }
    }
    out
}

pub fn migrate_markdown(src: &str, t: &Table, rename_strings: bool) -> (String, usize) {
    let mut out = String::with_capacity(src.len());
    let mut total = 0usize;
    let mut block = String::new();
    let mut in_code = false;
    let mut migrate_this = false;
    for line in src.split_inclusive('\n') {
        let tr = line.trim_start();
        if tr.starts_with("```") {
            if in_code {
                if migrate_this { let (o, n) = migrate_source(&block, t, rename_strings); total += n; out.push_str(&o); } else { out.push_str(&block); }
                block.clear();
                in_code = false;
                out.push_str(line);
            } else {
                in_code = true;
                let lang = tr.trim_start_matches('`').trim();
                migrate_this = lang.is_empty() || lang == "dragon" || lang == "dgn";
                out.push_str(line);
            }
        } else if in_code { block.push_str(line); } else { out.push_str(line); }
    }
    out.push_str(&block);
    (out, total)
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().map(|e| e == "dgn" || e == "md").unwrap_or(false) { out.push(path.to_path_buf()); }
    } else if let Ok(rd) = fs::read_dir(path) {
        let mut v: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        v.sort();
        for p in v {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') || name == "android" || name == "ios" || name == "node_modules" { continue; }
            collect(&p, out);
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut dry = false;
    let mut strings = false;
    let mut table_file: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dry-run" => dry = true,
            "--strings" => strings = true,
            "--table" => { i += 1; table_file = args.get(i).cloned(); }
            p => paths.push(PathBuf::from(p)),
        }
        i += 1;
    }
    if paths.is_empty() { paths.push(PathBuf::from(".")); }
    let table = match &table_file {
        Some(f) => match fs::read_to_string(f) { Ok(t) => parse_table(&t), Err(e) => { eprintln!("can't read {}: {}", f, e); return 1; } },
        None => builtin_table(),
    };
    let mut files = Vec::new();
    for p in &paths { collect(p, &mut files); }
    if files.is_empty() { eprintln!("no .dgn or .md files found"); return 1; }
    let sources: Vec<(PathBuf, String)> = files.into_iter().filter_map(|f| fs::read_to_string(&f).ok().map(|s| (f, s))).collect();
    let newnames: HashSet<&String> = table.names.values().collect();
    let mut conflicts = Vec::new();
    for (f, s) in sources.iter().filter(|(f, _)| f.extension().map(|e| e == "dgn").unwrap_or(false)) {
        if migrate_source(s, &table, strings).1 == 0 { continue; }
        for d in declared_names(s) {
            if newnames.contains(&d) && !table.names.contains_key(&d) {
                conflicts.push(format!("{}: declares '{}', which is now a built-in name; rename yours first", f.display(), d));
            }
        }
    }
    if !conflicts.is_empty() {
        for c in &conflicts { eprintln!("{}", c); }
        eprintln!("nothing changed.");
        return 1;
    }
    let mut total = 0usize;
    let mut changed_files = 0usize;
    for (f, s) in &sources {
        let is_md = f.extension().map(|e| e == "md").unwrap_or(false);
        let (out, n) = if is_md { migrate_markdown(s, &table, strings) } else { migrate_source(s, &table, strings) };
        if n > 0 {
            total += n; changed_files += 1;
            if !dry { if let Err(e) = fs::write(f, out) { eprintln!("can't write {}: {}", f.display(), e); return 1; } }
            println!("{} {} ({} names)", if dry { "would change" } else { "migrated" }, f.display(), n);
        }
    }
    println!("{} file(s), {} name(s){}", changed_files, total, if dry { " (dry run)" } else { "" });
    0
}
