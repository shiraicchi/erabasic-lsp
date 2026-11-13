//! Reference resolution for EraBasic symbols.

use crate::analysis::symbols::Symbol;
use crate::parser::ast::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// A reference to a symbol.
#[derive(Debug, Clone)]
pub struct Reference {
    pub symbol_name: String,
    pub range: lsp_types::Range,
    pub file: PathBuf,
    pub kind: ReferenceKind,
    pub context: ReferenceContext,
}

/// The kind of reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceKind {
    /// Declaration/definition of the symbol
    Definition,
    /// Reading a variable
    Read,
    /// Writing to a variable
    Write,
    /// Calling a function
    Call,
    /// Jumping to a label (GOTO/JUMP)
    Jump,
    /// Using a macro
    Macro,
}

/// Context of the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceContext {
    InFunction,
    InGlobalScope,
    InMacro,
}

/// Reference resolver: finds all references to symbols in a program.
pub struct ReferenceResolver {
    /// All symbols in the workspace
    symbols: HashMap<String, Vec<Symbol>>,
    /// References found
    references: Vec<Reference>,
    /// Current file being processed
    current_file: PathBuf,
}

impl ReferenceResolver {
    pub fn new() -> Self {
        Self {
            symbols: HashMap::new(),
            references: Vec::new(),
            current_file: PathBuf::new(),
        }
    }

    /// Add symbols from a file symbol table.
    pub fn add_symbols(
        &mut self,
        file_path: PathBuf,
        symbols: crate::analysis::symbols::FileSymbolTable,
    ) {
        for (name, symbol) in symbols.symbols {
            self.symbols.entry(name).or_default().push(symbol);
        }
    }

    /// Resolve all references in a program.
    pub fn resolve_program(
        &mut self,
        program: &crate::parser::Program,
        file_path: PathBuf,
    ) -> Vec<Reference> {
        self.current_file = file_path;
        self.references.clear();

        // Resolve global dims
        for dim in &program.global_dims {
            self.add_reference(Reference {
                symbol_name: dim.name.to_uppercase(),
                range: crate::util::position::span_to_range(
                    "",
                    dim.span,
                    crate::util::position::PositionEncoding::Utf16,
                ),
                file: self.current_file.clone(),
                kind: ReferenceKind::Definition,
                context: ReferenceContext::InGlobalScope,
            });
        }

        // Resolve defines
        for name in program.defines.keys() {
            self.add_reference(Reference {
                symbol_name: name.to_uppercase(),
                range: lsp_types::Range::default(),
                file: self.current_file.clone(),
                kind: ReferenceKind::Definition,
                context: ReferenceContext::InGlobalScope,
            });
        }

        // Resolve each function
        for func_rc in program.funcs.made() {
            self.resolve_function(func_rc.as_ref());
        }

        self.references.clone()
    }

    /// Resolve references in a function.
    fn resolve_function(&mut self, func: &Function) {
        let body = func.body();

        // Function definition
        self.add_reference(Reference {
            symbol_name: func.name.to_uppercase(),
            range: lsp_types::Range::default(),
            file: self.current_file.clone(),
            kind: ReferenceKind::Definition,
            context: ReferenceContext::InFunction,
        });

        // Parameters
        for param in &func.params {
            self.add_reference(Reference {
                symbol_name: param.target.name.to_uppercase(),
                range: crate::util::position::span_to_range(
                    "",
                    param.span,
                    crate::util::position::PositionEncoding::Utf16,
                ),
                file: self.current_file.clone(),
                kind: ReferenceKind::Definition,
                context: ReferenceContext::InFunction,
            });
        }

        // Local dims
        for dim in &body.dims {
            self.add_reference(Reference {
                symbol_name: dim.name.to_uppercase(),
                range: crate::util::position::span_to_range(
                    "",
                    dim.span,
                    crate::util::position::PositionEncoding::Utf16,
                ),
                file: self.current_file.clone(),
                kind: ReferenceKind::Definition,
                context: ReferenceContext::InFunction,
            });
        }

        // Instructions
        for (pc, instr) in body.code.iter().enumerate() {
            self.resolve_instr(instr, pc, &body);
        }
    }

