//! Built-in function documentation for EraBasic LSP.

use crate::parser::builtins::{
    is_builtin_function, returns_str, COMMANDS, FUNCTIONS, STR_FUNCTIONS,
};
use std::collections::HashMap;

/// Documentation for built-in commands and functions.
#[derive(Debug, Clone)]
pub struct BuiltinDocs {
    docs: HashMap<String, BuiltinDoc>,
}

#[derive(Debug, Clone)]
pub struct BuiltinDoc {
    pub name: String,
    pub is_command: bool,
    pub summary: String,
    pub syntax: String,
    pub parameters: Vec<ParamDoc>,
    pub return_type: Option<String>,
    pub return_description: Option<String>,
    pub description: String,
    pub examples: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ParamDoc {
    pub name: String,
    pub type_: String,
    pub description: String,
    pub optional: bool,
}

impl BuiltinDocs {
    /// Create a new builtin documentation provider.
    pub fn new() -> Self {
        let mut docs = Self {
            docs: HashMap::new(),
        };
        docs.init();
        docs
    }

    /// Initialize with all known built-ins.
    fn init(&mut self) {
        // Commands (includes print variants)
        for cmd in COMMANDS {
            self.docs.insert(cmd.to_string(), self.command_doc(cmd));
        }

        // Functions
        for func in FUNCTIONS {
            self.docs.insert(func.to_string(), self.function_doc(func));
        }
    }

