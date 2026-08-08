use super::*;

pub const KEYWORDS: &[&str] = &[
    "func","class","if","elif","else","while","return","True","False","null",
    "and","or","not","for","in","switch","case","default","break","continue",
    "import","as","try","catch","finally","raise","do","new","extends","this",
    "super", "const", "var", "async", "await"
];

pub fn lex(src: &str) -> Result<Vec<Token>, DgnError>{
    let mut tokens = Vec::new();
    let bytes = src.as_bytes();
    let len = src.len();
    let mut pos = 0;
    let mut line = 1usize;
    let mut col = 0usize;

    while pos < len {
        let start = pos;
        let col_start = col + 1;
        let b = bytes[pos];

        // Raw Strings (e.g., r"\d+" or r'\w+')
        if b == b'r' && pos + 1 < len && (bytes[pos+1] == b'"' || bytes[pos+1] == b'\'') {
            let quote = bytes[pos+1];
            pos += 2; // Consume 'r' and the opening quote
            let mut s = String::new();
            while pos < len && bytes[pos] != quote {
                s.push(bytes[pos] as char);
                if bytes[pos] == b'\n' { line += 1; col = 0; }
                pos += 1;
            }

            if pos >= len {
                return Err(DgnError::with_col("Unclosed raw string literal", line, col_start));
            }
            
            pos += 1; // Consume closing quote
            col += pos - start;
            
            // Output as a standard String token, but without ANY escape processing!
            tokens.push(Token { typ: TokenType::String, value: s, line, col: col_start, num_val: None });
            continue;
        }

        // Skip whitespace fast
        if b == b' ' || b == b'\t' || b == b'\r' || b == b'\xEF' || b == 0 {
            pos += 1; col += 1;
            continue;
        }

        if b == b'\n' {
            line += 1; col = 0; pos += 1;
            continue;
        }

        // Comments
        if b == b'/' && pos + 1 < len {
            if bytes[pos+1] == b'/' {
                while pos < len && bytes[pos] != b'\n' { pos += 1; }
                continue;
            }
            if bytes[pos+1] == b'*' {
                pos += 2;
                while pos + 1 < len && !(bytes[pos] == b'*' && bytes[pos+1] == b'/') {
                    if bytes[pos] == b'\n' { line += 1; col = 0; }
                    pos += 1;
                }
                pos += 2;
                continue;
            }
        }

        // Backtick strings (interpolated): `Hello {name}!`
        // We store the raw content in a String token with typ = TokenType::Name and value prefixed by `\x01`
        // so parse_primary can reconstruct the interpolation chain.
        if b == b'`' {
            pos += 1;
            let mut raw = String::new();
            while pos < len && bytes[pos] != b'`' {
                if bytes[pos] == b'\\' && pos + 1 < len {
                    pos += 1;
                    match bytes[pos] {
                        b'n'  => raw.push('\n'),
                        b't'  => raw.push('\t'),
                        b'r'  => raw.push('\r'),
                        b'0'  => raw.push('\0'),
                        b'`'  => raw.push('`'),
                        b'\\' => raw.push('\\'),
                        b'f'  => raw.push('\x0C'),
                        b'u'  => {
                            let mut hex = String::new();
                            for _ in 0..4 {
                                pos += 1;
                                if pos < len && (bytes[pos] as char).is_ascii_hexdigit() {
                                    hex.push(bytes[pos] as char);
                                }
                            }
                            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                if let Some(ch) = char::from_u32(code) {
                                    raw.push(ch);
                                }
                            }
                        }
                        c     => { raw.push('\\'); raw.push(c as char); }
                    }
                } else {
                    if bytes[pos] == b'\n' { line += 1; col = 0; }
                    raw.push(bytes[pos] as char);
                }
                pos += 1;
            }
            if pos >= len {
                return Err(DgnError::with_col("Unclosed backtick string literal", line, col_start));
            }
            pos += 1; // closing backtick
            col += pos - start;
            // Prefix with \x01 so parse_primary knows this is an interpolated string
            tokens.push(Token { typ: TokenType::String, value: format!("\x01{}", raw), line, col: col_start, num_val: None });
            continue;
        }

        // Strings
        if b == b'"' || b == b'\'' {
            pos += 1;
            let mut s = String::new();
            while pos < len && bytes[pos] != b {
                if bytes[pos] == b'\\' && pos + 1 < len {
                    pos += 1;
                    match bytes[pos] {
                        b'n'  => s.push('\n'),
                        b't'  => s.push('\t'),
                        b'r'  => s.push('\r'),
                        b'0'  => s.push('\0'),
                        b'"'  => s.push('"'),
                        b'\'' => s.push('\''),
                        b'\\' => s.push('\\'),
                        b'f'  => s.push('\x0C'),
                        b'u'  => {
                            // \uXXXX Unicode escape
                            let mut hex = String::new();
                            for _ in 0..4 {
                                pos += 1;
                                if pos < len && (bytes[pos] as char).is_ascii_hexdigit() {
                                    hex.push(bytes[pos] as char);
                                }
                            }
                            if let Ok(code) = u32::from_str_radix(&hex, 16) {
                                if let Some(ch) = char::from_u32(code) {
                                    s.push(ch);
                                }
                            }
                        }
                        c     => { s.push('\\'); s.push(c as char); }
                    }
                } else {
                    s.push(bytes[pos] as char);
                }
                pos += 1;
            }

            if pos >= len {
                return Err(DgnError::with_col("Unclosed string literal", line, col_start));
            }
            pos += 1; // closing quote
            col += pos - start;
            tokens.push(Token { typ: TokenType::String, value: s, line, col: col_start, num_val: None });
            continue;
        }

        // Numbers (including hex)
        if b.is_ascii_digit() || (b == b'0' && pos+1 < len && bytes[pos+1] == b'x') {
            if b == b'0' && pos+1 < len && bytes[pos+1] == b'x' {
                pos += 2;
                let hex_start = pos;
                while pos < len && bytes[pos].is_ascii_hexdigit() { pos += 1; }
                let hex_str = &src[hex_start..pos];
                let n = u64::from_str_radix(hex_str, 16).unwrap_or(0) as f64;
                col += pos - start;
                tokens.push(Token { typ: TokenType::Number, value: src[start..pos].to_string(), line, col: col_start, num_val: Some(n) });
            } else {
                // 1. Parse whole number part
                while pos < len && bytes[pos].is_ascii_digit() { pos += 1; }
                
                // 2. Parse fractional part (e.g., .4028)
                if pos < len && bytes[pos] == b'.' && pos+1 < len && bytes[pos+1].is_ascii_digit() {
                    pos += 1;
                    while pos < len && bytes[pos].is_ascii_digit() { pos += 1; }
                }
                
                // 3. Parse scientific notation (e.g., e38, e-300, E+10)
                if pos < len && (bytes[pos] == b'e' || bytes[pos] == b'E') {
                    let mut e_pos = pos + 1;
                    
                    // Allow optional + or - after the E
                    if e_pos < len && (bytes[e_pos] == b'+' || bytes[e_pos] == b'-') {
                        e_pos += 1;
                    }
                    
                    // Only consume if the exponent actually contains digits
                    if e_pos < len && bytes[e_pos].is_ascii_digit() {
                        pos = e_pos;
                        while pos < len && bytes[pos].is_ascii_digit() { pos += 1; }
                    }
                }

                let n: f64 = src[start..pos].parse().unwrap_or(0.0);
                col += pos - start;
                tokens.push(Token { typ: TokenType::Number, value: src[start..pos].to_string(), line, col: col_start, num_val: Some(n) });
            }
            continue;
        }

        // Identifiers and keywords
        if b.is_ascii_alphabetic() || b == b'_' {
            while pos < len && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
                pos += 1;
            }
            let word = &src[start..pos];
            col += pos - start;
            if KEYWORDS.contains(&word) {
                tokens.push(Token { typ: TokenType::Keyword(word.to_string()), value: word.to_string(), line, col: col_start, num_val: None });
            } else {
                tokens.push(Token { typ: TokenType::Name, value: word.to_string(), line, col: col_start, num_val: None });
            }
            continue;
        }

        if pos + 2 < len {
            let three = &src[pos..pos+3];
            if three == "..." {
                tokens.push(Token { typ: TokenType::Op, value: "...".to_string(), line, col: col_start, num_val: None });
                pos += 3; col += 3;
                continue;
            }
        }

        // Two-char operators
        if pos + 1 < len {
            let two = &src[pos..pos+2];
            let tok = match two {
                "++" => Some(TokenType::Inc),
                "--" => Some(TokenType::Dec),
                "+=" | "-=" | "*=" | "/=" | "%=" => Some(TokenType::Op),
                "==" | "!=" | "<=" | ">=" | "=>" | "<<" | ">>" | "??" | "?." => Some(TokenType::Op),
                "//" | "/*" => None, // handled above
                _ => None,
            };
            if let Some(t) = tok {
                tokens.push(Token { typ: t, value: two.to_string(), line, col: col_start, num_val: None });
                pos += 2; col += 2;
                continue;
            }
        }

        // Single-char operators
        let op_chars = b"!+-*/%<>=?:;,&|^~(){}[].";
        if op_chars.contains(&b) {
            tokens.push(Token { typ: TokenType::Op, value: (b as char).to_string(), line, col: col_start, num_val: None });
            pos += 1; col += 1;
            continue;
        }

        // Unrecognized
        return Err(DgnError::with_col(
            &format!("Unexpected character '{}' (Code: {})", b as char, b), 
            line, 
            col
        ));
    }

    tokens.push(Token { typ: TokenType::Eof, value: String::new(), line, col, num_val: None });
    Ok(tokens)
}

