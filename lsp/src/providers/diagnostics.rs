//! Diagnostics provider for EraBasic LSP.

use crate::document::Document;
use crate::lsp_ast::{CallTarget, Expr, Function, Instr, Program, Span};
use lsp_types::{
    Diagnostic, DiagnosticSeverity as LspDiagnosticSeverity, DiagnosticTag, Position,
    PublishDiagnosticsParams, Range,
};
use std::collections::{HashMap, HashSet};

/// Diagnostics provider for EraBasic.
pub struct DiagnosticsProvider {
    /// Lint rules configuration
    lint_rules: LintRules,
}

#[derive(Debug, Clone)]
pub struct LintRules {
    pub string_comparison: bool,
    pub malformed_names: bool,
    pub unclosed_form_holes: bool,
    pub unused_variables: bool,
    pub undefined_functions: bool,
}

impl Default for LintRules {
    fn default() -> Self {
        Self {
            string_comparison: true,
            malformed_names: true,
            unclosed_form_holes: true,
            unused_variables: true,
            undefined_functions: true,
        }
    }
}

/// Type tracking for variables (like original parser's Types struct).
struct Types {
    global_str: HashSet<String>,
    local: HashMap<String, bool>,
}

impl DiagnosticsProvider {
    pub fn new() -> Self {
        Self {
            lint_rules: LintRules::default(),
        }
    }

    /// Provide diagnostics for a document.
    pub fn provide(&self, document: &Document) -> PublishDiagnosticsParams {
        let mut diagnostics = Vec::new();

        if let Some(ast) = &document.ast {
            // Lint the program
            let lint_issues = lint(&ast.program);

            for issue in lint_issues {
                if self.should_report(&issue) {
                    diagnostics.push(self.lint_issue_to_diagnostic(&issue, document));
                }
            }

            // Parse errors from parser
            for error in &document.parse_errors {
                diagnostics.push(Diagnostic {
                    range: error.range,
                    severity: Some(match error.severity {
                        crate::document::ParseErrorSeverity::Error => LspDiagnosticSeverity::ERROR,
                        crate::document::ParseErrorSeverity::Warning => {
                            LspDiagnosticSeverity::WARNING
                        }
                    }),
                    code: Some(lsp_types::NumberOrString::String("parse-error".into())),
                    code_description: None,
                    source: Some("erabasic-lsp".into()),
                    message: error.message.clone(),
                    related_information: None,
                    tags: None,
                    data: None,
                });
            }

            // Additional semantic checks
            diagnostics.extend(self.semantic_checks(&ast.program, document));
        }

        PublishDiagnosticsParams {
            uri: document.uri.clone(),
            version: Some(document.version),
            diagnostics,
        }
    }

    /// Provide diagnostics from a parsed AST (for background parsing).
    pub fn provide_from_ast(
        &self,
        ast: &crate::document::ParsedDocument,
        uri: lsp_types::Url,
    ) -> PublishDiagnosticsParams {
        let mut diagnostics = Vec::new();

        // Lint the program
        let lint_issues = lint(&ast.program);

        for issue in lint_issues {
            if self.should_report(&issue) {
                diagnostics.push(self.lint_issue_to_diagnostic_from_ast(&issue));
            }
        }

        // Parse errors from parser
        for error in &ast.parse_errors {
            diagnostics.push(Diagnostic {
                range: error.range,
                severity: Some(match error.severity {
                    crate::document::ParseErrorSeverity::Error => LspDiagnosticSeverity::ERROR,
                    crate::document::ParseErrorSeverity::Warning => LspDiagnosticSeverity::WARNING,
                }),
                code: Some(lsp_types::NumberOrString::String("parse-error".into())),
                code_description: None,
                source: Some("erabasic-lsp".into()),
                message: error.message.clone(),
                related_information: None,
                tags: None,
                data: None,
            });
        }

        // Additional semantic checks
        diagnostics.extend(self.semantic_checks_from_ast(ast));

        PublishDiagnosticsParams {
            uri,
            version: Some(0), // Version not available in background parsing
            diagnostics,
        }
    }

    /// Check if a lint issue should be reported based on configuration.
    fn should_report(&self, issue: &LintIssue) -> bool {
        match issue.code.as_str() {
            "string-comparison" | "string-case-comparison" => self.lint_rules.string_comparison,
            "malformed-name" => self.lint_rules.malformed_names,
            "unclosed-form-hole" => self.lint_rules.unclosed_form_holes,
            _ => true,
        }
    }

