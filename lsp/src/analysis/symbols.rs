//! Symbol table for EraBasic: functions, labels, variables, macros, constants.

use crate::analysis::variant::Dialect;
use crate::lsp_ast::*;
use crate::parser::builtins::{is_builtin_command, is_builtin_function, KEYWORDS};
use crate::util::position::{span_to_range, TextRange};
use indexmap::IndexMap;
use lsp_types::{Position, Range, SymbolTag};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

// Re-export SymbolKind for other modules
pub use lsp_types::SymbolKind;

/// A symbol in the EraBasic language.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub range: Range,
    pub selection_range: Range,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    pub tags: Option<Vec<SymbolTag>>,
    pub deprecated: Option<bool>,
    pub file: PathBuf,
    /// For variables: the type (Int, Str, Array, etc.)
    pub var_type: Option<VarType>,
    /// For functions: parameters and return type
    pub signature: Option<FunctionSignature>,
    /// For labels: the function they belong to
    pub parent_function: Option<String>,
    /// Whether this is a built-in
    pub is_builtin: bool,
}

/// Variable type information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VarType {
    Int,
    Str,
    IntArray(Vec<usize>),
    StrArray(Vec<usize>),
    Chara,
    CharaArray(Vec<usize>),
    Unknown,
}

impl std::fmt::Display for VarType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VarType::Int => write!(f, "Int"),
            VarType::Str => write!(f, "Str"),
            VarType::IntArray(dims) => write!(
                f,
                "Int[{}]",
                dims.iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("][")
            ),
            VarType::StrArray(dims) => write!(
                f,
                "Str[{}]",
                dims.iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("][")
            ),
            VarType::Chara => write!(f, "Chara"),
            VarType::CharaArray(dims) => write!(
                f,
                "Chara[{}]",
                dims.iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("][")
            ),
            VarType::Unknown => write!(f, "Unknown"),
        }
    }
}

/// Function signature information.
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub params: Vec<Parameter>,
    pub return_type: Option<VarType>,
    pub is_function: bool, // #FUNCTION vs regular
    pub returns_str: bool,
    pub is_event: bool,
    pub event_order: Option<EventOrder>,
}

/// A function parameter.
#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub var_type: VarType,
    pub has_default: bool,
    pub default_value: Option<String>,
}

/// Symbol table for a single file.
#[derive(Debug, Default)]
pub struct FileSymbolTable {
    pub file_path: PathBuf,
    pub symbols: IndexMap<String, Symbol>,
    pub functions: IndexMap<String, Vec<Symbol>>,
    pub labels: IndexMap<String, Symbol>,
    pub variables: IndexMap<String, Symbol>,
    pub macros: IndexMap<String, Symbol>,
    pub constants: IndexMap<String, Symbol>,
}

impl FileSymbolTable {
    /// Create a new symbol table for a file.
    pub fn new(file_path: PathBuf) -> Self {
        Self {
            file_path,
            ..Default::default()
        }
    }

    /// Add a symbol to the table.
    pub fn add_symbol(&mut self, symbol: Symbol) {
        let name = symbol.name.to_uppercase();

        // Add to main symbols map
        self.symbols.insert(name.clone(), symbol.clone());

        // Add to specific category maps
        match symbol.kind {
            SymbolKind::FUNCTION | SymbolKind::METHOD => {
                self.functions.entry(name.clone()).or_default().push(symbol);
            }
            SymbolKind::NAMESPACE => {
                // Labels
                self.labels.insert(name, symbol);
            }
            SymbolKind::VARIABLE | SymbolKind::FIELD | SymbolKind::PROPERTY => {
                self.variables.insert(name, symbol);
            }
            SymbolKind::CONSTANT => {
                self.constants.insert(name, symbol);
            }
            SymbolKind::MODULE => {
                // Macros
                self.macros.insert(name, symbol);
            }
            _ => {}
        }
    }

