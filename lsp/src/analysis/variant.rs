//! Dialect detection for EraBasic (Emuera-EM, Emuera-Anchor, Emuera-EE).

use crate::parser::ast::*;
use std::collections::HashSet;

/// EraBasic dialect variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    Auto,
    EmueraEM,
    EmueraAnchor,
    EmueraEE,
}

impl Dialect {
    /// Detect dialect from source text.
    pub fn detect(text: &str) -> Self {
        let upper = text.to_uppercase();

        // Anchor-specific features
        if upper.contains("#FUNCTION") || upper.contains("#SINGLE") {
            return Dialect::EmueraAnchor;
        }

        // Check for Anchor built-ins
        let anchor_keywords = [
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

        for kw in anchor_keywords {
            if upper.contains(kw) {
                return Dialect::EmueraAnchor;
            }
        }

        // EE-specific features
        if upper.contains("VARI ") || upper.contains("VARS ") {
            return Dialect::EmueraEE;
        }

        // Check for EE built-ins
        let ee_keywords = ["LOCAL", "LOCALS", "ARG", "ARGS"];
        for kw in ee_keywords {
            if upper.contains(kw) {
                return Dialect::EmueraEE;
            }
        }

        Dialect::EmueraEM
    }

    /// Detect dialect from a program.
    pub fn detect_from_program(program: &crate::parser::Program) -> Self {
        // Check functions for Anchor/EE attributes
        for func_rc in program.funcs.made() {
            let func = func_rc.as_ref();
            if func.is_function || func.single || func.only {
                return Dialect::EmueraAnchor;
            }
        }

        // Check defines for Anchor patterns
        for body in program.defines.values() {
            let upper = body.to_uppercase();
            if upper.contains("PARAM") || upper.contains("TR_") {
                return Dialect::EmueraAnchor;
            }
        }

        Dialect::EmueraEM
    }

    /// Get the name of the dialect.
    pub fn name(&self) -> &'static str {
        match self {
            Dialect::Auto => "Auto",
            Dialect::EmueraEM => "Emuera-EM",
            Dialect::EmueraAnchor => "Emuera-Anchor",
            Dialect::EmueraEE => "Emuera-EE",
        }
    }

    /// Check if a feature is supported in this dialect.
    pub fn supports(&self, feature: DialectFeature) -> bool {
        match feature {
            DialectFeature::Functions => matches!(self, Dialect::EmueraAnchor | Dialect::EmueraEE),
            DialectFeature::SingleFunctions => matches!(self, Dialect::EmueraAnchor),
            DialectFeature::LocalVariables => matches!(self, Dialect::EmueraEE),
            DialectFeature::Classes => matches!(self, Dialect::EmueraAnchor),
            DialectFeature::Modules => matches!(self, Dialect::EmueraAnchor),
            DialectFeature::Interfaces => matches!(self, Dialect::EmueraAnchor),
            DialectFeature::Enums => matches!(self, Dialect::EmueraAnchor),
            DialectFeature::Types => matches!(self, Dialect::EmueraAnchor),
            DialectFeature::TRFunctions => matches!(self, Dialect::EmueraAnchor),
            DialectFeature::Param => matches!(self, Dialect::EmueraAnchor),
            DialectFeature::EventOrder => matches!(self, Dialect::EmueraAnchor | Dialect::EmueraEE),
            DialectFeature::ExportImport => matches!(self, Dialect::EmueraAnchor),
        }
    }
}

/// Dialect-specific features.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialectFeature {
    Functions,       // #FUNCTION
    SingleFunctions, // #SINGLE
    LocalVariables,  // VARI, VARS, LOCAL, LOCALS
    Classes,         // CLASS, ENDCLASS
    Modules,         // MODULE, ENDMODULE
    Interfaces,      // INTERFACE, ENDINTERFACE
    Enums,           // ENUM, ENDENUM
    Types,           // TYPE, ENDTYPE
    TRFunctions,     // TR_* functions
    Param,           // PARAM
    EventOrder,      // #PRI, #LATER
    ExportImport,    // EXPORT, IMPORT
}