    /// Resolve references in an instruction.
    fn resolve_instr(&mut self, instr: &Instr, pc: usize, body: &Body) {
        match instr {
            Instr::Assign(var, _, exprs) => {
                // Write to variable
                self.add_var_reference(&var.name, ReferenceKind::Write, var.span);
                for expr in exprs {
                    self.resolve_expr(expr, ReferenceKind::Read);
                }
            }
            Instr::AssignForm(var, _, form) => {
                self.add_var_reference(&var.name, ReferenceKind::Write, var.span);
                self.resolve_form(form, ReferenceKind::Read);
            }
            Instr::Eval(expr) | Instr::JumpIfNot(expr, _) | Instr::ReturnF(Some(expr)) => {
                self.resolve_expr(expr, ReferenceKind::Read);
            }
            Instr::Print(content, _) => match content {
                PrintContent::Expr(e) => self.resolve_expr(e, ReferenceKind::Read),
                PrintContent::Form(f) => self.resolve_form(f, ReferenceKind::Read),
                PrintContent::Exprs(es) => {
                    for e in es {
                        self.resolve_expr(e, ReferenceKind::Read);
                    }
                }
                PrintContent::Raw(_) => {}
            },
            Instr::PrintData(forms, _) | Instr::StrData(_, forms) => {
                for form_list in forms {
                    for f in form_list {
                        self.resolve_form(f, ReferenceKind::Read);
                    }
                }
            }
            Instr::Call(call) => {
                // Function call
                self.resolve_call_target(&call.target, ReferenceKind::Call);
                for arg in &call.args {
                    self.resolve_expr(arg, ReferenceKind::Read);
                }
            }
            Instr::CallList(calls, _) => {
                for (target, args) in calls {
                    self.resolve_call_target(target, ReferenceKind::Call);
                    for arg in args {
                        self.resolve_expr(arg, ReferenceKind::Read);
                    }
                }
            }
            Instr::Goto(goto) => {
                self.resolve_call_target(&goto.0, ReferenceKind::Jump);
            }
            Instr::GotoList(targets) => {
                for target in targets {
                    self.resolve_call_target(target, ReferenceKind::Jump);
                }
            }
            Instr::ForBegin(fb) => {
                self.add_var_reference(&fb.var.name, ReferenceKind::Write, fb.var.span);
                self.resolve_expr(&fb.start, ReferenceKind::Read);
                self.resolve_expr(&fb.end, ReferenceKind::Read);
                if let Some(step) = &fb.step {
                    self.resolve_expr(step, ReferenceKind::Read);
                }
            }
            Instr::ForNext { var, .. } | Instr::ForBreak { var, .. } => {
                self.add_var_reference(&var.name, ReferenceKind::Read, var.span);
            }
            Instr::SelectEval(expr, _, _) => {
                self.resolve_expr(expr, ReferenceKind::Read);
            }
            Instr::CaseTest(_, conds, _) => {
                for cond in conds {
                    match cond {
                        CaseCond::Eq(e) | CaseCond::Is(_, e) => {
                            self.resolve_expr(e, ReferenceKind::Read)
                        }
                        CaseCond::Range(a, b) => {
                            self.resolve_expr(a, ReferenceKind::Read);
                            self.resolve_expr(b, ReferenceKind::Read);
                        }
                    }
                }
            }
            Instr::Command(cmd) => {
                if let Args::Exprs(es) = &cmd.1 {
                    for e in es {
                        self.resolve_expr(e, ReferenceKind::Read);
                    }
                } else if let Args::Form(f) = &cmd.1 {
                    self.resolve_form(f, ReferenceKind::Read);
                }
            }
            _ => {}
        }
    }

    /// Resolve an expression for references.
    fn resolve_expr(&mut self, expr: &Expr, kind: ReferenceKind) {
        match expr {
            Expr::Var(var) => {
                self.add_var_reference(&var.name, kind, var.span);
            }
            Expr::Call(call) => {
                self.resolve_call_target(&CallTarget::Static(call.0.clone()), ReferenceKind::Call);
                for arg in &call.1 {
                    self.resolve_expr(arg, ReferenceKind::Read);
                }
            }
            Expr::Form(form) => {
                self.resolve_form(form, kind);
            }
            Expr::Unary(_, e) => self.resolve_expr(e, kind),
            Expr::Binary(_, a, b) => {
                self.resolve_expr(a, kind);
                self.resolve_expr(b, kind);
            }
            Expr::Ternary(c, a, b) => {
                self.resolve_expr(c, kind);
                self.resolve_expr(a, kind);
                self.resolve_expr(b, kind);
            }
            Expr::IncDec(var, _, _) => {
                self.add_var_reference(&var.name, kind, var.span);
            }
            _ => {}
        }
    }