    /// Convert a lint issue to an LSP diagnostic.
    fn lint_issue_to_diagnostic(&self, issue: &LintIssue, document: &Document) -> Diagnostic {
        let severity = match issue.severity {
            LintSeverity::Error => LspDiagnosticSeverity::ERROR,
            LintSeverity::Warning => LspDiagnosticSeverity::WARNING,
            LintSeverity::Information => LspDiagnosticSeverity::INFORMATION,
            LintSeverity::Hint => LspDiagnosticSeverity::HINT,
        };

        Diagnostic {
            range: if issue.span.is_empty() {
                Range::default()
            } else {
                document.span_to_range(issue.span)
            },
            severity: Some(severity),
            code: Some(lsp_types::NumberOrString::String(issue.code.clone())),
            code_description: None,
            source: Some("erabasic-lsp".into()),
            message: issue.message.clone(),
            related_information: None,
            tags: if issue.code == "malformed-name" {
                Some(vec![DiagnosticTag::UNNECESSARY])
            } else {
                None
            },
            data: None,
        }
    }

    /// Convert a lint issue to an LSP diagnostic (for AST-only context).
    fn lint_issue_to_diagnostic_from_ast(&self, issue: &LintIssue) -> Diagnostic {
        let severity = match issue.severity {
            LintSeverity::Error => LspDiagnosticSeverity::ERROR,
            LintSeverity::Warning => LspDiagnosticSeverity::WARNING,
            LintSeverity::Information => LspDiagnosticSeverity::INFORMATION,
            LintSeverity::Hint => LspDiagnosticSeverity::HINT,
        };

        Diagnostic {
            range: if issue.span.is_empty() {
                Range::default()
            } else {
                crate::util::position::span_to_range(
                    "",
                    issue.span,
                    crate::util::position::PositionEncoding::Utf16,
                )
            },
            severity: Some(severity),
            code: Some(lsp_types::NumberOrString::String(issue.code.clone())),
            code_description: None,
            source: Some("erabasic-lsp".into()),
            message: issue.message.clone(),
            related_information: None,
            tags: if issue.code == "malformed-name" {
                Some(vec![DiagnosticTag::UNNECESSARY])
            } else {
                None
            },
            data: None,
        }
    }

    /// Additional semantic checks from AST only (no document context).
    fn semantic_checks_from_ast(&self, ast: &crate::document::ParsedDocument) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Check for undefined functions
        if self.lint_rules.undefined_functions {
            diagnostics.extend(self.check_undefined_functions_from_ast(ast));
        }

        // Check for duplicate function definitions
        diagnostics.extend(self.check_duplicate_functions_from_ast(ast));

