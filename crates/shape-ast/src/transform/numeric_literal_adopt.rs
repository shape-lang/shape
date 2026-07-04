//! Numeric-conversion LITERAL ADOPTION — AST pass (numeric-conversion-spec §4,
//! THE RULE user 2026-06-01).
//!
//! When a bare integer literal is accepted in a `number`(f64) context, the
//! literal `5` IS the number literal `5.0` and must lower as a FLOAT constant.
//! The type checker only *accepts* the adoption (it leaves the AST node a
//! `Literal::Int`); if the node stays `Int`, BOTH lowering paths emit an i64
//! constant into a Float64-stamped slot — a bit-reinterpret soundness hole
//! (`takes_num(5)` reading raw i64 `5` as f64 bits → `2.5e-323`).
//!
//! This pass runs once over the AST (right after `desugar_program`, before
//! bytecode compilation AND before MIR lowering — both consume the same mutated
//! AST), and re-types every adopting bare int literal to `Literal::Number` at
//! the annotation-driven adoption sites:
//!   - `let x: number = 5` / `let x: Array<number> = [1, 2, 3]`
//!   - `f(5)` where `fn f(x: number)` (call-argument vs declared param type)
//!   - `fn g() -> number { 5 }` / `return 5` (tail/explicit return vs return type)
//!   - `P { x: 7 }` where `x: number` (struct field init)
//!   - `fn h(x: number = 5)` (parameter default)
//!
//! This is COMPILE-TIME literal re-typing, NOT a runtime coercion opcode — the
//! literal `5` is exactly `5.0` in a number context (no `IntToNumber`/
//! `Convert<X>To<Y>` opcode, no W4-δ defection). It is gated on losslessness
//! (the f64 exact-integer range `[-2^53, 2^53]`); an out-of-range int literal
//! is NOT rewritten and stays its natural `int`, which then fails the §2
//! lossless lattice against the float target downstream. A NON-literal `int`
//! value is never rewritten — `let m: int = 5; let n: number = m` stays a
//! COMPILE ERROR (`int` and `number` never unify; the p-var rejection is
//! upstream in the type checker and not weakened here).

use std::collections::HashMap;

use crate::ast::{Expr, Item, Literal, ObjectEntry, Statement, TypeAnnotation};

/// f64 exact-integer lossless range `[-2^53, 2^53]`.
fn fits_f64_lossless(v: i128) -> bool {
    v >= -(1i128 << 53) && v <= (1i128 << 53)
}

/// Whether `name` denotes the `number`/`f64` floating-point family.
fn is_number_type_name(name: &str) -> bool {
    matches!(name, "number" | "f64" | "float")
}

/// The element type name of an `Array<T>` / `T[]` annotation, if any.
fn array_element_name(ann: &TypeAnnotation) -> Option<&str> {
    match ann {
        TypeAnnotation::Generic { name, args } if name.as_str() == "Array" && args.len() == 1 => {
            args[0].as_type_name_str()
        }
        TypeAnnotation::Array(inner) => inner.as_type_name_str(),
        _ => None,
    }
}

/// Rewrite `expr` in place to a `Number` literal when it is a bare adoptable int
/// literal whose value losslessly fits f64. No-op otherwise.
fn widen_int_literal_in_place(expr: &mut Expr) {
    let (v, span) = match expr {
        Expr::Literal(Literal::Int(v), span) => (*v as i128, *span),
        Expr::Literal(Literal::UInt(v), span) => (*v as i128, *span),
        _ => return,
    };
    if !fits_f64_lossless(v) {
        return;
    }
    *expr = Expr::Literal(Literal::Number(v as f64), span);
}

/// Apply scalar-`number` adoption to `expr` when the target annotation is the
/// `number`/`f64` scalar family; recurses into the expr first.
fn widen_expr_against_annotation(expr: &mut Expr, ann: &TypeAnnotation, ctx: &SigCtx) {
    widen_expr(expr, ctx);
    if let Some(name) = ann.as_type_name_str() {
        if is_number_type_name(name) {
            widen_int_literal_in_place(expr);
            return;
        }
    }
    // `Array<number>` / `number[]` annotation on an array literal: adopt every
    // bare int element.
    if let (Some(elem_name), Expr::Array(elements, _)) = (array_element_name(ann), &mut *expr) {
        if is_number_type_name(elem_name) {
            for e in elements.iter_mut() {
                widen_int_literal_in_place(e);
            }
        }
    }
}