    /// Resolve a FORM for references.
    fn resolve_form(&mut self, form: &Form, kind: ReferenceKind) {
        for part in &form.parts {
            match part {
                FormPart::Str(e, _, _) | FormPart::Int(e, _, _) => {
                    self.resolve_expr(e, kind);
                }
                FormPart::Cond(c) => {
                    self.resolve_expr(&c.0, kind);
                    self.resolve_form(&c.1, kind);
                    self.resolve_form(&c.2, kind);
                }
                FormPart::Lit(_) => {}
            }
        }
    }

    /// Resolve a call target (function name or label).
    fn resolve_call_target(&mut self, target: &CallTarget, kind: ReferenceKind) {
        match target {
            CallTarget::Static(name) => {
                self.add_reference(Reference {
                    symbol_name: name.to_uppercase(),
                    range: lsp_types::Range::default(),
                    file: self.current_file.clone(),
                    kind,
                    context: ReferenceContext::InFunction,
                });
            }
            CallTarget::Form(form) => {
                self.resolve_form(form, kind);
            }
        }
    }

    /// Add a variable reference.
    fn add_var_reference(
        &mut self,
        name: &str,
        kind: ReferenceKind,
        span: crate::parser::ast::Span,
    ) {
        self.add_reference(Reference {
            symbol_name: name.to_uppercase(),
            range: crate::util::position::span_to_range(
                "",
                span,
                crate::util::position::PositionEncoding::Utf16,
            ),
            file: self.current_file.clone(),
            kind,
            context: ReferenceContext::InFunction,
        });
    }

    /// Add a reference.
    fn add_reference(&mut self, reference: Reference) {
        self.references.push(reference);
    }

    /// Find all references to a symbol.
    pub fn find_references(&self, symbol_name: &str) -> Vec<&Reference> {
        let name = symbol_name.to_uppercase();
        self.references
            .iter()
            .filter(|r| r.symbol_name == name)
            .collect()
    }

    /// Resolve all references in a program (LSP AST version).
    pub fn resolve_program_lsp(
        &mut self,
        program: &crate::lsp_ast::Program,
        file_path: PathBuf,
    ) -> Vec<Reference> {
        self.current_file = file_path;
        self.references.clear();

        // Resolve global dims
        for dim in &program.global_dims {
            self.add_reference(Reference {
                symbol_name: dim.name.to_uppercase(),
                range: crate::util::position::span_to_range(
                    "",
                    dim.span,
                    crate::util::position::PositionEncoding::Utf16,
                ),
                file: self.current_file.clone(),
                kind: ReferenceKind::Definition,
                context: ReferenceContext::InGlobalScope,
            });
        }

        // Resolve defines
        for name in program.defines.keys() {
            self.add_reference(Reference {
                symbol_name: name.to_uppercase(),
                range: lsp_types::Range::default(),
                file: self.current_file.clone(),
                kind: ReferenceKind::Definition,
                context: ReferenceContext::InGlobalScope,
            });
        }

        // Resolve each function
        for func_vec in program.functions.values() {
            for func in func_vec {
                self.resolve_function_lsp(func);
            }
        }

        self.references.clone()
    }

    /// Resolve references in a function (LSP version).
    fn resolve_function_lsp(&mut self, func: &crate::lsp_ast::Function) {
        let body = &func.body;

        // Function definition
        self.add_reference(Reference {
            symbol_name: func.name.to_uppercase(),
            range: lsp_types::Range::default(),
            file: self.current_file.clone(),
            kind: ReferenceKind::Definition,
            context: ReferenceContext::InFunction,
        });

        // Parameters
        for param in &func.params {
            self.add_reference(Reference {
                symbol_name: param.target.name.to_uppercase(),
                range: crate::util::position::span_to_range(
                    "",
                    param.target.span,
                    crate::util::position::PositionEncoding::Utf16,
                ),
                file: self.current_file.clone(),
                kind: ReferenceKind::Definition,
                context: ReferenceContext::InFunction,
            });
        }

        // Local dims
        for dim in &body.dims {
            self.add_reference(Reference {
                symbol_name: dim.name.to_uppercase(),
                range: crate::util::position::span_to_range(
                    "",
                    dim.span,
                    crate::util::position::PositionEncoding::Utf16,
                ),
                file: self.current_file.clone(),
                kind: ReferenceKind::Definition,
                context: ReferenceContext::InFunction,
            });
        }

        // Instructions
        for instr in &body.code {
            self.resolve_instr_lsp(instr);
        }
    }

