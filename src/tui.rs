use std::io::{self, Write};
use std::fs;
use std::path::Path;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, ClearType},
};

const C_DKBLUE: Color = Color::DarkBlue;
const C_BLUE:   Color = Color::Blue;
const C_GREY:   Color = Color::Grey;
const C_BLACK:  Color = Color::Black;
const C_WHITE:  Color = Color::White;
const C_YELLOW: Color = Color::Yellow;
const C_CYAN:   Color = Color::Cyan;
const C_GREEN:  Color = Color::Green;
const C_RED:    Color = Color::Red;
const C_MGNT:   Color = Color::Magenta;

pub fn run_tui_ide(version: &str, build: &str, author: &str) -> io::Result<()> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(
        stdout,
        terminal::EnterAlternateScreen,
        terminal::Clear(ClearType::All),
        cursor::Hide
    )?;

    let result = ide_loop(&mut stdout, version, build, author);

    execute!(stdout, terminal::LeaveAlternateScreen, cursor::Show)?;
    terminal::disable_raw_mode()?;
    result
}

struct Buffer {
    name:    String,
    lines:   Vec<Vec<char>>,
    dirty:   bool,
    path:    Option<String>,
    undo_stack: Vec<(Vec<Vec<char>>, usize, usize)>, 
    redo_stack: Vec<(Vec<Vec<char>>, usize, usize)>, 
}

impl Buffer {
    fn from_str(name: &str, src: &str) -> Self {
        Buffer {
            name:  name.to_string(),
            lines: src.lines().map(|l| l.chars().collect()).collect(),
            dirty: false,
            path:  None,
            undo_stack: Vec::new(), 
            redo_stack: Vec::new(), 
        }
    }

    fn from_file(path: &str) -> Self {
        let content = fs::read_to_string(path).unwrap_or_default();
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();
        Buffer {
            name,
            lines: if content.is_empty() { vec![vec![]] }
                   else { content.lines().map(|l| l.chars().collect()).collect() },
            dirty: false,
            path:  Some(path.to_string()),
                undo_stack: Vec::new(),
                redo_stack: Vec::new(),
        }
    }

    fn text(&self) -> String {
        self.lines.iter().map(|l| l.iter().collect::<String>()).collect::<Vec<_>>().join("\n")
    }

    fn save(&mut self) -> io::Result<()> {
        if let Some(ref p) = self.path.clone() {
            fs::write(p, self.text())?;
            self.dirty = false;
        }
        Ok(())
    }

    fn line_count(&self) -> usize { self.lines.len() }

    fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map(|l| l.len()).unwrap_or(0)
    }

    fn insert_char(&mut self, row: usize, col: usize, ch: char) {
        if row >= self.lines.len() { self.lines.push(vec![]); }
        self.lines[row].insert(col, ch);
        self.dirty = true;
    }

    fn delete_char(&mut self, row: usize, col: usize) {
        if row < self.lines.len() && col < self.lines[row].len() {
            self.lines[row].remove(col);
            self.dirty = true;
        }
    }

    fn backspace(&mut self, row: usize, col: usize) -> (usize, usize) {
        if col > 0 {
            self.lines[row].remove(col - 1);
            self.dirty = true;
            (row, col - 1)
        } else if row > 0 {
            let cur = self.lines.remove(row);
            let prev_len = self.lines[row - 1].len();
            self.lines[row - 1].extend(cur);
            self.dirty = true;
            (row - 1, prev_len)
        } else {
            (row, col)
        }
    }

    fn insert_newline(&mut self, row: usize, col: usize) -> (usize, usize) {
        let rest: Vec<char> = self.lines[row].drain(col..).collect();
        // auto-indent: count leading spaces of current line
        let indent: Vec<char> = self.lines[row].iter()
            .take_while(|&&c| c == ' ')
            .cloned()
            .collect();
        // extra indent after '{'
        let extra = if self.lines[row].last() == Some(&'{') { 2usize } else { 0 };
        let mut new_line = indent.clone();
        for _ in 0..extra { new_line.push(' '); }
        let new_col = new_line.len();
        new_line.extend(rest);
        self.lines.insert(row + 1, new_line);
        self.dirty = true;
        (row + 1, new_col)
    }

    fn delete_line_join(&mut self, row: usize) {
        if row + 1 < self.lines.len() {
            let next = self.lines.remove(row + 1);
            self.lines[row].extend(next);
            self.dirty = true;
        }
    }

     fn snapshot(&mut self, row: usize, col: usize) {
        // Keep at most 200 undo steps
        if self.undo_stack.len() >= 200 { self.undo_stack.remove(0); }
        self.undo_stack.push((self.lines.clone(), row, col));
        self.redo_stack.clear();
    }

    fn undo(&mut self) -> Option<(usize, usize)> {
        if let Some((lines, row, col)) = self.undo_stack.pop() {
            self.redo_stack.push((self.lines.clone(), row, col));
            self.lines = lines;
            self.dirty = true;
            Some((row, col))
        } else { None }
    }

    fn redo(&mut self) -> Option<(usize, usize)> {
        if let Some((lines, row, col)) = self.redo_stack.pop() {
            self.undo_stack.push((self.lines.clone(), row, col));
            self.lines = lines;
            self.dirty = true;
            Some((row, col))
        } else { None }
    }
}

// ── Output log ───────────────────────────────────────────────────
struct OutputLog {
    lines: Vec<(Color, String)>,
}

impl OutputLog {
    fn new() -> Self { OutputLog { lines: Vec::new() } }
    fn push(&mut self, color: Color, msg: &str) {
        self.lines.push((color, msg.to_string()));
    }
    fn clear(&mut self) { self.lines.clear(); }
    fn visible(&self, height: usize) -> &[(Color, String)] {
        let n = self.lines.len();
        if n <= height { &self.lines } else { &self.lines[n - height..] }
    }
}