        diagnostics
    }

    /// Check for calls to undefined functions (from AST only).
    fn check_undefined_functions_from_ast(&self, ast: &crate::document::ParsedDocument) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Collect all defined function names
        let mut defined = HashSet::new();
        for func_list in ast.program.functions.values() {
            for func in func_list {
                defined.insert(func.name.to_uppercase());
            }
        }

        // Add built-ins
        for cmd in crate::parser::builtins::COMMANDS {
            defined.insert(cmd.to_string());
        }
        for func in crate::parser::builtins::FUNCTIONS {
            defined.insert(func.to_string());
        }

        // Check each function body
        for func_list in ast.program.functions.values() {
            for func in func_list {
                let body = &func.body;

                for (pc, instr) in body.code.iter().enumerate() {
                    if let Instr::Call(call) = instr {
                        let name = match &call.target {
                            CallTarget::Static(s) => s.to_uppercase(),
                            CallTarget::Form(_) => continue, // Dynamic, can't check
                        };

                        if !defined.contains(&name) {
                            let line = body.lines.get(pc)
                                .and_then(|span| ast.line_map.iter()
                                    .find(|li| li.start_offset <= span.start && span.start < li.end_offset)
                                    .map(|li| li.original_line))
                                .unwrap_or(0);
                            let range = Range::new(
                                Position::new(line.saturating_sub(1), 0),
                                Position::new(line.saturating_sub(1), 100),
                            );
                            diagnostics.push(Diagnostic {
                                range,
                                severity: Some(LspDiagnosticSeverity::WARNING),
                                code: Some(lsp_types::NumberOrString::String(
                                    "undefined-function".into(),
                                )),
                                code_description: None,
                                source: Some("erabasic-lsp".into()),
                                message: format!("Call to undefined function '{}'", name),
                                related_information: None,
                                tags: None,
                                data: None,
                            });
                        }
                    }
                }
            }
        }

        diagnostics
    }

    /// Check for duplicate function definitions (from AST only).
    fn check_duplicate_functions_from_ast(&self, ast: &crate::document::ParsedDocument) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for (name, funcs) in &ast.program.functions {
            if funcs.len() > 1 {
                for func in funcs.iter().skip(1) {
                    diagnostics.push(Diagnostic {
                        range: Range::new(
                            Position::new(func.line.saturating_sub(1), 0),
                            Position::new(func.line.saturating_sub(1), 100),
                        ),
                        severity: Some(LspDiagnosticSeverity::WARNING),
                        code: Some(lsp_types::NumberOrString::String(
                            "duplicate-function".into(),
                        )),
                        code_description: None,
                        source: Some("erabasic-lsp".into()),
                        message: format!("Function '{}' already defined", name),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            }
        }

        diagnostics
    }

    /// Update lint rules.
    pub fn set_lint_rules(&mut self, rules: LintRules) {
        self.lint_rules = rules;
    }

    /// Additional semantic checks beyond linting (with document context).
    fn semantic_checks(&self, program: &Program, document: &Document) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Check for undefined functions
        if self.lint_rules.undefined_functions {
            diagnostics.extend(self.check_undefined_functions(program, document));
        }

        // Check for duplicate function definitions
        diagnostics.extend(self.check_duplicate_functions(program, document));

        diagnostics
    }

    /// Check for calls to undefined functions.
    fn check_undefined_functions(
        &self,
        program: &Program,
        document: &Document,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Collect all defined function names
        let mut defined = HashSet::new();
        for func_list in program.functions.values() {
            for func in func_list {
                defined.insert(func.name.to_uppercase());
            }
        }

        // Add built-ins
        for cmd in crate::parser::builtins::COMMANDS {
            defined.insert(cmd.to_string());
        }
        for func in crate::parser::builtins::FUNCTIONS {
            defined.insert(func.to_string());
        }

        // Check each function body
        for func_list in program.functions.values() {
            for func in func_list {
                let body = &func.body;

                for (pc, instr) in body.code.iter().enumerate() {
                    if let Instr::Call(call) = instr {
                        let name = match &call.target {
                            CallTarget::Static(s) => s.to_uppercase(),
                            CallTarget::Form(_) => continue, // Dynamic, can't check
                        };

                        if !defined.contains(&name) {
                            let line = body.lines.get(pc)
                                .and_then(|span| {
                                    let text = document.get_text();
                                    Some(crate::util::position::offset_to_position(
                                        &text,
                                        span.start,
                                        crate::util::position::PositionEncoding::Utf16,
                                    ))
                                })
                                .map(|pos| pos.line)
                                .unwrap_or(0);
                            let range = Range::new(
                                Position::new(line, 0),
                                Position::new(line, 100),
                            );
                            diagnostics.push(Diagnostic {
                                range,
                                severity: Some(LspDiagnosticSeverity::WARNING),
                                code: Some(lsp_types::NumberOrString::String(
                                    "undefined-function".into(),
                                )),
                                code_description: None,
                                source: Some("erabasic-lsp".into()),
                                message: format!("Call to undefined function '{}'", name),
                                related_information: None,
                                tags: None,
                                data: None,
                            });
                        }
                    }
                }
            }
        }

        diagnostics
    }

    /// Check for duplicate function definitions.
    fn check_duplicate_functions(
        &self,
        program: &Program,
        document: &Document,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for (name, funcs) in &program.functions {
            if funcs.len() > 1 {
                for func in funcs.iter().skip(1) {
                    let line = if !func.span.is_empty() {
                        let text = document.get_text();
                        crate::util::position::offset_to_position(
                            &text,
                            func.span.start,
                            crate::util::position::PositionEncoding::Utf16,
                        ).line
                    } else {
                        func.line.saturating_sub(1)
                    };
                    diagnostics.push(Diagnostic {
                        range: Range::new(
                            Position::new(line, 0),
                            Position::new(line, 100),
                        ),
                        severity: Some(LspDiagnosticSeverity::WARNING),
                        code: Some(lsp_types::NumberOrString::String(
                            "duplicate-function".into(),
                        )),
                        code_description: None,
                        source: Some("erabasic-lsp".into()),
                        message: format!("Function '{}' already defined", name),
                        related_information: None,
                        tags: None,
                        data: None,
                    });
                }
            }
        }

        diagnostics
    }
}

/// Lint issue from the parser.
#[derive(Debug, Clone)]
pub struct LintIssue {
    pub message: String,
    pub span: Span,
    pub severity: LintSeverity,
    pub code: String,
}

/// Lint severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

