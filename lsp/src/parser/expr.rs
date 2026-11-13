//! Expression and FORM-string parser (char-level recursive descent).
//!
//! Two entry points matter:
//!
//! * [`parse_expr`] / [`ExprParser`] — Emuera expressions. Precedence is
//!   handled by `binary(level)`, one level per row of the table in
//!   `binop`, lowest (`||`) first; `primary` handles literals, `@"..."`,
//!   `\@...\@`, `'text`, calls, `#DEFINE` macros and `VAR:idx:idx` chains.
//! * [`parse_form`] — FORM strings (`text %str% {int} \@ c ? a # b \@`).
//!
//! The awkward part is nesting: `@"a %F("x")% b"` contains a quote that
//! does *not* end the outer string, because it is inside a `%...%`
//! expression. The `scan_*` functions find where such regions really end,
//! mirroring Emuera's own lexer, so every caller agrees on the boundaries.
//!
//! **LSP Adaptation**: Added position tracking for all parsed expressions.

use crate::fxhash::HashMap;
use crate::parser::ast::*;
use std::sync::Arc as Rc;

pub type PResult<T> = Result<T, String>;

/// Whether full-width space is treated as whitespace (Emuera's SystemAllowFullSpace)
pub static FULL_SPACE_IS_SPACE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);
/// Whether triple symbols in FORM are expanded (SystemIgnoreTripleSymbol)
pub static EXPAND_TRIPLE_SYMBOLS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// Emuera's error for a full-width space in code
pub const FULL_SPACE_ERROR: &str =
    "予期しない全角スペースを発見しました(この警告はシステムオプション「全角スペースをホワイトスペースに含める」により無視できます)";

/// With 全角スペースをホワイトスペースに含める off: a full-width space in
/// expression code (outside string literals) is Emuera's error
pub fn check_full_space(src: &str) -> PResult<()> {
    if FULL_SPACE_IS_SPACE.load(std::sync::atomic::Ordering::Relaxed) || !src.contains('\u{3000}') {
        return Ok(());
    }
    let ch: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut yen = false;
    while i < ch.len() {
        match ch[i] {
            '@' if ch.get(i + 1) == Some(&'"') => i = skip_form_string(&ch, i + 2),
            '"' => i = skip_string(&ch, i + 1),
            '\\' if ch.get(i + 1) == Some(&'@') && !yen => {
                yen = true;
                i += 2;
            }
            '?' if yen => {
                yen = false;
                i += 1;
                while i < ch.len() && !(ch[i] == '\\' && ch.get(i + 1) == Some(&'@')) {
                    i += if ch[i] == '\\' { 2 } else { 1 };
                }
                i += 2;
            }
            '\u{3000}' => return Err(FULL_SPACE_ERROR.into()),
            _ => i += 1,
        }
    }
    Ok(())
}