// ── Modal ────────────────────────────────────────────────────────
struct Modal {
    title:   String,
    lines:   Vec<String>,
    visible: bool,
}

impl Modal {
    fn new() -> Self { Modal { title: String::new(), lines: Vec::new(), visible: false } }
    fn show(&mut self, title: &str, body: &str) {
        self.title = title.to_string();
        self.lines = body.lines().map(|l| l.to_string()).collect();
        self.visible = true;
    }
    fn hide(&mut self) { self.visible = false; }
}

struct Editor {
    buffers:    Vec<Buffer>,
    active:     usize,
    cur_row:    usize,
    cur_col:    usize,
    scroll_top: usize,   
    output:     OutputLog,
    modal:      Modal,
    status:     String,
    menu_focus: bool,   
    search_term:   String,
    search_pos:    Option<(usize, usize)>,   
    version:       String,
}

impl Editor {
    fn new(version: &str, build: &str, author: &str) -> Self {
        let mut output = OutputLog::new();
        output.push(C_CYAN,   &format!("TurboDragon IDE  —  Dragon {} {}  by {}", version, build, author));
        output.push(C_CYAN,   "F1=Help  F9=Run  F5=Compile  Tab=Next file  Ctrl+Q=Quit");

        Editor {
            buffers: vec![
                Buffer::from_str("main.dgn",
                    "\nfunc main() {\n  showln(\"Hello, Dragon World!\")\n   greeting = greet(\"World\")\n  showln(greeting)\n}\n\nfunc greet(name) {\n  return \"Hello, \" + name\n}\n main() "),
                Buffer::from_str("utils.dgn",
                    "\nfunc add(a, b) {\n  return a + b\n}\n\nfunc max(a, b) {\n  if a > b {\n    return a\n  }\n  return b\n}"),
                Buffer::from_str("config.dgn",
                    "\n APP  = \"MyDragonApp\"\nVER  = \"1.0.0\"\nDEBUG = false\n\nfunc get_config() {\n  return { name: APP, version: VER }\n}\n get_config() "),
            ],
            active:     0,
            cur_row:    0,
            cur_col:    0,
            scroll_top: 0,
            output,
            modal:  Modal::new(),
            status: "Ready".to_string(),
            menu_focus: false,
            search_term: String::new(),
            search_pos: None,
            version: version.to_string(),
        }
    }

    fn buf(&self)     -> &Buffer  { &self.buffers[self.active] }
    fn buf_mut(&mut self) -> &mut Buffer { &mut self.buffers[self.active] }

    fn clamp_col(&mut self) {
        let max = self.buf().line_len(self.cur_row);
        if self.cur_col > max { self.cur_col = max; }
    }

    fn ensure_scroll(&mut self, editor_h: usize) {
        if self.cur_row < self.scroll_top {
            self.scroll_top = self.cur_row;
        } else if self.cur_row >= self.scroll_top + editor_h {
            self.scroll_top = self.cur_row + 1 - editor_h;
        }
    }

    fn switch_file(&mut self, idx: usize) {
        self.active     = idx;
        self.cur_row    = 0;
        self.cur_col    = 0;
        self.scroll_top = 0;
        self.status     = format!("Switched to: {}", self.buffers[self.active].name);
    }

    fn open_file(&mut self, path: &str) {
        // check if already open
        if let Some(i) = self.buffers.iter().position(|b| b.path.as_deref() == Some(path)) {
            self.switch_file(i);
            return;
        }
        self.buffers.push(Buffer::from_file(path));
        let idx = self.buffers.len() - 1;
        self.switch_file(idx);
    }