/// Function-signature + struct-field context collected from the program, used to
/// resolve the adoption target type at call-argument / struct-field sites.
struct SigCtx {
    /// fn name → (param annotations, return annotation).
    fns: HashMap<String, (Vec<Option<TypeAnnotation>>, Option<TypeAnnotation>)>,
    /// struct/type name → (field name → field annotation).
    structs: HashMap<String, HashMap<String, TypeAnnotation>>,
}

impl SigCtx {
    fn collect(program: &crate::ast::Program) -> Self {
        let mut fns = HashMap::new();
        let mut structs = HashMap::new();
        for item in &program.items {
            Self::collect_item(item, &mut fns, &mut structs);
        }
        SigCtx { fns, structs }
    }

    fn collect_item(
        item: &Item,
        fns: &mut HashMap<String, (Vec<Option<TypeAnnotation>>, Option<TypeAnnotation>)>,
        structs: &mut HashMap<String, HashMap<String, TypeAnnotation>>,
    ) {
        match item {
            Item::Function(func, _) => {
                let params = func
                    .params
                    .iter()
                    .map(|p| p.type_annotation.clone())
                    .collect();
                fns.insert(func.name.clone(), (params, func.return_type.clone()));
            }
            Item::StructType(def, _) => {
                let fields = def
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.type_annotation.clone()))
                    .collect();
                structs.insert(def.name.clone(), fields);
            }
            Item::Module(module, _) => {
                for inner in &module.items {
                    Self::collect_item(inner, fns, structs);
                }
            }
            Item::Export(export, _) => {
                if let crate::ast::ExportItem::Function(func) = &export.item {
                    let params = func
                        .params
                        .iter()
                        .map(|p| p.type_annotation.clone())
                        .collect();
                    fns.insert(func.name.clone(), (params, func.return_type.clone()));
                }
            }
            _ => {}
        }
    }
}

/// Public entry point: re-type every annotation-driven adopting int literal in
/// the program to a `Number` literal.
pub fn widen_numeric_literals(program: &mut crate::ast::Program) {
    let ctx = SigCtx::collect(program);
    for item in &mut program.items {
        widen_item(item, &ctx);
    }
}

fn widen_item(item: &mut Item, ctx: &SigCtx) {
    match item {
        Item::Function(func, _) => {
            widen_fn_param_defaults(&mut func.params, ctx);
            widen_body(&mut func.body, func.return_type.clone(), ctx);
        }
        Item::Export(export, _) => {
            if let crate::ast::ExportItem::Function(func) = &mut export.item {
                widen_fn_param_defaults(&mut func.params, ctx);
                widen_body(&mut func.body, func.return_type.clone(), ctx);
            }
        }
        Item::VariableDecl(decl, _) => {
            if let Some(value) = &mut decl.value {
                if let Some(ann) = &decl.type_annotation {
                    widen_expr_against_annotation(value, ann, ctx);
                } else {
                    widen_expr(value, ctx);
                }
            }
        }
        Item::Assignment(assign, _) => widen_expr(&mut assign.value, ctx),
        Item::Expression(expr, _) => widen_expr(expr, ctx),
        Item::Statement(stmt, _) => widen_statement(stmt, None, ctx),
        Item::Module(module, _) => {
            for inner in &mut module.items {
                widen_item(inner, ctx);
            }
        }
        _ => {}
    }
}

fn widen_fn_param_defaults(params: &mut [crate::ast::FunctionParameter], ctx: &SigCtx) {
    for p in params.iter_mut() {
        if let (Some(ann), Some(default)) = (&p.type_annotation, &mut p.default_value) {
            widen_expr_against_annotation(default, ann, ctx);
        }
    }
}

