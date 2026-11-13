//! Eager parser for LSP: parses ERB/ERH files into fully-owned AST.
//!
//! This parser compiles everything immediately (no lazy compilation) and
//! produces an AST that owns all its text data.

use crate::fxhash::{HashMap, HashSet};
use crate::lsp_ast::*;
use crate::parser::expr::{parse_expr, parse_expr_list, parse_form, split_top};
use crate::parser::loader;
use std::sync::Arc as Rc;

/// Parser state for LSP (eager, owns text).
pub struct LspParser {
    source: SourceFile,
    defines: HashMap<String, String>,
    global_dims: Vec<DimDecl>,
    global_str: HashSet<String>,
    lines: Vec<String>,
    line_offsets: Vec<usize>, // Byte offset of each line start in source text
    line_no: usize,
    errors: Vec<String>,
    functions: HashMap<String, Vec<Function>>,
    current_function: Option<FunctionBuilder>,
}

#[derive(Debug)]
struct FuncAttrs {
    is_function: bool,
    returns_str: bool,
    order: EventOrder,
    single: bool,
    only: bool,
    kojo: bool,
    local_size: u32,
    locals_size: u32,
}

struct FunctionBuilder {
    name: String,
    params: Vec<Param>,
    is_function: bool,
    returns_str: bool,
    order: EventOrder,
    single: bool,
    only: bool,
    kojo: bool,
    local_size: u32,
    locals_size: u32,
    file: u32,
    line: u32,
    span: Span,
    code: Vec<Instr>,
    lines: Vec<Span>, // Byte spans for each instruction
    labels: HashMap<String, usize>,
    dims: Vec<DimDecl>,
    n_slots: usize,
    slot_counter: usize,
    local_str: HashSet<String>,
    ctl: Vec<Ctl>,
    sif_target: Option<usize>,
    body_line_start: usize, // Original line index where body starts
}

#[derive(Debug)]
enum Ctl {
    If {
        else_target: Option<usize>,
        endif_target: usize,
    },
    For {
        var: Rc<VarRef>,
        slot: usize,
        exit: usize,
        body: usize,
    },
    While {
        cond: Expr,
        slot: usize,
        exit: usize,
        body: usize,
    },
    Repeat {
        slot: usize,
        exit: usize,
        body: usize,
    },
    Do {
        slot: usize,
        exit: usize,
        body: usize,
    },
    Select {
        slot: usize,
        default: usize,
        cases: Vec<usize>,
    },
}

impl LspParser {
    /// Create a new LSP parser for a source file.
    pub fn new(source: SourceFile) -> Self {
        let text = &source.text;
        let mut lines = Vec::new();
        let mut line_offsets = Vec::new();
        let mut offset = 0;

        for line in loader::emuera_lines(text) {
            line_offsets.push(offset);
            offset += line.len();
            // Account for line ending
            let remaining = &text[offset..];
            let line_ending_len = if remaining.starts_with("\r\n") {
                2
            } else if remaining.starts_with('\n') || remaining.starts_with('\r') {
                1
            } else {
                0
            };
            offset += line_ending_len;
            lines.push(line.to_string());
        }

        Self {
            source,
            defines: HashMap::default(),
            global_dims: Vec::new(),
            global_str: HashSet::default(),
            lines,
            line_offsets,
            line_no: 0,
            errors: Vec::new(),
            functions: HashMap::default(),
            current_function: None,
        }
    }

    /// Get the byte span for the current line in the original source.
    fn current_line_span(&self) -> Span {
        if self.line_no < self.line_offsets.len() {
            let start = self.line_offsets[self.line_no];
            let end = if self.line_no + 1 < self.line_offsets.len() {
                self.line_offsets[self.line_no + 1]
            } else {
                self.source.text.len()
            };
            Span::new(start, end)
        } else {
            Span::empty()
        }
    }

    /// Get the byte span for a specific line in the function body (0-indexed within body).
    fn body_line_span(&self, body: &FunctionBuilder, body_line_idx: usize) -> Span {
        let original_line = body.body_line_start + body_line_idx;
        if original_line < self.line_offsets.len() {
            let start = self.line_offsets[original_line];
            let end = if original_line + 1 < self.line_offsets.len() {
                self.line_offsets[original_line + 1]
            } else {
                self.source.text.len()
            };
            Span::new(start, end)
        } else {
            Span::empty()
        }
    }

    /// Parse the entire file and return the program.
    pub fn parse(mut self) -> Program {
        // First pass: parse ERH-style declarations (#DEFINE, #DIM)
        self.parse_erh_declarations();

        // Second pass: parse functions
        self.parse_functions();

        Program {
            files: vec![self.source.path.clone()],
            functions: self.functions,
            global_dims: self.global_dims,
            defines: self.defines,
            errors: self.errors,
            warnings: Vec::new(),
        }
    }

