//! Semantic tokens provider for EraBasic LSP (syntax highlighting).

use crate::document::Document;
use crate::parser::ast::{
    Args, BinOp, CallInstr, CallTarget, CaseCond, ForBegin, FormPart, Instr, PrintContent, UnOp,
};
use lsp_types::{
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens, SemanticTokensLegend,
    SemanticTokensParams, SemanticTokensResult,
};
use std::collections::HashMap;

/// A token found during tokenization.
#[derive(Debug)]
struct Token {
    line: u32,
    start: u32,
    length: u32,
    token_type: u32,
    modifiers: u32,
}

/// Semantic tokens provider for EraBasic.
#[derive(Debug)]
pub struct SemanticTokensProvider {
    legend: SemanticTokensLegend,
    token_type_map: HashMap<String, u32>,
}

impl SemanticTokensProvider {
    pub fn new() -> Self {
        // Define token types
        let token_types = vec![
            SemanticTokenType::KEYWORD,   // 0: IF, FOR, WHILE, etc.
            SemanticTokenType::FUNCTION,  // 1: Function calls
            SemanticTokenType::METHOD,    // 2: #FUNCTION
            SemanticTokenType::VARIABLE,  // 3: Variables
            SemanticTokenType::PARAMETER, // 4: Function parameters
            SemanticTokenType::STRING,    // 5: String literals
            SemanticTokenType::NUMBER,    // 6: Numeric literals
            SemanticTokenType::COMMENT,   // 7: Comments
            SemanticTokenType::MACRO,     // 8: #DEFINE
            SemanticTokenType::VARIABLE,  // 9: $LABEL (use VARIABLE)
            SemanticTokenType::TYPE,      // 10: Type names
            SemanticTokenType::OPERATOR,  // 11: Operators
            SemanticTokenType::NAMESPACE, // 12: Modules/namespaces
        ];

        let token_modifiers = vec![
            SemanticTokenModifier::DECLARATION,
            SemanticTokenModifier::DEFINITION,
            SemanticTokenModifier::READONLY,
            SemanticTokenModifier::STATIC,
            SemanticTokenModifier::ASYNC,
        ];

        let legend = SemanticTokensLegend {
            token_types,
            token_modifiers,
        };

        let mut token_type_map = HashMap::new();
        token_type_map.insert("keyword".to_string(), 0);
        token_type_map.insert("function".to_string(), 1);
        token_type_map.insert("method".to_string(), 2);
        token_type_map.insert("variable".to_string(), 3);
        token_type_map.insert("parameter".to_string(), 4);
        token_type_map.insert("string".to_string(), 5);
        token_type_map.insert("number".to_string(), 6);
        token_type_map.insert("comment".to_string(), 7);
        token_type_map.insert("macro".to_string(), 8);
        token_type_map.insert("label".to_string(), 9);
        token_type_map.insert("type".to_string(), 10);
        token_type_map.insert("operator".to_string(), 11);
        token_type_map.insert("namespace".to_string(), 12);

        Self {
            legend,
            token_type_map,
        }
    }

    /// Get the semantic tokens legend.
    pub fn legend(&self) -> &SemanticTokensLegend {
        &self.legend
    }

    /// Provide semantic tokens for a document.
    pub fn provide(
        &self,
        params: SemanticTokensParams,
        document: &Document,
    ) -> SemanticTokensResult {
        let text = document.get_text();
        let mut tokens = Vec::new();
        let mut last_line = 0;
        let mut last_start = 0;

        // Simple approach: tokenize line by line
        let lines: Vec<&str> = text.lines().collect();

        for (line_idx, line) in lines.iter().enumerate() {
            let line_num = line_idx as u32;
            let tokens_in_line = self.tokenize_line(line, line_num);

            for token in tokens_in_line {
                let delta_line = token.line.saturating_sub(last_line);
                let delta_start = if delta_line == 0 {
                    token.start.saturating_sub(last_start)
                } else {
                    token.start
                };

                tokens.push(SemanticToken {
                    delta_line,
                    delta_start,
                    length: token.length,
                    token_type: token.token_type,
                    token_modifiers_bitset: token.modifiers,
                });

                last_line = token.line;
                last_start = token.start;
            }
        }

        SemanticTokensResult::Tokens(SemanticTokens {
            data: tokens,
            result_id: None,
        })
    }

    /// Tokenize a single line.
    fn tokenize_line(&self, line: &str, line_num: u32) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut chars = line.chars().peekable();
        let mut col = 0;