    /// Get a symbol by name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&Symbol> {
        self.symbols.get(&name.to_uppercase())
    }

    /// Get all symbols.
    pub fn all_symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.values()
    }

    /// Get all functions.
    pub fn all_functions(&self) -> impl Iterator<Item = &Symbol> {
        self.functions.values().flatten()
    }

    /// Get all labels.
    pub fn all_labels(&self) -> impl Iterator<Item = &Symbol> {
        self.labels.values()
    }

    /// Get all variables.
    pub fn all_variables(&self) -> impl Iterator<Item = &Symbol> {
        self.variables.values()
    }
}

/// Build a symbol table from a parsed program.
pub fn build_symbol_table(
    program: &Program,
    file_path: PathBuf,
    dialect: Dialect,
) -> FileSymbolTable {
    let mut table = FileSymbolTable::new(file_path);

    // Add built-in commands and functions as symbols
    add_builtins(&mut table, dialect);

    // Add keywords
    add_keywords(&mut table);

    // Add global #DIM variables
    for dim in &program.global_dims {
        add_dim_symbol(&mut table, dim, true);
    }

    // Add #DEFINE macros
    for (name, body) in &program.defines {
        add_macro_symbol(&mut table, name, body);
    }

    // Add functions
    for func_list in program.functions.values() {
        for func in func_list {
            add_function_symbols(&mut table, func, dialect);
        }
    }

    table
}

/// Add built-in commands and functions.
fn add_builtins(table: &mut FileSymbolTable, dialect: Dialect) {
    use crate::parser::builtins::{COMMANDS, FUNCTIONS, STR_FUNCTIONS};

    for cmd in COMMANDS {
        let symbol = Symbol {
            name: cmd.to_string(),
            kind: SymbolKind::FUNCTION,
            range: Range::default(),
            selection_range: Range::default(),
            detail: Some("Built-in command".into()),
            documentation: Some(get_builtin_doc(cmd, true)),
            tags: Some(vec![SymbolTag::DEPRECATED]),
            deprecated: None,
            file: PathBuf::from("<builtin>"),
            var_type: None,
            signature: Some(FunctionSignature {
                params: vec![],
                return_type: None,
                is_function: false,
                returns_str: false,
                is_event: false,
                event_order: None,
            }),
            parent_function: None,
            is_builtin: true,
        };
        table.add_symbol(symbol);
    }

    for func in FUNCTIONS {
        let returns_str = STR_FUNCTIONS.contains(&func);
        let symbol = Symbol {
            name: func.to_string(),
            kind: SymbolKind::FUNCTION,
            range: Range::default(),
            selection_range: Range::default(),
            detail: Some("Built-in function".into()),
            documentation: Some(get_builtin_doc(func, false)),
            tags: None,
            deprecated: None,
            file: PathBuf::from("<builtin>"),
            var_type: Some(if returns_str {
                VarType::Str
            } else {
                VarType::Int
            }),
            signature: Some(FunctionSignature {
                params: vec![],
                return_type: Some(if returns_str {
                    VarType::Str
                } else {
                    VarType::Int
                }),
                is_function: false,
                returns_str,
                is_event: false,
                event_order: None,
            }),
            parent_function: None,
            is_builtin: true,
        };
        table.add_symbol(symbol);
    }

    // Add Anchor-specific built-ins if needed
    if matches!(dialect, Dialect::EmueraAnchor) {
        add_anchor_builtins(table);
    }
}

/// Add Anchor-specific built-ins.
fn add_anchor_builtins(table: &mut FileSymbolTable) {
    let anchor_builtins = [
        "PARAM",
        "TR_NAME",
        "TR_WAIT",
        "TR_CLEAR",
        "TR_PRINT",
        "EXPORT",
        "IMPORT",
        "MODULE",
        "ENDMODULE",
        "CLASS",
        "ENDCLASS",
        "NEW",
        "THIS",
        "BASE",
        "INTERFACE",
        "ENDINTERFACE",
        "IMPLEMENTS",
        "ENUM",
        "ENDENUM",
        "TYPE",
        "ENDTYPE",
    ];

    for name in anchor_builtins {
        let symbol = Symbol {
            name: name.to_string(),
            kind: SymbolKind::FUNCTION,
            range: Range::default(),
            selection_range: Range::default(),
            detail: Some("Anchor built-in".into()),
            documentation: Some(format!("Emuera-Anchor built-in: {}", name)),
            tags: None,
            deprecated: None,
            file: PathBuf::from("<builtin>"),
            var_type: None,
            signature: None,
            parent_function: None,
            is_builtin: true,
        };
        table.add_symbol(symbol);
    }
}