    /// Parse #DEFINE and #DIM declarations (ERH style).
    fn parse_erh_declarations(&mut self) {
        let lines: Vec<String> = self.lines.clone();
        for (i, line) in lines.iter().enumerate() {
            self.line_no = i;
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if line.starts_with("#DEFINE") {
                self.parse_define(line);
            } else if line.starts_with("#DIM") || line.starts_with("#DIMS") {
                if let Some(dim) = self.parse_dim(line, true) {
                    self.global_dims.push(dim);
                }
            }
        }
    }

    /// Parse function definitions.
    fn parse_functions(&mut self) {
        let mut i = 0;
        while i < self.lines.len() {
            self.line_no = i;
            let line = self.lines[i].trim();

            if line.is_empty() || line.starts_with(';') {
                i += 1;
                continue;
            }

            if line.starts_with('@') {
                // Find function body end
                let body_start = i + 1;
                let mut body_end = body_start;
                let mut open_ifs = 0u16;

                while body_end < self.lines.len() {
                    let l = self.lines[body_end].trim();
                    if l.starts_with('@') {
                        break;
                    }
                    if l.starts_with("IF") && !l.starts_with("SIF") {
                        open_ifs += 1;
                    } else if l == "ENDIF" {
                        if open_ifs > 0 {
                            open_ifs -= 1;
                        }
                    }
                    body_end += 1;
                }

                // Parse function header
                if let Some((name, params, attrs)) = self.parse_func_header(line) {
                    let body_lines = &self.lines[body_start..body_end];
                    let body_text = body_lines.join("\n");

                    let mut func_builder = FunctionBuilder {
                        name: name.clone(),
                        params,
                        is_function: attrs.is_function,
                        returns_str: attrs.returns_str,
                        order: attrs.order,
                        single: attrs.single,
                        only: attrs.only,
                        kojo: attrs.kojo,
                        local_size: attrs.local_size,
                        locals_size: attrs.locals_size,
                        file: 0,
                        line: self.line_no as u32,
                        span: Span::empty(),
                        code: Vec::new(),
                        lines: Vec::new(),
                        labels: HashMap::default(),
                        dims: Vec::new(),
                        n_slots: 0,
                        slot_counter: 0,
                        local_str: HashSet::default(),
                        ctl: Vec::new(),
                        sif_target: None,
                        body_line_start: body_start,
                    };

                    // Set current function before parsing body
                    self.current_function = Some(func_builder);

                    // Parse function body
                    let saved_lines = self.lines.clone();
                    let body_lines_owned = body_lines.to_vec();
                    self.lines = body_lines_owned;
                    self.line_no = 0;

                    // We need to track the body line index separately
                    let mut body_line_idx = 0;
                    for body_line in self.lines.clone().iter() {
                        self.line_no = body_line_idx;
                        let trimmed = body_line.trim();
                        if trimmed.is_empty() || trimmed.starts_with(';') {
                            body_line_idx += 1;
                            continue;
                        }
                        self.parse_statement(trimmed, body_line_idx);
                        body_line_idx += 1;
                    }

                    // Restore lines and line_no for outer loop
                    self.lines = saved_lines.clone();
                    self.line_no = i;

                    // Finalize function
                    if let Some(mut builder) = self.current_function.take() {
                        // Patch remaining control structures
                        self.patch_remaining_controls(&mut builder);

                        let func = Function {
                            name: builder.name,
                            params: builder.params,
                            body: Body {
                                code: builder.code,
                                lines: builder.lines,
                                labels: builder.labels,
                                n_slots: builder.n_slots,
                                dims: builder.dims,
                            },
                            is_function: builder.is_function,
                            returns_str: builder.returns_str,
                            order: builder.order,
                            single: builder.single,
                            only: builder.only,
                            kojo: builder.kojo,
                            local_size: builder.local_size,
                            locals_size: builder.locals_size,
                            file: builder.file,
                            line: builder.line,
                            span: builder.span,
                        };

                        self.functions
                            .entry(func.name.to_uppercase())
                            .or_default()
                            .push(func);
                    }

                    self.lines = saved_lines;
                }
                i = body_end;
            } else {
                i += 1;
            }
        }
    }

    fn parse_define(&mut self, line: &str) {
        let rest = &line["#DEFINE".len()..].trim();
        let mut parts = rest.splitn(2, ' ');
        let name = parts.next().unwrap_or("").to_uppercase();
        let body = parts.next().unwrap_or("").to_string();
        if !name.is_empty() {
            self.defines.insert(name, body);
        }
    }