/// The index after the `"` that ends a string literal whose text starts at `i`.
fn skip_string(ch: &[char], mut i: usize) -> usize {
    while i < ch.len() {
        match ch[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// The index after the `"` that ends a FORM string (`@"..."`) whose text
/// starts at `i`; quotes inside its `%...%` and `{...}` code don't end it.
fn skip_form_string(ch: &[char], mut i: usize) -> usize {
    let (mut pct, mut brace) = (false, 0usize);
    while i < ch.len() {
        let code = pct || brace > 0;
        match ch[i] {
            '\\' => i += 2,
            '@' if code && ch.get(i + 1) == Some(&'"') => i = skip_form_string(ch, i + 2),
            '"' if code => i = skip_string(ch, i + 1),
            '"' => return i + 1,
            '%' if brace == 0 => {
                pct = !pct;
                i += 1;
            }
            '{' if !pct => {
                brace += 1;
                i += 1;
            }
            '}' if brace > 0 => {
                brace -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    i
}

/// Cursor over one expression's characters. `defs` are the program's
/// `#DEFINE`s (expanded inline, `depth` guards against runaway recursion).
pub struct ExprParser<'a> {
    s: Vec<char>,
    pub pos: usize,
    defs: &'a HashMap<String, String>,
    depth: usize,
    /// Start position of current token/expression for span tracking
    token_start: usize,
}

impl<'a> ExprParser<'a> {
    pub fn new(src: &str, defs: &'a HashMap<String, String>) -> Self {
        ExprParser {
            s: src.chars().collect(),
            pos: 0,
            defs,
            depth: 0,
            token_start: 0,
        }
    }

    /// A sub-parser for a `#DEFINE` body, one expansion level deeper.
    fn with_depth(src: &str, defs: &'a HashMap<String, String>, depth: usize) -> Self {
        ExprParser {
            s: src.chars().collect(),
            pos: 0,
            defs,
            depth,
            token_start: 0,
        }
    }

    /// The unparsed remainder (for error messages and callers that take
    /// over after an expression, e.g. assignments).
    pub fn rest(&self) -> String {
        self.s[self.pos.min(self.s.len())..].iter().collect()
    }

    /// Get current byte position in source
    pub fn current_byte_pos(&self, src: &str) -> usize {
        src.chars().take(self.pos).map(|c| c.len_utf8()).sum()
    }

    pub fn skip_ws(&mut self) {
        while self.pos < self.s.len()
            && (self.s[self.pos].is_whitespace() || self.s[self.pos] == '\u{3000}')
        {
            self.pos += 1;
        }
    }

    pub fn peek(&self) -> Option<char> {
        self.s.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.s.get(self.pos + n).copied()
    }

    pub fn at_end(&mut self) -> bool {
        self.skip_ws();
        self.pos >= self.s.len()
    }

    fn starts_with(&self, t: &str) -> bool {
        t.chars()
            .enumerate()
            .all(|(k, c)| self.s.get(self.pos + k) == Some(&c))
    }

    /// Skip whitespace and consume `t` if it comes next.
    pub fn eat(&mut self, t: &str) -> bool {
        self.skip_ws();
        if self.starts_with(t) {
            self.pos += t.chars().count();
            true
        } else {
            false
        }
    }

    /// Parse an identifier (variable name, function name, etc.)
    pub fn parse_ident(&mut self) -> PResult<String> {
        self.skip_ws();
        let start = self.pos;
        while self.pos < self.s.len() && is_ident_char(self.s[self.pos]) {
            self.pos += 1;
        }
        if start == self.pos {
            return Err("expected identifier".into());
        }
        Ok(self.s[start..self.pos].iter().collect())
    }

    /// Parse a number literal (decimal, hex, binary)
    pub fn parse_number(&mut self) -> PResult<i64> {
        self.skip_ws();
        let start = self.pos;

        // Check for hex (0x...) or binary (0b...)
        if self.starts_with("0x") || self.starts_with("0X") {
            self.pos += 2;
            while self.pos < self.s.len() && self.s[self.pos].is_ascii_hexdigit() {
                self.pos += 1;
            }
            let s: String = self.s[start + 2..self.pos].iter().collect();
            return i64::from_str_radix(&s, 16).map_err(|_| "invalid hex number".into());
        }
        if self.starts_with("0b") || self.starts_with("0B") {
            self.pos += 2;
            while self.pos < self.s.len() && matches!(self.s[self.pos], '0' | '1') {
                self.pos += 1;
            }
            let s: String = self.s[start + 2..self.pos].iter().collect();
            return i64::from_str_radix(&s, 2).map_err(|_| "invalid binary number".into());
        }

        // Decimal with optional exponent (1e3) or shift (1p3)
        let mut has_digits = false;
        while self.pos < self.s.len() && self.s[self.pos].is_ascii_digit() {
            self.pos += 1;
            has_digits = true;
        }

        if self.pos < self.s.len() && (self.s[self.pos] == 'e' || self.s[self.pos] == 'E') {
            self.pos += 1;
            let exp_sign = if self.pos < self.s.len()
                && (self.s[self.pos] == '+' || self.s[self.pos] == '-')
            {
                self.pos += 1;
                self.s[self.pos - 1] == '+'
            } else {
                true
            };
            let exp_start = self.pos;
            while self.pos < self.s.len() && self.s[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let exp: i64 = self.s[exp_start..self.pos]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            let base: i64 = self.s[start..exp_start - 1]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            let result = base * 10i64.pow(exp.abs() as u32);
            return Ok(if exp_sign {
                result
            } else {
                base / 10i64.pow(exp.abs() as u32)
            });
        }

        if self.pos < self.s.len() && (self.s[self.pos] == 'p' || self.s[self.pos] == 'P') {
            self.pos += 1;
            let shift_start = self.pos;
            while self.pos < self.s.len() && self.s[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let shift: i64 = self.s[shift_start..self.pos]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            let base: i64 = self.s[start..shift_start - 1]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            return Ok(base << shift);
        }

        if !has_digits {
            return Err("expected number".into());
        }

        let s: String = self.s[start..self.pos].iter().collect();
        s.parse().map_err(|_| "invalid number".into())
    }

    /// Parse a string literal
    pub fn parse_string(&mut self) -> PResult<String> {
        self.skip_ws();
        if !self.eat("\"") {
            return Err("expected string".into());
        }
        let mut result = String::new();
        while self.pos < self.s.len() {
            match self.s[self.pos] {
                '\\' => {
                    self.pos += 1;
                    if self.pos < self.s.len() {
                        match self.s[self.pos] {
                            'n' => result.push('\n'),
                            't' => result.push('\t'),
                            'r' => result.push('\r'),
                            '\\' => result.push('\\'),
                            '"' => result.push('"'),
                            's' => result.push(' '),
                            'S' => result.push('\u{3000}'),
                            c => result.push(c),
                        }
                        self.pos += 1;
                    }
                }
                '"' => {
                    self.pos += 1;
                    return Ok(result);
                }
                c => {
                    result.push(c);
                    self.pos += 1;
                }
            }
        }
        Err("unterminated string".into())
    }

    /// Parse a FORM string (@"...")
    pub fn parse_form_string(&mut self) -> PResult<Form> {
        self.skip_ws();
        if !self.eat("@\"") {
            return Err("expected form string".into());
        }
        let form = parse_form_with_parser(self)?;
        if !self.eat("\"") {
            return Err("unterminated form string".into());
        }
        Ok(form)
    }

    /// Main entry point: parse an expression with full precedence handling
    pub fn parse_expr(&mut self) -> PResult<Expr> {
        self.parse_binary(0)
    }

    /// Parse binary operations with precedence levels
    fn parse_binary(&mut self, level: usize) -> PResult<Expr> {
        // Precedence levels (lowest first):
        // 0: || &&
        // 1: | ^ &
        // 2: == !=
        // 3: < <= > >=
        // 4: << >>
        // 5: + -
        // 6: * / %
        // 7: unary

        if level >= 7 {
            return self.parse_unary();
        }

        let mut lhs = self.parse_binary(level + 1)?;

        loop {
            self.skip_ws();
            let op = match level {
                0 => {
                    if self.eat("||") {
                        Some(BinOp::Or)
                    } else if self.eat("&&") {
                        Some(BinOp::And)
                    } else {
                        None
                    }
                }
                1 => {
                    if self.eat("|") {
                        Some(BinOp::BitOr)
                    } else if self.eat("^") {
                        Some(BinOp::BitXor)
                    } else if self.eat("&") {
                        Some(BinOp::BitAnd)
                    } else {
                        None
                    }
                }
                2 => {
                    if self.eat("==") {
                        Some(BinOp::Eq)
                    } else if self.eat("!=") {
                        Some(BinOp::Ne)
                    } else {
                        None
                    }
                }
                3 => {
                    if self.eat("<=") {
                        Some(BinOp::Le)
                    } else if self.eat(">=") {
                        Some(BinOp::Ge)
                    } else if self.eat("<") {
                        Some(BinOp::Lt)
                    } else if self.eat(">") {
                        Some(BinOp::Gt)
                    } else {
                        None
                    }
                }
                4 => {
                    if self.eat("<<") {
                        Some(BinOp::Shl)
                    } else if self.eat(">>") {
                        Some(BinOp::Shr)
                    } else {
                        None
                    }
                }
                5 => {
                    if self.eat("+") {
                        Some(BinOp::Add)
                    } else if self.eat("-") {
                        Some(BinOp::Sub)
                    } else {
                        None
                    }
                }
                6 => {
                    if self.eat("*") {
                        Some(BinOp::Mul)
                    } else if self.eat("/") {
                        Some(BinOp::Div)
                    } else if self.eat("%") {
                        Some(BinOp::Mod)
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(op) = op {
                let rhs = self.parse_binary(level + 1)?;
                lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
            } else {
                break;
            }
        }

        Ok(lhs)
    }

    /// Parse unary operations
    fn parse_unary(&mut self) -> PResult<Expr> {
        self.skip_ws();
        if self.eat("-") {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary(UnOp::Neg, Box::new(expr)));
        }
        if self.eat("!") {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary(UnOp::Not, Box::new(expr)));
        }
        if self.eat("~") {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary(UnOp::BitNot, Box::new(expr)));
        }
        if self.eat("++") {
            let var = self.parse_var_ref()?;
            return Ok(Expr::IncDec(Rc::new(var), true, 1));
        }
        if self.eat("--") {
            let var = self.parse_var_ref()?;
            return Ok(Expr::IncDec(Rc::new(var), true, -1));
        }

        let mut expr = self.parse_primary()?;

        // Postfix ++/--
        self.skip_ws();
        if self.eat("++") {
            if let Expr::Var(v) = &expr {
                return Ok(Expr::IncDec(v.clone(), false, 1));
            }
        }
        if self.eat("--") {
            if let Expr::Var(v) = &expr {
                return Ok(Expr::IncDec(v.clone(), false, -1));
            }
        }

        // Ternary: c ? a # b
        self.skip_ws();
        if self.eat("?") {
            let then_expr = self.parse_expr()?;
            if !self.eat("#") {
                return Err("expected # in ternary".into());
            }
            let else_expr = self.parse_expr()?;
            expr = Expr::Ternary(Box::new(expr), Box::new(then_expr), Box::new(else_expr));
        }

        Ok(expr)
    }

    /// Parse primary expressions: literals, variables, calls, FORM strings, parenthesized
    fn parse_primary(&mut self) -> PResult<Expr> {
        self.skip_ws();

        // Parenthesized expression
        if self.eat("(") {
            let expr = self.parse_expr()?;
            if !self.eat(")") {
                return Err("expected )".into());
            }
            return Ok(expr);
        }

        // String literal
        if self.peek() == Some('"') {
            let s = self.parse_string()?;
            return Ok(Expr::Str(s));
        }

        // FORM string
        if self.starts_with("@\"") {
            let form = self.parse_form_string()?;
            return Ok(Expr::Form(Rc::new(form)));
        }

        // Number literal
        if self.peek().map_or(false, |c| c.is_ascii_digit())
            || self.starts_with("0x")
            || self.starts_with("0X")
            || self.starts_with("0b")
            || self.starts_with("0B")
        {
            let n = self.parse_number()?;
            return Ok(Expr::Int(n));
        }

        // Identifier (variable, function call, or #DEFINE)
        if let Some(c) = self.peek() {
            if is_ident_char(c) {
                let ident = self.parse_ident()?;
                let upper = ident.to_uppercase();

                // Check for #DEFINE expansion
                if self.depth < 32 {
                    if let Some(expansion) = self.defs.get(&upper) {
                        let mut sub = ExprParser::with_depth(expansion, self.defs, self.depth + 1);
                        return sub.parse_expr();
                    }
                }

                // Check if it's a function call
                self.skip_ws();
                if self.eat("(") {
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            if self.at_end() {
                                break;
                            }
                            if self.peek() == Some(',') {
                                args.push(Expr::Empty);
                                self.pos += 1;
                                continue;
                            }
                            let arg = self.parse_expr()?;
                            args.push(arg);
                            if self.eat(")") {
                                break;
                            }
                            if !self.eat(",") {
                                return Err("expected , or ) in function call".into());
                            }
                        }
                    }
                    return Ok(Expr::Call(Box::new((upper, args, Default::default()))));
                }

                // Variable reference with optional indices
                let var = self.parse_var_ref_after_name(upper)?;
                return Ok(Expr::Var(Rc::new(var)));
            }
        }

        // Special: 'text (string until comma/newline)
        if self.peek() == Some('\'') {
            self.pos += 1;
            let mut result = String::new();
            while self.pos < self.s.len()
                && self.s[self.pos] != ','
                && self.s[self.pos] != '\n'
                && self.s[self.pos] != '\r'
            {
                result.push(self.s[self.pos]);
                self.pos += 1;
            }
            return Ok(Expr::Str(result));
        }

        Err("expected expression".into())
    }

    /// Parse a variable reference (after the name has been parsed)
    fn parse_var_ref_after_name(&mut self, name: String) -> PResult<VarRef> {
        let mut idx = Vec::new();
        loop {
            self.skip_ws();
            if !self.eat(":") {
                break;
            }
            let idx_expr = self.parse_expr()?;
            idx.push(idx_expr);
        }
        let name_rc: Name = name.into();
        Ok(VarRef::new(name_rc, idx))
    }

    /// Parse a full variable reference
    pub fn parse_var_ref(&mut self) -> PResult<VarRef> {
        let ident = self.parse_ident()?;
        let upper = ident.to_uppercase();
        self.parse_var_ref_after_name(upper)
    }
}

/// Parse an expression from a string
pub fn parse_expr(src: &str, defs: &HashMap<String, String>) -> PResult<Expr> {
    let mut parser = ExprParser::new(src, defs);
    let expr = parser.parse_expr()?;
    parser.skip_ws();
    if !parser.at_end() {
        return Err(format!("unexpected trailing text: {}", parser.rest()));
    }
    Ok(expr)
}

/// Parse a comma-separated list of expressions
pub fn parse_expr_list(src: &str, defs: &HashMap<String, String>) -> PResult<Vec<Expr>> {
    let mut parser = ExprParser::new(src, defs);
    let mut exprs = Vec::new();
    if parser.at_end() {
        return Ok(exprs);
    }
    loop {
        let expr = parser.parse_expr()?;
        exprs.push(expr);
        parser.skip_ws();
        if parser.eat(",") {
            continue;
        }
        break;
    }
    Ok(exprs)
}

/// Split a string at top-level commas (not inside parentheses, brackets, strings, etc.)
pub fn split_top(src: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut paren: i32 = 0;
    let mut in_string = false;
    let mut in_form = false;
    let mut form_pct = false;
    let mut form_brace = 0;
    let mut yen = false;
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '(' if !in_string && !in_form => paren += 1,
            ')' if !in_string && !in_form => paren = paren.saturating_sub(1),
            '"' if !yen && !in_form => {
                if in_string {
                    in_string = false;
                } else if i + 1 < chars.len() && chars[i + 1] == '@' {
                    // This is handled below
                } else {
                    in_string = true;
                }
            }
            '@' if i + 1 < chars.len() && chars[i + 1] == '"' && !in_string && !in_form => {
                in_form = true;
                form_pct = false;
                form_brace = 0;
                i += 1; // skip the quote
            }
            '%' if in_form && form_brace == 0 => form_pct = !form_pct,
            '{' if in_form && !form_pct => form_brace += 1,
            '}' if in_form && form_brace > 0 => form_brace -= 1,
            '"' if in_form && !form_pct && form_brace == 0 => in_form = false,
            '\\' if i + 1 < chars.len() && chars[i + 1] == '@' && !in_string => {
                yen = !yen;
                i += 1;
            }
            '?' if yen => yen = false,
            ',' if paren == 0 && !in_string && !in_form => {
                result.push(current.trim().to_string());
                current.clear();
                i += 1;
                continue;
            }
            _ => {}
        }
        current.push(c);
        i += 1;
    }
    result.push(current.trim().to_string());
    result
}

/// Parse a FORM string (the content inside @\"...\")
fn parse_form_with_parser(parser: &mut ExprParser<'_>) -> PResult<Form> {
    let mut parts = Vec::new();
    let mut literal = String::new();

    while parser.pos < parser.s.len() {
        match parser.s[parser.pos] {
            '\\' => {
                parser.pos += 1;
                if parser.pos < parser.s.len() {
                    match parser.s[parser.pos] {
                        'n' => literal.push('\n'),
                        't' => literal.push('\t'),
                        'r' => literal.push('\r'),
                        '\\' => literal.push('\\'),
                        '"' => literal.push('"'),
                        's' => literal.push(' '),
                        'S' => literal.push('\u{3000}'),
                        '@' if parser.pos + 1 < parser.s.len()
                            && parser.s[parser.pos + 1] == '"' =>
                        {
                            // Nested @\"...\"
                            if !literal.is_empty() {
                                parts.push(FormPart::Lit(literal));
                                literal = String::new();
                            }
                            parser.pos += 1; // skip @
                            let nested = parse_form_with_parser(parser)?;
                            parts.push(FormPart::Lit(
                                nested
                                    .parts
                                    .iter()
                                    .map(|p| match p {
                                        FormPart::Lit(s) => s.clone(),
                                        _ => String::new(),
                                    })
                                    .collect::<String>(),
                            ));
                        }
                        c => literal.push(c),
                    }
                    parser.pos += 1;
                }
            }
            '"' => {
                parser.pos += 1;
                break;
            }
            '%' => {
                if !literal.is_empty() {
                    parts.push(FormPart::Lit(literal));
                    literal = String::new();
                }
                parser.pos += 1;
                let expr = parser.parse_expr()?;
                let mut width = None;
                let mut align = Align::Left;
                parser.skip_ws();
                if parser.eat(",") {
                    width = Some(Box::new(parser.parse_expr()?));
                    parser.skip_ws();
                    if parser.eat(",") {
                        align = match parser.parse_ident()?.to_uppercase().as_str() {
                            "CENTER" => Align::Center,
                            "RIGHT" => Align::Right,
                            _ => Align::Left,
                        };
                    }
                }
                if !parser.eat("%") {
                    return Err("expected % to close interpolation".into());
                }
                parts.push(FormPart::Str(expr, width, align));
            }
            '{' => {
                if !literal.is_empty() {
                    parts.push(FormPart::Lit(literal));
                    literal = String::new();
                }
                parser.pos += 1;
                let expr = parser.parse_expr()?;
                let mut width = None;
                let mut align = Align::Left;
                parser.skip_ws();
                if parser.eat(",") {
                    width = Some(Box::new(parser.parse_expr()?));
                    parser.skip_ws();
                    if parser.eat(",") {
                        align = match parser.parse_ident()?.to_uppercase().as_str() {
                            "CENTER" => Align::Center,
                            "RIGHT" => Align::Right,
                            _ => Align::Left,
                        };
                    }
                }
                if !parser.eat("}") {
                    return Err("expected } to close interpolation".into());
                }
                parts.push(FormPart::Int(expr, width, align));
            }
            '\\' if parser.pos + 1 < parser.s.len() && parser.s[parser.pos + 1] == '@' => {
                if !literal.is_empty() {
                    parts.push(FormPart::Lit(literal));
                    literal = String::new();
                }
                parser.pos += 2; // skip \@
                let cond = parser.parse_expr()?;
                if !parser.eat("?") {
                    return Err("expected ? in conditional".into());
                }
                let then_form = parse_form_with_parser(parser)?;
                if !parser.eat("#") {
                    return Err("expected # in conditional".into());
                }
                let else_form = parse_form_with_parser(parser)?;
                if !parser.eat("\\") || !parser.eat("@") {
                    return Err("expected \\@ to end conditional".into());
                }
                parts.push(FormPart::Cond(Box::new((cond, then_form, else_form))));
            }
            c => {
                literal.push(c);
                parser.pos += 1;
            }
        }
    }

    if !literal.is_empty() {
        parts.push(FormPart::Lit(literal));
    }

    Ok(Form {
        parts,
        span: Span::empty(),
    })
}

/// Parse a FORM string from source
pub fn parse_form(src: &str, defs: &HashMap<String, String>) -> PResult<Form> {
    let mut parser = ExprParser::new(src, defs);
    let form = parse_form_with_parser(&mut parser)?;
    Ok(form)
}

/// Can `c` appear in an identifier? Anything that isn't whitespace or an
/// operator/punctuation character — so Japanese names like `好感度` work.
pub fn is_ident_char(c: char) -> bool {
    if c.is_ascii() {
        const fn table() -> [bool; 128] {
            let mut t = [true; 128];
            let mut i = 0;
            while i < 128 {
                if (i as u8).is_ascii_whitespace() || i == 0x0b {
                    t[i] = false;
                }
                i += 1;
            }
            let ops = b"+-*/%=!<>&|^~?#:,()[]{}\"'@\\$;.";
            let mut k = 0;
            while k < ops.len() {
                t[ops[k] as usize] = false;
                k += 1;
            }
            t
        }
        static T: [bool; 128] = table();
        return T[c as usize];
    }
    !(c.is_whitespace() || c == '\u{3000}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fxhash::HashMap;

    #[test]
    fn test_parse_number() {
        let defs = HashMap::default();
        // Note: Expr doesn't implement PartialEq due to Rc/Box/Atomic fields
        // Just test that parsing succeeds
        assert!(parse_expr("123", &defs).is_ok());
        assert!(parse_expr("0xFF", &defs).is_ok());
        assert!(parse_expr("0b1010", &defs).is_ok());
    }

    #[test]
    fn test_parse_string() {
        let defs = HashMap::default();
        assert!(parse_expr("\"hello\"", &defs).is_ok());
    }

    #[test]
    fn test_parse_binary() {
        let defs = HashMap::default();
        let expr = parse_expr("1 + 2 * 3", &defs).unwrap();
        match expr {
            Expr::Binary(BinOp::Add, a, b) => {
                // Can't assert_eq on Expr due to no PartialEq impl
                match (a.as_ref(), b.as_ref()) {
                    (Expr::Int(1), Expr::Binary(BinOp::Mul, c, d)) => {
                        match (c.as_ref(), d.as_ref()) {
                            (Expr::Int(2), Expr::Int(3)) => {}
                            _ => panic!("expected 2 * 3"),
                        }
                    }
                    _ => panic!("expected 1 + (2 * 3)"),
                }
            }
            _ => panic!("expected add"),
        }
    }

    #[test]
    fn test_parse_var_ref() {
        let defs = HashMap::default();
        let expr = parse_expr("VAR:1:2", &defs).unwrap();
        match expr {
            Expr::Var(v) => {
                assert_eq!(v.name.as_ref(), "VAR");
                assert_eq!(v.idx.len(), 2);
            }
            _ => panic!("expected var"),
        }
    }
}