fn widen_body(body: &mut [Statement], return_type: Option<TypeAnnotation>, ctx: &SigCtx) {
    let len = body.len();
    for (idx, stmt) in body.iter_mut().enumerate() {
        // The tail expression statement of a fn body is an implicit return —
        // adopt the declared return type.
        let is_tail = idx + 1 == len;
        if is_tail {
            if let (Statement::Expression(expr, _), Some(ret)) = (&mut *stmt, &return_type) {
                widen_expr_against_annotation(expr, ret, ctx);
                continue;
            }
        }
        widen_statement(stmt, return_type.as_ref(), ctx);
    }
}

fn widen_statement(stmt: &mut Statement, return_type: Option<&TypeAnnotation>, ctx: &SigCtx) {
    match stmt {
        Statement::VariableDecl(decl, _) => {
            if let Some(value) = &mut decl.value {
                if let Some(ann) = &decl.type_annotation {
                    widen_expr_against_annotation(value, ann, ctx);
                } else {
                    widen_expr(value, ctx);
                }
            }
        }
        Statement::Return(Some(value), _) => {
            if let Some(ret) = return_type {
                widen_expr_against_annotation(value, ret, ctx);
            } else {
                widen_expr(value, ctx);
            }
        }
        Statement::Return(None, _) => {}
        Statement::Assignment(assign, _) => widen_expr(&mut assign.value, ctx),
        Statement::Expression(expr, _) => widen_expr(expr, ctx),
        Statement::For(for_loop, _) => {
            match &mut for_loop.init {
                crate::ast::ForInit::ForIn { iter, .. } => widen_expr(iter, ctx),
                crate::ast::ForInit::ForC {
                    init,
                    condition,
                    update,
                } => {
                    widen_statement(init, return_type, ctx);
                    widen_expr(condition, ctx);
                    widen_expr(update, ctx);
                }
            }
            for s in &mut for_loop.body {
                widen_statement(s, return_type, ctx);
            }
        }
        Statement::While(while_loop, _) => {
            widen_expr(&mut while_loop.condition, ctx);
            for s in &mut while_loop.body {
                widen_statement(s, return_type, ctx);
            }
        }
        Statement::If(if_stmt, _) => {
            widen_expr(&mut if_stmt.condition, ctx);
            for s in &mut if_stmt.then_body {
                widen_statement(s, return_type, ctx);
            }
            if let Some(else_body) = &mut if_stmt.else_body {
                for s in else_body {
                    widen_statement(s, return_type, ctx);
                }
            }
        }
        _ => {}
    }
}