    /// Resolve references in an instruction (LSP version).
    fn resolve_instr_lsp(&mut self, instr: &crate::lsp_ast::Instr) {
        match instr {
            crate::lsp_ast::Instr::Assign(var, _, exprs) => {
                // Write to variable
                self.add_var_reference_lsp(&var.name, ReferenceKind::Write, var.span);
                for expr in exprs {
                    self.resolve_expr_lsp(expr, ReferenceKind::Read);
                }
            }
            crate::lsp_ast::Instr::AssignForm(var, _, form) => {
                self.add_var_reference_lsp(&var.name, ReferenceKind::Write, var.span);
                self.resolve_form_lsp(form, ReferenceKind::Read);
            }
            crate::lsp_ast::Instr::Eval(expr)
            | crate::lsp_ast::Instr::JumpIfNot(expr, _)
            | crate::lsp_ast::Instr::ReturnF(Some(expr)) => {
                self.resolve_expr_lsp(expr, ReferenceKind::Read);
            }
            crate::lsp_ast::Instr::Print(content, _) => match content {
                crate::lsp_ast::PrintContent::Expr(e) => {
                    self.resolve_expr_lsp(e, ReferenceKind::Read)
                }
                crate::lsp_ast::PrintContent::Form(f) => {
                    self.resolve_form_lsp(f, ReferenceKind::Read)
                }
                crate::lsp_ast::PrintContent::Exprs(es) => {
                    for e in es {
                        self.resolve_expr_lsp(e, ReferenceKind::Read);
                    }
                }
                crate::lsp_ast::PrintContent::Raw(_) => {}
            },
            crate::lsp_ast::Instr::Call(call) => {
                // Function call
                self.resolve_call_target_lsp(&call.target, ReferenceKind::Call);
                for arg in &call.args {
                    self.resolve_expr_lsp(arg, ReferenceKind::Read);
                }
            }
            crate::lsp_ast::Instr::Goto(goto) => {
                self.resolve_call_target_lsp(&goto.0, ReferenceKind::Jump);
            }
            crate::lsp_ast::Instr::ForBegin(fb) => {
                self.add_var_reference_lsp(&fb.var.name, ReferenceKind::Write, fb.var.span);
                self.resolve_expr_lsp(&fb.start, ReferenceKind::Read);
                self.resolve_expr_lsp(&fb.end, ReferenceKind::Read);
                if let Some(step) = &fb.step {
                    self.resolve_expr_lsp(step, ReferenceKind::Read);
                }
            }
            crate::lsp_ast::Instr::SelectEval(expr, _, _) => {
                self.resolve_expr_lsp(expr, ReferenceKind::Read);
            }
            crate::lsp_ast::Instr::CaseTest(_, conds, _) => {
                for cond in conds {
                    match cond {
                        crate::lsp_ast::CaseCond::Eq(e) | crate::lsp_ast::CaseCond::Is(_, e) => {
                            self.resolve_expr_lsp(e, ReferenceKind::Read)
                        }
                        crate::lsp_ast::CaseCond::Range(a, b) => {
                            self.resolve_expr_lsp(a, ReferenceKind::Read);
                            self.resolve_expr_lsp(b, ReferenceKind::Read);
                        }
                    }
                }
            }
            crate::lsp_ast::Instr::Command(cmd) => {
                if let crate::lsp_ast::Args::Exprs(es) = &cmd.1 {
                    for e in es {
                        self.resolve_expr_lsp(e, ReferenceKind::Read);
                    }
                } else if let crate::lsp_ast::Args::Form(f) = &cmd.1 {
                    self.resolve_form_lsp(f, ReferenceKind::Read);
                }
            }
            _ => {}
        }
    }

