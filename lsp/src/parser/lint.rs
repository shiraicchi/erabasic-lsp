//! Linting and diagnostics for EraBasic.
//!
//! Detects common issues:
//! - String comparisons with >= / <= (Emuera bug: evaluates as <)
//! - Malformed function/label names
//! - Unclosed FORM holes (% { })
//!
//! **LSP Adaptation**: Returns structured diagnostics with codes and ranges.

use crate::parser::ast::*;
use crate::parser::parser::Program;
use std::collections::{HashMap, HashSet};

/// What decides whether an expression is a string, as the parser saw it.
struct Types<'a, 'b> {
    prog: &'a Program<'b>,
    global_str: HashSet<String>,
    /// the function's own `#DIM` / `#DIMS` (they hide globals)
    local: HashMap<String, bool>,
}

impl<'a, 'b> Types<'a, 'b> {
    fn var_is_str(&self, name: &str) -> bool {
        match self.local.get(name) {
            Some(&s) => s,
            None => crate::parser::is_system_str_var(name) || self.global_str.contains(name),
        }
    }

    fn is_str(&self, e: &Expr) -> bool {
        match e {
            Expr::Str(_) | Expr::Form(_) => true,
            Expr::Var(v) => self.var_is_str(&v.name),
            Expr::Call(c) => {
                let n = c.0.to_uppercase();
                match self.prog.funcs.get(&n) {
                    Some(f) => f[0].is_function && f[0].returns_str,
                    None => crate::parser::builtins::returns_str(&n),
                }
            }
            Expr::Binary(BinOp::Add | BinOp::Mul, a, b) => self.is_str(a) || self.is_str(b),
            Expr::Ternary(_, a, _) => self.is_str(a),
            _ => false,
        }
    }

    fn expr(&self, e: &Expr, out: &mut Vec<LintIssue>) {
        match e {
            Expr::Binary(op, a, b) => {
                if matches!(op, BinOp::Ge | BinOp::Le) && self.is_str(a) && self.is_str(b) {
                    out.push(LintIssue {
                        code: "string-comparison".into(),
                        message: format!(
                            "string {} (Emuera evaluates it as <)",
                            if *op == BinOp::Ge { ">=" } else { "<=" }
                        ),
                        severity: DiagnosticSeverity::Warning,
                        span: a.span(),
                    });
                }
                self.expr(a, out);
                self.expr(b, out);
            }
            Expr::Unary(_, a) => self.expr(a, out),
            Expr::Ternary(c, a, b) => [c, a, b].iter().for_each(|x| self.expr(x, out)),
            Expr::Call(c) => c.1.iter().for_each(|x| self.expr(x, out)),
            Expr::Var(v) | Expr::IncDec(v, ..) => v.idx.iter().for_each(|x| self.expr(x, out)),
            Expr::Form(f) => self.form(f, out),
            Expr::Int(_) | Expr::Str(_) | Expr::Empty | Expr::Span(_) => {}
        }
    }

    fn form(&self, f: &Form, out: &mut Vec<LintIssue>) {
        for p in &f.parts {
            match p {
                FormPart::Lit(_) => {}
                FormPart::Str(e, w, _) | FormPart::Int(e, w, _) => {
                    self.expr(e, out);
                    if let Some(w) = w {
                        self.expr(w, out);
                    }
                }
                FormPart::Cond(c) => {
                    self.expr(&c.0, out);
                    self.form(&c.1, out);
                    self.form(&c.2, out);
                }
            }
        }
    }

    fn target(&self, t: &CallTarget, out: &mut Vec<LintIssue>) {
        if let CallTarget::Form(f) = t {
            self.form(f, out);
        }
    }

    fn var(&self, v: &VarRef, out: &mut Vec<LintIssue>) {
        v.idx.iter().for_each(|x| self.expr(x, out));
    }
}

/// A lint issue with structured information
#[derive(Debug, Clone)]
pub struct LintIssue {
    pub code: String,
    pub message: String,
    pub severity: DiagnosticSeverity,
    pub span: Span,
}

/// Diagnostic severity levels matching LSP
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

