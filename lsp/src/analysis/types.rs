//! Type inference for EraBasic variables and expressions.

use crate::analysis::symbols::VarType;
use crate::lsp_ast::{
    BinOp as LspBinOp, Body as LspBody, CallInstr as LspCallInstr, DimDecl as LspDimDecl,
    Expr as LspExpr, ForBegin as LspForBegin, Function as LspFunction, Instr as LspInstr,
    Span as LspSpan, UnOp as LspUnOp, VarRef as LspVarRef,
};
use crate::parser::ast::*;
use std::collections::HashMap;

/// Type inference engine for EraBasic.
pub struct TypeInference {
    /// Variable types: name -> type
    var_types: HashMap<String, VarType>,
    /// Function return types
    func_return_types: HashMap<String, VarType>,
    /// Global string variables (from #DIMS)
    global_str_vars: HashMap<String, bool>,
}

impl TypeInference {
    pub fn new() -> Self {
        Self {
            var_types: HashMap::new(),
            func_return_types: HashMap::new(),
            global_str_vars: HashMap::new(),
        }
    }

    /// Initialize with global dimensions.
    pub fn with_globals(globals: &[DimDecl]) -> Self {
        let mut inf = Self::new();
        for dim in globals {
            let var_type = if dim.is_str {
                if dim.dims.is_empty() {
                    VarType::Str
                } else {
                    VarType::StrArray(vec![])
                }
            } else {
                if dim.dims.is_empty() {
                    VarType::Int
                } else {
                    VarType::IntArray(vec![])
                }
            };
            inf.var_types.insert(dim.name.to_uppercase(), var_type);
            if dim.is_str {
                inf.global_str_vars.insert(dim.name.to_uppercase(), true);
            }
        }
        inf
    }

    /// Add a function's return type.
    pub fn add_function(&mut self, name: &str, returns_str: bool) {
        self.func_return_types.insert(
            name.to_uppercase(),
            if returns_str {
                VarType::Str
            } else {
                VarType::Int
            },
        );
    }

    /// Add a local variable.
    pub fn add_local(&mut self, name: &str, var_type: VarType) {
        self.var_types.insert(name.to_uppercase(), var_type);
    }

    /// Infer the type of an expression.
    pub fn infer_expr(&self, expr: &Expr) -> VarType {
        match expr {
            Expr::Int(_) => VarType::Int,
            Expr::Str(_) => VarType::Str,
            Expr::Form(_) => VarType::Str,
            Expr::Var(var) => self.infer_var(var),
            Expr::Call(call) => self.infer_call(call),
            Expr::Unary(_, e) => self.infer_expr(e),
            Expr::Binary(op, a, b) => self.infer_binary(op, a, b),
            Expr::Ternary(c, a, b) => {
                // Type of ternary is the type of the true branch (typically)
                self.infer_expr(a)
            }
            Expr::IncDec(var, _, _) => self.infer_var(var),
            Expr::Empty => VarType::Unknown,
            Expr::Span(_) => VarType::Unknown,
        }
    }

    /// Infer the type of a variable reference.
    fn infer_var(&self, var: &VarRef) -> VarType {
        let name = var.name.to_uppercase();
        self.var_types.get(&name).cloned().unwrap_or_else(|| {
            // Check system variables
            if crate::parser::loader::is_system_str_var(&name) {
                VarType::Str
            } else {
                VarType::Unknown
            }
        })
    }

    /// Infer the return type of a function call.
    fn infer_call(&self, call: &(String, Vec<Expr>, crate::parser::ast::Resolved)) -> VarType {
        let name = call.0.to_uppercase();

        // Check user functions first
        if let Some(t) = self.func_return_types.get(&name) {
            return t.clone();
        }

        // Check built-ins
        if crate::parser::builtins::is_builtin_function(&name) {
            if crate::parser::builtins::returns_str(&name) {
                return VarType::Str;
            } else {
                return VarType::Int;
            }
        }

        VarType::Unknown
    }