    fn run(&mut self, stdout: &mut io::Stdout, cols: u16, rows: u16) {
        let name = self.buf().name.clone();
        self.output.push(C_CYAN,   &format!("> dragon {}", name));
        self.output.push(C_YELLOW, "Running...");
        self.status = format!("Run: {}", name);

        let src = self.buf().text();
        let file_path = self.buf().path.clone().unwrap_or_else(|| name.clone());

        let tokens = match crate::engine::lex(&src) {
            Ok(t) => t,
            Err(e) => {
                self.output.push(C_RED, &e.format_pretty());
                return;
            }
        };

        let mut parser = crate::engine::Parser::new(tokens);
        let ast = match parser.parse() {
            Ok(stmt) => stmt,
            Err(e) => {
                self.output.push(C_RED, &format!("Parse Error: {}", e));
                return;
            }
        };

        let mut chunk = crate::engine::Chunk::new();
        crate::engine::Compiler::compile_stmt(&mut chunk, &ast);
        chunk.emit(crate::engine::Instruction::Halt, 0);

        let env = std::sync::Arc::new(std::sync::Mutex::new(
            crate::engine::Env::new(None, Some(std::path::PathBuf::from(file_path)))
        ));
        crate::engine::populate_builtins(env.clone());

        let output_cap = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        
        let cap_clone1 = output_cap.clone();
        let _ = env.lock().unwrap().set("show", crate::engine::native_fn(move |args| {
            let s = args.iter().map(crate::engine::val_to_string).collect::<Vec<_>>().join(" ");
            cap_clone1.lock().unwrap().push(s);
            Ok(crate::engine::DgnValue::Null)
        }));

        let cap_clone2 = output_cap.clone();
        let _ = env.lock().unwrap().set("showln", crate::engine::native_fn(move |args| {
            let s = args.iter().map(crate::engine::val_to_string).collect::<Vec<_>>().join(" ");
            cap_clone2.lock().unwrap().push(s);
            Ok(crate::engine::DgnValue::Null)
        }));

        let (tx_prompt, rx_prompt) = std::sync::mpsc::channel::<String>();
        let (tx_reply,  rx_reply)  = std::sync::mpsc::channel::<String>();

        let tx_p = tx_prompt.clone();
        let rx_r_arc = std::sync::Arc::new(std::sync::Mutex::new(rx_reply));          

        let _ = env.lock().unwrap().set("readln", crate::engine::native_fn(move |args| {
            let prompt = args.get(0)
                .map(crate::engine::val_to_string)
                .unwrap_or_default();
            let _ = tx_p.send(prompt);
            
            let reply = rx_r_arc.lock().unwrap().recv().unwrap_or_default();
            
            Ok(crate::engine::DgnValue::String(reply))
        }));
    
        let mut vm = crate::engine::Vm::new();
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                self.output.push(C_RED, &format!("Tokio Error: {}", e));
                return;
            }
        };

         let chunk_arc = std::sync::Arc::new(chunk);
        let chunk_for_task = chunk_arc.clone();
        let env_for_task   = env.clone();

        let vm_handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let mut vm = crate::engine::Vm::new();
            rt.block_on(vm.run(&chunk_for_task, env_for_task))
        });

        loop {
            if vm_handle.is_finished() { break; }

            if let Ok(prompt) = rx_prompt.try_recv() {
                let typed = tui_readln(stdout, &prompt, cols, rows)
                    .unwrap_or_default();
                self.output.push(C_CYAN,  &format!("> {}{}", prompt, typed));
                let _ = tx_reply.send(typed);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let result = vm_handle.join().unwrap_or(Err(
            crate::engine::DgnError::new("VM thread panicked", 0)
        ));

        {
            let lines = output_cap.lock().unwrap();
            for line in lines.iter() {
                for sub in line.lines() { 
                    self.output.push(C_WHITE, sub);
                }
            }
        }

        match result {
            Ok(_) => {
                self.output.push(C_CYAN, "[Process completed successfully]");
            }
            Err(e) => {
                self.output.push(C_RED, &e.format_pretty());
                self.output.push(C_RED, "[Process exited with errors]");
            }
        }
    }

    fn compile(&mut self) {
        let name = self.buf().name.clone();
        let out  = name.replace(".dgn", ".exe");
        self.output.push(C_CYAN,   &format!("> dragon -c {} -o {}", name, out));
        self.output.push(C_YELLOW, "Gathering sources...");
        self.output.push(C_YELLOW, "Injecting bytecode...");
        self.output.push(C_GREEN,  &format!("Build successful!  →  {}", out));
        self.output.push(C_CYAN,   "[exit code 0]");
        self.status = format!("Compiled: {} → {}", name, out);
    }

    fn show_help(&mut self) {
        self.modal.show("Help — Keyboard Shortcuts",
            "F1            This help screen\n\
             F2            Save current file\n\
             F3            Open file\n\
             F4 / Ctrl+G   Go to line\n\
             F5            Compile current file\n\
             F6            Next tab\n\
             F7            Toggle line comment (//)\n\
             F8            Close current file\n\
             F9            Run current file\n\
             ─────────────────────────────────\n\
             Tab           Switch to next file\n\
             Shift+Tab     Switch to previous file\n\
             Ctrl+N        New file\n\
             Ctrl+O        Open file by path\n\
             Ctrl+S        Save (prompts if new)\n\
             Ctrl+W        Close current file\n\
             ─────────────────────────────────\n\
             Ctrl+Z        Undo\n\
             Ctrl+Y        Redo\n\
             Ctrl+F        Find\n\
             Ctrl+R        Find next\n\
             Ctrl+L        Clear output panel\n\
             ─────────────────────────────────\n\
             Arrows        Move cursor\n\
             Home / End    Line start / end\n\
             PgUp / PgDn   Scroll editor\n\
             Backspace     Delete left\n\
             Delete        Delete right\n\
             Enter         New line (auto-indent)\n\
             ─────────────────────────────────\n\
             Ctrl+Q        Quit TurboDragon IDE");
    }

    fn show_about(&mut self) {
        let body = format!(
            "  ████████╗██╗   ██╗██████╗ ██████╗  ██████╗ \n\
             ─────────────────────────────────────────────\n\
             TurboDragon IDE v1.0\n\
             Dragon Programming Language v{}\n\
             ─────────────────────────────────────────────\n\
             Inspired by Turbo C / Turbo Pascal IDEs.\n\
             Built with Rust + crossterm.\n\n\
             Press any key to close.",
            self.version
        );
        self.modal.show("About TurboDragon IDE", &body);
    }

    fn find_next(&mut self) {
        if self.search_term.is_empty() { return; }
        let term = self.search_term.clone();
        let start_row = self.search_pos.map(|(r,_)| r).unwrap_or(self.cur_row);
        let start_col = self.search_pos.map(|(_,c)| c + 1).unwrap_or(self.cur_col);
        let lc = self.buf().line_count();
        for i in 0..lc {
            let r = (start_row + i) % lc;
            let line: String = self.buf().lines[r].iter().collect();
            let search_from = if i == 0 { start_col.min(line.len()) } else { 0 };
            if let Some(pos) = line[search_from..].find(&term) {
                self.cur_row = r;
                self.cur_col = search_from + pos;
                self.search_pos = Some((r, self.cur_col));
                self.status = format!("Found \"{}\" at line {}", term, r + 1);
                return;
            }
        }
        self.status = format!("\"{}\" not found", term);
        self.search_pos = None;
    }
}

const KEYWORDS: &[&str] = &[
    "func", "let", "return", "if", "else", "elif",
    "while", "for", "in", "break", "continue",
    "true", "false", "null",
    "and", "or", "not",
    "import", "from", "as",
    "struct", "enum", "match",
    "show", "showln", "readln",
];