/// Every string `>=` / `<=`, "file:line: @FUNC: ...".
fn string_compares<'a, 'b>(prog: &'a Program<'b>, out: &mut Vec<LintIssue>) {
    let global_str: HashSet<String> = prog
        .global_dims
        .iter()
        .filter(|d| d.is_str)
        .map(|d| d.name.clone())
        .collect();
    for f in prog.funcs.made() {
        let body = f.body();
        let t = Types {
            prog,
            global_str: global_str.clone(),
            local: body
                .dims
                .iter()
                .map(|d| (d.name.clone(), d.is_str))
                .collect(),
        };
        let mut found = Vec::new();
        let at = |pc: usize| body.lines.get(pc).copied().unwrap_or((f.file, f.line));
        for p in &f.params {
            t.var(&p.target, &mut found);
            if let Some(d) = &p.default {
                t.expr(d, &mut found);
            }
        }
        for d in &body.dims {
            d.dims
                .iter()
                .chain(&d.init)
                .for_each(|x| t.expr(x, &mut found));
        }
        let mut select_str = HashMap::new();
        for (pc, ins) in body.code.iter().enumerate() {
            let exprs =
                |es: &[Expr], found: &mut Vec<LintIssue>| es.iter().for_each(|x| t.expr(x, found));
            match ins {
                Instr::Print(PrintContent::Form(fm), _) | Instr::Throw(fm) => {
                    t.form(fm, &mut found)
                }
                Instr::Print(PrintContent::Expr(e), _)
                | Instr::Eval(e)
                | Instr::JumpIfNot(e, _)
                | Instr::ReturnF(Some(e)) => t.expr(e, &mut found),
                Instr::Print(PrintContent::Exprs(es), _) | Instr::Return(es) => {
                    exprs(es, &mut found)
                }
                Instr::PrintData(fs, _) => {
                    fs.iter().flatten().for_each(|fm| t.form(fm, &mut found))
                }
                Instr::StrData(v, fs) => {
                    if let Some(v) = v {
                        t.var(v, &mut found);
                    }
                    fs.iter().flatten().for_each(|fm| t.form(fm, &mut found));
                }
                Instr::Assign(v, _, es) => {
                    t.var(v, &mut found);
                    exprs(es, &mut found);
                }
                Instr::AssignForm(v, _, fm) => {
                    t.var(v, &mut found);
                    t.form(fm, &mut found);
                }
                Instr::ForBegin(fb) => {
                    t.var(&fb.var, &mut found);
                    [&fb.start, &fb.end]
                        .into_iter()
                        .chain(&fb.step)
                        .for_each(|x| t.expr(x, &mut found));
                }
                Instr::ForNext { var, .. } => t.var(var, &mut found),
                Instr::ForBreak { var, .. } => t.var(var, &mut found),
                Instr::SelectEval(e, slot, _) => {
                    t.expr(e, &mut found);
                    select_str.insert(*slot, t.is_str(e));
                }
                Instr::CaseTest(slot, conds, _) => {
                    for c in conds {
                        match c {
                            CaseCond::Eq(e) => t.expr(e, &mut found),
                            CaseCond::Range(a, b) => {
                                t.expr(a, &mut found);
                                t.expr(b, &mut found);
                            }
                            CaseCond::Is(op, e) => {
                                if matches!(op, BinOp::Ge | BinOp::Le)
                                    && select_str.get(slot) == Some(&true)
                                {
                                    found.push(LintIssue {
                                        code: "string-case-comparison".into(),
                                        message: format!(
                                            "CASE IS {} on a string (Emuera evaluates it as <)",
                                            if *op == BinOp::Ge { ">=" } else { "<=" }
                                        ),
                                        severity: DiagnosticSeverity::Warning,
                                        span: e.span(),
                                    });
                                }
                                t.expr(e, &mut found);
                            }
                        }
                    }
                }
                Instr::Call(c) => {
                    t.target(&c.target, &mut found);
                    exprs(&c.args, &mut found);
                }
                Instr::CallList(es, _) => es.iter().for_each(|(tg, a)| {
                    t.target(tg, &mut found);
                    exprs(a, &mut found);
                }),
                Instr::Goto(g) => t.target(&g.0, &mut found),
                Instr::GotoList(ts) => ts.iter().for_each(|tg| t.target(tg, &mut found)),
                Instr::Command(c) => match &c.1 {
                    Args::Exprs(es) => exprs(es, &mut found),
                    Args::Form(fm) => t.form(fm, &mut found),
                    Args::None | Args::Raw(_) => {}
                },
                Instr::Print(PrintContent::Raw(_), _)
                | Instr::ReturnF(None)
                | Instr::BadLine(..)
                | Instr::Jump(_)
                | Instr::End
                | Instr::Begin(_)
                | Instr::Restart
                | Instr::Quit => {}
            }
        }
    }
}