    /// Infer the type of a binary operation.
    fn infer_binary(&self, op: &BinOp, a: &Expr, b: &Expr) -> VarType {
        let ta = self.infer_expr(a);
        let tb = self.infer_expr(b);

        // String concatenation with +
        if matches!(op, BinOp::Add) && (matches!(ta, VarType::Str) || matches!(tb, VarType::Str)) {
            return VarType::Str;
        }

        // String multiplication (repeat)
        if matches!(op, BinOp::Mul) && (matches!(ta, VarType::Str) || matches!(tb, VarType::Str)) {
            return VarType::Str;
        }

        // Comparisons return Int (0 or 1)
        if matches!(
            op,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge
        ) {
            return VarType::Int;
        }

        // Logical ops return Int
        if matches!(
            op,
            BinOp::And | BinOp::Or | BinOp::Xor | BinOp::Nand | BinOp::Nor
        ) {
            return VarType::Int;
        }

        // Bitwise ops return Int
        if matches!(
            op,
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr
        ) {
            return VarType::Int;
        }

        // Arithmetic ops
        if matches!(
            op,
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod
        ) {
            if matches!(ta, VarType::Str) || matches!(tb, VarType::Str) {
                return VarType::Str;
            }
            return VarType::Int;
        }

        VarType::Int
    }

    /// Check if a variable is a string type.
    pub fn is_string_var(&self, name: &str) -> bool {
        let name = name.to_uppercase();
        self.global_str_vars.contains_key(&name)
            || matches!(
                self.var_types.get(&name),
                Some(VarType::Str) | Some(VarType::StrArray(_))
            )
    }

    /// Get the type of a variable.
    pub fn get_var_type(&self, name: &str) -> Option<VarType> {
        self.var_types.get(&name.to_uppercase()).cloned()
    }

    /// Merge another type inference into this one.
    pub fn merge(&mut self, other: TypeInference) {
        self.var_types.extend(other.var_types);
        self.func_return_types.extend(other.func_return_types);
        self.global_str_vars.extend(other.global_str_vars);
    }
}

impl Default for TypeInference {
    fn default() -> Self {
        Self::new()
    }
}

/// Infer types for an entire function body.
pub fn infer_function_types(func: &Function, globals: &[DimDecl]) -> TypeInference {
    let mut inf = TypeInference::with_globals(globals);

    // Add function return type
    inf.add_function(&func.name, func.returns_str);

    // Add parameters
    for param in &func.params {
        let var_type = if param.target.name.to_uppercase().ends_with('$')
            || crate::parser::loader::is_system_str_var(&param.target.name)
        {
            VarType::Str
        } else {
            VarType::Int
        };
        inf.add_local(&param.target.name, var_type);
    }

    // Add local dims
    let body = func.body();
    for dim in &body.dims {
        let var_type = if dim.is_str {
            if dim.dims.is_empty() {
                VarType::Str
            } else {
                VarType::StrArray(vec![])
            }
        } else {
            if dim.dims.is_empty() {
                VarType::Int
            } else {
                VarType::IntArray(vec![])
            }
        };
        inf.add_local(&dim.name, var_type);
    }

    // Infer types from instructions
    for instr in &body.code {
        infer_instr_types(&mut inf, instr);
    }

    inf
}

/// Infer types from a single instruction.
fn infer_instr_types(inf: &mut TypeInference, instr: &Instr) {
    match instr {
        Instr::Assign(var, _, exprs) => {
            let var_type = inf.infer_var(var);
            for expr in exprs {
                let expr_type = inf.infer_expr(expr);
                // If assigning string to int var or vice versa, it's a type mismatch
                // but EraBasic allows it (dynamic typing)
            }
        }
        Instr::AssignForm(var, _, form) => {
            // String assignment
            inf.add_local(&var.name, VarType::Str);
        }
        Instr::Eval(expr) | Instr::JumpIfNot(expr, _) | Instr::ReturnF(Some(expr)) => {
            inf.infer_expr(expr);
        }
        Instr::Print(PrintContent::Expr(e), _) => {
            inf.infer_expr(e);
        }
        Instr::Print(PrintContent::Form(_f), _) => {
            // FORM would need special handling
        }
        Instr::Print(PrintContent::Exprs(es), _) => {
            for e in es {
                inf.infer_expr(e);
            }
        }
        Instr::Call(call) => {
            for arg in &call.args {
                inf.infer_expr(arg);
            }
        }
        Instr::ForBegin(fb) => {
            inf.infer_expr(&fb.start);
            inf.infer_expr(&fb.end);
            if let Some(step) = &fb.step {
                inf.infer_expr(step);
            }
        }
        Instr::SelectEval(e, _, _) => {
            inf.infer_expr(e);
        }
        Instr::CaseTest(_, conds, _) => {
            for cond in conds {
                match cond {
                    CaseCond::Eq(e) | CaseCond::Is(_, e) => {
                        inf.infer_expr(e);
                    }
                    CaseCond::Range(a, b) => {
                        inf.infer_expr(a);
                        inf.infer_expr(b);
                    }
                }
            }
        }
        Instr::Command(cmd) => {
            if let Args::Exprs(es) = &cmd.1 {
                for e in es {
                    inf.infer_expr(e);
                }
            } else if let Args::Form(f) = &cmd.1 {
                // FORM would need special handling
            }
        }
        _ => {}
    }
}