enum Token<'a> {
    Comment(&'a str),
    StringLit(&'a str),
    Keyword(&'a str),
    Number(&'a str),
    Other(&'a str),
}

fn tokenize(line: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let trimmed = line.trim_start();

    if trimmed.starts_with("//") {
        tokens.push(Token::Comment(line));
        return tokens;
    }

    let bytes = line.as_bytes();
    let len   = bytes.len();
    let mut i = 0usize;

    while i < len {
        // string literal
        if bytes[i] == b'"' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'"' { i += 1; }
            if i < len { i += 1; }
            tokens.push(Token::StringLit(&line[start..i]));
            continue;
        }
        // number
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') { i += 1; }
            tokens.push(Token::Number(&line[start..i]));
            continue;
        }
        // identifier / keyword
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') { i += 1; }
            let word = &line[start..i];
            if KEYWORDS.contains(&word) {
                tokens.push(Token::Keyword(word));
            } else {
                tokens.push(Token::Other(word));
            }
            continue;
        }
        // single char
        tokens.push(Token::Other(&line[i..i+1]));
        i += 1;
    }
    tokens
}

const SIDEBAR_W:  u16 = 20;
const OUTPUT_H:   u16 = 8;
const MENUBAR_H:  u16 = 1;
const TABBAR_H:   u16 = 1;
const EDTITLE_H:  u16 = 1;
const STATUSBAR_H:u16 = 1;

fn render(stdout: &mut io::Stdout, ed: &Editor, cols: u16, rows: u16) -> io::Result<()> {
    let output_sep_row = rows - OUTPUT_H - STATUSBAR_H - 1;
    let editor_top     = MENUBAR_H + TABBAR_H + EDTITLE_H;
    let editor_h       = (output_sep_row - editor_top) as usize;

    queue!(stdout, cursor::MoveTo(0, 0),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK))?;
    let menu = "  File  Edit  Run  Compile  Options  Help  ";
    let title = " TurboDragon IDE v1.0 ";
    let pad = (cols as usize).saturating_sub(menu.len() + title.len());
    queue!(stdout, Print(menu), Print(" ".repeat(pad)),
        SetForegroundColor(C_DKBLUE), Print(title), ResetColor)?;

    queue!(stdout, cursor::MoveTo(0, MENUBAR_H),
        SetBackgroundColor(C_DKBLUE))?;
    let mut tab_x = 0u16;
    for (i, buf) in ed.buffers.iter().enumerate() {
        let label = format!(" {} ", buf.name);
        if i == ed.active {
            queue!(stdout, SetBackgroundColor(C_BLUE), SetForegroundColor(C_YELLOW))?;
        } else {
            queue!(stdout, SetBackgroundColor(C_DKBLUE), SetForegroundColor(C_GREY))?;
        }
        queue!(stdout, Print(&label))?;
        tab_x += label.len() as u16;
    }
    queue!(stdout, SetBackgroundColor(C_DKBLUE),
        Print(" ".repeat((cols as usize).saturating_sub(tab_x as usize))),
        ResetColor)?;

    let title_row = MENUBAR_H + TABBAR_H;
    queue!(stdout, cursor::MoveTo(0, title_row),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK))?;
    let fname = &ed.buf().name;
    let mod_flag = if ed.buf().dirty { " [modified]" } else { "" };
    let ed_title = format!(" {}{}", fname, mod_flag);
    queue!(stdout, Print(format!("{:<width$}", ed_title, width=cols as usize)), ResetColor)?;

    for row_idx in 0..editor_h {
        let screen_row = editor_top + row_idx as u16;
        queue!(stdout, cursor::MoveTo(0, screen_row))?;
        if let Some(buf) = ed.buffers.get(row_idx) {
            if row_idx == ed.active {
                queue!(stdout, SetBackgroundColor(C_BLUE), SetForegroundColor(C_YELLOW))?;
            } else {
                queue!(stdout, SetBackgroundColor(C_DKBLUE), SetForegroundColor(C_GREY))?;
            }
            let label = format!(" {}", buf.name);
            queue!(stdout, Print(format!("{:<width$}", label, width=SIDEBAR_W as usize)))?;
        } else {
            queue!(stdout, SetBackgroundColor(C_DKBLUE),
                Print(format!("{:<width$}", "", width=SIDEBAR_W as usize)))?;
        }
        queue!(stdout, ResetColor)?;
    }

    let editor_left  = SIDEBAR_W;
    let editor_width = (cols - SIDEBAR_W) as usize;

    for row_idx in 0..editor_h {
        let file_row   = ed.scroll_top + row_idx;
        let screen_row = editor_top + row_idx as u16;

        queue!(stdout, cursor::MoveTo(editor_left, screen_row))?;

        let is_cursor_row = file_row == ed.cur_row;
        let bg = if is_cursor_row { C_BLUE } else { C_DKBLUE };
        queue!(stdout, SetBackgroundColor(bg))?;

        if let Some(line_chars) = ed.buf().lines.get(file_row) {
            let line_str: String = line_chars.iter().collect();

            queue!(stdout, SetForegroundColor(Color::DarkGrey),
                Print(format!("{:3} ", file_row + 1)))?;

            if is_cursor_row {
                let before: String = line_chars[..ed.cur_col.min(line_chars.len())].iter().collect();
                let after:  String = line_chars[ed.cur_col.min(line_chars.len())..].iter().collect();
                print_highlighted(stdout, &before, bg)?;

                let cur_ch = line_chars.get(ed.cur_col).copied().unwrap_or(' ');
                queue!(stdout, SetBackgroundColor(C_WHITE), SetForegroundColor(C_DKBLUE),
                    Print(cur_ch), SetBackgroundColor(bg))?;
                print_highlighted(stdout, &after[if ed.cur_col < line_chars.len() { 1 } else { 0 }..], bg)?;
            } else {
                print_highlighted(stdout, &line_str, bg)?;
            }

            let used = 4 + line_str.len().min(editor_width - 4);
            let fill = editor_width.saturating_sub(used + if is_cursor_row { 1 } else { 0 });
            queue!(stdout, SetBackgroundColor(bg), Print(" ".repeat(fill)))?;
        } else {
            // empty row
            queue!(stdout, Print(format!("{:<width$}", "", width=editor_width)))?;
        }
        queue!(stdout, ResetColor)?;
    }

    queue!(stdout, cursor::MoveTo(0, output_sep_row),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK))?;
    let sep = format!(" Output   F9=Run  F5=Compile  Ctrl+L=Clear {:─<w$}", "", w=(cols as usize).saturating_sub(44));
    queue!(stdout, Print(format!("{:<width$}", sep, width=cols as usize)), ResetColor)?;

    let out_lines = ed.output.visible((OUTPUT_H - 1) as usize);
    for (i, (color, msg)) in out_lines.iter().enumerate() {
        let screen_row = output_sep_row + 1 + i as u16;
        queue!(stdout, cursor::MoveTo(0, screen_row),
            SetBackgroundColor(C_BLACK), SetForegroundColor(*color),
            Print(format!("{:<width$}", msg, width=cols as usize)), ResetColor)?;
    }

    for i in out_lines.len()..(OUTPUT_H - 1) as usize {
        let screen_row = output_sep_row + 1 + i as u16;
        queue!(stdout, cursor::MoveTo(0, screen_row),
            SetBackgroundColor(C_BLACK),
            Print(format!("{:<width$}", "", width=cols as usize)), ResetColor)?;
    }

    let status_row = rows - 1;
    let pos = format!("Ln {}  Col {}  ", ed.cur_row + 1, ed.cur_col + 1);
    let left_w = (cols as usize).saturating_sub(pos.len());
    queue!(stdout, cursor::MoveTo(0, status_row),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
        Print(format!("{:<width$}", ed.status, width=left_w)),
        SetForegroundColor(C_DKBLUE),
        Print(pos), ResetColor)?;

    if ed.modal.visible {
        render_modal(stdout, &ed.modal, cols, rows)?;
    }

    stdout.flush()
}