        while let Some(c) = chars.next() {
            match c {
                // Skip whitespace
                ' ' | '\t' => {
                    col += 1;
                }

                // Comments: ; or //
                ';' => {
                    let rest: String = chars.collect();
                    let len = rest.len() + 1;
                    tokens.push(Token {
                        line: line_num,
                        start: col,
                        length: len as u32,
                        token_type: 7, // comment
                        modifiers: 0,
                    });
                    break;
                }
                '/' => {
                    if let Some(&'/') = chars.peek() {
                        chars.next();
                        let rest: String = chars.collect();
                        let len = rest.len() + 2;
                        tokens.push(Token {
                            line: line_num,
                            start: col,
                            length: len as u32,
                            token_type: 7, // comment
                            modifiers: 0,
                        });
                        break;
                    }
                    // Operator
                    tokens.push(Token {
                        line: line_num,
                        start: col,
                        length: 1,
                        token_type: 11, // operator
                        modifiers: 0,
                    });
                    col += 1;
                }

                // Strings
                '"' => {
                    let mut len = 1;
                    let mut escaped = false;
                    for c2 in chars.by_ref() {
                        len += 1;
                        if escaped {
                            escaped = false;
                        } else if c2 == '\\' {
                            escaped = true;
                        } else if c2 == '"' {
                            break;
                        }
                    }
                    tokens.push(Token {
                        line: line_num,
                        start: col,
                        length: len as u32,
                        token_type: 5, // string
                        modifiers: 0,
                    });
                    col += len as u32;
                }

                // Numbers
                c if c.is_ascii_digit() => {
                    let mut len = 1;
                    let mut is_hex = false;
                    if c == '0' {
                        if let Some(&'x') | Some(&'X') = chars.peek() {
                            is_hex = true;
                            chars.next();
                            len += 1;
                        }
                    }
                    while let Some(&c2) = chars.peek() {
                        if c2.is_ascii_digit() || (is_hex && c2.is_ascii_hexdigit()) {
                            chars.next();
                            len += 1;
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token {
                        line: line_num,
                        start: col,
                        length: len as u32,
                        token_type: 6, // number
                        modifiers: 0,
                    });
                    col += len as u32;
                }

                // Identifiers (keywords, functions, variables, labels, macros)
                c if c.is_alphabetic() || c == '_' || c == '@' || c == '#' || c == '$' => {
                    let mut ident = String::new();
                    ident.push(c);
                    while let Some(&c2) = chars.peek() {
                        if c2.is_alphanumeric() || c2 == '_' || c2 == '$' {
                            ident.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    let len = ident.len();
                    let upper = ident.to_uppercase();

                    // Determine token type
                    let token_type = if upper.starts_with('@') || upper.starts_with('#') {
                        if crate::parser::builtins::NOT_IN_FUNCTION
                            .contains(&upper.trim_start_matches('@').trim_start_matches('#'))
                        {
                            0 // keyword
                        } else if upper.starts_with('#') {
                            8 // macro/preprocessor
                        } else {
                            2 // method (#FUNCTION)
                        }
                    } else if upper.starts_with('$') {
                        9 // label
                    } else if crate::parser::builtins::KEYWORDS.contains(&upper.as_ref()) {
                        0 // keyword
                    } else if crate::parser::builtins::is_builtin_command(&upper) {
                        0 // keyword (commands are keywords)
                    } else if crate::parser::builtins::is_builtin_function(&upper) {
                        1 // function
                    } else {
                        3 // variable
                    };

                    tokens.push(Token {
                        line: line_num,
                        start: col,
                        length: len as u32,
                        token_type,
                        modifiers: 0,
                    });
                    col += len as u32;
                }

                // Operators
                '+' | '-' | '*' | '/' | '%' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '?'
                | ':' | ',' | '(' | ')' | '[' | ']' | '{' | '}' => {
                    // Check for multi-char operators
                    let mut op = String::new();
                    op.push(c);
                    if let Some(&c2) = chars.peek() {
                        let two_char = format!("{}{}", c, c2);
                        if matches!(
                            two_char.as_str(),
                            ">=" | "<="
                                | "!="
                                | "=="
                                | "&&"
                                | "||"
                                | "++"
                                | "--"
                                | "+="
                                | "-="
                                | "*="
                                | "/="
                                | "%="
                                | "&="
                                | "|="
                                | "^="
                                | "<<="
                                | ">>="
                                | "!&"
                                | "!|"
                                | "^^"
                        ) {
                            op.push(chars.next().unwrap());
                        }
                    }
                    let len = op.len();
                    tokens.push(Token {
                        line: line_num,
                        start: col,
                        length: len as u32,
                        token_type: 11, // operator
                        modifiers: 0,
                    });
                    col += len as u32;
                }

                _ => {
                    col += 1;
                }
            }
        }

        tokens
    }
}

impl Default for SemanticTokensProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_tokens() {
        let provider = SemanticTokensProvider::new();

        let line = "PRINT \"hello\"";
        let tokens = provider.tokenize_line(line, 0);
        assert!(!tokens.is_empty());

        // Check PRINT is keyword
        let print_token = tokens.iter().find(|t| t.start == 0).unwrap();
        assert_eq!(print_token.token_type, 0); // keyword
    }
}