/// Simple lint function for the LSP AST.
pub fn lint(program: &Program) -> Vec<LintIssue> {
    let mut issues = Vec::new();

    // Build type information from global DIMS declarations
    let mut global_str = HashSet::new();
    for dim in &program.global_dims {
        if dim.is_str {
            global_str.insert(dim.name.to_uppercase());
        }
    }

    // Check each function
    for func_list in program.functions.values() {
        for func in func_list {
            // Build local type info from function's DIMS
            let mut local = HashMap::new();
            for dim in &func.body.dims {
                local.insert(dim.name.to_uppercase(), dim.is_str);
            }

            let types = Types { global_str: global_str.clone(), local };

            for instr in &func.body.code {
                check_instr_for_lint_with_types(instr, &types, &mut issues);
            }
        }
    }

    issues
}

fn check_instr_for_lint_with_types(instr: &Instr, types: &Types, issues: &mut Vec<LintIssue>) {
    match instr {
        Instr::Assign(var, _, exprs) => {
            for expr in exprs {
                check_expr_for_string_comparison_with_types(expr, types, issues);
                // Also recurse into nested expressions
                check_expr_nested_with_types(expr, types, issues);
            }
        }
        Instr::JumpIfNot(expr, _) => {
            check_expr_for_string_comparison_with_types(expr, types, issues);
            check_expr_nested_with_types(expr, types, issues);
        }
        Instr::Call(call) => {
            if let CallTarget::Static(name) = &call.target {
                // Check for malformed names
                if name.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '$') {
                    issues.push(LintIssue {
                        message: format!("Malformed function name: {}", name),
                        span: Span::empty(),
                        severity: LintSeverity::Warning,
                        code: "malformed-name".to_string(),
                    });
                }
            }
        }
        _ => {}
    }
}

fn check_expr_for_string_comparison(expr: &Expr, issues: &mut Vec<LintIssue>) {
    if let Expr::Binary(op, left, right) = expr {
        // Check for >= / <= with string operands (Emuera bug: evaluates as <)
        if matches!(op, crate::lsp_ast::BinOp::Ge | crate::lsp_ast::BinOp::Le)
            && is_string_expr(left)
            && is_string_expr(right)
        {
            issues.push(LintIssue {
                message: format!(
                    "string {} (Emuera evaluates it as <)",
                    if *op == crate::lsp_ast::BinOp::Ge {
                        ">="
                    } else {
                        "<="
                    }
                ),
                span: expr.span(),
                severity: LintSeverity::Warning,
                code: "string-comparison".to_string(),
            });
        }
        // Check for = used as string comparison (should be ==)
        if *op == crate::lsp_ast::BinOp::Eq && is_string_expr(right) {
            issues.push(LintIssue {
                message: "String comparison with '=' - use '==' instead".to_string(),
                span: expr.span(),
                severity: LintSeverity::Warning,
                code: "string-comparison".to_string(),
            });
        }
    }
}

fn check_expr_for_string_comparison_with_types(expr: &Expr, types: &Types, issues: &mut Vec<LintIssue>) {
    if let Expr::Binary(op, left, right) = expr {
        // Check for >= / <= with string operands (Emuera bug: evaluates as <)
        if matches!(op, crate::lsp_ast::BinOp::Ge | crate::lsp_ast::BinOp::Le)
            && is_string_expr_with_types(left, types)
            && is_string_expr_with_types(right, types)
        {
            issues.push(LintIssue {
                message: format!(
                    "string {} (Emuera evaluates it as <)",
                    if *op == crate::lsp_ast::BinOp::Ge {
                        ">="
                    } else {
                        "<="
                    }
                ),
                span: expr.span(),
                severity: LintSeverity::Warning,
                code: "string-comparison".to_string(),
            });
        }
        // Check for = used as string comparison (should be ==)
        if *op == crate::lsp_ast::BinOp::Eq && is_string_expr_with_types(right, types) {
            issues.push(LintIssue {
                message: "String comparison with '=' - use '==' instead".to_string(),
                span: expr.span(),
                severity: LintSeverity::Warning,
                code: "string-comparison".to_string(),
            });
        }
    }
}

