use super::*;

pub struct Parser {
    pub tokens: Vec<Token>,
    pub pos: usize,
    destruct_counter: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0, destruct_counter: 0 }
    }

    fn peek(&self, offset: usize) -> &Token {
        if self.pos + offset < self.tokens.len() {
            &self.tokens[self.pos + offset]
        } else {
            &self.tokens[self.tokens.len() - 1]
        }
    }

    fn is_lambda_start(&self) -> bool {
        if !self.cur_is("(") { return false; }
        let mut i = 1;
        loop {
            let t = self.peek(i);
            if t.typ == TokenType::Eof || t.typ == TokenType::Newline { return false; }
            if t.typ == TokenType::String { i += 1; continue; }
            if t.value == "{" { return false; }
            if t.value == ")" {
                return self.peek_is(i + 1, "=>");
            }
            if t.value == "..." { i += 1; continue; }
            i += 1;
        }
    }

    fn is_destructuring_assignment(&self) -> bool {
        if !self.cur_is("[") && !self.cur_is("{") { return false; }

        let mut depth = 0;
        let mut i = 0;
        loop {
            let t = self.peek(i);
            if t.typ == TokenType::Eof { return false; }

            if t.typ == TokenType::String { i += 1; continue; }
            if t.value == "[" || t.value == "{" || t.value == "(" { depth += 1; }
            else if t.value == "]" || t.value == "}" || t.value == ")" {
                depth -= 1;
                if depth == 0 {
                    return self.peek_is(i + 1, "=");
                }
            }

            i += 1;
            if i > 200 { return false; }
        }
    }

    fn cur_is(&self, s: &str) -> bool { self.peek_is(0, s) }
    fn peek_is(&self, n: usize, s: &str) -> bool {
        let t = self.peek(n);
        t.typ != TokenType::String && t.value == s
    }

    fn current(&self) -> &Token {
        self.peek(0)
    }

    fn advance(&mut self) -> Token {
        let t = self.current().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn match_token(&mut self, s: &str) -> bool {

        if self.current().typ != TokenType::String && self.current().value == s {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, s: &str) -> Result<Token, String> {
        if (self.current().typ != TokenType::String && self.current().value == s) || (s == "NAME" && matches!(self.current().typ, TokenType::Name)) {
            Ok(self.advance())
        } else {
            let friendly_expected = if s == "NAME" { "identifier" } else { s };
           Err(DgnError::with_col(
                &format!("Expected '{}', got '{}'", friendly_expected, self.current().value),
                self.current().line,
                self.current().col
            ).format_pretty())
        }
    }

    pub fn parse(&mut self) -> Result<Stmt, String> {
        let mut stmts = Vec::new();
        let start_line = self.current().line;
        while self.current().typ != TokenType::Eof {
            stmts.push(self.parse_stmt()?);
        }
        Ok(Stmt { node: StmtNode::Block(stmts), line: start_line })
    }

    fn parse_block(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        if self.match_token("{") {
            let mut stmts = Vec::new();
            while !self.cur_is("}") && self.current().typ != TokenType::Eof {
                stmts.push(self.parse_stmt()?);
            }
            self.expect("}")?;
            Ok(Stmt { node: StmtNode::Block(stmts), line })
        } else {
             let stmt = self.parse_stmt()?;
             Ok(Stmt { node: StmtNode::Block(vec![stmt]), line })
        }
    }

    pub fn parse_stmt(&mut self) -> Result<Stmt, String> {
        while self.current().typ == TokenType::Newline {
            self.advance();
        }
        if self.current().typ == TokenType::Eof {
            return Ok(Stmt { node: StmtNode::Block(vec![]), line: 0 });
        }

        let line = self.current().line;
        let cur = if self.current().typ == TokenType::String { String::new() } else { self.current().value.clone() };

        let is_var = cur == "var";
        let is_destruct = self.is_destructuring_assignment();

        if is_var || is_destruct {
            if is_var { self.advance(); }
            if self.cur_is("[") {
                self.advance();
                let mut names: Vec<Option<String>> = Vec::new();
                let mut rest_name: Option<String> = None;
                while !self.cur_is("]") && self.current().typ != TokenType::Eof {
                    if self.cur_is(",") { self.advance(); continue; }
                    if self.cur_is("...") {
                        self.advance();
                        rest_name = Some(self.expect("NAME")?.value);
                        break;
                    }
                    let n = self.expect("NAME")?.value;
                    names.push(if n == "_" { None } else { Some(n) });
                    if !self.match_token(",") { break; }
                }
                self.expect("]")?;
                self.expect("=")?;
                let rhs = self.parse_expr()?;
                self.match_end_of_stmt()?;
                let tmp = { self.destruct_counter += 1; format!("__destruct_{}__", self.destruct_counter) };
                let mut stmts = vec![
                    Stmt { node: StmtNode::Expr(Expr { node: ExprNode::Assign(
                        Box::new(Expr { node: ExprNode::Var(tmp.clone()), line }),
                        Box::new(rhs),
                    ), line }), line }
                ];
                for (i, maybe_name) in names.iter().enumerate() {
                    if let Some(n) = maybe_name {
                        stmts.push(Stmt { node: StmtNode::Expr(Expr { node: ExprNode::Assign(
                            Box::new(Expr { node: ExprNode::Var(n.clone()), line }),
                            Box::new(Expr { node: ExprNode::Index(
                                Box::new(Expr { node: ExprNode::Var(tmp.clone()), line }),
                                Box::new(Expr { node: ExprNode::Number(i as f64), line }),
                            ), line }),
                        ), line }), line });
                    }
                }
                if let Some(rn) = rest_name {
                    let n = names.len();
                    stmts.push(Stmt { node: StmtNode::Expr(Expr { node: ExprNode::Assign(
                        Box::new(Expr { node: ExprNode::Var(rn), line }),
                        Box::new(Expr { node: ExprNode::Call(
                            Box::new(Expr { node: ExprNode::GetAttr(
                                Box::new(Expr { node: ExprNode::Var(tmp.clone()), line }),
                                "slice".to_string(),
                            ), line }),
                            vec![Expr { node: ExprNode::Number(n as f64), line }],
                        ), line }),
                    ), line }), line });
                }
                return Ok(Stmt { node: StmtNode::Multi(stmts), line });
            }
            if self.cur_is("{") {
                self.advance();
                let mut bindings: Vec<(String, String)> = Vec::new();
                while !self.cur_is("}") && self.current().typ != TokenType::Eof {
                    if self.cur_is(",") { self.advance(); continue; }
                    let key = self.expect("NAME")?.value;
                    let local = if self.match_token(":") {
                        self.expect("NAME")?.value
                    } else {
                        key.clone()
                    };
                    bindings.push((key, local));
                    if !self.match_token(",") { break; }
                }
                self.expect("}")?;
                self.expect("=")?;
                let rhs = self.parse_expr()?;
                self.match_end_of_stmt()?;
                let tmp = { self.destruct_counter += 1; format!("__destruct_{}__", self.destruct_counter) };
                let mut stmts = vec![
                    Stmt { node: StmtNode::Expr(Expr { node: ExprNode::Assign(
                        Box::new(Expr { node: ExprNode::Var(tmp.clone()), line }),
                        Box::new(rhs),
                    ), line }), line }
                ];
                for (key, local) in bindings {
                    stmts.push(Stmt { node: StmtNode::Expr(Expr { node: ExprNode::Assign(
                        Box::new(Expr { node: ExprNode::Var(local), line }),
                        Box::new(Expr { node: ExprNode::Index(
                            Box::new(Expr { node: ExprNode::Var(tmp.clone()), line }),
                            Box::new(Expr { node: ExprNode::String(key), line }),
                        ), line }),
                    ), line }), line });
                }
                return Ok(Stmt { node: StmtNode::Multi(stmts), line });
            }
            if is_var {
                let name = self.expect("NAME")?.value;
                self.expect("=")?;
                let val = self.parse_expr()?;
                self.match_end_of_stmt()?;
                return Ok(Stmt { node: StmtNode::Expr(Expr { node: ExprNode::Assign(
                    Box::new(Expr { node: ExprNode::Var(name), line }),
                    Box::new(val),
                ), line }), line });
            }
        }

        if cur == "augment" {
            self.advance();
            let target_type = self.expect("NAME")?.value;
            self.expect("{")?;
            let mut methods = Vec::new();
            while !self.cur_is("}") && self.current().typ != TokenType::Eof {
                methods.push(self.parse_func(false)?);
            }
            self.expect("}")?;
            return Ok(Stmt { node: StmtNode::Augment(target_type, methods), line });
        }

        if cur == "const" { return self.parse_const(); }
        if cur == "async" {
            self.advance();
            return self.parse_func(true);
        }
        if cur == "func" { return self.parse_func(false); }
        if cur == "class" { return self.parse_class(); }
        if cur == "return" {
            self.advance();
            let mut val = None;
            if !self.cur_is(";") && !self.cur_is("}") && self.current().typ != TokenType::Newline {
                val = Some(self.parse_expr()?);
            }
            self.match_end_of_stmt()?;
            return Ok(Stmt { node: StmtNode::Return(val), line });
        }
        if cur == "if" { return self.parse_if(); }
        if cur == "while" { return self.parse_while(); }
        if cur == "for" { return self.parse_for(); }
        if cur == "try" { return self.parse_try(); }
        if cur == "switch" { return self.parse_switch(); }
        if cur == "break" { self.advance(); self.match_end_of_stmt()?; return Ok(Stmt { node: StmtNode::Break, line }); }
        if cur == "continue" { self.advance(); self.match_end_of_stmt()?; return Ok(Stmt { node: StmtNode::Continue, line }); }
        if cur == "import" { return self.parse_import(); }
        if cur == "do" { return self.parse_dowhile(); }
        if cur == "raise" {
            self.advance();
            let e = self.parse_expr()?;
            self.match_end_of_stmt()?;
            return Ok(Stmt { node: StmtNode::Raise(e), line });
        }

        if cur == "{" {
            if self.is_object_literal() {
                let e = self.parse_expr()?;
                self.match_end_of_stmt()?;
                return Ok(Stmt { node: StmtNode::Expr(e), line });
            }
            return self.parse_block();
        }

        let expr = self.parse_expr()?;
        if self.cur_is("=") {
             self.advance();
             let rhs = self.parse_expr()?;
             self.match_end_of_stmt()?;
             return Ok(Stmt { node: StmtNode::Expr(Expr { node: ExprNode::Assign(Box::new(expr), Box::new(rhs)), line }), line });
        }

        let current_val = self.current().value.clone();
        if matches!(current_val.as_str(), "+=" | "-=" | "*=" | "/=" | "%=") {
            let op = &current_val[..1];
            self.advance();
            let rhs = self.parse_expr()?;
            self.match_end_of_stmt()?;
            let bin = Expr {
                node: ExprNode::Binary(op.to_string(), Box::new(expr.clone()), Box::new(rhs)),
                line,
            };
            return Ok(Stmt {
                node: StmtNode::Expr(Expr { node: ExprNode::Assign(Box::new(expr), Box::new(bin)), line }),
                line,
            });
        }

        self.match_end_of_stmt()?;
        Ok(Stmt { node: StmtNode::Expr(expr), line })
    }

   fn match_end_of_stmt(&mut self) -> Result<(), String> {
        if self.match_token(";") {
            return Ok(());
        }

        if self.current().typ == TokenType::Newline || self.current().typ == TokenType::Eof || self.cur_is("}") {
            return Ok(());
        }
        Ok(())
    }

    fn is_object_literal(&self) -> bool {
        let mut i = 1;
        let mut depth = 0;
        loop {
            let t = self.peek(i);
            if t.typ == TokenType::Eof { return false; }
            if t.value == "{" { depth += 1; }
            else if t.value == "}" {
                if depth == 0 { return false; }
                depth -= 1;
            }
            else if t.value == ":" && depth == 0 { return true; }
            i += 1;
            if i > 50 { return false; }
        }
    }

     fn parse_const(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("const")?;
        let name = self.expect("NAME")?.value;
        self.expect("=")?;
        let val = self.parse_expr()?;
        self.match_end_of_stmt()?;
        Ok(Stmt { node: StmtNode::ConstDecl(name, val), line })
    }

    fn parse_func(&mut self, is_async: bool) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("func")?;
        let mut name = String::new();
        if let TokenType::Name = self.current().typ {
            name = self.advance().value;
        }
        self.expect("(")?;
        let mut params = Vec::new();
        let mut default_inits = Vec::new();
        let mut req_params = 0;

        if !self.cur_is(")") {
            loop {
                if self.cur_is("...") {
                    self.advance();
                    let rest_name = self.expect("NAME")?.value;
                    params.push((format!("...{}", rest_name), None));
                    self.match_token(",");
                    break;
                }

                let p_name = self.expect("NAME")?.value;
                let mut p_type = None;
                if self.match_token(":") {
                    p_type = Some(self.expect("NAME")?.value);
                }
                params.push((p_name.clone(), p_type));

                if self.match_token("=") {
                    let def_expr = self.parse_expr()?;
                    let p_var = Expr { node: ExprNode::Var(p_name.clone()), line };
                    let coalesce = Expr { node: ExprNode::Binary("??".to_string(), Box::new(p_var.clone()), Box::new(def_expr)), line };
                    let assign_expr = Expr { node: ExprNode::Assign(Box::new(p_var), Box::new(coalesce)), line };
                    default_inits.push(Stmt { node: StmtNode::Expr(assign_expr), line });
                } else {
                    if !default_inits.is_empty() {
                        return Err(format!("Required parameter '{}' cannot follow optional parameters at line {}", p_name, line));
                    }
                    req_params += 1;
                }

                if !self.match_token(",") { break; }
            }
        }
        self.expect(")")?;
        let mut body = self.parse_block()?;

        if !default_inits.is_empty() {
            if let StmtNode::Block(ref mut stmts) = body.node {
                for (i, stmt) in default_inits.into_iter().enumerate() {
                    stmts.insert(i, stmt);
                }
            }
        }

        Ok(Stmt { node: StmtNode::FuncDef(name, params, req_params, Box::new(body), is_async), line })
    }

    fn parse_bitwise(&mut self) -> Result<Expr, String> {
    let mut node = self.parse_add()?;
    while self.current().typ == TokenType::Op {
        let op = self.current().value.clone();
        if ["<<", ">>", "&", "|", "^"].contains(&op.as_str()) {
            self.advance();
            let rhs = self.parse_add()?;
            node = Expr {
                node: ExprNode::Binary(op, Box::new(node.clone()), Box::new(rhs)),
                line: node.line
            };
            continue;
        }
        break;
    }
    Ok(node)
}

    fn parse_class(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("class")?;
        let name = self.expect("NAME")?.value;
        let mut parent = None;
        if self.match_token("extends") {
            parent = Some(self.expect("NAME")?.value);
        }
        let body = self.parse_block()?;
        Ok(Stmt { node: StmtNode::ClassDef(name, parent, Box::new(body)), line })
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("if")?;
        let has_paren = self.match_token("(");
        let cond = self.parse_expr()?;
        if has_paren { self.expect(")")?; }
        let body = self.parse_block()?;

        let mut chain = vec![(Some(cond), body)];

        while self.current().typ == TokenType::Newline {
            self.advance();
        }

        while self.match_token("elif") {
            let p = self.match_token("(");
            let c = self.parse_expr()?;
            if p { self.expect(")")?; }
            let b = self.parse_block()?;
            chain.push((Some(c), b));
            while self.current().typ == TokenType::Newline {
                self.advance();
            }
        }

        if self.match_token("else") {
            let b = self.parse_block()?;
            chain.push((None, b));
        }
        Ok(Stmt { node: StmtNode::IfChain(chain), line })
    }
    fn parse_while(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("while")?;
        let p = self.match_token("(");
        let cond = self.parse_expr()?;
        if p { self.expect(")")?; }
        let body = self.parse_block()?;
        Ok(Stmt { node: StmtNode::While(cond, Box::new(body)), line })
    }

    fn parse_dowhile(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("do")?;
        let body = self.parse_block()?;
        self.expect("while")?;
        let p = self.match_token("(");
        let cond = self.parse_expr()?;
        if p { self.expect(")")?; }
        self.match_end_of_stmt()?;
        Ok(Stmt { node: StmtNode::DoWhile(Box::new(body), cond), line })
    }

    fn parse_for(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("for")?;
        if self.match_token("(") {
            self.match_token("var");
            if matches!(self.current().typ, TokenType::Name)
                && (self.peek_is(1, "in") || self.peek_is(1, ","))
            {
                let name = self.advance().value;
                let value_var = if self.match_token(",") {
                    self.match_token("var");
                    Some(self.expect("NAME")?.value)
                } else { None };
                self.expect("in")?;
                let iter = self.parse_expr()?;
                self.expect(")")?;
                let body = self.parse_block()?;
                return Ok(Stmt { node: StmtNode::For(name, value_var, iter, Box::new(body)), line });
            }
            let mut init = None;
            if !self.cur_is(";") {
                let ex = self.parse_expr()?;
                if self.cur_is("=") {
                    self.advance();
                    let rhs = self.parse_expr()?;
                    init = Some(Expr { node: ExprNode::Assign(Box::new(ex.clone()), Box::new(rhs)), line: ex.line });
                } else {
                    init = Some(ex);
                }
            }
            self.expect(";")?;
            let mut cond = None;
            if !self.cur_is(";") { cond = Some(self.parse_expr()?); }
            self.expect(";")?;
            let mut step = None;
            if !self.cur_is(")") {
                 let ex = self.parse_expr()?;
                 if self.cur_is("=") {
                    self.advance();
                    let rhs = self.parse_expr()?;
                    step = Some(Expr { node: ExprNode::Assign(Box::new(ex.clone()), Box::new(rhs)), line: ex.line });
                 } else {
                    step = Some(ex);
                 }
            }
            self.expect(")")?;
            let body = self.parse_block()?;
            return Ok(Stmt { node: StmtNode::CFor(init, cond, step, Box::new(body)), line });
        }
        self.match_token("var");

        let name = self.expect("NAME")?.value;
        let value_var = if self.match_token(",") {
            self.match_token("var");
            Some(self.expect("NAME")?.value)
        } else { None };
        self.expect("in")?;
        let iter = self.parse_expr()?;
        let body = self.parse_block()?;
        Ok(Stmt { node: StmtNode::For(name, value_var, iter, Box::new(body)), line })
    }

    fn parse_import(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("import")?;
        let path = if self.current().typ == TokenType::String {
            self.advance().value
        } else {
            self.expect("NAME")?.value
        };
        let mut as_name = None;
        if self.match_token("as") {
            if self.cur_is("*") {
                self.advance();
                as_name = Some("*".to_string());
            } else {
                as_name = Some(self.expect("NAME")?.value);
            }
        }
        self.match_end_of_stmt()?;
        Ok(Stmt { node: StmtNode::Import(path, as_name), line })
    }

    fn parse_try(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("try")?;
        let try_blk = self.parse_block()?;
        let mut catch_var = None;
        let mut catch_blk = None;
        if self.match_token("catch") {
            self.expect("(")?;
            catch_var = Some(self.expect("NAME")?.value);
            self.expect(")")?;
            catch_blk = Some(self.parse_block()?);
        }
        let mut fin_blk = None;
        if self.match_token("finally") {
            fin_blk = Some(self.parse_block()?);
        }
        Ok(Stmt { node: StmtNode::Try(Box::new(try_blk), catch_var, catch_blk.map(Box::new), fin_blk.map(Box::new)), line })
    }

    fn parse_switch(&mut self) -> Result<Stmt, String> {
        let line = self.current().line;
        self.expect("switch")?;
        let p = self.match_token("(");
        let val = self.parse_expr()?;
        if p { self.expect(")")?; }
        self.expect("{")?;
        let mut cases = Vec::new();
        let mut default = None;
        while !self.cur_is("}") && self.current().typ != TokenType::Eof {
            if self.match_token("case") {
                let v = self.parse_expr()?;
                self.expect(":")?;
                let blk = self.parse_block()?;
                cases.push((v, blk));
            } else if self.match_token("default") {
                self.expect(":")?;
                default = Some(Box::new(self.parse_block()?));
            } else {
                 return Err(format!("Unexpected token in switch at line {}", self.current().line));
            }
        }
        self.expect("}")?;
        Ok(Stmt { node: StmtNode::Switch(val, cases, default), line })
    }

    pub fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<Expr, String> {
        let node = self.parse_null_coalesce()?;
        if self.match_token("?") {
            let t = self.parse_expr()?;
            self.expect(":")?;
            let f = self.parse_expr()?;
            return Ok(Expr { node: ExprNode::Ternary(Box::new(node.clone()), Box::new(t), Box::new(f)), line: node.line });
        }
        Ok(node)
    }

    fn parse_null_coalesce(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_or()?;
        while self.match_token("??") {
            let rhs = self.parse_or()?;
            node = Expr {
                node: ExprNode::Binary("??".to_string(), Box::new(node.clone()), Box::new(rhs)),
                line: node.line
            };
        }
        Ok(node)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_and()?;
        while self.match_token("or") {
            let rhs = self.parse_and()?;
            node = Expr { node: ExprNode::Binary("or".to_string(), Box::new(node.clone()), Box::new(rhs)), line: node.line };
        }
        Ok(node)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_not()?;
        while self.match_token("and") {
            let rhs = self.parse_not()?;
            node = Expr { node: ExprNode::Binary("and".to_string(), Box::new(node.clone()), Box::new(rhs)), line: node.line };
        }
        Ok(node)
    }

    fn parse_not(&mut self) -> Result<Expr, String> {
        let line = self.current().line;

        if self.match_token("not") {
            let e = self.parse_not()?;
            return Ok(Expr { node: ExprNode::Unary("not".to_string(), Box::new(e)), line });
        }
        if self.current().typ == TokenType::Op && self.cur_is("!") {
            self.advance();
            let e = self.parse_not()?;
            return Ok(Expr { node: ExprNode::Unary("not".to_string(), Box::new(e)), line });
        }

        self.parse_cmp()
    }

    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_add()?;
        loop {
            if self.current().typ == TokenType::Op {
                let op = self.current().value.clone();
                if ["==","!=","<",">","<=",">=","in"].contains(&op.as_str()) {
                    self.advance();
                    let rhs = self.parse_add()?;
                    node = Expr { node: ExprNode::Binary(op, Box::new(node.clone()), Box::new(rhs)), line: node.line };
                    continue;
                }
            }
            break;
        }
        Ok(node)
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_mul()?;
        loop {
            if self.current().typ == TokenType::Op {
                let op = self.current().value.clone();
                if ["+", "-", "<<", ">>", "&", "|", "^"].contains(&op.as_str()) {
                self.advance();
                let rhs = self.parse_mul()?;
                node = Expr { node: ExprNode::Binary(op, Box::new(node.clone()), Box::new(rhs)), line: node.line };
                continue;
            }
            }
            break;
        }
        Ok(node)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_unary()?;
        loop {
            if self.current().typ == TokenType::Op {
                let op = self.current().value.clone();
                if ["*","/","%"].contains(&op.as_str()) {
                    self.advance();
                    let rhs = self.parse_unary()?;
                    node = Expr { node: ExprNode::Binary(op, Box::new(node.clone()), Box::new(rhs)), line: node.line };
                    continue;
                }
            }
            break;
        }
        Ok(node)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.match_token("await") {
            let line = self.current().line;
            let expr = self.parse_unary()?;
            return Ok(Expr { node: ExprNode::Await(Box::new(expr)), line });
        }

        if self.current().typ == TokenType::Op && ["+", "-"].contains(&self.current().value.as_str()) {
            let op = self.advance().value;
            let v = self.parse_unary()?;
            Ok(Expr { node: ExprNode::Unary(op, Box::new(v)), line: 0 })
        } else if self.match_token("new") {
            let mut cls_expr = self.parse_primary()?;

            while self.match_token(".") {
                let t = self.advance();
                let attr = match t.typ {
                    TokenType::Name | TokenType::Keyword(_) => t.value,
                    _ => return Err(format!("Expected property name after '.', got '{}' at line {}", t.value, t.line))
                };
                let line = cls_expr.line;
                cls_expr = Expr {
                    node: ExprNode::GetAttr(Box::new(cls_expr), attr),
                    line
                };
            }

            let args = if self.match_token("(") {
                self.parse_call_args()?
            } else {
                Vec::new()
            };

            return Ok(Expr { node: ExprNode::New(Box::new(cls_expr), args), line: 0 });
        } else {
            self.parse_postfix()
        }
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, String> {
        let mut args: Vec<Expr> = Vec::new();
        let mut named: Vec<(String, Expr)> = Vec::new();
        let line = self.current().line;
        loop {
            while self.current().typ == TokenType::Newline { self.advance(); }
            if self.cur_is(")") { break; }
            let cur_is_name = matches!(self.current().typ, TokenType::Name | TokenType::Keyword(_));
            if cur_is_name && self.peek_is(1, ":") && self.peek(1).typ == TokenType::Op {
                let name = self.advance().value;
                self.advance();
                while self.current().typ == TokenType::Newline { self.advance(); }
                let val = self.parse_expr()?;
                if named.iter().any(|(n, _)| *n == name) {
                    return Err(format!("Duplicate named argument '{}' at line {}", name, line));
                }
                named.push((name, val));
            } else {
                if !named.is_empty() {
                    return Err(format!(
                        "Positional argument cannot follow named arguments at line {}",
                        self.current().line
                    ));
                }
                args.push(self.parse_expr()?);
            }
            while self.current().typ == TokenType::Newline { self.advance(); }
            if !self.match_token(",") { break; }
        }
        while self.current().typ == TokenType::Newline { self.advance(); }
        self.expect(")")?;
        if !named.is_empty() {
            let mut pairs = vec![(
                "__named__".to_string(),
                Expr { node: ExprNode::Bool(true), line },
            )];
            pairs.extend(named);
            args.push(Expr { node: ExprNode::Obj(pairs), line });
        }
        Ok(args)
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut node = self.parse_primary()?;
        loop {
            if self.match_token("(") {
                let args = self.parse_call_args()?;
                node = Expr { node: ExprNode::Call(Box::new(node.clone()), args), line: node.line };
            } else if self.match_token("?.") {
                let t = self.advance();
                let attr = match t.typ {
                    TokenType::Name | TokenType::Keyword(_) => t.value,
                    _ => return Err(format!("Expected property name after '?.', got '{}' at line {}", t.value, t.line))
                };
                node = Expr { node: ExprNode::OptionalGetAttr(Box::new(node.clone()), attr), line: node.line };
            } else if self.match_token(".") {
                let t = self.advance();
                let attr = match t.typ {
                    TokenType::Name | TokenType::Keyword(_) => t.value,
                    _ => return Err(format!("Expected property name after '.', got '{}' at line {}", t.value, t.line))
                };

                node = Expr { node: ExprNode::GetAttr(Box::new(node.clone()), attr), line: node.line };
            } else if self.match_token("[") {
                let idx = self.parse_expr()?;
                self.expect("]")?;
                node = Expr { node: ExprNode::Index(Box::new(node.clone()), Box::new(idx)), line: node.line };
            } else if matches!(self.current().typ, TokenType::Inc | TokenType::Dec) {
                let op = self.advance().value;
                node = Expr { node: ExprNode::PostOp(if op=="++"{"INC".into()}else{"DEC".into()}, Box::new(node.clone())), line: node.line };
            } else {
                break;
            }
        }
        Ok(node)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let t = self.current().clone();
        let line = t.line;
        match t.typ {
            TokenType::Number => { self.advance(); Ok(Expr { node: ExprNode::Number(t.num_val.unwrap()), line }) },
            TokenType::String => {
                self.advance();
                if let Some(raw) = t.value.strip_prefix('\x01') {
                    let expr = parse_interp_string(raw, t.line)?;
                    Ok(expr)
                } else {
                    Ok(Expr { node: ExprNode::String(t.value), line })
                }
            },
            TokenType::Keyword(ref k) => match k.as_str() {
                "True" => { self.advance(); Ok(Expr { node: ExprNode::Bool(true), line }) },
                "False" => { self.advance(); Ok(Expr { node: ExprNode::Bool(false), line }) },
                "null" => { self.advance(); Ok(Expr { node: ExprNode::Null, line }) },
                "this" => { self.advance(); Ok(Expr { node: ExprNode::This, line }) },
                "super" => {
                    self.advance();
                    self.expect(".")?;
                    let method = self.expect("NAME")?.value;
                    self.expect("(")?;
                    let args = self.parse_call_args()?;
                    Ok(Expr { node:ExprNode::Super(method, args), line})
                },
                "func" => {
                    self.advance();
                    self.expect("(")?;
                    let mut params = Vec::new();
                    let mut default_inits = Vec::new();
                    let mut req_params = 0;

                    if !self.cur_is(")") {
                        loop {
                            if self.cur_is("...") {
                                self.advance();
                                let rest_name = self.expect("NAME")?.value;
                                params.push((format!("...{}", rest_name), None));
                                self.match_token(",");
                                break;
                            }

                            let p_name = self.expect("NAME")?.value;
                            let mut p_type = None;
                            if self.match_token(":") {
                                p_type = Some(self.expect("NAME")?.value);
                            }
                            params.push((p_name.clone(), p_type));

                            if self.match_token("=") {
                                let def_expr = self.parse_expr()?;
                                let p_var = Expr { node: ExprNode::Var(p_name.clone()), line };
                                let coalesce = Expr { node: ExprNode::Binary("??".to_string(), Box::new(p_var.clone()), Box::new(def_expr)), line };
                                let assign_expr = Expr { node: ExprNode::Assign(Box::new(p_var), Box::new(coalesce)), line };
                                default_inits.push(Stmt { node: StmtNode::Expr(assign_expr), line });
                            } else {
                                if !default_inits.is_empty() {
                                    return Err(format!("Required parameter '{}' cannot follow optional parameters at line {}", p_name, line));
                                }
                                req_params += 1;
                            }
                            if !self.match_token(",") { break; }
                        }
                    }
                    self.expect(")")?;
                    let mut body = self.parse_block()?;
                    if !default_inits.is_empty() {
                        if let StmtNode::Block(ref mut stmts) = body.node {
                            for (i, stmt) in default_inits.into_iter().enumerate() {
                                stmts.insert(i, stmt);
                            }
                        }
                    }
                    Ok(Expr { node: ExprNode::FuncExpr(params, req_params, Box::new(body)), line })
                },
                _ => { self.advance(); Ok(Expr { node: ExprNode::Var(t.value), line }) }
            },
            TokenType::Name => { self.advance(); Ok(Expr { node: ExprNode::Var(t.value), line }) },
            TokenType::Op => {
                if t.value == "(" {

                    if self.is_lambda_start() {
                        self.advance();
                        let mut params = Vec::new();
                        let mut default_inits = Vec::new();
                        let mut req_params = 0;

                        if !self.cur_is(")") {
                            loop {
                                if self.cur_is("...") {
                                    self.advance();
                                    let rest_name = self.expect("NAME")?.value;
                                    params.push((format!("...{}", rest_name), None));
                                    self.match_token(",");
                                    break;
                                }

                                let p_name = self.expect("NAME")?.value;
                                let mut p_type = None;
                                if self.match_token(":") {
                                    p_type = Some(self.expect("NAME")?.value);
                                }
                                params.push((p_name.clone(), p_type));

                                if self.match_token("=") {
                                    let def_expr = self.parse_expr()?;
                                    let p_var = Expr { node: ExprNode::Var(p_name.clone()), line };
                                    let coalesce = Expr { node: ExprNode::Binary("??".to_string(), Box::new(p_var.clone()), Box::new(def_expr)), line };
                                    let assign_expr = Expr { node: ExprNode::Assign(Box::new(p_var), Box::new(coalesce)), line };
                                    default_inits.push(Stmt { node: StmtNode::Expr(assign_expr), line });
                                } else {
                                    if !default_inits.is_empty() {
                                        return Err(format!("Required parameter '{}' cannot follow optional parameters at line {}", p_name, line));
                                    }
                                    req_params += 1;
                                }
                                if !self.match_token(",") { break; }
                            }
                        }
                        self.expect(")")?;
                        self.expect("=>")?;

                        let mut body = if self.cur_is("{") {
                            self.parse_block()?
                        } else {
                            let expr = self.parse_expr()?;
                            Stmt { node: StmtNode::Return(Some(expr)), line: self.current().line }
                        };

                        if !default_inits.is_empty() {
                            if !matches!(body.node, StmtNode::Block(_)) {
                                body = Stmt { node: StmtNode::Block(vec![body]), line: line };
                            }
                            if let StmtNode::Block(ref mut stmts) = body.node {
                                for (i, stmt) in default_inits.into_iter().enumerate() {
                                    stmts.insert(i, stmt);
                                }
                            }
                        }
                        return Ok(Expr { node: ExprNode::FuncExpr(params, req_params, Box::new(body)), line });
                    }

                    self.advance();
                    let e = self.parse_expr()?;
                    self.expect(")")?;
                    Ok(e)
                } else if t.value == "[" {
                    self.advance();
                    let mut list = Vec::new();
                    let is_end = self.current().typ == TokenType::Op && self.cur_is("]");
                    if !is_end {
                        loop {
                            list.push(self.parse_expr()?);
                            if !self.match_token(",") { break; }
                        }
                    }
                    self.expect("]")?;
                    Ok(Expr { node: ExprNode::List(list), line })
                } else if t.value == "{" {
                    self.advance();
                    let mut obj = Vec::new();
                    let is_end = self.current().typ == TokenType::Op && self.cur_is("}");
                    if !is_end {
                        loop {
                            let k = if self.current().typ == TokenType::String {
                                self.advance().value
                            } else {
                                self.expect("NAME")?.value
                            };
                            self.expect(":")?;
                            let v = self.parse_expr()?;
                            obj.push((k, v));
                            if !self.match_token(",") { break; }
                        }
                    }
                    self.expect("}")?;
                    Ok(Expr { node: ExprNode::Obj(obj), line })
                } else {
                    Err(format!("Unexpected token {} at line {}", t.value, line))
                }
            },
            _ => Err(format!("Unexpected token {:?} at line {}", t, line)),
        }
    }
}