/// Add keywords as symbols.
fn add_keywords(table: &mut FileSymbolTable) {
    for kw in KEYWORDS {
        let kind = match kw.as_ref() {
            "IF" | "ELSEIF" | "ELSE" | "ENDIF" | "SIF" | "SELECTCASE" | "CASE" | "CASEELSE"
            | "ENDSELECT" | "FOR" | "NEXT" | "BREAK" | "CONTINUE" | "WHILE" | "WEND" | "REPEAT"
            | "REND" | "DO" | "LOOP" | "CALL" | "JUMP" | "GOTO" | "RETURN" | "RETURNF"
            | "RETURNFORM" | "TRYCALL" | "TRYJUMP" | "TRYGOTO" | "CALLFORM" | "JUMPFORM"
            | "GOTOFORM" | "CALLF" | "CALLFORMF" | "TRYCALLFORM" | "TRYJUMPFORM"
            | "TRYGOTOFORM" | "TRYCALLLIST" | "TRYJUMPLIST" | "TRYGOTOLIST" | "TRYCCALL"
            | "TRYCCALLFORM" | "CATCH" | "ENDCATCH" | "CALLEVENT" | "ENDFUNC" | "ENDPROGRAM" => {
                SymbolKind::FUNCTION
            }
            "DIM" | "DIMS" | "CONST" | "STATIC" | "GLOBAL" | "SAVEDATA" | "CHARADATA" | "REF"
            | "DYNAMIC" | "VARI" | "VARS" | "LOCAL" | "LOCALS" | "ARG" | "ARGS" => {
                SymbolKind::VARIABLE
            }
            "DEFINE" | "IFDEF" | "IFNDEF" | "ELSE" | "ENDIF" | "SKIPSTART" | "SKIPEND" => {
                SymbolKind::MODULE
            }
            _ => SymbolKind::FUNCTION,
        };

        let symbol = Symbol {
            name: kw.to_string(),
            kind,
            range: Range::default(),
            selection_range: Range::default(),
            detail: Some("Keyword".into()),
            documentation: Some(get_keyword_doc(kw)),
            tags: None,
            deprecated: None,
            file: PathBuf::from("<keyword>"),
            var_type: None,
            signature: None,
            parent_function: None,
            is_builtin: true,
        };
        table.add_symbol(symbol);
    }
}

/// Add a #DIM variable symbol.
fn add_dim_symbol(table: &mut FileSymbolTable, dim: &DimDecl, is_global: bool) {
    let var_type = if dim.is_str {
        if dim.dims.is_empty() {
            VarType::Str
        } else {
            VarType::StrArray(dim.dims.iter().map(|e| expr_to_size(e)).collect())
        }
    } else {
        if dim.dims.is_empty() {
            VarType::Int
        } else {
            VarType::IntArray(dim.dims.iter().map(|e| expr_to_size(e)).collect())
        }
    };

    let symbol = Symbol {
        name: dim.name.clone(),
        kind: SymbolKind::VARIABLE,
        range: span_to_range("", dim.span, crate::util::position::PositionEncoding::Utf16),
        selection_range: Range::default(),
        detail: Some(
            if is_global {
                "Global variable"
            } else {
                "Local variable"
            }
            .into(),
        ),
        documentation: Some(format!(
            "{} {}",
            if dim.is_str { "String" } else { "Integer" },
            if dim.dims.is_empty() {
                String::new()
            } else {
                format!("Array[{}]", dim.dims.len())
            }
        )),
        tags: if dim.is_const {
            Some(vec![SymbolTag::DEPRECATED])
        } else {
            None
        },
        deprecated: None,
        file: table.file_path.clone(),
        var_type: Some(var_type),
        signature: None,
        parent_function: None,
        is_builtin: false,
    };
    table.add_symbol(symbol);
}