/// Parse a backtick-interpolated string body (without the surrounding backticks).
/// Segments like `Hello {name + "!"}` become: "Hello " + str(name + "!")
/// Returns an Expr that is a chain of string concatenations.
/// Parse a backtick-interpolated string body (without the surrounding backticks).
/// Segments like `Hello ${name + "!"}` become: "Hello " + str(name + "!")
/// Returns an Expr that is a chain of string concatenations.
pub fn parse_interp_string(raw: &str, line: usize) -> Result<Expr, String> {
    // Split raw into alternating literal and expression segments
    let mut parts: Vec<Expr> = Vec::new();
    let mut chars = raw.chars().peekable();
    let mut cur = String::new();

    while let Some(c) = chars.next() {
        // Look for the '$' character followed by '{'
        if c == '$' {
            if let Some(&'{') = chars.peek() {
                chars.next(); // Consume the '{'
                
                // Push the current literal segment (possibly empty)
                if !cur.is_empty() {
                    parts.push(Expr { node: ExprNode::String(cur.clone()), line });
                    cur.clear();
                }
                
                // Collect expression source up to matching '}'
                let mut depth = 1usize;
                let mut expr_src = String::new();
                for ec in chars.by_ref() {
                    match ec {
                        '{' => { depth += 1; expr_src.push(ec); }
                        '}' => {
                            depth -= 1;
                            if depth == 0 { break; }
                            expr_src.push(ec);
                        }
                        _ => expr_src.push(ec),
                    }
                }
                
                // Parse the expression, wrap in str() call
                let toks = lex(&expr_src).map_err(|e| e.msg)?;
                let mut p = Parser::new(toks);
                let inner = p.parse_expr()?;
                // Coerce to string via str() builtin
                let str_call = Expr {
                    node: ExprNode::Call(
                        Box::new(Expr { node: ExprNode::Var("str".to_string()), line }),
                        vec![inner],
                    ),
                    line,
                };
                parts.push(str_call);
                
                // Skip the default push below, move to next character
                continue; 
            }
        }
        
        // If it's not `${`, just treat the character (including standalone '$' or '{') as plain text
        cur.push(c);
    }
    
    if !cur.is_empty() {
        parts.push(Expr { node: ExprNode::String(cur), line });
    }
    if parts.is_empty() {
        return Ok(Expr { node: ExprNode::String(String::new()), line });
    }
    
    // Fold into left-associative "+" chain
    let mut result = parts.remove(0);
    for part in parts {
        result = Expr {
            node: ExprNode::Binary("+".to_string(), Box::new(result), Box::new(part)),
            line,
        };
    }
    Ok(result)
}

