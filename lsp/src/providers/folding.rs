//! Folding range provider for EraBasic LSP.

use crate::document::Document;
use crate::lsp_ast::{Body, ForBegin, Function, Instr};
use crate::util::position::PositionEncoding;
use lsp_types::{FoldingRange, FoldingRangeKind, FoldingRangeParams, Position, Range};

/// Folding range provider for EraBasic.
pub struct FoldingProvider;

impl FoldingProvider {
    pub fn new() -> Self {
        Self
    }

    /// Provide folding ranges for a document.
    pub fn provide(&self, params: FoldingRangeParams, document: &Document) -> Vec<FoldingRange> {
        let mut ranges = Vec::new();

        if let Some(ast) = &document.ast {
            for func_list in ast.program.functions.values() {
                for func in func_list {
                    let body = &func.body;
                    ranges.extend(self.function_folding_ranges(body, func, &ast.line_map));
                }
            }
        }

        ranges
    }

    /// Get folding ranges for a function body.
    fn function_folding_ranges(&self, body: &Body, func: &Function, line_map: &[crate::document::LineInfo]) -> Vec<FoldingRange> {
        let mut ranges = Vec::new();

        for (pc, instr) in body.code.iter().enumerate() {
            let line = body.lines.get(pc)
                .and_then(|span| line_map.iter()
                    .find(|li| li.start_offset <= span.start && span.start < li.end_offset)
                    .map(|li| li.original_line.saturating_sub(1)))
                .unwrap_or(0);

            match instr {
                Instr::JumpIfNot(_, target_pc) => {
                    // IF/SIF - find matching ENDIF/ELSE
                    let end_line = body.lines.get(*target_pc)
                        .and_then(|span| line_map.iter()
                            .find(|li| li.start_offset <= span.start && span.start < li.end_offset)
                            .map(|li| li.original_line.saturating_sub(1)))
                        .unwrap_or(0);
                    if end_line > line {
                        ranges.push(FoldingRange {
                            start_line: line,
                            start_character: None,
                            end_line,
                            end_character: None,
                            kind: Some(FoldingRangeKind::Region),
                            collapsed_text: Some("IF ... ENDIF".into()),
                        });
                    }
                }

                Instr::ForBegin(fb) => {
                    // FOR loop - find matching NEXT
                    let end_line = body.lines.get(fb.exit)
                        .and_then(|span| line_map.iter()
                            .find(|li| li.start_offset <= span.start && span.start < li.end_offset)
                            .map(|li| li.original_line.saturating_sub(1)))
                        .unwrap_or(0);
                    if end_line > line {
                        ranges.push(FoldingRange {
                            start_line: line,
                            start_character: None,
                            end_line,
                            end_character: None,
                            kind: Some(FoldingRangeKind::Region),
                            collapsed_text: Some("FOR ... NEXT".into()),
                        });
                    }
                }

                Instr::SelectEval(_, _, _) => {
                    // SELECTCASE - find matching ENDSELECT
                    if let Some(end_pc) = self.find_endselect(body, pc) {
                        let end_line = body.lines.get(end_pc)
                            .and_then(|span| line_map.iter()
                                .find(|li| li.start_offset <= span.start && span.start < li.end_offset)
                                .map(|li| li.original_line.saturating_sub(1)))
                            .unwrap_or(0);
                        if end_line > line {
                            ranges.push(FoldingRange {
                                start_line: line,
                                start_character: None,
                                end_line,
                                end_character: None,
                                kind: Some(FoldingRangeKind::Region),
                                collapsed_text: Some("SELECTCASE ... ENDSELECT".into()),
                            });
                        }
                    }
                }

                Instr::Jump(_target) => {
                    // Labels can be folded
                }
                Instr::Goto(_) => {
                    // Labels can be folded
                }

                _ => {}
            }
        }

        // Function itself can be folded
        if let (Some(first_span), Some(last_span)) = (body.lines.first(), body.lines.last()) {
            let start_line = line_map.iter()
                .find(|li| li.start_offset <= first_span.start && first_span.start < li.end_offset)
                .map(|li| li.original_line.saturating_sub(1))
                .unwrap_or(0);
            let end_line = line_map.iter()
                .find(|li| li.start_offset <= last_span.start && last_span.start < li.end_offset)
                .map(|li| li.original_line.saturating_sub(1))
                .unwrap_or(0);
            if end_line > start_line {
                ranges.push(FoldingRange {
                    start_line,
                    start_character: None,
                    end_line,
                    end_character: None,
                    kind: Some(FoldingRangeKind::Region),
                    collapsed_text: Some(format!(
                        "{} {}",
                        if func.is_function { "#FUNCTION" } else { "@" },
                        func.name
                    )),
                });
            }
        }

        ranges
    }

    /// Find the ENDSELECT instruction for a SELECTCASE.
    fn find_endselect(&self, body: &Body, start_pc: usize) -> Option<usize> {
        let mut depth = 0;
        for (pc, instr) in body.code.iter().enumerate().skip(start_pc + 1) {
            match instr {
                Instr::SelectEval(_, _, _) => depth += 1,
                Instr::JumpIfNot(_, _) => {
                    // Could be CASE or CASEELSE
                }
                Instr::Jump(target) if *target > pc => {
                    // Check if this is ENDSELECT
                    if let Some(line_text) = self.get_line_text(body, *target) {
                        if line_text.trim().to_uppercase() == "ENDSELECT" {
                            if depth == 0 {
                                return Some(*target);
                            } else {
                                depth -= 1;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn get_line_text(&self, _body: &Body, _pc: usize) -> Option<String> {
        // Would need access to source text
        None
    }
}

impl Default for FoldingProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_folding() {
        let provider = FoldingProvider::new();
        // Would need document with AST to test
    }
}
