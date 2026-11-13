//! Signature help provider for EraBasic LSP.

use crate::analysis::index::WorkspaceIndex;
use crate::analysis::symbols::{FunctionSignature, Parameter, Symbol, SymbolKind};
use crate::analysis::variant::Dialect;
use crate::document::Document;
use crate::parser::builtins::{
    is_builtin_command, is_builtin_function, COMMANDS, FUNCTIONS, NOT_IN_FUNCTION,
};
use lsp_types::{
    ParameterInformation, ParameterLabel, Position, SignatureHelp, SignatureHelpParams,
    SignatureInformation,
};

/// Context for a function call.
#[derive(Debug)]
struct CallContext {
    function_name: String,
    args_start: usize,
    current_arg: usize,
}

/// Signature help provider for EraBasic.
pub struct SignatureHelpProvider {
    workspace_index: WorkspaceIndex,
    dialect: Dialect,
    /// Signatures for built-in commands
    builtin_command_sigs: HashMap<String, SignatureInformation>,
    /// Signatures for built-in functions
    builtin_function_sigs: HashMap<String, SignatureInformation>,
}

impl SignatureHelpProvider {
    pub fn new(workspace_index: WorkspaceIndex, dialect: Dialect) -> Self {
        let mut provider = Self {
            workspace_index,
            dialect,
            builtin_command_sigs: HashMap::new(),
            builtin_function_sigs: HashMap::new(),
        };
        provider.init_builtin_signatures();
        provider
    }

    /// Provide signature help at the given position.
    pub fn provide(
        &self,
        params: SignatureHelpParams,
        document: &Document,
    ) -> Option<SignatureHelp> {
        let position = params.text_document_position_params.position;
        let text = document.get_text();

        // Check if we're inside a function call
        let context = self.get_call_context(&text, position)?;

        // Find the function signature
        let signatures = self.get_signatures(&context.function_name);

        if signatures.is_empty() {
            return None;
        }

        // Determine active parameter
        let active_parameter = self.active_parameter(&context, &signatures[0]);

        Some(SignatureHelp {
            signatures,
            active_signature: Some(0),
            active_parameter: Some(active_parameter),
        })
    }

    /// Get the call context at the position.
    fn get_call_context(&self, text: &str, position: Position) -> Option<CallContext> {
        let offset = crate::util::position::position_to_offset(
            text,
            position,
            crate::util::position::PositionEncoding::Utf16,
        );
        let before = &text[..offset.min(text.len())];

        // Look for CALL/JUMP/GOTO or function call pattern
        let upper = before.to_uppercase();

        // Check for CALL/JUMP/GOTO
        for keyword in [
            "CALL ",
            "JUMP ",
            "GOTO ",
            "CALLFORM ",
            "JUMPFORM ",
            "GOTOFORM ",
        ] {
            if let Some(pos) = upper.rfind(keyword) {
                let after_keyword = &before[pos + keyword.len()..];
                let name_end = after_keyword
                    .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
                    .unwrap_or(after_keyword.len());
                let func_name = after_keyword[..name_end].trim().to_uppercase();
                if !func_name.is_empty() {
                    return Some(CallContext {
                        function_name: func_name,
                        args_start: pos + keyword.len() + name_end,
                        current_arg: self.count_args(&before[pos + keyword.len() + name_end..]),
                    });
                }
            }
        }

        // Check for function call: NAME(
        if let Some(paren_pos) = before.rfind('(') {
            let before_paren = &before[..paren_pos];
            let name_start = before_paren
                .rfind(|c: char| !c.is_alphanumeric() && c != '_' && c != '$')
                .map(|i| i + 1)
                .unwrap_or(0);
            let func_name = before_paren[name_start..].trim().to_uppercase();
            if !func_name.is_empty()
                && (is_builtin_function(&func_name)
                    || self.workspace_index.find_symbol(&func_name).is_some())
            {
                return Some(CallContext {
                    function_name: func_name,
                    args_start: paren_pos + 1,
                    current_arg: self.count_args(&before[paren_pos + 1..]),
                });
            }
        }

        None
    }