/// Text a function / label name can't hold.
fn bad_name(s: &str) -> bool {
    s.chars()
        .any(|c| !crate::parser::expr::is_ident_char(c) && c != '@')
}

fn check_target(t: &CallTarget, what: &str, out: &mut Vec<LintIssue>) {
    match t {
        CallTarget::Static(s) if bad_name(s) => out.push(LintIssue {
            code: "malformed-name".into(),
            message: format!("{} name `{}`", what, s),
            severity: DiagnosticSeverity::Error,
            span: Span::empty(),
        }),
        CallTarget::Form(f) => {
            for p in &f.parts {
                if let FormPart::Lit(s) = p {
                    if s.chars()
                        .any(|c| matches!(c, '(' | ')' | ',' | '"' | ' ' | '\t' | '%'))
                    {
                        out.push(LintIssue {
                            code: "malformed-name".into(),
                            message: format!("{} name part `{}`", what, s),
                            severity: DiagnosticSeverity::Error,
                            span: f.span,
                        });
                    }
                }
            }
        }
        _ => {}
    }
}

/// FORM text that still holds `%`, `{` or `}`: a hole the parser could
/// not close (kept as text).
fn check_form(f: &Form, out: &mut Vec<LintIssue>) {
    for p in &f.parts {
        match p {
            FormPart::Lit(s) if s.contains(['%', '{', '}']) => out.push(LintIssue {
                code: "unclosed-form-hole".into(),
                message: format!("FORM text `{}`", s),
                severity: DiagnosticSeverity::Warning,
                span: f.span,
            }),
            FormPart::Cond(c) => {
                check_form(&c.1, out);
                check_form(&c.2, out);
            }
            _ => {}
        }
    }
}

/// Run all lints on a program and return issues
pub fn lint(prog: &Program) -> Vec<LintIssue> {
    let mut out = Vec::new();
    for f in prog.funcs.made() {
        let body = f.body();
        for (pc, ins) in body.code.iter().enumerate() {
            let mut found = Vec::new();
            match ins {
                Instr::Call(c) => check_target(&c.target, "call", &mut found),
                Instr::CallList(es, _) => es
                    .iter()
                    .for_each(|(t, _)| check_target(t, "call list", &mut found)),
                Instr::Goto(g) => check_target(&g.0, "goto", &mut found),
                Instr::GotoList(ts) => ts
                    .iter()
                    .for_each(|t| check_target(t, "goto list", &mut found)),
                Instr::Print(PrintContent::Form(f), _)
                | Instr::AssignForm(_, _, f)
                | Instr::Throw(f) => check_form(f, &mut found),
                Instr::PrintData(es, _) | Instr::StrData(_, es) => {
                    es.iter().flatten().for_each(|f| check_form(f, &mut found))
                }
                Instr::Command(c) => {
                    if let Args::Form(f) = &c.1 {
                        check_form(f, &mut found);
                    }
                }
                _ => {}
            }
            out.extend(found);
        }
    }
    string_compares(prog, &mut out);

    // Sort by file, then line number
    out.sort_by_cached_key(|l| {
        // We'd need file/line info from the span - simplified for now
        l.code.clone()
    });
    out
}

/// Convert lint issues to LSP diagnostics
pub fn lint_to_diagnostics(issues: Vec<LintIssue>, files: &[String]) -> Vec<lsp_types::Diagnostic> {
    issues
        .into_iter()
        .map(|issue| {
            let range = if issue.span.is_empty() {
                lsp_types::Range::default()
            } else {
                // Convert byte span to LSP range (simplified)
                lsp_types::Range {
                    start: lsp_types::Position::new(0, issue.span.start as u32),
                    end: lsp_types::Position::new(0, issue.span.end as u32),
                }
            };

            lsp_types::Diagnostic {
                range,
                severity: Some(lsp_types::DiagnosticSeverity::ERROR),
                code: Some(lsp_types::NumberOrString::String(issue.code)),
                code_description: None,
                source: Some("erabasic-lsp".into()),
                message: issue.message,
                related_information: None,
                tags: None,
                data: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fxhash::HashMap;

    #[test]
    fn test_bad_name() {
        assert!(bad_name("FUNC("));
        assert!(bad_name("FUNC,"));
        assert!(bad_name("FUNC "));
        assert!(bad_name("FUNC\""));
        assert!(!bad_name("FUNC"));
        assert!(!bad_name("FUNC_1"));
        assert!(!bad_name("好感度"));
    }
}