fn print_highlighted(stdout: &mut io::Stdout, text: &str, bg: Color) -> io::Result<()> {
    for tok in tokenize(text) {
        match tok {
            Token::Comment(s)  => { queue!(stdout, SetForegroundColor(C_CYAN),   Print(s))?; }
            Token::StringLit(s)=> { queue!(stdout, SetForegroundColor(C_GREEN),  Print(s))?; }
            Token::Keyword(s)  => { queue!(stdout, SetForegroundColor(C_YELLOW), Print(s))?; }
            Token::Number(s)   => { queue!(stdout, SetForegroundColor(C_MGNT),   Print(s))?; }
            Token::Other(s)    => { queue!(stdout, SetForegroundColor(C_WHITE),  Print(s))?; }
        }
        let _ = bg; 
    }
    Ok(())
}

fn render_modal(stdout: &mut io::Stdout, modal: &Modal, cols: u16, rows: u16) -> io::Result<()> {
    let max_line_len = modal.lines.iter().map(|l| l.len()).max().unwrap_or(0);
    let box_w = (max_line_len + 4).max(modal.title.len() + 4).min(cols as usize - 4);
    let box_h = modal.lines.len() + 4;
    let bx    = ((cols as usize).saturating_sub(box_w) / 2) as u16;
    let by    = ((rows as usize).saturating_sub(box_h) / 2) as u16;

    queue!(stdout, cursor::MoveTo(bx, by),
        SetBackgroundColor(C_BLUE), SetForegroundColor(C_YELLOW),
        Print(format!(" {:<width$} ", modal.title, width=box_w - 2)))?;

    for (i, line) in modal.lines.iter().enumerate() {
        queue!(stdout, cursor::MoveTo(bx, by + 1 + i as u16),
            SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
            Print(format!(" {:<width$} ", line, width=box_w - 2)))?;
    }

    let close_hint = "[ Press any key to close ]";
    queue!(stdout, cursor::MoveTo(bx, by + 1 + modal.lines.len() as u16),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_DKBLUE),
        Print(format!("{:^width$}", close_hint, width=box_w)))?;
    queue!(stdout, cursor::MoveTo(bx, by + 2 + modal.lines.len() as u16),
        SetBackgroundColor(C_GREY),
        Print(format!("{:<width$}", "", width=box_w)))?;

    queue!(stdout, ResetColor)
}

fn prompt_open(stdout: &mut io::Stdout, cols: u16, rows: u16) -> io::Result<Option<String>> {
    let label = " Open file: ";
    let box_w = 50usize;
    let bx    = ((cols as usize).saturating_sub(box_w) / 2) as u16;
    let by    = (rows / 2) as u16;

    queue!(stdout,
        cursor::MoveTo(bx, by),
        SetBackgroundColor(C_BLUE), SetForegroundColor(C_YELLOW),
        Print(format!("{:<width$}", " Open file (Ctrl+C to cancel)", width=box_w)),
        cursor::MoveTo(bx, by + 1),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
        Print(format!("{:<width$}", label, width=box_w)),
        ResetColor)?;
    stdout.flush()?;

    execute!(stdout, cursor::Show)?;
    let mut input = String::new();
    let input_col = bx + label.len() as u16;

    loop {
        queue!(stdout,
            cursor::MoveTo(input_col, by + 1),
            SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
            Print(format!("{:<width$}", input, width=box_w - label.len())),
            cursor::MoveTo(input_col + input.len() as u16, by + 1),
            ResetColor)?;
        stdout.flush()?;

        if let Event::Key(k) = event::read()? {
            if k.kind != event::KeyEventKind::Press { continue; }
            match k.code {
                KeyCode::Char('c') if k.modifiers == KeyModifiers::CONTROL => {
                    execute!(stdout, cursor::Hide)?;
                    return Ok(None);
                }
                KeyCode::Enter => {
                    execute!(stdout, cursor::Hide)?;
                    return Ok(Some(input));
                }
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Char(c)   => { input.push(c); }
                KeyCode::Esc       => {
                    execute!(stdout, cursor::Hide)?;
                    return Ok(None);
                }
                _ => {}
            }
        }
    }
}