    /// Count the number of arguments before the cursor.
    fn count_args(&self, text: &str) -> usize {
        let mut count = 0;
        let mut depth = 0;
        let mut in_string = false;
        let mut chars = text.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '"' if !in_string => in_string = true,
                '"' if in_string => in_string = false,
                '(' if !in_string => depth += 1,
                ')' if !in_string => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                ',' if !in_string && depth == 0 => count += 1,
                _ => {}
            }
        }

        if !text.trim().is_empty() {
            count += 1;
        }
        count
    }

    /// Get signatures for a function name.
    fn get_signatures(&self, name: &str) -> Vec<SignatureInformation> {
        let upper = name.to_uppercase();

        // Check built-in commands
        if let Some(sig) = self.builtin_command_sigs.get(&upper) {
            return vec![sig.clone()];
        }

        // Check built-in functions
        if let Some(sig) = self.builtin_function_sigs.get(&upper) {
            return vec![sig.clone()];
        }

        // Check user functions
        if let Some(symbol) = self.workspace_index.find_symbol(&upper) {
            if let Some(sig) = &symbol.signature {
                return vec![self.signature_to_lsp(sig, &symbol.name)];
            }
        }

        Vec::new()
    }

    /// Convert our signature to LSP format.
    fn signature_to_lsp(&self, sig: &FunctionSignature, name: &str) -> SignatureInformation {
        let params: Vec<ParameterInformation> = sig
            .params
            .iter()
            .map(|p| ParameterInformation {
                label: ParameterLabel::Simple(p.name.clone()),
                documentation: Some(lsp_types::Documentation::MarkupContent(
                    lsp_types::MarkupContent {
                        kind: lsp_types::MarkupKind::Markdown,
                        value: var_type_str(&p.var_type),
                    },
                )),
            })
            .collect();

        let label = format!(
            "{}({})",
            name,
            sig.params
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        );

        let documentation = if sig.is_function {
            Some(lsp_types::Documentation::MarkupContent(
                lsp_types::MarkupContent {
                    kind: lsp_types::MarkupKind::Markdown,
                    value: format!(
                        "#FUNCTION {} returns {}",
                        name,
                        sig.return_type
                            .as_ref()
                            .map(var_type_str)
                            .unwrap_or("Void".into())
                    ),
                },
            ))
        } else {
            Some(lsp_types::Documentation::MarkupContent(
                lsp_types::MarkupContent {
                    kind: lsp_types::MarkupKind::Markdown,
                    value: format!(
                        "@{} returns {}",
                        name,
                        sig.return_type
                            .as_ref()
                            .map(var_type_str)
                            .unwrap_or("Void".into())
                    ),
                },
            ))
        };

        SignatureInformation {
            label: label,
            documentation,
            parameters: Some(params),
            active_parameter: None,
        }
    }

    /// Determine the active parameter index.
    fn active_parameter(&self, context: &CallContext, sig: &SignatureInformation) -> u32 {
        let param_count = sig.parameters.as_ref().map(|p| p.len()).unwrap_or(0);
        context.current_arg.min(param_count.saturating_sub(1)) as u32
    }

    /// Initialize built-in signatures.
    fn init_builtin_signatures(&mut self) {
        // Built-in commands with signatures
        let command_sigs: &[(&str, &[&str], &str)] = &[
            ("PRINT", &["text"], "Output text to the console"),
            ("PRINTL", &["text"], "Output text with newline"),
            (
                "PRINTV",
                &["expr1", "expr2", "..."],
                "Output multiple expressions",
            ),
            ("PRINTS", &["expr"], "Output a single expression"),
            ("PRINTC", &["text"], "Output text with color"),
            ("PRINTFORM", &["form"], "Output formatted string"),
            ("IF", &["condition"], "Conditional block"),
            ("FOR", &["var", "start", "end", "step?"], "For loop"),
            ("WHILE", &["condition"], "While loop"),
            ("SELECTCASE", &["value"], "Switch statement"),
            ("CALL", &["func", "args..."], "Call a function"),
            ("JUMP", &["func", "args..."], "Tail call a function"),
            ("GOTO", &["label"], "Jump to a label"),
            ("RETURN", &["values..."], "Return from function"),
            ("DIM", &["name", "size..."], "Declare global variable"),
            (
                "DIMS",
                &["name", "size..."],
                "Declare global string variable",
            ),
            ("DEFINE", &["name", "body"], "Define a macro"),
        ];

        for (name, params, doc) in command_sigs {
            let param_info: Vec<ParameterInformation> = params
                .iter()
                .map(|p| ParameterInformation {
                    label: ParameterLabel::Simple(p.to_string()),
                    documentation: None,
                })
                .collect();

            self.builtin_command_sigs.insert(
                name.to_string(),
                SignatureInformation {
                    label: format!("{}({})", name, params.join(", ")),
                    documentation: Some(lsp_types::Documentation::MarkupContent(
                        lsp_types::MarkupContent {
                            kind: lsp_types::MarkupKind::Markdown,
                            value: doc.to_string(),
                        },
                    )),
                    parameters: Some(param_info),
                    active_parameter: None,
                },
            );
        }

        // Built-in functions with signatures
        let function_sigs: &[(&str, &[&str], &str, &str)] = &[
            ("RAND", &["max"], "Int", "Random integer 0 to max-1"),
            ("ABS", &["x"], "Int", "Absolute value"),
            ("MAX", &["a", "b"], "Int", "Maximum of two values"),
            ("MIN", &["a", "b"], "Int", "Minimum of two values"),
            ("STRLEN", &["str"], "Int", "String length in characters"),
            (
                "STRLENS",
                &["str"],
                "Int",
                "String length in Shift_JIS bytes",
            ),
            (
                "SUBSTRING",
                &["str", "start", "len"],
                "Str",
                "Substring by characters",
            ),
            (
                "SUBSTRINGU",
                &["str", "start", "len"],
                "Str",
                "Substring by Unicode codepoints",
            ),
            ("TOSTR", &["value"], "Str", "Convert to string"),
            ("TOINT", &["str"], "Int", "Convert string to integer"),
            ("CSVNAME", &["no"], "Str", "Get character name from CSV"),
            (
                "CSVBASE",
                &["no", "field"],
                "Int",
                "Get character base stat",
            ),
            ("GETCHARA", &["no"], "Int", "Check if character exists"),
            ("GETTIME", &[], "Int", "Get current game time"),
        ];

        for (name, params, ret, doc) in function_sigs {
            let param_info: Vec<ParameterInformation> = params
                .iter()
                .map(|p| ParameterInformation {
                    label: ParameterLabel::Simple(p.to_string()),
                    documentation: None,
                })
                .collect();

            self.builtin_function_sigs.insert(
                name.to_string(),
                SignatureInformation {
                    label: format!("{}({}) -> {}", name, params.join(", "), ret),
                    documentation: Some(lsp_types::Documentation::MarkupContent(
                        lsp_types::MarkupContent {
                            kind: lsp_types::MarkupKind::Markdown,
                            value: doc.to_string(),
                        },
                    )),
                    parameters: Some(param_info),
                    active_parameter: None,
                },
            );
        }
    }
}

use std::collections::HashMap;

fn var_type_str(var_type: &crate::analysis::symbols::VarType) -> String {
    use crate::analysis::symbols::VarType;
    match var_type {
        VarType::Int => "Int".into(),
        VarType::Str => "Str".into(),
        VarType::IntArray(dims) => format!(
            "Int[{}]",
            dims.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("][")
        ),
        VarType::StrArray(dims) => format!(
            "Str[{}]",
            dims.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("][")
        ),
        VarType::Chara => "Chara".into(),
        VarType::CharaArray(dims) => format!(
            "Chara[{}]",
            dims.iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("][")
        ),
        VarType::Unknown => "Unknown".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::index::WorkspaceIndex;
    use crate::analysis::variant::Dialect;

    #[test]
    fn test_signature_help() {
        let index = WorkspaceIndex::new();
        let provider = SignatureHelpProvider::new(index, Dialect::EmueraEM);

        // Test call context detection
        let text = "CALL MYFUNC(1, 2";
        let pos = Position::new(0, text.len() as u32);
        let context = provider.get_call_context(text, pos);
        assert!(context.is_some());
        assert_eq!(context.unwrap().function_name, "MYFUNC");
    }
}