/// Add a #DEFINE macro symbol.
fn add_macro_symbol(table: &mut FileSymbolTable, name: &str, body: &str) {
    let symbol = Symbol {
        name: name.to_string(),
        kind: SymbolKind::MODULE,
        range: Range::default(),
        selection_range: Range::default(),
        detail: Some("Macro".into()),
        documentation: Some(format!("#DEFINE {} {}", name, body)),
        tags: None,
        deprecated: None,
        file: table.file_path.clone(),
        var_type: None,
        signature: None,
        parent_function: None,
        is_builtin: false,
    };
    table.add_symbol(symbol);
}

/// Add a function and its labels/variables.
fn add_function_symbols(table: &mut FileSymbolTable, func: &Function, dialect: Dialect) {
    // Function symbol
    let func_symbol = Symbol {
        name: func.name.to_string(),
        kind: SymbolKind::FUNCTION,
        range: Range::default(),
        selection_range: Range::default(),
        detail: Some(
            if func.is_function {
                "#FUNCTION"
            } else {
                "Function"
            }
            .into(),
        ),
        documentation: Some(format!(
            "{}{} {}({}): {}",
            if func.is_function { "#FUNCTION" } else { "@" },
            if func.returns_str { "$" } else { "" },
            func.name,
            func.params
                .iter()
                .map(|p| p.target.name.as_ref())
                .collect::<Vec<_>>()
                .join(", "),
            if func.returns_str { "Str" } else { "Int" }
        )),
        tags: None,
        deprecated: None,
        file: table.file_path.clone(),
        var_type: Some(if func.returns_str {
            VarType::Str
        } else {
            VarType::Int
        }),
        signature: Some(FunctionSignature {
            params: func
                .params
                .iter()
                .map(|p| Parameter {
                    name: p.target.name.to_string(),
                    var_type: VarType::Unknown,
                    has_default: p.default.is_some(),
                    default_value: p.default.as_ref().map(|e| format!("{:?}", e)),
                })
                .collect(),
            return_type: Some(if func.returns_str {
                VarType::Str
            } else {
                VarType::Int
            }),
            is_function: func.is_function,
            returns_str: func.returns_str,
            is_event: func.order != EventOrder::Normal,
            event_order: Some(match func.order {
                EventOrder::Pri => EventOrder::Pri,
                EventOrder::Normal => EventOrder::Normal,
                EventOrder::Later => EventOrder::Later,
            }),
        }),
        parent_function: None,
        is_builtin: false,
    };
    table.add_symbol(func_symbol);

    // Labels in the function
    for (label, pc) in &func.body.labels {
        if let Some(line_info) = func.body.lines.get(*pc) {
            let symbol = Symbol {
                name: label.clone(),
                kind: SymbolKind::NAMESPACE,
                range: Range::default(),
                selection_range: Range::default(),
                detail: Some("Label".into()),
                documentation: Some(format!("Label in function {}", func.name)),
                tags: None,
                deprecated: None,
                file: table.file_path.clone(),
                var_type: None,
                signature: None,
                parent_function: Some(func.name.to_string()),
                is_builtin: false,
            };
            table.add_symbol(symbol);
        }
    }

    // Local variables (#DIM in function)
    for dim in &func.body.dims {
        add_dim_symbol(table, dim, false);
    }
}

/// Convert an expression to an array size (for constant expressions).
fn expr_to_size(expr: &Expr) -> usize {
    match expr {
        Expr::Int(n) => *n as usize,
        _ => 0,
    }
}