    /// Create documentation for a command.
    fn command_doc(&self, name: &str) -> BuiltinDoc {
        let (summary, syntax, params, description, examples) = match name {
            "PRINT" => (
                "Output text to the console",
                "PRINT text",
                vec![ParamDoc { name: "text".into(), type_: "string".into(), description: "Text to output".into(), optional: false }],
                "Outputs the given text to the game console without a trailing newline.",
                vec!["PRINT \"Hello World\"".into()],
            ),
            "PRINTL" => (
                "Output text with newline",
                "PRINTL text",
                vec![ParamDoc { name: "text".into(), type_: "string".into(), description: "Text to output".into(), optional: false }],
                "Outputs the given text followed by a newline.",
                vec!["PRINTL \"Line 1\"\nPRINTL \"Line 2\"".into()],
            ),
            "PRINTV" => (
                "Output multiple expressions",
                "PRINTV expr1, expr2, ...",
                vec![ParamDoc { name: "expr1".into(), type_: "any".into(), description: "First expression".into(), optional: false },
                     ParamDoc { name: "expr2".into(), type_: "any".into(), description: "Second expression".into(), optional: true }],
                "Outputs multiple expressions concatenated together.",
                vec!["PRINTV \"Value: \", X, \" Y: \", Y".into()],
            ),
            "PRINTS" => (
                "Output a single expression",
                "PRINTS expr",
                vec![ParamDoc { name: "expr".into(), type_: "any".into(), description: "Expression to output".into(), optional: false }],
                "Outputs a single expression (variable or function result).",
                vec!["PRINTS X".into()],
            ),
            "PRINTC" => (
                "Output text with color",
                "PRINTC text",
                vec![ParamDoc { name: "text".into(), type_: "string".into(), description: "Colored text".into(), optional: false }],
                "Outputs text with the current SETCOLOR applied.",
                vec!["SETCOLOR 255,0,0\nPRINTC \"Red text\"".into()],
            ),
            "PRINTFORM" => (
                "Output formatted string",
                "PRINTFORM @\"form string\"",
                vec![ParamDoc { name: "form".into(), type_: "form".into(), description: "FORM string with holes".into(), optional: false }],
                "Outputs a FORM string with interpolated values.",
                vec!["PRINTFORM @\"Name: %NAME%\"".into()],
            ),
            "IF" => (
                "Conditional block",
                "IF condition ... [ELSEIF condition ...] [ELSE ...] ENDIF",
                vec![ParamDoc { name: "condition".into(), type_: "int".into(), description: "Condition to test (non-zero = true)".into(), optional: false }],
                "Executes the following block if condition is true. Supports ELSEIF and ELSE branches.",
                vec!["IF X >= 10\n  PRINTL \"X is large\"\nELSE\n  PRINTL \"X is small\"\nENDIF".into()],
            ),
            "FOR" => (
                "For loop",
                "FOR var, start, end[, step] ... NEXT",
                vec![
                    ParamDoc { name: "var".into(), type_: "int".into(), description: "Loop variable".into(), optional: false },
                    ParamDoc { name: "start".into(), type_: "int".into(), description: "Starting value".into(), optional: false },
                    ParamDoc { name: "end".into(), type_: "int".into(), description: "Ending value".into(), optional: false },
                    ParamDoc { name: "step".into(), type_: "int".into(), description: "Step (default 1)".into(), optional: true },
                ],
                "Loops from start to end, incrementing by step each iteration.",
                vec!["FOR I, 1, 10\n  PRINTL I\nNEXT".into()],
            ),
            "WHILE" => (
                "While loop",
                "WHILE condition ... WEND",
                vec![ParamDoc { name: "condition".into(), type_: "int".into(), description: "Loop condition".into(), optional: false }],
                "Repeats the block while condition is true.",
                vec!["WHILE X > 0\n  X = X - 1\nWEND".into()],
            ),
            "SELECTCASE" => (
                "Switch statement",
                "SELECTCASE value CASE val ... CASEELSE ... ENDSELECT",
                vec![ParamDoc { name: "value".into(), type_: "any".into(), description: "Value to test".into(), optional: false }],
                "Multi-way branch based on the value. Supports literal matching, ranges, and IS comparisons.",
                vec!["SELECTCASE X\nCASE 1\n  PRINTL \"One\"\nCASE 2 TO 5\n  PRINTL \"Two to five\"\nCASE IS >= 10\n  PRINTL \"Ten or more\"\nCASEELSE\n  PRINTL \"Other\"\nENDSELECT".into()],
            ),
            "CALL" => (
                "Call a function",
                "CALL func(args...)",
                vec![
                    ParamDoc { name: "func".into(), type_: "string".into(), description: "Function name".into(), optional: false },
                    ParamDoc { name: "args".into(), type_: "any".into(), description: "Arguments".into(), optional: true },
                ],
                "Calls a function and returns to the next line after it completes.",
                vec!["CALL MYFUNC(1, 2)".into()],
            ),
            "JUMP" => (
                "Tail call a function",
                "JUMP func(args...)",
                vec![
                    ParamDoc { name: "func".into(), type_: "string".into(), description: "Function name".into(), optional: false },
                    ParamDoc { name: "args".into(), type_: "any".into(), description: "Arguments".into(), optional: true },
                ],
                "Calls a function but does not return - the current function also returns.",
                vec!["JUMP NEXT_SCENE()".into()],
            ),
            "GOTO" => (
                "Jump to label",
                "GOTO $label",
                vec![ParamDoc { name: "label".into(), type_: "label".into(), description: "Label to jump to".into(), optional: false }],
                "Unconditionally jumps to the specified label within the same function.",
                vec!["GOTO $LOOP_START".into()],
            ),
            "RETURN" => (
                "Return from function",
                "RETURN [value...]",
                vec![ParamDoc { name: "value".into(), type_: "any".into(), description: "Return values".into(), optional: true }],
                "Returns from the current function with optional return values placed in RESULT:0, RESULT:1, etc.",
                vec!["RETURN X, Y".into()],
            ),
            "DIM" => (
                "Declare global integer variable",
                "#DIM name[, size...]",
                vec![
                    ParamDoc { name: "name".into(), type_: "string".into(), description: "Variable name".into(), optional: false },
                    ParamDoc { name: "size".into(), type_: "int".into(), description: "Array dimensions".into(), optional: true },
                ],
                "Declares a global integer variable or array. Persists across function calls unless RESETDATA.",
                vec!["#DIM MYVAR\n#DIM MYARRAY, 10, 10".into()],
            ),
            "DIMS" => (
                "Declare global string variable",
                "#DIMS name[, size...]",
                vec![
                    ParamDoc { name: "name".into(), type_: "string".into(), description: "Variable name".into(), optional: false },
                    ParamDoc { name: "size".into(), type_: "int".into(), description: "Array dimensions".into(), optional: true },
                ],
                "Declares a global string variable or array.",
                vec!["#DIMS MYSTR\n#DIMS MYSTRARRAY, 5".into()],
            ),
            "DEFINE" => (
                "Define a macro",
                "#DEFINE name replacement",
                vec![
                    ParamDoc { name: "name".into(), type_: "string".into(), description: "Macro name".into(), optional: false },
                    ParamDoc { name: "replacement".into(), type_: "string".into(), description: "Replacement text".into(), optional: false },
                ],
                "Defines a text macro that is expanded in expressions and statements.",
                vec!["#DEFINE MAXHP 100\nIF HP >= MAXHP".into()],
            ),
            _ => (
                "Built-in command",
                name,
                vec![],
                "No documentation available.",
                vec![],
            ),
        };

        BuiltinDoc {
            name: name.to_string(),
            is_command: true,
            summary: summary.into(),
            syntax: syntax.into(),
            parameters: params,
            return_type: None,
            return_description: None,
            description: description.into(),
            examples,
        }
    }