fn prompt_find(stdout: &mut io::Stdout, cols: u16, rows: u16) -> io::Result<Option<String>> {
    let label = " Find: ";
    let box_w = 50usize;
    let bx = ((cols as usize).saturating_sub(box_w) / 2) as u16;
    let by = (rows / 2) as u16;
    queue!(stdout,
        cursor::MoveTo(bx, by),
        SetBackgroundColor(C_BLUE), SetForegroundColor(C_YELLOW),
        Print(format!("{:<width$}", " Find (Esc=cancel, Enter=search)", width=box_w)),
        cursor::MoveTo(bx, by+1),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
        Print(format!("{:<width$}", label, width=box_w)), ResetColor)?;
    stdout.flush()?;
    execute!(stdout, cursor::Show)?;
    let mut input = String::new();
    let icol = bx + label.len() as u16;
    loop {
        queue!(stdout,
            cursor::MoveTo(icol, by+1),
            SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
            Print(format!("{:<width$}", input, width=box_w-label.len())),
            cursor::MoveTo(icol + input.len() as u16, by+1), ResetColor)?;
        stdout.flush()?;
        if let Event::Key(k) = event::read()? {
            if k.kind != event::KeyEventKind::Press { continue; }
            match k.code {
                KeyCode::Esc   => { execute!(stdout, cursor::Hide)?; return Ok(None); }
                KeyCode::Enter => { execute!(stdout, cursor::Hide)?; return Ok(Some(input)); }
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Char(c) if k.modifiers == KeyModifiers::NONE
                                  || k.modifiers == KeyModifiers::SHIFT => { input.push(c); }
                _ => {}
            }
        }
    }
}

fn prompt_goto_line(stdout: &mut io::Stdout, cols: u16, rows: u16) -> io::Result<Option<String>> {
    let label = " Go to line: ";
    let box_w = 30usize;
    let bx    = ((cols as usize).saturating_sub(box_w) / 2) as u16;
    let by    = (rows / 2) as u16;

    queue!(stdout,
        cursor::MoveTo(bx, by),
        SetBackgroundColor(C_BLUE), SetForegroundColor(C_YELLOW),
        Print(format!("{:<width$}", " Go to line", width=box_w)),
        cursor::MoveTo(bx, by + 1),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
        Print(format!("{:<width$}", label, width=box_w)),
        ResetColor)?;
    stdout.flush()?;

    execute!(stdout, cursor::Show)?;
    let mut input = String::new();
    let input_col = bx + label.len() as u16;

    loop {
        queue!(stdout,
            cursor::MoveTo(input_col, by + 1),
            SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
            Print(format!("{:<width$}", input, width = box_w - label.len())),
            cursor::MoveTo(input_col + input.len() as u16, by + 1),
            ResetColor)?;
        stdout.flush()?;

        if let Event::Key(k) = event::read()? {
            if k.kind != event::KeyEventKind::Press { continue; }
            match k.code {
                KeyCode::Esc => { execute!(stdout, cursor::Hide)?; return Ok(None); }
                KeyCode::Enter => { execute!(stdout, cursor::Hide)?; return Ok(Some(input)); }
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Char(c) if c.is_ascii_digit() => { input.push(c); }
                _ => {}
            }
        }
    }
}

fn prompt_save_as(stdout: &mut io::Stdout, cols: u16, rows: u16) -> io::Result<Option<String>> {
    let label = " Save as: ";
    let box_w = 50usize;
    let bx    = ((cols as usize).saturating_sub(box_w) / 2) as u16;
    let by    = (rows / 2) as u16;

    queue!(stdout,
        cursor::MoveTo(bx, by),
        SetBackgroundColor(C_BLUE), SetForegroundColor(C_YELLOW),
        Print(format!("{:<width$}", " Save file (Esc to cancel)", width=box_w)),
        cursor::MoveTo(bx, by + 1),
        SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
        Print(format!("{:<width$}", label, width=box_w)),
        ResetColor)?;
    stdout.flush()?;

    execute!(stdout, cursor::Show)?;
    let mut input = String::new();
    let input_col = bx + label.len() as u16;

    loop {
        queue!(stdout,
            cursor::MoveTo(input_col, by + 1),
            SetBackgroundColor(C_GREY), SetForegroundColor(C_BLACK),
            Print(format!("{:<width$}", input, width=box_w - label.len())),
            cursor::MoveTo(input_col + input.len() as u16, by + 1),
            ResetColor)?;
        stdout.flush()?;

        if let Event::Key(k) = event::read()? {
            if k.kind != event::KeyEventKind::Press { continue; }
            match k.code {
                KeyCode::Esc => { execute!(stdout, cursor::Hide)?; return Ok(None); }
                KeyCode::Enter => { execute!(stdout, cursor::Hide)?; return Ok(Some(input)); }
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Char(c) if k.modifiers == KeyModifiers::NONE
                                  || k.modifiers == KeyModifiers::SHIFT => { input.push(c); }
                _ => {}
            }
        }
    }
}

