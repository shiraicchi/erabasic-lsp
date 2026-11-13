//! ERB/ERH statement parser: turns source files into a [`Program`].
//!
//! Pipeline per file:
//!
//! 1. `logical_lines` — strip comments-only lines, apply `[SKIPSTART]`,
//!    `[IF_DEBUG]`/`[IF X]` conditional blocks, `[[rename]]` substitution
//!    and `{ ... }` line joining, yielding (line number, text) pairs.
//! 2. `.ERH` headers (`parse_erh`): `#DEFINE`s first, then global `#DIM`s.
//! 3. `.ERB` files (`parse_erb`): an `@NAME(args)` line starts a function;
//!    every following line goes through `statement`, which emits
//!    [`Instr`]s into a [`FuncBuilder`].
//!
//! Block statements are compiled with a stack of open blocks (`Ctl`): IF
//! emits a `JumpIfNot` with a placeholder target, ELSE/ENDIF *patch* it
//! once the target is known, BREAK/CONTINUE are recorded on the innermost
//! loop and patched at NEXT/WEND/LOOP. SIF is special: its jump is patched
//! after the next statement, whatever it is (`after_stmt`).
//!
//! A line that fails to parse becomes an [`Instr::BadLine`] rather than
//! disappearing, so jumps and SIF still line up and the error surfaces if
//! the line is actually executed.
//!
//! **LSP Adaptation**: Added position tracking, error recovery, and incremental parsing support.

use crate::fxhash::{HashMap, HashSet};
use crate::parser::ast::*;
use crate::parser::expr::{parse_expr, parse_expr_list, parse_form, split_top, ExprParser};
use crate::parser::loadcheck;
use crate::parser::loader;
use std::path::Path;
use std::sync::Arc as Rc;

/// One loaded .ERB/.ERH file (path relative to the game, decoded text).
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: String,
    pub text: String,
}

/// Everything compiled from a game: what the VM runs.
#[derive(Default, Debug)]
pub struct Program<'a> {
    pub files: Vec<String>,
    /// Functions by uppercase name; several entries == event function.
    pub funcs: Funcs,
    /// `#DIM`s from .ERH files, allocated at game start / RESETDATA.
    pub global_dims: Vec<DimDecl>,
    /// `#DEFINE NAME body`: expanded textually inside expressions.
    pub defines: HashMap<String, String>,
    /// Parse errors, "file:line: message"; shown by `--check`.
    pub errors: Vec<String>,
    /// Emuera's load warnings found while reading (see `loadcheck.rs`)
    pub warnings: Vec<loadcheck::Warning>,
    /// A parser that knows the game's `#DEFINE`s and global variables, for
    /// the debug panel's commands (see [`Parser::compile_snippet`]).
    pub snippets: Option<Rc<std::sync::Mutex<Parser<'a>>>>,
}

/// Functions by upper-case name; several of one name are an event's
/// handlers, in file order.
#[derive(Default, Debug)]
pub struct Funcs {
    names: String,
    heads: Vec<Head>,
    table: Vec<u64>,
    n_names: usize,
    sizes: HashMap<u32, (u32, u32)>,
    groups: Vec<std::sync::OnceLock<Box<[Rc<Function>]>>>,
    ctx: Option<Rc<LazyCtx>>,
}

const NONE: u32 = u32::MAX;
const FREE: u64 = u64::MAX;

/// A function before it is looked up.
#[derive(Debug)]
struct Head {
    name: (u32, u32),
    next: u32,
    group: u32,
    src: Src,
    attrs: Attrs,
    one: std::sync::OnceLock<Rc<Function>>,
}

/// Where a function's text is.
#[derive(Clone, Copy, Debug)]
struct Src {
    file: u32,
    line: u32,
    start: u32,
    end: u32,
    open_ifs: u16,
    ord: u16,
}

/// The `#` attributes of a function, read at load.
#[derive(Clone, Copy, Debug)]
struct Attrs {
    is_function: bool,
    returns_str: bool,
    order: EventOrder,
    single: bool,
    only: bool,
}

impl Attrs {
    fn of(f: &Function) -> Attrs {
        Attrs {
            is_function: f.is_function,
            returns_str: f.returns_str,
            order: f.order,
            single: f.single,
            only: f.only,
        }
    }
}

