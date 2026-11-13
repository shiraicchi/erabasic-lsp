//! Completion provider for EraBasic LSP.

use crate::analysis::index::WorkspaceIndex;
use crate::analysis::symbols::{FileSymbolTable, Symbol, SymbolKind};
use crate::analysis::variant::Dialect;
use crate::document::Document;
use crate::parser::builtins::{
    builtin_completions, keyword_completions, COMMANDS, FUNCTIONS, KEYWORDS,
};
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionList, CompletionParams, CompletionResponse,
    Documentation, InsertTextFormat, MarkupContent, MarkupKind, Position, Range, TextEdit,
};
use std::path::Path;

/// Completion provider for EraBasic.
pub struct CompletionProvider {
    workspace_index: WorkspaceIndex,
    dialect: Dialect,
}

impl CompletionProvider {
    pub fn new(workspace_index: WorkspaceIndex, dialect: Dialect) -> Self {
        Self {
            workspace_index,
            dialect,
        }
    }

    /// Provide completions at the given position.
    pub fn provide(&self, params: CompletionParams, document: &Document) -> CompletionResponse {
        let position = params.text_document_position.position;
        let text = document.get_text();

        // Get the word at the cursor
        let prefix = self.get_completion_prefix(&text, position);

        // Determine completion context
        let context = self.determine_context(&text, position, document);

        let mut items = Vec::new();

        match context {
            CompletionContext::FunctionCall => {
                items.extend(self.complete_functions(&prefix));
            }
            CompletionContext::Label => {
                items.extend(self.complete_labels(&prefix, document));
            }
            CompletionContext::Variable => {
                items.extend(self.complete_variables(&prefix, document));
            }
            CompletionContext::Keyword => {
                items.extend(self.complete_keywords(&prefix));
            }
            CompletionContext::Macro => {
                items.extend(self.complete_macros(&prefix, document));
            }
            CompletionContext::General => {
                // Provide everything
                items.extend(self.complete_keywords(&prefix));
                items.extend(self.complete_functions(&prefix));
                items.extend(self.complete_variables(&prefix, document));
                items.extend(self.complete_labels(&prefix, document));
                items.extend(self.complete_macros(&prefix, document));
            }
        }

        CompletionResponse::Array(items)
    }

    /// Get the text prefix at the cursor position.
    fn get_completion_prefix(&self, text: &str, position: Position) -> String {
        let offset = crate::util::position::position_to_offset(
            text,
            position,
            crate::util::position::PositionEncoding::Utf16,
        );
        let before = &text[..offset.min(text.len())];

        // Find the last word boundary
        let mut end = before.len();
        let mut start = end;

        for (i, c) in before.char_indices().rev() {
            if c.is_alphanumeric()
                || c == '_'
                || c == '$'
                || c == '@'
                || c == '#'
                || c == '%'
                || c == ':'
            {
                start = i;
            } else {
                break;
            }
        }

        before[start..end].to_string()
    }

    /// Determine the completion context based on surrounding text.
    fn determine_context(
        &self,
        text: &str,
        position: Position,
        document: &Document,
    ) -> CompletionContext {
        let offset = crate::util::position::position_to_offset(
            text,
            position,
            crate::util::position::PositionEncoding::Utf16,
        );
        let before = &text[..offset.min(text.len())];

        // Check for CALL/JUMP/GOTO context
        let trimmed = before.trim_end();
        let upper = trimmed.to_uppercase();

        if upper.ends_with("CALL ")
            || upper.ends_with("JUMP ")
            || upper.ends_with("GOTO ")
            || upper.ends_with("CALLFORM ")
            || upper.ends_with("JUMPFORM ")
            || upper.ends_with("GOTOFORM ")
        {
            return CompletionContext::FunctionCall;
        }

        // Check for $label context
        if before.ends_with('$') || before.trim_end().ends_with("$") {
            return CompletionContext::Label;
        }

        // Check for variable context (after #DIM, LOCAL, etc.)
        let keywords_before = [
            "DIM",
            "DIMS",
            "VARI",
            "VARS",
            "LOCAL",
            "LOCALS",
            "ARG",
            "ARGS",
            "CONST",
            "STATIC",
            "GLOBAL",
            "SAVEDATA",
            "CHARADATA",
            "REF",
            "DYNAMIC",
        ];
        for kw in keywords_before {
            if upper.ends_with(&format!("{} ", kw)) || upper.ends_with(&format!("#{} ", kw)) {
                return CompletionContext::Variable;
            }
        }

        // Check for macro context
        if upper.ends_with("#DEFINE ") {
            return CompletionContext::Macro;
        }

        // Check if we're at the start of a line (likely keyword)
        let line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line_text = &before[line_start..].trim_start();
        if line_text.is_empty()
            || line_text
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '@' || c == '#')
        {
            return CompletionContext::Keyword;
        }

