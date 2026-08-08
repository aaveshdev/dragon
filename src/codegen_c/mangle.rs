#![allow(dead_code)]

pub fn mangle(name: &str) -> String {
    let mut out = String::from("_v_");
    for c in name.chars() {
        if c.is_alphanumeric() || c == '_' { out.push(c); } else { out.push('_'); }
    }
    out
}

pub(crate) fn escape_c_string(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 32 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