/// Get documentation for a built-in.
fn get_builtin_doc(name: &str, is_command: bool) -> String {
    if is_command {
        match name {
            "PRINT" => "PRINT text — Output text to the console".into(),
            "PRINTL" => "PRINTL text — Output text with newline".into(),
            "PRINTV" => "PRINTV expr1, expr2, ... — Output multiple expressions".into(),
            "PRINTS" => "PRINTS expr — Output a single expression".into(),
            "PRINTC" => "PRINTC text — Output text with color".into(),
            "PRINTFORM" => "PRINTFORM @\"form string\" — Output formatted string".into(),
            "IF" => "IF condition ... ENDIF — Conditional block".into(),
            "FOR" => "FOR var, start, end[, step] ... NEXT — For loop".into(),
            "WHILE" => "WHILE condition ... WEND — While loop".into(),
            "SELECTCASE" => "SELECTCASE value ... ENDSELECT — Switch statement".into(),
            "CALL" => "CALL func(args...) — Call a function".into(),
            "JUMP" => "JUMP func(args...) — Tail call a function".into(),
            "GOTO" => "GOTO $label — Jump to a label".into(),
            "RETURN" => "RETURN [value...] — Return from function".into(),
            "DIM" => "#DIM name[, size...] — Declare global variable".into(),
            "DIMS" => "#DIMS name[, size...] — Declare global string variable".into(),
            "DEFINE" => "#DEFINE name body — Define a macro".into(),
            _ => format!("Built-in command: {}", name),
        }
    } else {
        match name {
            "RAND" => "RAND(max) — Random integer 0 to max-1".into(),
            "ABS" => "ABS(x) — Absolute value".into(),
            "MAX" => "MAX(a, b) — Maximum of two values".into(),
            "MIN" => "MIN(a, b) — Minimum of two values".into(),
            "STRLEN" => "STRLEN(str) — String length in characters".into(),
            "STRLENS" => "STRLENS(str) — String length in Shift_JIS bytes".into(),
            "SUBSTRING" => "SUBSTRING(str, start, len) — Substring by characters".into(),
            "SUBSTRINGU" => "SUBSTRINGU(str, start, len) — Substring by Unicode codepoints".into(),
            "TOSTR" => "TOSTR(value) — Convert to string".into(),
            "TOINT" => "TOINT(str) — Convert string to integer".into(),
            "CSVNAME" => "CSVNAME(no) — Get character name from CSV".into(),
            "CSVBASE" => "CSVBASE(no, field) — Get character base stat".into(),
            "GETCHARA" => "GETCHARA(no) — Check if character exists".into(),
            "GETTIME" => "GETTIME() — Get current game time".into(),
            _ => format!("Built-in function: {}", name),
        }
    }
}

/// Get documentation for a keyword.
fn get_keyword_doc(name: &str) -> String {
    match name {
        "IF" => {
            "IF condition ... ELSEIF condition ... ELSE ... ENDIF — Conditional execution".into()
        }
        "FOR" => "FOR var, start, end[, step] ... NEXT — Loop with counter".into(),
        "WHILE" => "WHILE condition ... WEND — Loop while condition is true".into(),
        "SELECTCASE" => {
            "SELECTCASE value CASE value ... CASEELSE ... ENDSELECT — Multi-way branch".into()
        }
        "CALL" => "CALL function(args...) — Call a function/subroutine".into(),
        "JUMP" => "JUMP function(args...) — Tail call (does not return)".into(),
        "GOTO" => "GOTO $label — Unconditional jump to label".into(),
        "RETURN" => "RETURN [value...] — Return from function with optional values".into(),
        "DIM" => "#DIM name[, size...] — Declare a global variable".into(),
        "DIMS" => "#DIMS name[, size...] — Declare a global string variable".into(),
        "DEFINE" => "#DEFINE name replacement — Define a text macro".into(),
        _ => format!("Keyword: {}", name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_symbol_table() {
        let mut table = FileSymbolTable::new(PathBuf::from("test.erb"));

        let symbol = Symbol {
            name: "TEST_FUNC".into(),
            kind: SymbolKind::FUNCTION,
            range: Range::default(),
            selection_range: Range::default(),
            detail: None,
            documentation: None,
            tags: None,
            deprecated: None,
            file: PathBuf::from("test.erb"),
            var_type: None,
            signature: None,
            parent_function: None,
            is_builtin: false,
        };

        table.add_symbol(symbol);
        assert!(table.get("test_func").is_some());
        assert!(table.get("TEST_FUNC").is_some());
    }
}
