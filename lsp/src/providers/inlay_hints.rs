//! Inlay hints provider for EraBasic LSP.

use crate::analysis::symbols::VarType;
use crate::analysis::types::TypeInference;
use crate::document::Document;
use crate::lsp_ast::{Body, CallInstr, CallTarget, DimDecl, Expr, ForBegin, Function, Instr, Span};
use lsp_types::{
    InlayHint, InlayHintKind, InlayHintLabel, InlayHintParams, InlayHintTooltip, Position, Range,
};

/// Inlay hints provider for EraBasic.
pub struct InlayHintProvider;

impl InlayHintProvider {
    pub fn new() -> Self {
        Self
    }

    /// Provide inlay hints for a document.
    pub fn provide(&self, params: InlayHintParams, document: &Document) -> Vec<InlayHint> {
        let mut hints = Vec::new();

        if let Some(ast) = &document.ast {
            let global_dims = &ast.program.global_dims;
            for func_list in ast.program.functions.values() {
                for func in func_list {
                    hints.extend(self.function_inlay_hints(func, document, global_dims));
                }
            }
        }

        hints
    }

    /// Get inlay hints for a function.
    fn function_inlay_hints(
        &self,
        func: &Function,
        document: &Document,
        global_dims: &[DimDecl],
    ) -> Vec<InlayHint> {
        let mut hints = Vec::new();
        let body = &func.body;

        // Type hints for variables
        let type_inf = crate::analysis::types::infer_function_types_lsp(func, global_dims);

        for dim in &body.dims {
            if let Some(var_type) = type_inf.get_var_type(&dim.name) {
                if !matches!(var_type, VarType::Unknown) {
                    hints.push(InlayHint {
                        position: crate::util::position::offset_to_position(
                            &document.get_text(),
                            dim.span.start,
                            crate::util::position::PositionEncoding::Utf16,
                        ),
                        label: InlayHintLabel::String(format!(": {}", var_type_str(&var_type))),
                        kind: Some(InlayHintKind::TYPE),
                        text_edits: None,
                        tooltip: None,
                        padding_left: Some(true),
                        padding_right: Some(true),
                        data: None,
                    });
                }
            }
        }

        // Parameter name hints for function calls
        for (pc, instr) in body.code.iter().enumerate() {
            if let Instr::Call(call) = instr {
                if let Some(symbol) = self.find_function_symbol(&call.target, document) {
                    if let Some(sig) = &symbol.signature {
                        for (i, arg) in call.args.iter().enumerate() {
                            if i < sig.params.len() {
                                let param = &sig.params[i];
                                hints.push(InlayHint {
                                    position: crate::util::position::offset_to_position(
                                        &document.get_text(),
                                        arg.span().start,
                                        crate::util::position::PositionEncoding::Utf16,
                                    ),
                                    label: InlayHintLabel::String(format!("{} = ", param.name)),
                                    kind: Some(InlayHintKind::PARAMETER),
                                    text_edits: None,
                                    tooltip: Some(InlayHintTooltip::String(format!(
                                        "Parameter: {}",
                                        param.name
                                    ))),
                                    padding_left: Some(false),
                                    padding_right: Some(true),
                                    data: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Loop counter hints
        for instr in &body.code {
            if let Instr::ForBegin(fb) = instr {
                hints.push(InlayHint {
                    position: crate::util::position::offset_to_position(
                        &document.get_text(),
                        fb.var.span.start,
                        crate::util::position::PositionEncoding::Utf16,
                    ),
                    label: InlayHintLabel::String(" (loop)".into()),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::String(format!(
                        "Loop variable: {} to {}",
                        crate::analysis::types::TypeInference::new().infer_expr_lsp(&fb.start),
                        crate::analysis::types::TypeInference::new().infer_expr_lsp(&fb.end),
                    ))),
                    padding_left: Some(true),
                    padding_right: Some(true),
                    data: None,
                });
            }
        }

        hints
    }

    fn find_function_symbol(
        &self,
        target: &CallTarget,
        document: &Document,
    ) -> Option<crate::analysis::symbols::Symbol> {
        match target {
            CallTarget::Static(name) => {
                // Would need workspace index
                None
            }
            CallTarget::Form(_) => None,
        }
    }
}

fn var_type_str(var_type: &VarType) -> String {
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

impl Default for InlayHintProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inlay_hints() {
        let provider = InlayHintProvider::new();
        // Would need document with AST to test
    }
}