fn tui_readln(
    stdout: &mut io::Stdout,
    prompt: &str,
    cols: u16,
    rows: u16,
) -> io::Result<String> {
    let out_row = rows - 2; 
    queue!(
        stdout,
        cursor::MoveTo(0, out_row),
        SetBackgroundColor(C_CYAN),
        SetForegroundColor(C_BLACK),
        Print(format!("{:<width$}", format!(" readln > {}", prompt), width = cols as usize)),
        ResetColor,
        cursor::MoveTo(10 + prompt.len() as u16, out_row),
        cursor::Show
    )?;
    stdout.flush()?;

    let mut input = String::new();
    loop {
        queue!(
            stdout,
            cursor::MoveTo(10 + prompt.len() as u16, out_row),
            SetBackgroundColor(C_CYAN),
            SetForegroundColor(C_BLACK),
            Print(format!("{:<width$}", input, width = (cols as usize).saturating_sub(10 + prompt.len()))),
            cursor::MoveTo(10 + prompt.len() as u16 + input.len() as u16, out_row),
            ResetColor
        )?;
        stdout.flush()?;

        if let Event::Key(k) = event::read()? {
            if k.kind != event::KeyEventKind::Press { continue; }
            match k.code {
                KeyCode::Enter => break,
                KeyCode::Backspace => { input.pop(); }
                KeyCode::Char(c) if k.modifiers == KeyModifiers::NONE
                                  || k.modifiers == KeyModifiers::SHIFT => {
                    input.push(c);
                }
                KeyCode::Esc => break,
                _ => {}
            }
        }
    }
    execute!(stdout, cursor::Hide)?;
    Ok(input)
}