    /// Create documentation for a function.
    fn function_doc(&self, name: &str) -> BuiltinDoc {
        let is_str = STR_FUNCTIONS.contains(&name);
        let return_type = if is_str {
            Some("string".into())
        } else {
            Some("int".into())
        };

        let (summary, syntax, params, description, examples) = match name {
            "RAND" => (
                "Random integer",
                "RAND(max)",
                vec![ParamDoc { name: "max".into(), type_: "int".into(), description: "Upper bound (exclusive)".into(), optional: false }],
                "Returns a random integer from 0 to max-1.",
                vec!["X = RAND(10)  // 0-9".into()],
            ),
            "ABS" => (
                "Absolute value",
                "ABS(x)",
                vec![ParamDoc { name: "x".into(), type_: "int".into(), description: "Value".into(), optional: false }],
                "Returns the absolute value of x.",
                vec!["X = ABS(-5)  // 5".into()],
            ),
            "MAX" => (
                "Maximum of two values",
                "MAX(a, b)",
                vec![
                    ParamDoc { name: "a".into(), type_: "int".into(), description: "First value".into(), optional: false },
                    ParamDoc { name: "b".into(), type_: "int".into(), description: "Second value".into(), optional: false },
                ],
                "Returns the larger of a and b.",
                vec!["X = MAX(A, B)".into()],
            ),
            "MIN" => (
                "Minimum of two values",
                "MIN(a, b)",
                vec![
                    ParamDoc { name: "a".into(), type_: "int".into(), description: "First value".into(), optional: false },
                    ParamDoc { name: "b".into(), type_: "int".into(), description: "Second value".into(), optional: false },
                ],
                "Returns the smaller of a and b.",
                vec!["X = MIN(A, B)".into()],
            ),
            "STRLEN" => (
                "String length in characters",
                "STRLEN(str)",
                vec![ParamDoc { name: "str".into(), type_: "string".into(), description: "Input string".into(), optional: false }],
                "Returns the number of characters in the string.",
                vec!["X = STRLEN(\"abc\")  // 3".into()],
            ),
            "STRLENS" => (
                "String length in Shift_JIS bytes",
                "STRLENS(str)",
                vec![ParamDoc { name: "str".into(), type_: "string".into(), description: "Input string".into(), optional: false }],
                "Returns the byte length of the string in Shift_JIS encoding.",
                vec!["X = STRLENS(\"あ\")  // 2".into()],
            ),
            "SUBSTRING" => (
                "Substring by characters",
                "SUBSTRING(str, start, len)",
                vec![
                    ParamDoc { name: "str".into(), type_: "string".into(), description: "Input string".into(), optional: false },
                    ParamDoc { name: "start".into(), type_: "int".into(), description: "Start position (0-indexed)".into(), optional: false },
                    ParamDoc { name: "len".into(), type_: "int".into(), description: "Length".into(), optional: false },
                ],
                "Returns a substring of the given length starting at the given position (character-based).",
                vec!["X = SUBSTRING(\"hello\", 1, 3)  // \"ell\"".into()],
            ),
            "SUBSTRINGU" => (
                "Substring by Unicode codepoints",
                "SUBSTRINGU(str, start, len)",
                vec![
                    ParamDoc { name: "str".into(), type_: "string".into(), description: "Input string".into(), optional: false },
                    ParamDoc { name: "start".into(), type_: "int".into(), description: "Start position".into(), optional: false },
                    ParamDoc { name: "len".into(), type_: "int".into(), description: "Length".into(), optional: false },
                ],
                "Returns a substring by Unicode codepoints (not bytes).",
                vec!["X = SUBSTRINGU(\"あいう\", 1, 1)  // \"い\"".into()],
            ),
            "TOSTR" => (
                "Convert to string",
                "TOSTR(value)",
                vec![ParamDoc { name: "value".into(), type_: "any".into(), description: "Value to convert".into(), optional: false }],
                "Converts any value to its string representation.",
                vec!["X = TOSTR(123)  // \"123\"".into()],
            ),
            "TOINT" => (
                "Convert string to integer",
                "TOINT(str)",
                vec![ParamDoc { name: "str".into(), type_: "string".into(), description: "String to parse".into(), optional: false }],
                "Parses a string as an integer. Returns 0 if not a valid number.",
                vec!["X = TOINT(\"456\")  // 456".into()],
            ),
            "CSVNAME" => (
                "Get character name from CSV",
                "CSVNAME(no)",
                vec![ParamDoc { name: "no".into(), type_: "int".into(), description: "Character index".into(), optional: false }],
                "Returns the name of the character at the given index from the CSV data.",
                vec!["X = CSVNAME(0)".into()],
            ),
            "CSVBASE" => (
                "Get character base stat",
                "CSVBASE(no, field)",
                vec![
                    ParamDoc { name: "no".into(), type_: "int".into(), description: "Character index".into(), optional: false },
                    ParamDoc { name: "field".into(), type_: "int".into(), description: "Stat field index".into(), optional: false },
                ],
                "Returns a base stat for the character from CSV data.",
                vec!["X = CSVBASE(0, 1)".into()],
            ),
            "GETCHARA" => (
                "Check if character exists",
                "GETCHARA(no)",
                vec![ParamDoc { name: "no".into(), type_: "int".into(), description: "Character index".into(), optional: false }],
                "Returns 1 if the character exists in CSV data, 0 otherwise.",
                vec!["IF GETCHARA(5)".into()],
            ),
            "GETTIME" => (
                "Get current game time",
                "GETTIME()",
                vec![],
                "Returns the current in-game time as an integer.",
                vec!["X = GETTIME()".into()],
            ),
            _ => (
                "Built-in function",
                name,
                vec![],
                "No documentation available.",
                vec![],
            ),
        };

        BuiltinDoc {
            name: name.to_string(),
            is_command: false,
            summary: summary.into(),
            syntax: syntax.into(),
            parameters: params,
            return_type,
            return_description: Some(
                if is_str {
                    "Returns a string"
                } else {
                    "Returns an integer"
                }
                .into(),
            ),
            description: description.into(),
            examples,
        }
    }