/// LSP AST version of TypeInference methods
impl TypeInference {
    /// Infer the type of an LSP expression.
    pub fn infer_expr_lsp(&self, expr: &LspExpr) -> VarType {
        match expr {
            LspExpr::Int(_) => VarType::Int,
            LspExpr::Str(_) => VarType::Str,
            LspExpr::Form(_) => VarType::Str,
            LspExpr::Var(var) => self.infer_var_lsp(var),
            LspExpr::Call(call) => self.infer_call_lsp(call),
            LspExpr::Unary(_, e) => self.infer_expr_lsp(e),
            LspExpr::Binary(op, a, b) => self.infer_binary_lsp(op, a, b),
            LspExpr::Ternary(c, a, b) => self.infer_expr_lsp(a),
            LspExpr::IncDec(var, _, _) => self.infer_var_lsp(var),
            LspExpr::Empty => VarType::Unknown,
            LspExpr::Span(_) => VarType::Unknown,
        }
    }

    /// Infer the type of a variable reference (LSP version).
    fn infer_var_lsp(&self, var: &LspVarRef) -> VarType {
        let name = var.name.to_uppercase();
        self.var_types.get(&name).cloned().unwrap_or_else(|| {
            if crate::parser::loader::is_system_str_var(&name) {
                VarType::Str
            } else {
                VarType::Unknown
            }
        })
    }

    /// Infer the return type of a function call (LSP version).
    fn infer_call_lsp(&self, call: &(String, Vec<LspExpr>, crate::lsp_ast::Resolved)) -> VarType {
        let name = call.0.to_uppercase();

        if let Some(t) = self.func_return_types.get(&name) {
            return t.clone();
        }

        if crate::parser::builtins::is_builtin_function(&name) {
            if crate::parser::builtins::returns_str(&name) {
                return VarType::Str;
            } else {
                return VarType::Int;
            }
        }

        VarType::Unknown
    }

    /// Infer the type of a binary operation (LSP version).
    fn infer_binary_lsp(&self, op: &LspBinOp, a: &LspExpr, b: &LspExpr) -> VarType {
        let ta = self.infer_expr_lsp(a);
        let tb = self.infer_expr_lsp(b);

        if matches!(op, LspBinOp::Add) && (matches!(ta, VarType::Str) || matches!(tb, VarType::Str))
        {
            return VarType::Str;
        }
        if matches!(op, LspBinOp::Mul) && (matches!(ta, VarType::Str) || matches!(tb, VarType::Str))
        {
            return VarType::Str;
        }
        if matches!(
            op,
            LspBinOp::Eq | LspBinOp::Ne | LspBinOp::Lt | LspBinOp::Le | LspBinOp::Gt | LspBinOp::Ge
        ) {
            return VarType::Int;
        }
        if matches!(
            op,
            LspBinOp::And | LspBinOp::Or | LspBinOp::Xor | LspBinOp::Nand | LspBinOp::Nor
        ) {
            return VarType::Int;
        }
        if matches!(
            op,
            LspBinOp::BitAnd | LspBinOp::BitOr | LspBinOp::BitXor | LspBinOp::Shl | LspBinOp::Shr
        ) {
            return VarType::Int;
        }
        if matches!(
            op,
            LspBinOp::Add | LspBinOp::Sub | LspBinOp::Mul | LspBinOp::Div | LspBinOp::Mod
        ) {
            if matches!(ta, VarType::Str) || matches!(tb, VarType::Str) {
                return VarType::Str;
            }
            return VarType::Int;
        }
        VarType::Int
    }
}

/// Infer types for an entire function body (LSP version).
pub fn infer_function_types_lsp(func: &LspFunction, globals: &[LspDimDecl]) -> TypeInference {
    let mut inf = TypeInference::with_globals_lsp(globals);

    // Add function return type
    inf.add_function(&func.name, func.returns_str);

    // Add parameters
    for param in &func.params {
        let var_type = if param.target.name.to_uppercase().ends_with('$')
            || crate::parser::loader::is_system_str_var(&param.target.name)
        {
            VarType::Str
        } else {
            VarType::Int
        };
        inf.add_local(&param.target.name, var_type);
    }

    // Add local dims
    let body = &func.body;
    for dim in &body.dims {
        let var_type = if dim.is_str {
            if dim.dims.is_empty() {
                VarType::Str
            } else {
                VarType::StrArray(vec![])
            }
        } else {
            if dim.dims.is_empty() {
                VarType::Int
            } else {
                VarType::IntArray(vec![])
            }
        };
        inf.add_local(&dim.name, var_type);
    }

    // Infer types from instructions
    for instr in &body.code {
        infer_instr_types_lsp(&mut inf, instr);
    }

    inf
}