    /// Resolve an expression for references (LSP version).
    fn resolve_expr_lsp(&mut self, expr: &crate::lsp_ast::Expr, kind: ReferenceKind) {
        match expr {
            crate::lsp_ast::Expr::Var(var) => {
                self.add_var_reference_lsp(&var.name, kind, var.span);
            }
            crate::lsp_ast::Expr::Call(call) => {
                self.resolve_call_target_lsp(
                    &crate::lsp_ast::CallTarget::Static(call.0.clone()),
                    ReferenceKind::Call,
                );
                for arg in &call.1 {
                    self.resolve_expr_lsp(arg, ReferenceKind::Read);
                }
            }
            crate::lsp_ast::Expr::Form(form) => {
                self.resolve_form_lsp(form, kind);
            }
            crate::lsp_ast::Expr::Unary(_, e) => self.resolve_expr_lsp(e, kind),
            crate::lsp_ast::Expr::Binary(_, a, b) => {
                self.resolve_expr_lsp(a, kind);
                self.resolve_expr_lsp(b, kind);
            }
            crate::lsp_ast::Expr::Ternary(c, a, b) => {
                self.resolve_expr_lsp(c, kind);
                self.resolve_expr_lsp(a, kind);
                self.resolve_expr_lsp(b, kind);
            }
            crate::lsp_ast::Expr::IncDec(var, _, _) => {
                self.add_var_reference_lsp(&var.name, kind, var.span);
            }
            _ => {}
        }
    }

    /// Resolve a FORM for references (LSP version).
    fn resolve_form_lsp(&mut self, form: &crate::lsp_ast::Form, kind: ReferenceKind) {
        for part in &form.parts {
            match part {
                crate::lsp_ast::FormPart::Str(e, _, _) | crate::lsp_ast::FormPart::Int(e, _, _) => {
                    self.resolve_expr_lsp(e, kind);
                }
                crate::lsp_ast::FormPart::Cond(c) => {
                    self.resolve_expr_lsp(&c.0, kind);
                    self.resolve_form_lsp(&c.1, kind);
                    self.resolve_form_lsp(&c.2, kind);
                }
                crate::lsp_ast::FormPart::Lit(_) => {}
            }
        }
    }

    /// Resolve a call target (function name or label) (LSP version).
    fn resolve_call_target_lsp(
        &mut self,
        target: &crate::lsp_ast::CallTarget,
        kind: ReferenceKind,
    ) {
        match target {
            crate::lsp_ast::CallTarget::Static(name) => {
                self.add_reference(Reference {
                    symbol_name: name.to_uppercase(),
                    range: lsp_types::Range::default(),
                    file: self.current_file.clone(),
                    kind,
                    context: ReferenceContext::InFunction,
                });
            }
            crate::lsp_ast::CallTarget::Form(form) => {
                self.resolve_form_lsp(form, kind);
            }
        }
    }

    /// Add a variable reference (LSP version).
    fn add_var_reference_lsp(
        &mut self,
        name: &str,
        kind: ReferenceKind,
        span: crate::lsp_ast::Span,
    ) {
        self.add_reference(Reference {
            symbol_name: name.to_uppercase(),
            range: crate::util::position::span_to_range(
                "",
                span,
                crate::util::position::PositionEncoding::Utf16,
            ),
            file: self.current_file.clone(),
            kind,
            context: ReferenceContext::InFunction,
        });
    }
}

impl Default for ReferenceResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Workspace-wide reference index.
#[derive(Debug, Clone)]
pub struct WorkspaceReferenceIndex {
    /// All references across all files
    references: HashMap<PathBuf, Vec<Reference>>,
    /// Symbol definitions
    definitions: HashMap<String, Vec<Reference>>,
}

impl WorkspaceReferenceIndex {
    pub fn new() -> Self {
        Self {
            references: HashMap::new(),
            definitions: HashMap::new(),
        }
    }

    /// Add references from a file.
    pub fn add_file(&mut self, file: PathBuf, refs: Vec<Reference>) {
        for r in &refs {
            if r.kind == ReferenceKind::Definition {
                self.definitions
                    .entry(r.symbol_name.clone())
                    .or_default()
                    .push(r.clone());
            }
        }
        self.references.insert(file, refs);
    }

    /// Find all references to a symbol across the workspace.
    pub fn find_all_references(&self, symbol_name: &str) -> Vec<&Reference> {
        let name = symbol_name.to_uppercase();
        self.references
            .values()
            .flat_map(|refs| refs.iter())
            .filter(|r| r.symbol_name == name)
            .collect()
    }

    /// Find the definition of a symbol.
    pub fn find_definition(&self, symbol_name: &str) -> Option<&Reference> {
        let name = symbol_name.to_uppercase();
        self.definitions.get(&name)?.first()
    }

    /// Get all definitions.
    pub fn all_definitions(&self) -> impl Iterator<Item = (&String, &Vec<Reference>)> {
        self.definitions.iter()
    }
}

impl Default for WorkspaceReferenceIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reference_resolver() {
        let mut resolver = ReferenceResolver::new();
        // Would need a full program to test
    }
}