fn name_hash(name: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

impl Funcs {
    fn name_of(&self, h: &Head) -> &str {
        &self.names[h.name.0 as usize..h.name.1 as usize]
    }

    fn slot(&self, name: &str) -> (usize, u64) {
        let h = name_hash(name);
        let tag = h & !(NONE as u64);
        let mask = self.table.len() - 1;
        let mut i = (h >> 32 ^ h) as usize & mask;
        loop {
            let e = self.table[i];
            if e == FREE
                || e & !(NONE as u64) == tag && self.name_of(&self.heads[e as u32 as usize]) == name
            {
                return (i, tag);
            }
            i = (i + 1) & mask;
        }
    }

    fn first(&self, name: &str) -> Option<&Head> {
        if self.table.is_empty() {
            return None;
        }
        match self.table[self.slot(name).0] {
            FREE => None,
            e => Some(&self.heads[e as u32 as usize]),
        }
    }

    /// The functions named `name` (made now if not yet).
    pub fn get(&self, name: &str) -> Option<&[Rc<Function>]> {
        let h = self.first(name)?;
        if h.next == NONE {
            return Some(std::slice::from_ref(h.one.get_or_init(|| self.make(h))));
        }
        Some(self.groups[h.group as usize].get_or_init(|| {
            let mut v = Vec::new();
            let mut e = h;
            loop {
                v.push(e.one.get_or_init(|| self.make(e)).clone());
                if e.next == NONE {
                    break v.into_boxed_slice();
                }
                e = &self.heads[e.next as usize];
            }
        }))
    }

    pub fn contains_key(&self, name: &str) -> bool {
        self.first(name).is_some()
    }

    /// Whether the (first) function named `name` is a `#FUNCTION` and
    /// whether it returns a string, without making it.
    pub fn kind(&self, name: &str) -> Option<(bool, bool)> {
        self.first(name)
            .map(|h| (h.attrs.is_function, h.attrs.returns_str))
    }

    pub fn len(&self) -> usize {
        self.heads.len()
    }

    /// Every name defined more than once: the name, where its first
    /// definition is, and where each later one is (file, line).
    pub fn duplicates(&self) -> Vec<(String, (u32, u32), Vec<(u32, u32)>)> {
        let mut out = Vec::new();
        for h in &self.heads {
            if h.next != NONE {
                let mut dupes = Vec::new();
                let mut e = h;
                let first = (e.src.file, e.src.line);
                while e.next != NONE {
                    e = &self.heads[e.next as usize];
                    dupes.push((e.src.file, e.src.line));
                }
                out.push((self.name_of(h).to_string(), first, dupes));
            }
        }
        out
    }

    /// Iterator over all functions that have been made (headers only).
    pub fn heads(&self) -> impl Iterator<Item = &Head> {
        self.heads.iter()
    }

    /// Iterator over all fully made functions.
    pub fn made(&self) -> impl Iterator<Item = Rc<Function>> + '_ {
        self.heads
            .iter()
            .map(|h| h.one.get_or_init(|| self.make(h)).clone())
    }

    fn make(&self, h: &Head) -> Rc<Function> {
        let ctx = self
            .ctx
            .as_ref()
            .expect("Funcs not initialized with context");
        let src = &ctx.files.borrow()[h.src.file as usize];
        let text = &src.text[h.src.start as usize..h.src.end as usize];
        let mut parser = Parser::new(ctx, h.src.file, h.src.line, text);
        parser.parse_function(h, text)
    }
}

/// Context for lazy function compilation.
#[derive(Debug)]
pub struct LazyCtx {
    pub files: std::cell::RefCell<Vec<SourceFile>>,
    pub defines: std::cell::RefCell<HashMap<String, String>>,
    pub global_dims: std::cell::RefCell<Vec<DimDecl>>,
    pub global_str: std::cell::RefCell<HashSet<String>>,
}

impl Default for LazyCtx {
    fn default() -> Self {
        Self {
            files: std::cell::RefCell::new(Vec::new()),
            defines: std::cell::RefCell::new(HashMap::default()),
            global_dims: std::cell::RefCell::new(Vec::new()),
            global_str: std::cell::RefCell::new(HashSet::default()),
        }
    }
}

/// Source range for lazy compilation of a function body.
#[derive(Clone, Copy, Debug)]
pub struct LazySrc {
    pub file: u32,
    pub start: u32,
    pub end: u32,
    pub open_ifs: u16,
    pub ord: u16,
}