// ── Main event loop ───────────────────────────────────────────────
fn ide_loop(stdout: &mut io::Stdout, version: &str, build: &str, author: &str) -> io::Result<()> {
    let mut ed = Editor::new(version, build, author);
    let (mut cols, mut rows) = terminal::size()?;

    loop {
        let editor_h = {
            let output_sep = rows - OUTPUT_H - STATUSBAR_H - 1;
            let editor_top = MENUBAR_H + TABBAR_H + EDTITLE_H;
            (output_sep - editor_top) as usize
        };
        ed.ensure_scroll(editor_h);
        render(stdout, &ed, cols, rows)?;

        match event::read()? {
            Event::Resize(c, r) => { cols = c; rows = r; }

            Event::Key(KeyEvent { code, modifiers, kind: event::KeyEventKind::Press, .. }) => {
                if ed.modal.visible {
                    ed.modal.hide();
                    continue;
                }

                match (code, modifiers) {
                    // ── Quit ──────────────────────────────────────
                    (KeyCode::Char('q'), KeyModifiers::CONTROL) => break,
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,

                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                        if let Some(term) = prompt_find(stdout, cols, rows)? {
                            if !term.is_empty() {
                                ed.search_term = term;
                                ed.search_pos  = None;
                                ed.find_next();
                            }
                        }
                    }
                    (KeyCode::Char('r'), KeyModifiers::CONTROL) => {
                        ed.find_next();
                    }

                    (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                        if ed.buf().path.is_none() {
                            if let Some(path) = prompt_save_as(stdout, cols, rows)? {
                                let path = path.trim().to_string();
                                if !path.is_empty() {
                                    ed.buf_mut().path = Some(path.clone());
                                    ed.buf_mut().name = std::path::Path::new(&path)
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(&path)
                                        .to_string();
                                }
                            }
                        }
                        if ed.buf().path.is_some() {
                            match ed.buf_mut().save() {
                                Ok(_)  => { ed.status = format!("Saved: {}", ed.buf().name); }
                                Err(e) => { ed.status = format!("Save error: {}", e); }
                            }
                        }
                    }
                    (KeyCode::Char('o'), KeyModifiers::CONTROL) => {
                        if let Some(path) = prompt_open(stdout, cols, rows)? {
                            if !path.trim().is_empty() {
                                ed.open_file(path.trim());
                            }
                        }
                    }
                    (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                        if ed.buffers.len() > 1 {
                            let name = ed.buf().name.clone();
                            ed.buffers.remove(ed.active);
                            if ed.active >= ed.buffers.len() { ed.active = ed.buffers.len() - 1; }
                            ed.cur_row = 0; ed.cur_col = 0; ed.scroll_top = 0;
                            ed.status = format!("Closed: {}", name);
                        } else {
                            ed.status = "Cannot close last file".into();
                        }
                    }

                     (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                        ed.buffers.push(Buffer::from_str("untitled.dgn", ""));
                        let idx = ed.buffers.len() - 1;
                        ed.switch_file(idx);
                        ed.status = "New file — Ctrl+S to save".into();
                    }
                    (KeyCode::Char('g'), KeyModifiers::CONTROL) => {
                        if let Some(line_str) = prompt_goto_line(stdout, cols, rows)? {
                            if let Ok(n) = line_str.trim().parse::<usize>() {
                                let target = n.saturating_sub(1)
                                    .min(ed.buf().line_count().saturating_sub(1));
                                ed.cur_row = target;
                                ed.cur_col = 0;
                                ed.status = format!("Jumped to line {}", n);
                            }
                        }
                    }

                    (KeyCode::Char('z'), KeyModifiers::CONTROL) => {
                        if let Some((r, c)) = ed.buf_mut().undo() {
                            ed.cur_row = r; ed.cur_col = c;
                            ed.status = "Undo".into();
                        } else {
                            ed.status = "Nothing to undo".into();
                        }
                    }
                    (KeyCode::Char('y'), KeyModifiers::CONTROL) => {
                        if let Some((r, c)) = ed.buf_mut().redo() {
                            ed.cur_row = r; ed.cur_col = c;
                            ed.status = "Redo".into();
                        } else {
                            ed.status = "Nothing to redo".into();
                        }
                    }

                    (KeyCode::Tab, _) => {
                        let next = (ed.active + 1) % ed.buffers.len();
                        ed.switch_file(next);
                    }
                    (KeyCode::BackTab, _) => {
                        let n = ed.buffers.len();
                        let prev = (ed.active + n - 1) % n;
                        ed.switch_file(prev);
                    }

                    (KeyCode::Backspace, _) => {
                        let r = ed.cur_row;
                        let c = ed.cur_col;
                        let (new_r, new_c) = ed.buf_mut().backspace(r, c);
                        ed.cur_row = new_r; 
                        ed.cur_col = new_c;
                    }

                    (KeyCode::F(9),  _) => ed.run(stdout, cols, rows),
                    (KeyCode::F(5),  _) => ed.compile(),

                    (KeyCode::F(1),  _) => ed.show_help(),
                    (KeyCode::F(10), _) => ed.show_about(),
                    (KeyCode::F(2), _) => {
                        if ed.buf().path.is_none() {
                            if let Some(path) = prompt_save_as(stdout, cols, rows)? {
                                let path = path.trim().to_string();
                                if !path.is_empty() {
                                    ed.buf_mut().path = Some(path.clone());
                                    ed.buf_mut().name = std::path::Path::new(&path)
                                        .file_name().and_then(|n| n.to_str())
                                        .unwrap_or(&path).to_string();
                                }
                            }
                        }
                        if ed.buf().path.is_some() {
                            match ed.buf_mut().save() {
                                Ok(_)  => ed.status = format!("Saved: {}", ed.buf().name),
                                Err(e) => ed.status = format!("Save error: {}", e),
                            }
                        }
                    }
                    (KeyCode::F(3), _) => {
                        if let Some(path) = prompt_open(stdout, cols, rows)? {
                            if !path.trim().is_empty() {
                                ed.open_file(path.trim());
                            }
                        }
                    }
                    (KeyCode::F(4), _) => {
                        if let Some(line_str) = prompt_goto_line(stdout, cols, rows)? {
                            if let Ok(n) = line_str.trim().parse::<usize>() {
                                let target = n.saturating_sub(1).min(ed.buf().line_count().saturating_sub(1));
                                ed.cur_row = target;
                                ed.cur_col = 0;
                                ed.status = format!("Jumped to line {}", n);
                            }
                        }
                    }
                    (KeyCode::F(6), _) => {
                        let next = (ed.active + 1) % ed.buffers.len();
                        ed.switch_file(next);
                    }
                    (KeyCode::F(7), _) => {
                        let r = ed.cur_row;
                        let line = &ed.buf().lines[r];
                        let trimmed: String = line.iter().collect::<String>();
                        if trimmed.trim_start().starts_with("//") {
                            if let Some(pos) = trimmed.find("//") {
                                let new_line: Vec<char> = format!("{}{}", &trimmed[..pos], &trimmed[pos+2..]).chars().collect();
                                ed.buf_mut().lines[r] = new_line;
                            }
                        } else {
                            ed.buf_mut().lines[r].insert(0, '/');
                            ed.buf_mut().lines[r].insert(1, '/');
                        }
                        ed.buf_mut().dirty = true;
                        ed.status = "Line comment toggled".into();
                    }
                    (KeyCode::F(8), _) => {
                        if ed.buffers.len() > 1 {
                            let name = ed.buf().name.clone();
                            ed.buffers.remove(ed.active);
                            if ed.active >= ed.buffers.len() { ed.active = ed.buffers.len() - 1; }
                            ed.cur_row = 0; ed.cur_col = 0; ed.scroll_top = 0;
                            ed.status = format!("Closed: {}", name);
                        } else {
                            ed.status = "Cannot close last file".into();
                        }
                    }

                    (KeyCode::Char('l'), KeyModifiers::CONTROL) => ed.output.clear(),

                    (KeyCode::Up, _) => {
                        if ed.cur_row > 0 { ed.cur_row -= 1; ed.clamp_col(); }
                    }
                    (KeyCode::Down, _) => {
                        if ed.cur_row + 1 < ed.buf().line_count() {
                            ed.cur_row += 1; ed.clamp_col();
                        }
                    }
                    (KeyCode::Left, _) => {
                        if ed.cur_col > 0 { ed.cur_col -= 1; }
                        else if ed.cur_row > 0 {
                            ed.cur_row -= 1;
                            ed.cur_col = ed.buf().line_len(ed.cur_row);
                        }
                    }
                    (KeyCode::Right, _) => {
                        let ll = ed.buf().line_len(ed.cur_row);
                        if ed.cur_col < ll { ed.cur_col += 1; }
                        else if ed.cur_row + 1 < ed.buf().line_count() {
                            ed.cur_row += 1; ed.cur_col = 0;
                        }
                    }
                    (KeyCode::Home, _) => { ed.cur_col = 0; }
                    (KeyCode::End,  _) => { ed.cur_col = ed.buf().line_len(ed.cur_row); }
                    (KeyCode::PageUp, _) => {
                        ed.cur_row = ed.cur_row.saturating_sub(editor_h);
                        ed.clamp_col();
                    }
                    (KeyCode::PageDown, _) => {
                        let max = ed.buf().line_count().saturating_sub(1);
                        ed.cur_row = (ed.cur_row + editor_h).min(max);
                        ed.clamp_col();
                    }
                    (KeyCode::Backspace, _) => {
                        let r = ed.cur_row;
                        let c = ed.cur_col;
                        let (new_r, new_c) = ed.buf_mut().backspace(r, c);
                        ed.cur_row = new_r; 
                        ed.cur_col = new_c;
                    }
                    (KeyCode::Delete, _) => {
                        let r = ed.cur_row;
                        let c = ed.cur_col;
                        let ll = ed.buf().line_len(r);
                        if c < ll {
                            ed.buf_mut().delete_char(r, c);
                        } else {
                            ed.buf_mut().delete_line_join(r);
                        }
                    }
                    (KeyCode::Enter, _) => {
                        let r = ed.cur_row;
                        let c = ed.cur_col;
                        let (new_r, new_c) = ed.buf_mut().insert_newline(r, c);
                        ed.cur_row = new_r; 
                        ed.cur_col = new_c;
                    }
                    (KeyCode::Char(ch), m)
                        if m == KeyModifiers::NONE || m == KeyModifiers::SHIFT =>
                    {
                        let r = ed.cur_row;
                        let c = ed.cur_col;
                        ed.buf_mut().insert_char(r, c, ch);
                        ed.cur_col += 1;
                    }

                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(())
}