        CompletionContext::General
    }

    /// Complete keywords.
    fn complete_keywords(&self, prefix: &str) -> Vec<CompletionItem> {
        let upper_prefix = prefix.to_uppercase();
        KEYWORDS
            .iter()
            .filter(|kw| kw.starts_with(&upper_prefix))
            .map(|kw| CompletionItem {
                label: kw.to_string(),
                kind: Some(CompletionItemKind::KEYWORD),
                detail: Some("Keyword".into()),
                documentation: Some(Documentation::MarkupContent(self.keyword_doc(kw))),
                insert_text: Some(kw.to_string()),
                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                ..Default::default()
            })
            .collect()
    }

    /// Complete built-in and user functions.
    fn complete_functions(&self, prefix: &str) -> Vec<CompletionItem> {
        let upper_prefix = prefix.to_uppercase();
        let mut items = Vec::new();

        // Built-in functions
        for func in FUNCTIONS {
            if func.starts_with(&upper_prefix) {
                items.push(CompletionItem {
                    label: func.to_string(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some("Built-in function".into()),
                    documentation: Some(Documentation::MarkupContent(self.function_doc(func))),
                    insert_text: Some(func.to_string()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }

        // Built-in commands
        for cmd in COMMANDS {
            if cmd.starts_with(&upper_prefix) {
                items.push(CompletionItem {
                    label: cmd.to_string(),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some("Built-in command".into()),
                    documentation: Some(Documentation::MarkupContent(self.command_doc(cmd))),
                    insert_text: Some(cmd.to_string()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }

        // User functions from workspace
        for func in self.workspace_index.all_functions() {
            if func.name.to_uppercase().starts_with(&upper_prefix) {
                items.push(CompletionItem {
                    label: func.name.clone(),
                    kind: Some(if func.signature.as_ref().is_some_and(|s| s.is_function) {
                        CompletionItemKind::METHOD
                    } else {
                        CompletionItemKind::FUNCTION
                    }),
                    detail: Some(format!(
                        "{} ({})",
                        if func.signature.as_ref().is_some_and(|s| s.is_function) {
                            "#FUNCTION"
                        } else {
                            "Function"
                        },
                        func.file.display()
                    )),
                    documentation: func.documentation.clone().map(|s| {
                        Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: s,
                        })
                    }),
                    insert_text: Some(func.name.clone()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }

        items
    }

    /// Complete variables.
    fn complete_variables(&self, prefix: &str, document: &Document) -> Vec<CompletionItem> {
        let upper_prefix = prefix.to_uppercase();
        let mut items = Vec::new();

        // Global variables from current document
        if let Some(ast) = &document.ast {
            for dim in &ast.program.global_dims {
                if dim.name.to_uppercase().starts_with(&upper_prefix) {
                    items.push(self.make_var_completion(dim, true));
                }
            }

            // Local variables from functions
            for func_list in ast.program.functions.values() {
                for func in func_list {
                    let body = &func.body;
                    for dim in &body.dims {
                        if dim.name.to_uppercase().starts_with(&upper_prefix) {
                            items.push(self.make_var_completion(dim, false));
                        }
                    }
                }
            }
        }

        // Variables from workspace
        for var in self.workspace_index.all_variables() {
            if var.name.to_uppercase().starts_with(&upper_prefix) {
                items.push(CompletionItem {
                    label: var.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    detail: Some(format!("Variable ({})", var.file.display())),
                    documentation: var.documentation.clone().map(|s| {
                        Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: s,
                        })
                    }),
                    insert_text: Some(var.name.clone()),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }

        items
    }

    fn make_var_completion(
        &self,
        dim: &crate::lsp_ast::DimDecl,
        is_global: bool,
    ) -> CompletionItem {
        CompletionItem {
            label: dim.name.clone(),
            kind: Some(CompletionItemKind::VARIABLE),
            detail: Some(format!(
                "{} {}",
                if is_global { "Global" } else { "Local" },
                if dim.is_str { "Str" } else { "Int" }
            )),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: format!(
                    "{}{} {}",
                    if dim.is_const { "CONST " } else { "" },
                    if dim.is_str { "DIMS" } else { "DIM" },
                    dim.name
                ),
            })),
            insert_text: Some(dim.name.clone()),
            insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
            ..Default::default()
        }
    }

    /// Complete labels.
    fn complete_labels(&self, prefix: &str, document: &Document) -> Vec<CompletionItem> {
        let upper_prefix = prefix.to_uppercase();
        let mut items = Vec::new();

        // Labels from current document
        if let Some(ast) = &document.ast {
            for func_list in ast.program.functions.values() {
                for func in func_list {
                    let body = &func.body;
                    for (label, pc) in &body.labels {
                        if label.to_uppercase().starts_with(&upper_prefix) {
                            let line = body.lines.get(*pc)
                                .and_then(|span| ast.line_map.iter()
                                    .find(|li| li.start_offset <= span.start && span.start < li.end_offset)
                                    .map(|li| li.original_line))
                                .unwrap_or(0);
                            items.push(CompletionItem {
                                label: label.clone(),
                                kind: Some(CompletionItemKind::FIELD),
                                detail: Some(format!("Label in {} (line {})", func.name, line)),
                                documentation: Some(Documentation::MarkupContent(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: format!("Label in function {}", func.name),
                                })),
                                insert_text: Some(format!("${}", label)),
                                insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
        }

        // Labels from workspace
        for label in self.workspace_index.all_labels() {
            if label.name.to_uppercase().starts_with(&upper_prefix) {
                items.push(CompletionItem {
                    label: label.name.clone(),
                    kind: Some(CompletionItemKind::FIELD),
                    detail: Some(format!("Label ({})", label.file.display())),
                    documentation: label.documentation.clone().map(|s| {
                        Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: s,
                        })
                    }),
                    insert_text: Some(format!("${}", label.name)),
                    insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                    ..Default::default()
                });
            }
        }

        items
    }

    /// Complete macros (#DEFINE).
    fn complete_macros(&self, prefix: &str, document: &Document) -> Vec<CompletionItem> {
        let upper_prefix = prefix.to_uppercase();
        let mut items = Vec::new();

        if let Some(ast) = &document.ast {
            for (name, body) in &ast.program.defines {
                if name.to_uppercase().starts_with(&upper_prefix) {
                    items.push(CompletionItem {
                        label: name.clone(),
                        kind: Some(CompletionItemKind::SNIPPET),
                        detail: Some("Macro".into()),
                        documentation: Some(Documentation::MarkupContent(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: format!("#DEFINE {} {}", name, body),
                        })),
                        insert_text: Some(name.clone()),
                        insert_text_format: Some(InsertTextFormat::PLAIN_TEXT),
                        ..Default::default()
                    });
                }
            }
        }

        items
    }

    /// Get documentation for a keyword.
    fn keyword_doc(&self, kw: &str) -> MarkupContent {
        let doc = match kw {
            "IF" => "IF condition ... ELSEIF condition ... ELSE ... ENDIF — Conditional execution",
            "FOR" => "FOR var, start, end[, step] ... NEXT — Loop with counter",
            "WHILE" => "WHILE condition ... WEND — Loop while condition is true",
            "SELECTCASE" => {
                "SELECTCASE value CASE value ... CASEELSE ... ENDSELECT — Multi-way branch"
            }
            "CALL" => "CALL function(args...) — Call a function/subroutine",
            "JUMP" => "JUMP function(args...) — Tail call (does not return)",
            "GOTO" => "GOTO $label — Unconditional jump to label",
            "RETURN" => "RETURN [value...] — Return from function with optional values",
            "DIM" => "#DIM name[, size...] — Declare a global variable",
            "DIMS" => "#DIMS name[, size...] — Declare a global string variable",
            "DEFINE" => "#DEFINE name replacement — Define a text macro",
            _ => "Keyword",
        };
        MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.into(),
        }
    }

    /// Get documentation for a function.
    fn function_doc(&self, name: &str) -> MarkupContent {
        let doc = match name {
            "RAND" => "RAND(max) — Random integer 0 to max-1",
            "ABS" => "ABS(x) — Absolute value",
            "MAX" => "MAX(a, b) — Maximum of two values",
            "MIN" => "MIN(a, b) — Minimum of two values",
            "STRLEN" => "STRLEN(str) — String length in characters",
            "STRLENS" => "STRLENS(str) — String length in Shift_JIS bytes",
            "SUBSTRING" => "SUBSTRING(str, start, len) — Substring by characters",
            "SUBSTRINGU" => "SUBSTRINGU(str, start, len) — Substring by Unicode codepoints",
            "TOSTR" => "TOSTR(value) — Convert to string",
            "TOINT" => "TOINT(str) — Convert string to integer",
            "CSVNAME" => "CSVNAME(no) — Get character name from CSV",
            "CSVBASE" => "CSVBASE(no, field) — Get character base stat",
            "GETCHARA" => "GETCHARA(no) — Check if character exists",
            "GETTIME" => "GETTIME() — Get current game time",
            _ => "Built-in function",
        };
        MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.into(),
        }
    }

    /// Get documentation for a command.
    fn command_doc(&self, name: &str) -> MarkupContent {
        let doc = match name {
            "PRINT" => "PRINT text — Output text to the console",
            "PRINTL" => "PRINTL text — Output text with newline",
            "PRINTV" => "PRINTV expr1, expr2, ... — Output multiple expressions",
            "PRINTS" => "PRINTS expr — Output a single expression",
            "PRINTC" => "PRINTC text — Output text with color",
            "PRINTFORM" => "PRINTFORM @\"form string\" — Output formatted string",
            "IF" => "IF condition ... ENDIF — Conditional block",
            "FOR" => "FOR var, start, end[, step] ... NEXT — For loop",
            "WHILE" => "WHILE condition ... WEND — While loop",
            "SELECTCASE" => "SELECTCASE value ... ENDSELECT — Switch statement",
            "CALL" => "CALL func(args...) — Call a function",
            "JUMP" => "JUMP func(args...) — Tail call a function",
            "GOTO" => "GOTO $label — Jump to a label",
            "RETURN" => "RETURN [value...] — Return from function",
            "DIM" => "#DIM name[, size...] — Declare global variable",
            "DIMS" => "#DIMS name[, size...] — Declare global string variable",
            "DEFINE" => "#DEFINE name body — Define a macro",
            _ => "Built-in command",
        };
        MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.into(),
        }
    }
}

/// Completion context types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionContext {
    General,
    Keyword,
    FunctionCall,
    Variable,
    Label,
    Macro,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::index::WorkspaceIndex;
    use crate::analysis::variant::Dialect;

    #[test]
    fn test_completion_provider() {
        let index = WorkspaceIndex::new();
        let provider = CompletionProvider::new(index, Dialect::EmueraEM);

        // Test prefix extraction
        let text = "PRINT HELLO";
        let pos = Position::new(0, 11); // After "HELLO"
        let prefix = provider.get_completion_prefix(text, pos);
        assert_eq!(prefix, "HELLO");
    }
}