    /// Get documentation for a built-in.
    pub fn get(&self, name: &str) -> Option<&BuiltinDoc> {
        self.docs.get(&name.to_uppercase())
    }

    /// Format documentation for hover.
    pub fn format_hover(&self, name: &str) -> Option<String> {
        let doc = self.get(name)?;

        let mut output = String::new();

        output.push_str(&format!("# {}\n\n", doc.name));
        output.push_str(&format!("{}\n\n", doc.summary));

        output.push_str(&format!("## Syntax\n```erabasic\n{}\n```\n\n", doc.syntax));

        if !doc.parameters.is_empty() {
            output.push_str("## Parameters\n\n");
            for param in &doc.parameters {
                let opt = if param.optional { " (optional)" } else { "" };
                output.push_str(&format!(
                    "- **{}** ({}){}: {}\n",
                    param.name, param.type_, opt, param.description
                ));
            }
            output.push('\n');
        }

        if let Some(ret) = &doc.return_type {
            output.push_str(&format!("## Returns\n`{}`\n\n", ret));
            if let Some(ret_desc) = &doc.return_description {
                output.push_str(&format!("{}\n\n", ret_desc));
            }
        }

        if !doc.description.is_empty() && doc.description != "No documentation available." {
            output.push_str(&format!("## Description\n{}\n\n", doc.description));
        }

        if !doc.examples.is_empty() {
            output.push_str("## Examples\n\n");
            for example in &doc.examples {
                output.push_str(&format!("```erabasic\n{}\n```\n\n", example));
            }
        }

        Some(output)
    }
}

impl Default for BuiltinDocs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_docs() {
        let docs = BuiltinDocs::new();

        let print_doc = docs.get("PRINT").unwrap();
        assert_eq!(print_doc.name, "PRINT");
        assert!(print_doc.is_command);
        assert!(!print_doc.parameters.is_empty());

        let rand_doc = docs.get("RAND").unwrap();
        assert_eq!(rand_doc.name, "RAND");
        assert!(!rand_doc.is_command);
        assert_eq!(rand_doc.return_type, Some("int".into()));

        let hover = docs.format_hover("PRINT").unwrap();
        assert!(hover.contains("PRINT"));
        assert!(hover.contains("Syntax"));
    }
}