    fn parse_dim(&mut self, line: &str, is_global: bool) -> Option<DimDecl> {
        let is_str = line.starts_with("#DIMS");
        let prefix_len = if is_str { 5 } else { 4 };
        let rest = &line[prefix_len..].trim();

        let mut parts = split_top(rest);
        if parts.is_empty() {
            return None;
        }

        let name = parts[0].to_uppercase();
        let mut dims = Vec::new();
        let mut init = Vec::new();
        let mut is_const = false;
        let mut is_static = true;
        let mut is_savedata = false;
        let mut is_chara = false;
        let mut is_ref = false;
        let mut is_dynamic = false;
        let decl_name = name.clone();

        for part in &parts[1..] {
            let upper = part.to_uppercase();
            match upper.as_ref() {
                "CONST" => is_const = true,
                "STATIC" | "GLOBAL" => is_static = true,
                "SAVEDATA" => is_savedata = true,
                "CHARADATA" => is_chara = true,
                "REF" => is_ref = true,
                "DYNAMIC" => is_dynamic = true,
                _ => {
                    if let Ok(expr) = parse_expr(part, &self.defines) {
                        let converted = convert_expr(expr);
                        if init.is_empty() && dims.is_empty() {
                            dims.push(converted);
                        } else {
                            init.push(converted);
                        }
                    }
                }
            }
        }

        Some(DimDecl {
            name,
            decl_name,
            is_str,
            dims,
            init,
            is_const,
            is_static,
            is_global,
            is_savedata,
            is_chara,
            is_ref,
            is_dynamic,
            span: Span::empty(),
        })
    }

    fn parse_func_header(&self, line: &str) -> Option<(String, Vec<Param>, FuncAttrs)> {
        let rest = &line[1..].trim();
        let mut parts = split_top(rest);
        if parts.is_empty() {
            return None;
        }

        let name = parts[0].to_uppercase();
        let mut params = Vec::new();
        let mut is_function = false;
        let mut returns_str = false;
        let mut order = EventOrder::Normal;
        let mut single = false;
        let mut only = false;
        let mut kojo = false;
        let mut local_size = 0u32;
        let mut locals_size = 0u32;

        for part in &parts[1..] {
            let upper = part.to_uppercase();
            if upper.starts_with('#') {
                match upper.as_ref() {
                    "#FUNCTION" => is_function = true,
                    "#FUNCTIONS" => {
                        is_function = true;
                        returns_str = true;
                    }
                    "#PRI" => order = EventOrder::Pri,
                    "#LATER" => order = EventOrder::Later,
                    "#SINGLE" => single = true,
                    "#ONLY" => only = true,
                    "#KOJO" => kojo = true,
                    _ => {
                        if upper.starts_with("#LOCALSIZE") {
                            local_size = upper["#LOCALSIZE".len()..].trim().parse().unwrap_or(0);
                        } else if upper.starts_with("#LOCALSSIZE") {
                            locals_size = upper["#LOCALSSIZE".len()..].trim().parse().unwrap_or(0);
                        }
                    }
                }
            } else {
                let mut p = split_top(part);
                let param_name = p[0].to_uppercase();
                let default = if p.len() > 1 {
                    parse_expr(&p[1..].join(","), &self.defines)
                        .ok()
                        .map(convert_expr)
                } else {
                    None
                };
                params.push(Param {
                    target: Rc::new(VarRef::new(param_name.into(), Vec::new())),
                    default,
                    span: Span::empty(),
                });
            }
        }

        Some((
            name,
            params,
            FuncAttrs {
                is_function,
                returns_str,
                order,
                single,
                only,
                kojo,
                local_size,
                locals_size,
            },
        ))
    }

