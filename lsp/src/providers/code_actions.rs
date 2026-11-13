//! Code actions provider for EraBasic LSP (quick fixes).

use crate::document::Document;
use crate::parser::lint::{lint, DiagnosticSeverity, LintIssue};
use lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, CodeActionParams, Diagnostic, Position, Range,
    TextEdit, WorkspaceEdit,
};
use std::collections::HashMap;

/// Code actions provider for EraBasic.
pub struct CodeActionProvider;

impl CodeActionProvider {
    pub fn new() -> Self {
        Self
    }

    /// Provide code actions for the given range.
    pub fn provide(
        &self,
        params: CodeActionParams,
        document: &Document,
    ) -> Vec<CodeActionOrCommand> {
        let mut actions = Vec::new();

        // Get diagnostics for this range
        let diagnostics = self.get_diagnostics_for_range(document, &params.range);

        for diagnostic in diagnostics {
            // String comparison fix
            if matches!(diagnostic.code.as_ref(), Some(lsp_types::NumberOrString::String(s)) if s == "string-comparison")
            {
                actions.push(self.fix_string_comparison(&diagnostic, document));
            }

            // Malformed name fix
            if matches!(diagnostic.code.as_ref(), Some(lsp_types::NumberOrString::String(s)) if s == "malformed-name")
            {
                actions.push(self.fix_malformed_name(&diagnostic, document));
            }

            // Unclosed form hole fix
            if matches!(diagnostic.code.as_ref(), Some(lsp_types::NumberOrString::String(s)) if s == "unclosed-form-hole")
            {
                actions.push(self.fix_unclosed_form_hole(&diagnostic, document));
            }
        }

        // Add missing ENDIF
        actions.extend(self.add_missing_endif(document, &params.range));

        // Add missing NEXT
        actions.extend(self.add_missing_next(document, &params.range));

        // Add missing WEND
        actions.extend(self.add_missing_wend(document, &params.range));

        // Add missing ENDSELECT
        actions.extend(self.add_missing_endselect(document, &params.range));

        actions
            .into_iter()
            .map(CodeActionOrCommand::CodeAction)
            .collect()
    }

    /// Get diagnostics that overlap with the given range.
    fn get_diagnostics_for_range(&self, document: &Document, range: &Range) -> Vec<Diagnostic> {
        // In a real implementation, this would use the document's parse errors
        // For now, return empty
        Vec::new()
    }