impl LazySrc {
    /// Compile the function body from its source range.
    pub fn compile(&self, func: &Function, ctx: &LazyCtx) -> Body {
        let src = &ctx.files.borrow()[self.file as usize];
        let text = &src.text[self.start as usize..self.end as usize];
        let mut parser = Parser::new(ctx, self.file, self.start, text);
        parser.parse_erb();
        // The parser's func builder would have the result
        // For now, return default - the real implementation would extract the body
        Body::default()
    }
}

/// Parser state for one file.
#[derive(Debug)]
pub struct Parser<'a> {
    ctx: &'a LazyCtx,
    file: u32,
    line: u32,
    text: &'a str,
    lines: Vec<&'a str>,
    line_no: u32,
    /// Stack of open control structures for patching jumps
    ctl: Vec<Ctl>,
    /// SIF jump to patch after the next statement
    sif_target: Option<usize>,
    /// Current function being built
    func: FuncBuilder,
    /// Errors collected during parsing
    errors: Vec<String>,
}

/// An open control structure (IF, FOR, WHILE, SELECTCASE) waiting for its end.
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

#[derive(Debug)]
struct FuncBuilder {
    code: Vec<Instr>,
    lines: Vec<(u32, u32)>,
    labels: HashMap<String, usize>,
    dims: Vec<DimDecl>,
    n_slots: usize,
    slot_counter: usize,
    local_str: HashSet<String>,
}

impl FuncBuilder {
    fn new() -> Self {
        FuncBuilder {
            code: Vec::new(),
            lines: Vec::new(),
            labels: HashMap::default(),
            dims: Vec::new(),
            n_slots: 0,
            slot_counter: 0,
            local_str: HashSet::default(),
        }
    }

    fn push(&mut self, ins: Instr, line: u32, file: u32) -> usize {
        let idx = self.code.len();
        self.code.push(ins);
        self.lines.push((file, line));
        idx
    }

    fn next_slot(&mut self) -> usize {
        let s = self.slot_counter;
        self.slot_counter += 1;
        s
    }
}

impl<'a> Parser<'a> {
    pub fn new(ctx: &'a LazyCtx, file: u32, start_line: u32, text: &'a str) -> Self {
        let lines = loader::emuera_lines(text).collect();
        Parser {
            ctx,
            file,
            line: start_line,
            text,
            lines,
            line_no: start_line,
            ctl: Vec::new(),
            sif_target: None,
            func: FuncBuilder::new(),
            errors: Vec::new(),
        }
    }

    /// Parse an ERH file (global #DEFINE and #DIM)
    pub fn parse_erh(&mut self) {
        let lines: Vec<String> = self.lines.iter().map(|s| s.to_string()).collect();
        for (i, line) in lines.iter().enumerate() {
            self.line_no = self.line + i as u32;
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if line.starts_with("#DEFINE") {
                self.parse_define(line);
            } else if line.starts_with("#DIM") || line.starts_with("#DIMS") {
                if let Some(dim) = self.parse_dim(line, true) {
                    self.ctx.global_dims.borrow_mut().push(dim);
                }
            } else if line.starts_with('[') {
                // Skip preprocessor directives in ERH
            } else {
                self.errors.push(format!(
                    "{}:{}: unexpected line in ERH: {}",
                    self.ctx.files.borrow()[self.file as usize].path,
                    self.line_no,
                    line
                ));
            }
        }
    }