/// Configuration for a specific dialect.
#[derive(Debug, Clone)]
pub struct DialectConfig {
    pub dialect: Dialect,
    pub allowed_commands: HashSet<String>,
    pub allowed_functions: HashSet<String>,
    pub allowed_keywords: HashSet<String>,
}

impl DialectConfig {
    /// Create a config for the given dialect.
    pub fn for_dialect(dialect: Dialect) -> Self {
        let mut config = Self {
            dialect,
            allowed_commands: HashSet::new(),
            allowed_functions: HashSet::new(),
            allowed_keywords: HashSet::new(),
        };

        // Add common commands/functions/keywords
        config.add_common();

        // Add dialect-specific ones
        match dialect {
            Dialect::EmueraAnchor => config.add_anchor(),
            Dialect::EmueraEE => config.add_ee(),
            _ => {}
        }

        config
    }

    fn add_common(&mut self) {
        use crate::parser::builtins::{COMMANDS, FUNCTIONS, KEYWORDS};

        self.allowed_commands
            .extend(COMMANDS.iter().map(|s| s.to_string()));
        self.allowed_functions
            .extend(FUNCTIONS.iter().map(|s| s.to_string()));
        self.allowed_keywords
            .extend(KEYWORDS.iter().map(|s| s.to_string()));
    }

    fn add_anchor(&mut self) {
        let anchor_keywords = [
            "FUNCTION",
            "FUNCTIONS",
            "SINGLE",
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
            "PRI",
            "LATER",
            "ONLY",
        ];

        for kw in anchor_keywords {
            self.allowed_keywords.insert(kw.to_string());
        }
    }

    fn add_ee(&mut self) {
        let ee_keywords = [
            "VARI", "VARS", "LOCAL", "LOCALS", "ARG", "ARGS", "PRI", "LATER",
        ];

        for kw in ee_keywords {
            self.allowed_keywords.insert(kw.to_string());
        }
    }

    /// Check if a command is allowed in this dialect.
    pub fn is_command_allowed(&self, name: &str) -> bool {
        self.allowed_commands.contains(&name.to_uppercase())
    }

    /// Check if a function is allowed in this dialect.
    pub fn is_function_allowed(&self, name: &str) -> bool {
        self.allowed_functions.contains(&name.to_uppercase())
    }

    /// Check if a keyword is allowed in this dialect.
    pub fn is_keyword_allowed(&self, name: &str) -> bool {
        self.allowed_keywords.contains(&name.to_uppercase())
    }
}

impl Default for DialectConfig {
    fn default() -> Self {
        Self::for_dialect(Dialect::EmueraEM)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialect_detection() {
        assert_eq!(Dialect::detect("@FUNC() END"), Dialect::EmueraEM);
        assert_eq!(
            Dialect::detect("#FUNCTION FUNC() END"),
            Dialect::EmueraAnchor
        );
        assert_eq!(Dialect::detect("#SINGLE FUNC() END"), Dialect::EmueraAnchor);
        assert_eq!(Dialect::detect("VARI X"), Dialect::EmueraEE);
        assert_eq!(Dialect::detect("PARAM X"), Dialect::EmueraAnchor);
        assert_eq!(Dialect::detect("CLASS FOO ENDCLASS"), Dialect::EmueraAnchor);
    }

    #[test]
    fn test_dialect_features() {
        assert!(Dialect::EmueraAnchor.supports(DialectFeature::Functions));
        assert!(Dialect::EmueraAnchor.supports(DialectFeature::SingleFunctions));
        assert!(!Dialect::EmueraEM.supports(DialectFeature::Functions));
        assert!(Dialect::EmueraEE.supports(DialectFeature::LocalVariables));
        assert!(!Dialect::EmueraEM.supports(DialectFeature::LocalVariables));
    }
}