    /// Fix string comparison (>= / <= on strings).
    fn fix_string_comparison(&self, diagnostic: &Diagnostic, document: &Document) -> CodeAction {
        let range = diagnostic.range;
        let text = document.get_range_text(range).unwrap_or_default();

        let (fixed, replacement) = if text.contains(">=") {
            (text.replace(">=", ">"), ">")
        } else if text.contains("<=") {
            (text.replace("<=", "<"), "<")
        } else {
            (text, "")
        };

        CodeAction {
            title: format!("Fix string comparison: use {} instead", replacement),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some({
                    let mut map = HashMap::new();
                    map.insert(
                        document.uri.clone(),
                        vec![TextEdit {
                            range,
                            new_text: fixed,
                        }],
                    );
                    map
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Fix malformed name.
    fn fix_malformed_name(&self, diagnostic: &Diagnostic, document: &Document) -> CodeAction {
        CodeAction {
            title: "Fix malformed name".into(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            // Would need to provide specific fix based on the issue
            ..Default::default()
        }
    }

    /// Fix unclosed form hole.
    fn fix_unclosed_form_hole(&self, diagnostic: &Diagnostic, document: &Document) -> CodeAction {
        CodeAction {
            title: "Close FORM hole".into(),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            ..Default::default()
        }
    }

    /// Add missing ENDIF.
    fn add_missing_endif(&self, document: &Document, range: &Range) -> Vec<CodeAction> {
        let text = document.get_text();
        let mut actions = Vec::new();

        // Count IF/ELSEIF/ELSE vs ENDIF
        let mut if_count = 0;
        let mut endif_count = 0;

        for line in text.lines() {
            let upper = line.trim().to_uppercase();
            if upper.starts_with("IF ") || upper == "IF" {
                if_count += 1;
            } else if upper.starts_with("ELSEIF ") || upper == "ELSEIF" {
                // ELSEIF doesn't change count
            } else if upper == "ELSE" {
                // ELSE doesn't change count
            } else if upper == "ENDIF" {
                endif_count += 1;
            }
        }

        if if_count > endif_count {
            let missing = if_count - endif_count;
            for _ in 0..missing {
                actions.push(CodeAction {
                    title: "Add missing ENDIF".into(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: None,
                    edit: Some(WorkspaceEdit {
                        changes: Some({
                            let mut map = HashMap::new();
                            let last_line = text.lines().count() as u32;
                            map.insert(
                                document.uri.clone(),
                                vec![TextEdit {
                                    range: Range::new(
                                        Position::new(last_line, 0),
                                        Position::new(last_line, 0),
                                    ),
                                    new_text: "\nENDIF".into(),
                                }],
                            );
                            map
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
        }

        actions
    }

    /// Add missing NEXT.
    fn add_missing_next(&self, document: &Document, range: &Range) -> Vec<CodeAction> {
        let text = document.get_text();
        let mut actions = Vec::new();

        let mut for_count = 0;
        let mut next_count = 0;

        for line in text.lines() {
            let upper = line.trim().to_uppercase();
            if upper.starts_with("FOR ") {
                for_count += 1;
            } else if upper == "NEXT" {
                next_count += 1;
            }
        }

        if for_count > next_count {
            let missing = for_count - next_count;
            for _ in 0..missing {
                actions.push(CodeAction {
                    title: "Add missing NEXT".into(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: None,
                    edit: Some(WorkspaceEdit {
                        changes: Some({
                            let mut map = HashMap::new();
                            let last_line = text.lines().count() as u32;
                            map.insert(
                                document.uri.clone(),
                                vec![TextEdit {
                                    range: Range::new(
                                        Position::new(last_line, 0),
                                        Position::new(last_line, 0),
                                    ),
                                    new_text: "\nNEXT".into(),
                                }],
                            );
                            map
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
        }

        actions
    }

    /// Add missing WEND.
    fn add_missing_wend(&self, document: &Document, range: &Range) -> Vec<CodeAction> {
        let text = document.get_text();
        let mut actions = Vec::new();

        let mut while_count = 0;
        let mut wend_count = 0;

        for line in text.lines() {
            let upper = line.trim().to_uppercase();
            if upper.starts_with("WHILE ") {
                while_count += 1;
            } else if upper == "WEND" {
                wend_count += 1;
            }
        }

        if while_count > wend_count {
            let missing = while_count - wend_count;
            for _ in 0..missing {
                actions.push(CodeAction {
                    title: "Add missing WEND".into(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: None,
                    edit: Some(WorkspaceEdit {
                        changes: Some({
                            let mut map = HashMap::new();
                            let last_line = text.lines().count() as u32;
                            map.insert(
                                document.uri.clone(),
                                vec![TextEdit {
                                    range: Range::new(
                                        Position::new(last_line, 0),
                                        Position::new(last_line, 0),
                                    ),
                                    new_text: "\nWEND".into(),
                                }],
                            );
                            map
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
        }

        actions
    }

    /// Add missing ENDSELECT.
    fn add_missing_endselect(&self, document: &Document, range: &Range) -> Vec<CodeAction> {
        let text = document.get_text();
        let mut actions = Vec::new();

        let mut select_count = 0;
        let mut endselect_count = 0;

        for line in text.lines() {
            let upper = line.trim().to_uppercase();
            if upper.starts_with("SELECTCASE ") {
                select_count += 1;
            } else if upper == "ENDSELECT" {
                endselect_count += 1;
            }
        }

        if select_count > endselect_count {
            let missing = select_count - endselect_count;
            for _ in 0..missing {
                actions.push(CodeAction {
                    title: "Add missing ENDSELECT".into(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: None,
                    edit: Some(WorkspaceEdit {
                        changes: Some({
                            let mut map = HashMap::new();
                            let last_line = text.lines().count() as u32;
                            map.insert(
                                document.uri.clone(),
                                vec![TextEdit {
                                    range: Range::new(
                                        Position::new(last_line, 0),
                                        Position::new(last_line, 0),
                                    ),
                                    new_text: "\nENDSELECT".into(),
                                }],
                            );
                            map
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                });
            }
        }

        actions
    }
}

impl Default for CodeActionProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_actions() {
        let provider = CodeActionProvider::new();
        // Would need document with diagnostics to test
    }
}