    /// Parse a statement with span tracking for function body lines.
    fn parse_statement(&mut self, line: &str, body_line_idx: usize) {
        let span = if let Some(builder) = &self.current_function {
            self.body_line_span(builder, body_line_idx)
        } else {
            Span::empty()
        };

        let upper = line.to_uppercase();

        // Handle labels
        if upper.starts_with('$') {
            let label = upper[1..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if !label.is_empty() {
                if let Some(builder) = &mut self.current_function {
                    let idx = builder.code.len();
                    builder.labels.insert(label, idx);
                }
            }
            // Labels can be on same line as statement, continue parsing
        }

        // Skip if just a label
        let stmt = if upper.starts_with('$') {
            let parts: Vec<&str> = upper.splitn(2, ' ').collect();
            if parts.len() > 1 {
                parts[1]
            } else {
                return;
            }
        } else {
            upper.as_str()
        };

        // Dispatch to specific parsers
        if stmt.starts_with("IF") {
            self.parse_if(stmt, span);
        } else if stmt == "ELSE" {
            self.parse_else(span);
        } else if stmt == "ELSEIF" {
            self.parse_elseif(line, span);
        } else if stmt == "ENDIF" {
            self.parse_endif(span);
        } else if stmt.starts_with("FOR") {
            self.parse_for(stmt, span);
        } else if stmt == "NEXT" {
            self.parse_next(span);
        } else if stmt.starts_with("WHILE") {
            self.parse_while(stmt, span);
        } else if stmt == "WEND" {
            self.parse_wend(span);
        } else if stmt.starts_with("REPEAT") {
            self.parse_repeat(stmt, span);
        } else if stmt == "LOOP" {
            self.parse_loop(span);
        } else if stmt.starts_with("SELECTCASE") {
            self.parse_selectcase(stmt, span);
        } else if stmt.starts_with("CASE") {
            self.parse_case(stmt, span);
        } else if stmt == "CASEELSE" {
            self.parse_caseelse(span);
        } else if stmt == "ENDSELECT" {
            self.parse_endselect(span);
        } else if stmt.starts_with("CALL") {
            self.parse_call(stmt, line, span);
        } else if stmt.starts_with("JUMP") {
            self.parse_jump(stmt, line, CallKind::Jump, span);
        } else if stmt.starts_with("CALLF") {
            self.parse_jump(stmt, line, CallKind::CallF, span);
        } else if stmt.starts_with("GOTO") {
            self.parse_goto(stmt, line, span);
        } else if stmt.starts_with("RETURN") {
            self.parse_return(stmt, line, span);
        } else if stmt.starts_with("PRINT") {
            self.parse_print(stmt, line, span);
        } else if stmt.starts_with("DIM") || stmt.starts_with("DIMS") {
            if let Some(dim) = self.parse_dim(line, false) {
                if let Some(builder) = &mut self.current_function {
                    builder.dims.push(dim);
                }
            }
        } else if stmt == "BREAK" {
            self.parse_break(span);
        } else if stmt == "CONTINUE" {
            self.parse_continue(span);
        } else if stmt == "END" {
            self.add_instr(Instr::End, span);
        } else if stmt == "RESTART" {
            self.add_instr(Instr::Restart, span);
        } else if stmt == "QUIT" {
            self.add_instr(Instr::Quit, span);
        } else {
            // Try assignment: VAR = ...
            if let Some((var_ref, op, exprs)) = self.parse_assignment(line) {
                if let Some(var) = var_ref {
                    let asg_op = match op.as_str() {
                        "=" => AsgOp::Set,
                        "+=" => AsgOp::Add,
                        "-=" => AsgOp::Sub,
                        "*=" => AsgOp::Mul,
                        "/=" => AsgOp::Div,
                        "%=" => AsgOp::Mod,
                        "&=" => AsgOp::And,
                        "|=" => AsgOp::Or,
                        "^=" => AsgOp::Xor,
                        "<<=" => AsgOp::Shl,
                        ">>=" => AsgOp::Shr,
                        _ => AsgOp::Set,
                    };
                    if exprs.len() == 1 {
                        if let Expr::Form(f) = &exprs[0] {
                            self.add_instr(Instr::AssignForm(var, asg_op, f.as_ref().clone()), span);
                        } else {
                            self.add_instr(Instr::Assign(var, asg_op, exprs), span);
                        }
                    }
                }
            } else {
                // Unknown command - treat as generic command
                self.add_instr(Instr::Command(Box::new((
                    stmt.to_string(),
                    Args::Raw(line.to_string()),
                ))), span);
            }
        }
    }

    // Control flow parsing methods (simplified)
    fn parse_if(&mut self, stmt: &str, span: Span) {
        let cond_str = stmt["IF".len()..].trim();
        let cond = parse_expr(cond_str, &self.defines)
            .map(convert_expr)
            .unwrap_or(Expr::Int(0));
        let builder = self.current_function.as_mut().unwrap();
        let jump_idx = builder.code.len();
        builder.code.push(Instr::JumpIfNot(cond, 0));
        builder.lines.push(span);
        builder.ctl.push(Ctl::If {
            else_target: None,
            endif_target: jump_idx,
        });
        builder.sif_target = None;
    }

    fn parse_else(&mut self, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::If {
            else_target,
            endif_target,
        }) = builder.ctl.last_mut()
        {
            let else_idx = builder.code.len();
            builder.code.push(Instr::Jump(0));
            builder.lines.push(span);
            *else_target = Some(else_idx);
            // Patch the IF's JumpIfNot to jump here
            if let Some(Instr::JumpIfNot(_, target)) = builder.code.get_mut(*endif_target) {
                *target = else_idx + 1;
            }
        }
    }