/// Recurse into `expr`, applying call-argument / struct-field adoption at the
/// nodes that carry their own declared-type context, and widening sub-exprs.
fn widen_expr(expr: &mut Expr, ctx: &SigCtx) {
    match expr {
        Expr::FunctionCall {
            name,
            const_args,
            args,
            named_args,
            ..
        } => {
            for arg in const_args {
                widen_expr(arg, ctx);
            }
            if let Some((params, _)) = ctx.fns.get(name) {
                for (i, arg) in args.iter_mut().enumerate() {
                    if let Some(Some(ann)) = params.get(i) {
                        widen_expr_against_annotation(arg, ann, ctx);
                    } else {
                        widen_expr(arg, ctx);
                    }
                }
            } else {
                for arg in args.iter_mut() {
                    widen_expr(arg, ctx);
                }
            }
            for (_, v) in named_args.iter_mut() {
                widen_expr(v, ctx);
            }
        }
        Expr::StructLiteral {
            type_name, fields, ..
        } => {
            let struct_fields = ctx.structs.get(type_name.as_str());
            for (fname, fexpr) in fields.iter_mut() {
                if let Some(ann) = struct_fields.and_then(|m| m.get(fname)) {
                    widen_expr_against_annotation(fexpr, ann, ctx);
                } else {
                    widen_expr(fexpr, ctx);
                }
            }
        }
        Expr::Object(entries, _) => {
            for entry in entries.iter_mut() {
                match entry {
                    ObjectEntry::Field { value, .. } => widen_expr(value, ctx),
                    ObjectEntry::Spread(e) => widen_expr(e, ctx),
                }
            }
        }
        Expr::Array(elements, _) => {
            for e in elements.iter_mut() {
                widen_expr(e, ctx);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            widen_expr(left, ctx);
            widen_expr(right, ctx);
        }
        Expr::UnaryOp { operand, .. } => widen_expr(operand, ctx),
        Expr::PropertyAccess { object, .. } => widen_expr(object, ctx),
        Expr::IndexAccess {
            object,
            index,
            end_index,
            ..
        } => {
            widen_expr(object, ctx);
            widen_expr(index, ctx);
            if let Some(e) = end_index {
                widen_expr(e, ctx);
            }
        }
        Expr::Conditional {
            condition,
            then_expr,
            else_expr,
            ..
        } => {
            widen_expr(condition, ctx);
            widen_expr(then_expr, ctx);
            if let Some(e) = else_expr {
                widen_expr(e, ctx);
            }
        }
        Expr::MethodCall {
            receiver,
            args,
            named_args,
            ..
        } => {
            widen_expr(receiver, ctx);
            for arg in args.iter_mut() {
                widen_expr(arg, ctx);
            }
            for (_, v) in named_args.iter_mut() {
                widen_expr(v, ctx);
            }
        }
        Expr::FunctionExpr { body, .. } => {
            for s in body.iter_mut() {
                widen_statement(s, None, ctx);
            }
        }
        Expr::Block(block, _) => {
            for item in &mut block.items {
                match item {
                    crate::ast::BlockItem::VariableDecl(decl) => {
                        if let Some(value) = &mut decl.value {
                            if let Some(ann) = &decl.type_annotation {
                                widen_expr_against_annotation(value, ann, ctx);
                            } else {
                                widen_expr(value, ctx);
                            }
                        }
                    }
                    crate::ast::BlockItem::Assignment(assign) => widen_expr(&mut assign.value, ctx),
                    crate::ast::BlockItem::Statement(stmt) => widen_statement(stmt, None, ctx),
                    crate::ast::BlockItem::Expression(e) => widen_expr(e, ctx),
                }
            }
        }
        Expr::TypeAssertion { expr: inner, .. } => widen_expr(inner, ctx),
        Expr::InstanceOf { expr: inner, .. } => widen_expr(inner, ctx),
        Expr::Spread(inner, _) => widen_expr(inner, ctx),
        Expr::TryOperator(inner, _) => widen_expr(inner, ctx),
        Expr::UsingImpl { expr: inner, .. } => widen_expr(inner, ctx),
        Expr::Await(inner, _) => widen_expr(inner, ctx),
        Expr::TimeframeContext { expr: inner, .. } => widen_expr(inner, ctx),
        Expr::Return(Some(inner), _) => widen_expr(inner, ctx),
        Expr::Break(Some(inner), _) => widen_expr(inner, ctx),
        Expr::Range { start, end, .. } => {
            if let Some(s) = start {
                widen_expr(s, ctx);
            }
            if let Some(e) = end {
                widen_expr(e, ctx);
            }
        }
        Expr::If(if_expr, _) => {
            widen_expr(&mut if_expr.condition, ctx);
            widen_expr(&mut if_expr.then_branch, ctx);
            if let Some(e) = &mut if_expr.else_branch {
                widen_expr(e, ctx);
            }
        }
        Expr::While(while_expr, _) => {
            widen_expr(&mut while_expr.condition, ctx);
            widen_expr(&mut while_expr.body, ctx);
        }
        Expr::For(for_expr, _) => {
            widen_expr(&mut for_expr.iterable, ctx);
            widen_expr(&mut for_expr.body, ctx);
        }
        Expr::Loop(loop_expr, _) => widen_expr(&mut loop_expr.body, ctx),
        Expr::Let(let_expr, _) => {
            if let Some(val) = &mut let_expr.value {
                widen_expr(val, ctx);
            }
            widen_expr(&mut let_expr.body, ctx);
        }
        Expr::Assign(assign, _) => {
            widen_expr(&mut assign.target, ctx);
            widen_expr(&mut assign.value, ctx);
        }
        Expr::Match(match_expr, _) => {
            widen_expr(&mut match_expr.scrutinee, ctx);
            for arm in &mut match_expr.arms {
                if let Some(guard) = &mut arm.guard {
                    widen_expr(guard, ctx);
                }
                widen_expr(&mut arm.body, ctx);
            }
        }
        // Leaf / context-free nodes — nothing to adopt, nothing to recurse.
        _ => {}
    }
}
