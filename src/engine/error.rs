use super::*;

#[derive(Debug, Clone)]
pub struct DgnError {
    pub msg: String,
    pub line: usize,
    pub col: usize,
    pub source_line: Option<String>,
}

impl DgnError {
    pub fn new(msg: &str, line: usize) -> Self {
        DgnError { msg: msg.to_string(), line, col: 0, source_line: None }
    }

    pub fn with_col(msg: &str, line: usize, col: usize) -> Self {
        DgnError { msg: msg.to_string(), line, col, source_line: None }
    }

    pub fn with_source(mut self, src: &str) -> Self {
        if self.line > 0 {
            if let Some(line_text) = src.lines().nth(self.line - 1) {
                self.source_line = Some(line_text.to_string());
            }
        }
        self
    }

    pub fn format_pretty(&self) -> String {
        let mut out = format!("Error at line {}", self.line);
        if self.col > 0 { out.push_str(&format!(", column {}", self.col)); }
        out.push_str(&format!(": {}", self.msg));
        if let Some(src_line) = &self.source_line {
            out.push('\n');
            out.push_str(&format!("  | {}", src_line));
            if self.col > 0 {
                let spaces = self.col.saturating_sub(1);
                out.push('\n');
                out.push_str(&format!("  | {}^", " ".repeat(spaces)));
            }
        }
        out
    }
}

#[derive(Clone)]
pub enum EvalResult {
    Val(DgnValue),
    Return(DgnValue),
    Break,
    Continue,
    None,
}