    fn parse_elseif(&mut self, line: &str, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::If {
            else_target,
            endif_target,
        }) = builder.ctl.last_mut()
        {
            let elseif_idx = builder.code.len();
            let cond_str = line["ELSEIF".len()..].trim();
            let cond = parse_expr(cond_str, &self.defines)
                .map(convert_expr)
                .unwrap_or(Expr::Int(0));
            builder.code.push(Instr::JumpIfNot(cond, 0));
            builder.lines.push(span);
            *else_target = Some(elseif_idx);
            if let Some(Instr::JumpIfNot(_, target)) = builder.code.get_mut(*endif_target) {
                *target = elseif_idx + 1;
            }
        }
    }

    fn parse_endif(&mut self, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::If {
            else_target,
            endif_target,
        }) = builder.ctl.pop()
        {
            let endif_idx = builder.code.len();
            builder.lines.push(span);
            // Patch the last jump (from ELSE or last ELSEIF)
            if let Some(target) = else_target {
                if let Some(Instr::Jump(t)) = builder.code.get_mut(target) {
                    *t = endif_idx;
                }
            } else {
                // No ELSE - patch the IF's JumpIfNot
                if let Some(Instr::JumpIfNot(_, target)) = builder.code.get_mut(endif_target) {
                    *target = endif_idx;
                }
            }
        }
    }

    fn parse_for(&mut self, stmt: &str, span: Span) {
        let rest = stmt["FOR".len()..].trim();
        let parts: Vec<String> = split_top(rest);
        if parts.len() < 3 {
            return;
        }

        let var_name = parts[0].to_uppercase();
        let start = parse_expr(&parts[1], &self.defines)
            .map(convert_expr)
            .unwrap_or(Expr::Int(0));
        let end = parse_expr(&parts[2], &self.defines)
            .map(convert_expr)
            .unwrap_or(Expr::Int(0));
        let step = if parts.len() > 3 {
            Some(
                parse_expr(&parts[3], &self.defines)
                    .map(convert_expr)
                    .unwrap_or(Expr::Int(1)),
            )
        } else {
            None
        };

        let builder = self.current_function.as_mut().unwrap();
        let var = Rc::new(VarRef::new(var_name.into(), Vec::new()));
        let slot = builder.next_slot();
        let exit = builder.code.len() + 2; // Will be patched
        let body = builder.code.len() + 1;

        builder.code.push(Instr::ForBegin(Box::new(ForBegin {
            var: var.clone(),
            start,
            end,
            step,
            slot,
            exit,
        })));
        builder.lines.push(span);
        builder.ctl.push(Ctl::For {
            var,
            slot,
            exit: 0,
            body,
        });
    }

    fn parse_next(&mut self, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::For {
            var,
            slot,
            exit,
            body,
        }) = builder.ctl.pop()
        {
            let next_idx = builder.code.len();
            builder.code.push(Instr::ForNext { var, slot, body });
            builder.lines.push(span);
            // Patch the FOR's exit
            if let Some(Instr::ForBegin(f)) = builder.code.get_mut(body - 1) {
                f.exit = next_idx + 1;
            }
        }
    }

    fn parse_while(&mut self, stmt: &str, span: Span) {
        let cond_str = stmt["WHILE".len()..].trim();
        let cond = parse_expr(cond_str, &self.defines)
            .map(convert_expr)
            .unwrap_or(Expr::Int(0));
        let builder = self.current_function.as_mut().unwrap();
        let slot = builder.next_slot();
        let body = builder.code.len();
        builder.ctl.push(Ctl::While {
            cond: cond.clone(),
            slot,
            exit: 0,
            body,
        });
        builder.code.push(Instr::JumpIfNot(cond, 0));
        builder.lines.push(span);
    }

    fn parse_wend(&mut self, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::While {
            cond,
            slot,
            exit,
            body,
        }) = builder.ctl.pop()
        {
            let wend_idx = builder.code.len();
            builder.code.push(Instr::Jump(body));
            builder.lines.push(span);
            // Patch the WHILE's JumpIfNot
            for i in (0..builder.code.len()).rev() {
                if let Instr::JumpIfNot(_, target) = &mut builder.code[i] {
                    *target = wend_idx + 1;
                    break;
                }
            }
        }
    }

    fn parse_repeat(&mut self, _stmt: &str, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        let slot = builder.next_slot();
        let body = builder.code.len();
        builder.ctl.push(Ctl::Repeat {
            slot,
            exit: 0,
            body,
        });
    }

    fn parse_loop(&mut self, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::Repeat { slot, exit, body }) = builder.ctl.pop() {
            let loop_idx = builder.code.len();
            builder.code.push(Instr::Jump(body));
            builder.lines.push(span);
        }
    }

    fn parse_selectcase(&mut self, stmt: &str, span: Span) {
        let expr_str = stmt["SELECTCASE".len()..].trim();
        let expr = parse_expr(expr_str, &self.defines)
            .map(convert_expr)
            .unwrap_or(Expr::Int(0));
        let builder = self.current_function.as_mut().unwrap();
        let slot = builder.next_slot();
        builder.ctl.push(Ctl::Select {
            slot,
            default: 0,
            cases: Vec::new(),
        });
        builder.code.push(Instr::SelectEval(
            expr,
            slot,
            Box::new(std::sync::OnceLock::new()),
        ));
        builder.lines.push(span);
    }

    fn parse_case(&mut self, stmt: &str, span: Span) {
        let rest = stmt["CASE".len()..].trim();
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::Select { slot, cases, .. }) = builder.ctl.last_mut() {
            let case_idx = builder.code.len();
            let cond = if rest.starts_with("IS ") {
                let op_str = rest[3..].trim();
                // Simplified: just parse as expression
                let expr = parse_expr(op_str, &self.defines)
                    .map(convert_expr)
                    .unwrap_or(Expr::Int(0));
                CaseCond::Is(BinOp::Eq, expr)
            } else if rest.contains(" TO ") {
                let parts: Vec<&str> = rest.split(" TO ").collect();
                let from = parse_expr(parts[0], &self.defines)
                    .map(convert_expr)
                    .unwrap_or(Expr::Int(0));
                let to = parse_expr(parts[1], &self.defines)
                    .map(convert_expr)
                    .unwrap_or(Expr::Int(0));
                CaseCond::Range(from, to)
            } else {
                let expr = parse_expr(rest, &self.defines)
                    .map(convert_expr)
                    .unwrap_or(Expr::Int(0));
                CaseCond::Eq(expr)
            };
            builder.code.push(Instr::CaseTest(*slot, vec![cond], 0));
            builder.lines.push(span);
            cases.push(case_idx);
        }
    }

    fn parse_caseelse(&mut self, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::Select { default, cases, .. }) = builder.ctl.last_mut() {
            *default = builder.code.len();
            builder.lines.push(span);
            // Patch previous CASE's jump target
            if let Some(last_case) = cases.last() {
                if let Some(Instr::CaseTest(_, _, target)) = builder.code.get_mut(*last_case) {
                    *target = *default;
                }
            }
        }
    }

    fn parse_endselect(&mut self, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        if let Some(Ctl::Select { default, cases, .. }) = builder.ctl.pop() {
            let end_idx = builder.code.len();
            builder.lines.push(span);
            // Patch all CASE jumps
            for case_idx in cases {
                if let Some(Instr::CaseTest(_, _, target)) = builder.code.get_mut(case_idx) {
                    *target = end_idx;
                }
            }
        }
    }

    fn parse_call(&mut self, stmt: &str, original: &str, span: Span) {
        let kind = if stmt.starts_with("TRYCALL") {
            CallKind::Call
        } else {
            CallKind::Call
        };
        let try_ = stmt.starts_with("TRY");
        let rest = if try_ {
            &stmt["TRYCALL".len()..].trim()
        } else {
            &stmt["CALL".len()..].trim()
        };
        let (target, args) = self.parse_call_args(rest);
        let catch = if try_ {
            Some(self.current_function.as_ref().unwrap().code.len() + 2)
        } else {
            None
        };
        self.add_instr(Instr::Call(Box::new(CallInstr {
            target,
            args,
            kind,
            try_,
            catch,
        })), span);
    }

    fn parse_jump(&mut self, stmt: &str, original: &str, kind: CallKind, span: Span) {
        let try_ = stmt.starts_with("TRY");
        let rest = if try_ {
            &stmt["TRYJUMP".len()..].trim()
        } else {
            &stmt["JUMP".len()..].trim()
        };
        let (target, args) = self.parse_call_args(rest);
        let catch = if try_ {
            Some(self.current_function.as_ref().unwrap().code.len() + 2)
        } else {
            None
        };
        self.add_instr(Instr::Call(Box::new(CallInstr {
            target,
            args,
            kind,
            try_,
            catch,
        })), span);
    }

    fn parse_goto(&mut self, stmt: &str, original: &str, span: Span) {
        let try_ = stmt.starts_with("TRY");
        let rest = if try_ {
            &stmt["TRYGOTO".len()..].trim()
        } else {
            &stmt["GOTO".len()..].trim()
        };
        let target = if rest.starts_with('@') || rest.starts_with('$') {
            CallTarget::Static(rest.to_uppercase())
        } else {
            CallTarget::Static(rest.to_uppercase())
        };
        let catch = if try_ {
            Some(self.current_function.as_ref().unwrap().code.len() + 2)
        } else {
            None
        };
        self.add_instr(Instr::Goto(Box::new((target, try_, catch))), span);
    }

    fn parse_return(&mut self, stmt: &str, original: &str, span: Span) {
        let rest = stmt["RETURN".len()..].trim();
        let exprs = if rest.is_empty() {
            Vec::new()
        } else {
            parse_expr_list(rest, &self.defines)
                .unwrap_or_default()
                .into_iter()
                .map(convert_expr)
                .collect()
        };
        self.add_instr(Instr::Return(exprs), span);
    }

    fn parse_print(&mut self, stmt: &str, original: &str, span: Span) {
        // Simplified print parsing
        let rest = original[stmt.len()..].trim();
        let content = if stmt.contains("FORM") {
            PrintContent::Form(
                parse_form(rest, &self.defines)
                    .map(|f| Form {
                        parts: f.parts.into_iter().map(convert_form_part).collect(),
                        span: Span::empty(),
                    })
                    .unwrap_or_default(),
            )
        } else if stmt == "PRINTS" {
            PrintContent::Expr(
                parse_expr(rest, &self.defines)
                    .map(convert_expr)
                    .unwrap_or(Expr::Str("".into())),
            )
        } else if stmt == "PRINTV" {
            PrintContent::Exprs(
                parse_expr_list(rest, &self.defines)
                    .unwrap_or_default()
                    .into_iter()
                    .map(convert_expr)
                    .collect(),
            )
        } else {
            PrintContent::Raw(rest.to_string())
        };
        let flags = PrintFlags::default();
        self.add_instr(Instr::Print(content, flags), span);
    }

    fn parse_break(&mut self, span: Span) {
        let builder = self.current_function.as_mut().unwrap();
        for ctl in builder.ctl.iter().rev() {
            if let Ctl::For {
                var, slot, exit, ..
            } = ctl
            {
                let break_idx = builder.code.len();
                builder.code.push(Instr::ForBreak {
                    var: var.clone(),
                    slot: *slot,
                    exit: *exit,
                });
                builder.lines.push(span);
                break;
            }
        }
    }

    fn parse_continue(&mut self, span: Span) {
        // Similar to BREAK but continues loop
        let builder = self.current_function.as_mut().unwrap();
        for ctl in builder.ctl.iter().rev() {
            if let Ctl::For {
                var,
                slot,
                exit,
                body,
            } = ctl
            {
                let cont_idx = builder.code.len();
                builder.code.push(Instr::ForNext {
                    var: var.clone(),
                    slot: *slot,
                    body: *body,
                });
                builder.lines.push(span);
                break;
            }
        }
    }

    fn parse_assignment(&self, line: &str) -> Option<(Option<Rc<VarRef>>, String, Vec<Expr>)> {
        // Find assignment operator
        let ops = [
            "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>=", "=",
        ];
        for op in &ops {
            if let Some(pos) = line.find(op) {
                let left = line[..pos].trim();
                let right = line[pos + op.len()..].trim();
                if let Ok(var) = parse_expr(left, &self.defines).map(convert_expr) {
                    if let Expr::Var(v) = var {
                        let exprs = parse_expr_list(right, &self.defines)
                            .unwrap_or_default()
                            .into_iter()
                            .map(convert_expr)
                            .collect();
                        return Some((Some(v), op.to_string(), exprs));
                    }
                }
            }
        }
        None
    }

    fn parse_call_args(&self, rest: &str) -> (CallTarget, Vec<Expr>) {
        // Simplified: assume static target
        let target = CallTarget::Static(rest.split('(').next().unwrap_or("").to_uppercase());
        let args = if let Some(start) = rest.find('(') {
            let end = rest.rfind(')').unwrap_or(rest.len());
            let args_str = &rest[start + 1..end];
            parse_expr_list(args_str, &self.defines)
                .unwrap_or_default()
                .into_iter()
                .map(convert_expr)
                .collect()
        } else {
            Vec::new()
        };
        (target, args)
    }

    fn patch_remaining_controls(&self, builder: &mut FunctionBuilder) {
        // Patch any unclosed control structures
        for ctl in &builder.ctl {
            match ctl {
                Ctl::If {
                    else_target,
                    endif_target,
                } => {
                    let end = builder.code.len();
                    if let Some(target) = else_target {
                        if let Some(Instr::Jump(t)) = builder.code.get_mut(*target) {
                            *t = end;
                        }
                    } else {
                        if let Some(Instr::JumpIfNot(_, t)) = builder.code.get_mut(*endif_target) {
                            *t = end;
                        }
                    }
                }
                Ctl::For { exit, body, .. } => {
                    let end = builder.code.len();
                    if let Some(Instr::ForBegin(f)) = builder.code.get_mut(*body - 1) {
                        f.exit = end;
                    }
                }
                Ctl::While { .. } => {
                    // Patch jumps to end
                }
                Ctl::Repeat { .. } => {
                    // Patch jumps to end
                }
                Ctl::Select { default, .. } => {
                    // Patch jumps to end
                    let _ = default;
                }
                _ => {}
            }
        }
    }

    fn add_instr(&mut self, instr: Instr, span: Span) {
        if let Some(builder) = &mut self.current_function {
            builder.code.push(instr);
            builder.lines.push(span);
        }
    }
}