/// Infer types from a single instruction (LSP version).
fn infer_instr_types_lsp(inf: &mut TypeInference, instr: &LspInstr) {
    match instr {
        LspInstr::Assign(var, _, exprs) => {
            let var_type = inf.infer_var_lsp(var);
            for expr in exprs {
                inf.infer_expr_lsp(expr);
            }
        }
        LspInstr::AssignForm(var, _, _form) => {
            inf.add_local(&var.name, VarType::Str);
        }
        LspInstr::Eval(expr) | LspInstr::JumpIfNot(expr, _) | LspInstr::ReturnF(Some(expr)) => {
            inf.infer_expr_lsp(expr);
        }
        LspInstr::Print(crate::lsp_ast::PrintContent::Expr(e), _) => {
            inf.infer_expr_lsp(e);
        }
        LspInstr::Print(crate::lsp_ast::PrintContent::Form(_f), _) => {}
        LspInstr::Print(crate::lsp_ast::PrintContent::Exprs(es), _) => {
            for e in es {
                inf.infer_expr_lsp(e);
            }
        }
        LspInstr::Call(call) => {
            for arg in &call.args {
                inf.infer_expr_lsp(arg);
            }
        }
        LspInstr::ForBegin(fb) => {
            inf.infer_expr_lsp(&fb.start);
            inf.infer_expr_lsp(&fb.end);
            if let Some(step) = &fb.step {
                inf.infer_expr_lsp(step);
            }
        }
        LspInstr::SelectEval(e, _, _) => {
            inf.infer_expr_lsp(e);
        }
        LspInstr::CaseTest(_, conds, _) => {
            for cond in conds {
                match cond {
                    crate::lsp_ast::CaseCond::Eq(e) | crate::lsp_ast::CaseCond::Is(_, e) => {
                        inf.infer_expr_lsp(e);
                    }
                    crate::lsp_ast::CaseCond::Range(a, b) => {
                        inf.infer_expr_lsp(a);
                        inf.infer_expr_lsp(b);
                    }
                }
            }
        }
        LspInstr::Command(cmd) => {
            if let crate::lsp_ast::Args::Exprs(es) = &cmd.1 {
                for e in es {
                    inf.infer_expr_lsp(e);
                }
            } else if let crate::lsp_ast::Args::Form(f) = &cmd.1 {
                // FORM would need special handling
            }
        }
        _ => {}
    }
}

/// Initialize with global dimensions (LSP version).
impl TypeInference {
    pub fn with_globals_lsp(globals: &[LspDimDecl]) -> Self {
        let mut inf = Self::new();
        for dim in globals {
            let var_type = if dim.is_str {
                if dim.dims.is_empty() {
                    VarType::Str
                } else {
                    VarType::StrArray(vec![])
                }
            } else {
                if dim.dims.is_empty() {
                    VarType::Int
                } else {
                    VarType::IntArray(vec![])
                }
            };
            inf.var_types.insert(dim.name.to_uppercase(), var_type);
            if dim.is_str {
                inf.global_str_vars.insert(dim.name.to_uppercase(), true);
            }
        }
        inf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast::{BinOp, Expr};

    #[test]
    fn test_type_inference_basic() {
        let inf = TypeInference::new();
        assert_eq!(inf.infer_expr(&Expr::Int(42)), VarType::Int);
        assert_eq!(inf.infer_expr(&Expr::Str("hello".into())), VarType::Str);
    }

    #[test]
    fn test_type_inference_binary() {
        let inf = TypeInference::new();

        // Int + Int = Int
        let add = Expr::Binary(BinOp::Add, Box::new(Expr::Int(1)), Box::new(Expr::Int(2)));
        assert_eq!(inf.infer_expr(&add), VarType::Int);

        // Str + Str = Str
        let add = Expr::Binary(
            BinOp::Add,
            Box::new(Expr::Str("a".into())),
            Box::new(Expr::Str("b".into())),
        );
        assert_eq!(inf.infer_expr(&add), VarType::Str);

        // Int + Str = Str
        let add = Expr::Binary(
            BinOp::Add,
            Box::new(Expr::Int(1)),
            Box::new(Expr::Str("b".into())),
        );
        assert_eq!(inf.infer_expr(&add), VarType::Str);
    }
}