fn check_expr_nested(expr: &Expr, issues: &mut Vec<LintIssue>) {
    match expr {
        Expr::Binary(_, left, right) => {
            check_expr_nested(left, issues);
            check_expr_nested(right, issues);
        }
        Expr::Unary(_, inner) => check_expr_nested(inner, issues),
        Expr::Ternary(cond, left, right) => {
            check_expr_nested(cond, issues);
            check_expr_nested(left, issues);
            check_expr_nested(right, issues);
        }
        Expr::Call(call) => {
            // call is Box<(String, Vec<Expr>, Resolved)>
            for arg in &call.1 {
                check_expr_nested(arg, issues);
            }
        }
        Expr::Var(v) => {
            for idx in &v.idx {
                check_expr_nested(idx, issues);
            }
        }
        Expr::Form(f) => {
            for part in &f.parts {
                check_form_part(part, issues);
            }
        }
        Expr::IncDec(v, _, _) => {
            for idx in &v.idx {
                check_expr_nested(idx, issues);
            }
        }
        Expr::Int(_) | Expr::Str(_) | Expr::Empty | Expr::Span(_) => {}
    }
}

fn check_form_part(part: &crate::lsp_ast::FormPart, issues: &mut Vec<LintIssue>) {
    use crate::lsp_ast::FormPart;
    match part {
        FormPart::Lit(_) => {}
        FormPart::Str(e, w, _) | FormPart::Int(e, w, _) => {
            check_expr_nested(e, issues);
            if let Some(w) = w {
                check_expr_nested(w, issues);
            }
        }
        FormPart::Cond(c) => {
            // c is Box<(Expr, Form, Form)>
            check_expr_nested(&c.0, issues);
            // The true/false branches are Forms, need to check their parts
            for part in &c.1.parts {
                check_form_part(part, issues);
            }
            for part in &c.2.parts {
                check_form_part(part, issues);
            }
        }
    }
}

fn is_string_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Str(_) | Expr::Form(_))
}

fn is_string_expr_with_types(expr: &Expr, types: &Types) -> bool {
    match expr {
        Expr::Str(_) | Expr::Form(_) => true,
        Expr::Var(v) => {
            let name = v.name.to_uppercase();
            match types.local.get(&name) {
                Some(&s) => s,
                None => types.global_str.contains(&name),
            }
        }
        Expr::Call(call) => {
            let name = call.0.to_uppercase();
            // Check built-in functions that return strings
            crate::parser::builtins::returns_str(&name)
        }
        Expr::Binary(crate::lsp_ast::BinOp::Add | crate::lsp_ast::BinOp::Mul, a, b) => {
            is_string_expr_with_types(a, types) || is_string_expr_with_types(b, types)
        }
        Expr::Ternary(_, a, _) => is_string_expr_with_types(a, types),
        _ => false,
    }
}

fn check_expr_nested_with_types(expr: &Expr, types: &Types, issues: &mut Vec<LintIssue>) {
    match expr {
        Expr::Binary(_, left, right) => {
            check_expr_nested_with_types(left, types, issues);
            check_expr_nested_with_types(right, types, issues);
        }
        Expr::Unary(_, inner) => check_expr_nested_with_types(inner, types, issues),
        Expr::Ternary(cond, left, right) => {
            check_expr_nested_with_types(cond, types, issues);
            check_expr_nested_with_types(left, types, issues);
            check_expr_nested_with_types(right, types, issues);
        }
        Expr::Call(call) => {
            for arg in &call.1 {
                check_expr_nested_with_types(arg, types, issues);
            }
        }
        Expr::Var(v) => {
            for idx in &v.idx {
                check_expr_nested_with_types(idx, types, issues);
            }
        }
        Expr::Form(f) => {
            for part in &f.parts {
                check_form_part_with_types(part, types, issues);
            }
        }
        Expr::IncDec(v, _, _) => {
            for idx in &v.idx {
                check_expr_nested_with_types(idx, types, issues);
            }
        }
        Expr::Int(_) | Expr::Str(_) | Expr::Empty | Expr::Span(_) => {}
    }
}

fn check_form_part_with_types(part: &crate::lsp_ast::FormPart, types: &Types, issues: &mut Vec<LintIssue>) {
    use crate::lsp_ast::FormPart;
    match part {
        FormPart::Lit(_) => {}
        FormPart::Str(e, w, _) | FormPart::Int(e, w, _) => {
            check_expr_nested_with_types(e, types, issues);
            if let Some(w) = w {
                check_expr_nested_with_types(w, types, issues);
            }
        }
        FormPart::Cond(c) => {
            // c is Box<(Expr, Form, Form)>
            check_expr_nested_with_types(&c.0, types, issues);
            // The true/false branches are Forms, need to check their parts
            for part in &c.1.parts {
                check_form_part_with_types(part, types, issues);
            }
            for part in &c.2.parts {
                check_form_part_with_types(part, types, issues);
            }
        }
    }
}

impl Default for DiagnosticsProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostics() {
        let provider = DiagnosticsProvider::new();
        // Would need document with AST to test
    }
}