impl FunctionBuilder {
    fn next_slot(&mut self) -> usize {
        let s = self.slot_counter;
        self.slot_counter += 1;
        if s >= self.n_slots {
            self.n_slots = s + 1;
        }
        s
    }
}

/// Convert ast::Expr to lsp_ast::Expr
fn convert_expr(expr: crate::parser::ast::Expr) -> Expr {
    use crate::parser::ast::Expr as AstExpr;
    match expr {
        AstExpr::Int(n) => Expr::Int(n),
        AstExpr::Str(s) => Expr::Str(s),
        AstExpr::Form(f) => Expr::Form(Rc::new(Form {
            parts: f.parts.clone().into_iter().map(convert_form_part).collect(),
            span: Span::empty(),
        })),
        AstExpr::Var(v) => Expr::Var(Rc::new(VarRef {
            name: v.name.clone().into(),
            idx: v.idx.clone().into_iter().map(convert_expr).collect(),
            span: Span::empty(),
        })),
        AstExpr::Call(c) => Expr::Call(Box::new((
            c.0.clone(),
            c.1.clone().into_iter().map(convert_expr).collect(),
            Resolved::new(),
        ))),
        AstExpr::Unary(op, e) => Expr::Unary(convert_unop(op), Box::new(convert_expr(*e))),
        AstExpr::Binary(op, a, b) => Expr::Binary(
            convert_binop(op),
            Box::new(convert_expr(*a)),
            Box::new(convert_expr(*b)),
        ),
        AstExpr::Ternary(c, a, b) => Expr::Ternary(
            Box::new(convert_expr(*c)),
            Box::new(convert_expr(*a)),
            Box::new(convert_expr(*b)),
        ),
        AstExpr::IncDec(v, is_inc, step) => Expr::IncDec(
            Rc::new(VarRef {
                name: v.name.clone().into(),
                idx: v.idx.clone().into_iter().map(convert_expr).collect(),
                span: Span::empty(),
            }),
            is_inc,
            step,
        ),
        AstExpr::Empty => Expr::Empty,
        AstExpr::Span(s) => Expr::Span(convert_span(s)),
    }
}

