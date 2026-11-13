//! Load-time warnings (from emuera-tty's loadcheck.rs)

use crate::parser::ast::Span;

/// A warning found during loading/parsing.
#[derive(Debug, Clone)]
pub struct Warning {
    pub file: String,
    pub span: Span,
    pub message: String,
}

impl Warning {
    pub fn new(file: String, span: Span, message: String) -> Self {
        Self {
            file,
            span,
            message,
        }
    }
}