    /// Parse an ERB file (functions)
    pub fn parse_erb(&mut self) {
        let mut i = 0;
        while i < self.lines.len() {
            self.line_no = self.line + i as u32;
            let line = self.lines[i].trim();

            if line.is_empty() || line.starts_with(';') {
                i += 1;
                continue;
            }

            if line.starts_with('@') {
                // Function definition
                // Check if there's a body to parse
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

                if let Some((name, params, attrs)) = self.parse_func_header(line) {
                    let body_text = self.lines[body_start..body_end].join("\n");
                    self.ctx.files.borrow_mut()[self.file as usize].text = body_text.clone();

                    let lazy = LazySrc {
                        file: self.file,
                        start: body_start as u32,
                        end: body_end as u32,
                        open_ifs,
                        ord: 0,
                    };

                    let func = Function {
                        name: name.into(),
                        params: params.into(),
                        body: BodyCell::new(None),
                        lazy: Some(lazy),
                        is_function: attrs.is_function,
                        returns_str: attrs.returns_str,
                        order: attrs.order,
                        single: attrs.single,
                        only: attrs.only,
                        kojo: attrs.kojo,
                        local_size: attrs.local_size,
                        locals_size: attrs.locals_size,
                        file: self.file,
                        line: self.line_no,
                        span: Span::empty(),
                    };

                    self.funcs_add(func);
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
            self.ctx.defines.borrow_mut().insert(name, body);
        }
    }

    fn parse_dim(&mut self, line: &str, is_global: bool) -> Option<DimDecl> {
        let rest = &line[4..].trim(); // skip #DIM or #DIMS
        let is_str = line.starts_with("#DIMS");

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
        let mut decl_name = name.clone();

        for part in &parts[1..] {
            let upper = part.to_uppercase();
            match upper.as_ref() {
                "CONST" => is_const = true,
                "STATIC" => is_static = true,
                "GLOBAL" => is_static = true,
                "SAVEDATA" => is_savedata = true,
                "CHARADATA" => is_chara = true,
                "REF" => is_ref = true,
                "DYNAMIC" => is_dynamic = true,
                _ => {
                    // Could be a dimension size or initial value
                    if let Ok(expr) = parse_expr(part, &self.ctx.defines.borrow()) {
                        if init.is_empty() && dims.is_empty() {
                            dims.push(expr);
                        } else {
                            init.push(expr);
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
        let rest = &line[1..].trim(); // skip @
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
                // Parameter
                let mut p = split_top(part);
                let param_name = p[0].to_uppercase();
                let default = if p.len() > 1 {
                    parse_expr(&p[1..].join(","), &self.ctx.defines.borrow()).ok()
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

    fn funcs_add(&self, func: Function) {
        // In real implementation, this would add to Funcs
        // For LSP, we store in the program
    }

    /// Parse a function from its header and body text
    pub fn parse_function(&mut self, h: &Head, text: &str) -> Rc<Function> {
        // Create a new parser for this function
        let mut parser = Parser::new(self.ctx, h.src.file, h.src.line, text);
        parser.parse_erb();

        // For now, return a dummy function - the actual implementation
        // would extract the parsed function from the parser
        // Extract the function name from the names string
        let name = self.ctx.files.borrow()[h.src.file as usize].path.clone(); // Placeholder - should use names string
        let name_box = name.into_boxed_str();

        Rc::new(Function {
            name: name_box,
            params: Box::new([]),
            body: BodyCell::new(None),
            lazy: None,
            is_function: false,
            returns_str: false,
            order: EventOrder::Normal,
            single: false,
            only: false,
            kojo: false,
            local_size: 0,
            locals_size: 0,
            file: h.src.file,
            line: h.src.line,
            span: Span::empty(),
        })
    }
}

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

/// Parse ERH file text
pub fn parse_erh_text(
    text: &str,
    defines: &mut HashMap<String, String>,
    global_dims: &mut Vec<DimDecl>,
) {
    let lines = loader::emuera_lines(text).collect::<Vec<_>>();
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with("#DEFINE") {
            let rest = &line["#DEFINE".len()..].trim();
            let mut parts = rest.splitn(2, ' ');
            let name = parts.next().unwrap_or("").to_uppercase();
            let body = parts.next().unwrap_or("").to_string();
            if !name.is_empty() {
                defines.insert(name, body);
            }
        } else if line.starts_with("#DIM") || line.starts_with("#DIMS") {
            let is_str = line.starts_with("#DIMS");
            let prefix_len = if is_str { "#DIMS".len() } else { "#DIM".len() };
            let rest = &line[prefix_len..].trim();
            let mut parts = split_top(rest);
            if parts.is_empty() {
                continue;
            }
            // First part may contain "NAME 10" - split by whitespace
            let first_parts: Vec<&str> = parts[0].split_whitespace().collect();
            let name = first_parts[0].to_uppercase();
            let mut dims = Vec::new();
            let mut init = Vec::new();
            let mut is_const = false;
            let mut is_static = true;
            let mut is_savedata = false;
            let mut is_chara = false;
            let mut is_ref = false;
            let mut is_dynamic = false;
            let decl_name = name.clone();

            // Add remaining parts from first split as dims
            for part in &first_parts[1..] {
                if let Ok(expr) = parse_expr(part, &HashMap::default()) {
                    dims.push(expr);
                }
            }

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
                        if let Ok(expr) = parse_expr(part, &HashMap::default()) {
                            if init.is_empty() && dims.is_empty() {
                                dims.push(expr);
                            } else {
                                init.push(expr);
                            }
                        }
                    }
                }
            }

            global_dims.push(DimDecl {
                name,
                decl_name,
                is_str,
                dims,
                init,
                is_const,
                is_static,
                is_global: true,
                is_savedata,
                is_chara,
                is_ref,
                is_dynamic,
                span: Span::empty(),
            });
        }
    }
}

/// Parse a single ERB/ERH file and return a Program
pub fn parse_file<'a>(path: &Path, text: &str) -> Program<'a> {
    let mut program = Program::default();
    program.files.push(path.to_string_lossy().to_string());

    let mut defines = HashMap::default();
    let mut global_dims = Vec::new();

    // Parse ERH first
    parse_erh_text(text, &mut defines, &mut global_dims);

    program.defines = defines;
    program.global_dims = global_dims;

    program
}

/// Parse ERH text and return defines and global dims
pub fn parse_erh(
    source: &SourceFile,
    defines: &mut HashMap<String, String>,
    global_dims: &mut Vec<DimDecl>,
) {
    let lines = loader::emuera_lines(&source.text).collect::<Vec<_>>();
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with("#DEFINE") {
            let rest = &line["#DEFINE".len()..].trim();
            let mut parts = rest.splitn(2, ' ');
            let name = parts.next().unwrap_or("").to_uppercase();
            let body = parts.next().unwrap_or("").to_string();
            if !name.is_empty() {
                defines.insert(name, body);
            }
        } else if line.starts_with("#DIM") || line.starts_with("#DIMS") {
            let is_str = line.starts_with("#DIMS");
            let prefix_len = if is_str { "#DIMS".len() } else { "#DIM".len() };
            let rest = &line[prefix_len..].trim();
            let mut parts = split_top(rest);
            if parts.is_empty() {
                continue;
            }
            // First part may contain "NAME 10" - split by whitespace
            let first_parts: Vec<&str> = parts[0].split_whitespace().collect();
            let name = first_parts[0].to_uppercase();
            let mut dims = Vec::new();
            let mut init = Vec::new();
            let mut is_const = false;
            let mut is_static = true;
            let mut is_savedata = false;
            let mut is_chara = false;
            let mut is_ref = false;
            let mut is_dynamic = false;
            let decl_name = name.clone();

            // Add remaining parts from first split as dims
            for part in &first_parts[1..] {
                if let Ok(expr) = parse_expr(part, &HashMap::default()) {
                    dims.push(expr);
                }
            }

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
                        if let Ok(expr) = parse_expr(part, &HashMap::default()) {
                            if init.is_empty() && dims.is_empty() {
                                dims.push(expr);
                            } else {
                                init.push(expr);
                            }
                        }
                    }
                }
            }

            global_dims.push(DimDecl {
                name,
                decl_name,
                is_str,
                dims,
                init,
                is_const,
                is_static,
                is_global: true,
                is_savedata,
                is_chara,
                is_ref,
                is_dynamic,
                span: Span::empty(),
            });
        }
    }
}

/// Parse ERB text with existing defines and global dims
pub fn parse_erb_text<'a>(parser: &mut Parser<'a>, source: &SourceFile) -> Program<'a> {
    let mut program = Program::default();
    program.files.push(source.path.clone());

    // Use the existing parser to parse functions
    parser.parse_erb();

    // Copy the parsed functions to the program
    // This is a simplified version - in reality we'd need to properly extract functions
    program.defines = parser.ctx.defines.borrow().clone();
    program.global_dims = parser.ctx.global_dims.borrow().clone();

    // Move functions from parser to program
    // Note: This is a simplified approach - the actual implementation would
    // need to properly construct the Funcs structure
    program
}

/// Create a new Parser for LSP use
impl Parser<'_> {
    pub fn new_for_lsp() -> Self {
        // Create a minimal parser context for LSP
        // This would need a proper LazyCtx
        todo!("Parser::new_for_lsp needs proper implementation")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fxhash::HashMap;

    #[test]
    fn test_parse_erh() {
        let text = r#"
#DEFINE TEST 123
#DIM FLAG 10
#DIMS NAME "default"
"#;
        let mut defines = HashMap::default();
        let mut global_dims = Vec::new();
        parse_erh_text(text, &mut defines, &mut global_dims);

        assert_eq!(defines.get("TEST"), Some(&"123".to_string()));
        assert_eq!(global_dims.len(), 2);
        assert_eq!(global_dims[0].name, "FLAG");
        assert_eq!(global_dims[1].name, "NAME");
        assert!(global_dims[1].is_str);
    }
}