fn convert_form_part(part: crate::parser::ast::FormPart) -> FormPart {
    use crate::parser::ast::FormPart as AstFormPart;
    match part {
        AstFormPart::Lit(s) => FormPart::Lit(s),
        AstFormPart::Str(e, w, a) => FormPart::Str(
            convert_expr(e),
            w.map(|x| Box::new(convert_expr(*x))),
            convert_align(a),
        ),
        AstFormPart::Int(e, w, a) => FormPart::Int(
            convert_expr(e),
            w.map(|x| Box::new(convert_expr(*x))),
            convert_align(a),
        ),
        AstFormPart::Cond(c) => FormPart::Cond(Box::new((
            convert_expr(c.0),
            Form {
                parts: c
                    .1
                    .parts
                    .clone()
                    .into_iter()
                    .map(convert_form_part)
                    .collect(),
                span: Span::empty(),
            },
            Form {
                parts: c
                    .2
                    .parts
                    .clone()
                    .into_iter()
                    .map(convert_form_part)
                    .collect(),
                span: Span::empty(),
            },
        ))),
    }
}

fn convert_unop(op: crate::parser::ast::UnOp) -> UnOp {
    match op {
        crate::parser::ast::UnOp::Neg => UnOp::Neg,
        crate::parser::ast::UnOp::Not => UnOp::Not,
        crate::parser::ast::UnOp::BitNot => UnOp::BitNot,
    }
}

fn convert_binop(op: crate::parser::ast::BinOp) -> BinOp {
    match op {
        crate::parser::ast::BinOp::Mul => BinOp::Mul,
        crate::parser::ast::BinOp::Div => BinOp::Div,
        crate::parser::ast::BinOp::Mod => BinOp::Mod,
        crate::parser::ast::BinOp::Add => BinOp::Add,
        crate::parser::ast::BinOp::Sub => BinOp::Sub,
        crate::parser::ast::BinOp::Shl => BinOp::Shl,
        crate::parser::ast::BinOp::Shr => BinOp::Shr,
        crate::parser::ast::BinOp::Lt => BinOp::Lt,
        crate::parser::ast::BinOp::Le => BinOp::Le,
        crate::parser::ast::BinOp::Gt => BinOp::Gt,
        crate::parser::ast::BinOp::Ge => BinOp::Ge,
        crate::parser::ast::BinOp::Eq => BinOp::Eq,
        crate::parser::ast::BinOp::Ne => BinOp::Ne,
        crate::parser::ast::BinOp::BitAnd => BinOp::BitAnd,
        crate::parser::ast::BinOp::BitXor => BinOp::BitXor,
        crate::parser::ast::BinOp::BitOr => BinOp::BitOr,
        crate::parser::ast::BinOp::And => BinOp::And,
        crate::parser::ast::BinOp::Or => BinOp::Or,
        crate::parser::ast::BinOp::Nand => BinOp::Nand,
        crate::parser::ast::BinOp::Nor => BinOp::Nor,
        crate::parser::ast::BinOp::Xor => BinOp::Xor,
    }
}

fn convert_align(a: crate::parser::ast::Align) -> Align {
    match a {
        crate::parser::ast::Align::Left => Align::Left,
        crate::parser::ast::Align::Center => Align::Center,
        crate::parser::ast::Align::Right => Align::Right,
    }
}

fn convert_span(s: crate::parser::ast::Span) -> Span {
    Span::new(s.start, s.end)
}

/// Parse a file into a Program using the eager LSP parser.
pub fn parse_file(source: SourceFile) -> Program {
    let parser = LspParser::new(source);
    parser.parse()
}
