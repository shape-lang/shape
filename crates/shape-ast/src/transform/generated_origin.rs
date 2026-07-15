//! ADR-009 generated-closure provenance stamping.
//!
//! The capture gate reads compiler-issued provenance on each closure node,
//! replacing the former generated-function name predicate that failed across
//! monomorphization, replacement bodies, and nested closures. The exhaustive
//! walk makes new AST variants compile failures and stamps deterministic
//! structural paths (`extend:Type/method:name/closure:N`). Those compiler-issued
//! declaration-path labels are structural provenance; capture/binding spelling,
//! source spans, owner-display prose, and traversal/file order are not identity.
//! Independent serde tests prove recursive coverage over every carrier.

use crate::ast::expr_helpers::{BlockItem, ComprehensionClause, QueryClause};
use crate::ast::patterns::DestructurePattern;
use crate::ast::provenance::GeneratedNodeOrigin;
use crate::ast::statements::ForInit;
use crate::ast::windows::{WindowExpr, WindowFunction, WindowSpec};
use crate::ast::{Expr, ObjectEntry, Statement};

mod source_paths;
pub use source_paths::{GeneratedClosureSourcePath, generated_closure_source_paths};

/// Stamp every closure literal in a generated body (and every closure nested
/// inside those) with its provenance.
///
/// Idempotent: re-stamping an already-stamped body with the same origin
/// produces the same stamps (the path index is traversal-derived, not a
/// counter that advances across calls).
pub fn stamp_generated_closures(body: &mut [Statement], origin: &GeneratedNodeOrigin) {
    let mut walker = Stamper {
        origin: Some(origin),
        node_path: origin.node_path().to_vec(),
        source_paths: None,
        next_index: 0,
    };
    walker.statements(body);
}

struct Stamper<'origin, 'paths> {
    origin: Option<&'origin GeneratedNodeOrigin>,
    node_path: Vec<String>,
    source_paths: Option<&'paths mut Vec<GeneratedClosureSourcePath>>,
    next_index: u32,
}
impl Stamper<'_, '_> {
    fn statements(&mut self, stmts: &mut [Statement]) {
        for stmt in stmts {
            self.statement(stmt);
        }
    }

    fn statement(&mut self, stmt: &mut Statement) {
        match stmt {
            Statement::Return(value, _) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }
            Statement::Break(_) | Statement::Continue(_) | Statement::RemoveTarget(_) => {}
            Statement::VariableDecl(decl, _) => {
                self.destructure_pattern(&mut decl.pattern);
                if let Some(value) = decl.value.as_mut() {
                    self.expr(value);
                }
            }
            Statement::Assignment(assign, _) => {
                self.destructure_pattern(&mut assign.pattern);
                self.expr(&mut assign.value);
            }
            Statement::Expression(expr, _) => self.expr(expr),
            Statement::For(for_loop, _) => {
                match &mut for_loop.init {
                    ForInit::ForIn { pattern, iter } => {
                        self.destructure_pattern(pattern);
                        self.expr(iter);
                    }
                    ForInit::ForC {
                        init,
                        condition,
                        update,
                    } => {
                        self.statement(init);
                        self.expr(condition);
                        self.expr(update);
                    }
                }
                self.statements(&mut for_loop.body);
            }
            Statement::While(while_loop, _) => {
                self.expr(&mut while_loop.condition);
                self.statements(&mut while_loop.body);
            }
            Statement::If(if_stmt, _) => {
                self.expr(&mut if_stmt.condition);
                self.statements(&mut if_stmt.then_body);
                if let Some(else_body) = if_stmt.else_body.as_mut() {
                    self.statements(else_body);
                }
            }
            Statement::Extend(extend, _) => {
                for method in &mut extend.methods {
                    for param in &mut method.params {
                        if let Some(default) = param.default_value.as_mut() {
                            self.expr(default);
                        }
                    }
                    if let Some(when_clause) = method.when_clause.as_mut() {
                        self.expr(when_clause);
                    }
                    self.statements(&mut method.body);
                }
            }
            Statement::SetParamType { .. } | Statement::SetReturnType { .. } => {}
            Statement::SetParamTypeExpr { expression, .. }
            | Statement::SetParamValue { expression, .. }
            | Statement::SetReturnExpr { expression, .. }
            | Statement::ReplaceBodyExpr { expression, .. }
            | Statement::ReplaceModuleExpr { expression, .. }
            | Statement::ExtendItemsExpr { expression, .. } => self.expr(expression),
            Statement::ReplaceBody { body, .. } => self.statements(body),
        }
    }

    /// Exhaustive even though binding patterns carry no expressions today.
    fn destructure_pattern(&mut self, pattern: &mut DestructurePattern) {
        match pattern {
            DestructurePattern::Identifier(_, _) | DestructurePattern::Decomposition(_) => {}
            DestructurePattern::Array(elements) => {
                for element in elements {
                    self.destructure_pattern(element);
                }
            }
            DestructurePattern::Object(fields) => {
                for field in fields {
                    self.destructure_pattern(&mut field.pattern);
                }
            }
            DestructurePattern::Rest(inner) => self.destructure_pattern(inner),
        }
    }

    fn exprs(&mut self, exprs: &mut [Expr]) {
        for expr in exprs {
            self.expr(expr);
        }
    }

    fn named(&mut self, named: &mut [(String, Expr)]) {
        for (_, expr) in named {
            self.expr(expr);
        }
    }

    fn expr(&mut self, expr: &mut Expr) {
        match expr {
            // ── the node the whole module exists for ────────────────────────
            Expr::FunctionExpr {
                params,
                return_type: _,
                body,
                generated_origin,
                // Capture clauses are authored by the generator; stamping only
                // attaches provenance and never rewrites the declaration.
                captures,
                span: _,
            } => {
                let index = self.next_index;
                self.next_index += 1;
                let segment = format!("closure:{index}");
                let mut closure_path = self.node_path.clone();
                closure_path.push(segment.clone());
                if let Some(source_paths) = self.source_paths.as_deref_mut() {
                    source_paths.push(GeneratedClosureSourcePath {
                        node_path: closure_path.clone(),
                        params: params.clone(),
                        body: body.clone(),
                        captures: captures.clone(),
                    });
                }
                // Parameter defaults are evaluated in the ENCLOSING scope, so
                // they belong to the enclosing level's sibling numbering.
                for param in params.iter_mut() {
                    if let Some(default) = param.default_value.as_mut() {
                        self.expr(default);
                    }
                }
                let closure_origin = self.origin.map(|origin| origin.child(segment));
                if let Some(closure_origin) = closure_origin.as_ref() {
                    debug_assert_eq!(closure_origin.node_path(), closure_path);
                    *generated_origin = Some(closure_origin.clone());
                }
                // Closures nested in this body hang off THIS closure's path.
                let mut nested = Stamper {
                    origin: closure_origin.as_ref(),
                    node_path: closure_path,
                    source_paths: self.source_paths.as_deref_mut(),
                    next_index: 0,
                };
                nested.statements(body);
            }

            // ── leaves ──────────────────────────────────────────────────────
            Expr::Literal(_, _)
            | Expr::Identifier(_, _)
            | Expr::DataRef(_, _)
            | Expr::DataDateTimeRef(_, _)
            | Expr::TimeRef(_, _)
            | Expr::DateTime(_, _)
            | Expr::PatternRef(_, _)
            | Expr::TypeSyntax(_, _)
            | Expr::Duration(_, _)
            | Expr::Continue(_)
            | Expr::Unit(_) => {}

            // ── single-child carriers ───────────────────────────────────────
            Expr::DataRelativeAccess { reference, .. } => self.expr(reference),
            Expr::PropertyAccess { object, .. } => self.expr(object),
            Expr::UnaryOp { operand, .. } => self.expr(operand),
            Expr::Spread(inner, _) => self.expr(inner),
            Expr::TryOperator(inner, _) => self.expr(inner),
            Expr::UsingImpl { expr: inner, .. } => self.expr(inner),
            Expr::Await(inner, _) => self.expr(inner),
            Expr::AsyncScope(inner, _) => self.expr(inner),
            Expr::TypeAssertion {
                expr: inner,
                meta_param_overrides,
                ..
            } => {
                self.expr(inner);
                if let Some(overrides) = meta_param_overrides {
                    for value in overrides.values_mut() {
                        self.expr(value);
                    }
                }
            }
            Expr::InstanceOf { expr: inner, .. } => self.expr(inner),
            Expr::TimeframeContext { expr: inner, .. } => self.expr(inner),
            Expr::Reference { expr: inner, .. } => self.expr(inner),
            Expr::Annotated {
                annotation, target, ..
            } => {
                self.exprs(&mut annotation.args);
                self.expr(target);
            }
            Expr::Break(value, _) | Expr::Return(value, _) => {
                if let Some(value) = value {
                    self.expr(value);
                }
            }

            // ── multi-child carriers ────────────────────────────────────────
            Expr::IndexAccess {
                object,
                index,
                end_index,
                ..
            } => {
                self.expr(object);
                self.expr(index);
                if let Some(end_index) = end_index {
                    self.expr(end_index);
                }
            }
            Expr::BinaryOp { left, right, .. } | Expr::FuzzyComparison { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::FunctionCall {
                const_args,
                args,
                named_args,
                ..
            }
            | Expr::QualifiedFunctionCall {
                const_args,
                args,
                named_args,
                ..
            } => {
                self.exprs(const_args);
                self.exprs(args);
                self.named(named_args);
            }
            Expr::MethodCall {
                receiver,
                args,
                named_args,
                ..
            } => {
                self.expr(receiver);
                self.exprs(args);
                self.named(named_args);
            }
            Expr::EnumConstructor { payload, .. } => match payload {
                crate::ast::expressions::EnumConstructorPayload::Unit => {}
                crate::ast::expressions::EnumConstructorPayload::Tuple(values) => {
                    self.exprs(values)
                }
                crate::ast::expressions::EnumConstructorPayload::Struct(fields) => {
                    self.named(fields)
                }
            },
            Expr::Conditional {
                condition,
                then_expr,
                else_expr,
                ..
            } => {
                self.expr(condition);
                self.expr(then_expr);
                if let Some(else_expr) = else_expr {
                    self.expr(else_expr);
                }
            }
            Expr::Object(entries, _) => {
                for entry in entries {
                    match entry {
                        ObjectEntry::Field { value, .. } => self.expr(value),
                        ObjectEntry::Spread(value) => self.expr(value),
                    }
                }
            }
            Expr::Array(elements, _) => self.exprs(elements),
            Expr::TableRows(rows, _) => {
                for row in rows {
                    self.exprs(row);
                }
            }
            Expr::StructLiteral { fields, .. } => self.named(fields),
            Expr::SimulationCall { params, .. } => self.named(params),
            Expr::ListComprehension(comprehension, _) => {
                self.expr(&mut comprehension.element);
                for ComprehensionClause {
                    pattern,
                    iterable,
                    filter,
                } in &mut comprehension.clauses
                {
                    self.destructure_pattern(pattern);
                    self.expr(iterable);
                    if let Some(filter) = filter {
                        self.expr(filter);
                    }
                }
            }
            Expr::Block(block, _) => {
                for item in &mut block.items {
                    match item {
                        BlockItem::VariableDecl(decl) => {
                            self.destructure_pattern(&mut decl.pattern);
                            if let Some(value) = decl.value.as_mut() {
                                self.expr(value);
                            }
                        }
                        BlockItem::Assignment(assign) => {
                            self.destructure_pattern(&mut assign.pattern);
                            self.expr(&mut assign.value);
                        }
                        BlockItem::Statement(stmt) => self.statement(stmt),
                        BlockItem::Expression(expr) => self.expr(expr),
                    }
                }
            }
            Expr::If(if_expr, _) => {
                self.expr(&mut if_expr.condition);
                self.expr(&mut if_expr.then_branch);
                if let Some(else_branch) = if_expr.else_branch.as_mut() {
                    self.expr(else_branch);
                }
            }
            Expr::While(while_expr, _) => {
                self.expr(&mut while_expr.condition);
                self.expr(&mut while_expr.body);
            }
            Expr::For(for_expr, _) => {
                self.expr(&mut for_expr.iterable);
                self.expr(&mut for_expr.body);
            }
            Expr::Loop(loop_expr, _) => self.expr(&mut loop_expr.body),
            Expr::Let(let_expr, _) => {
                if let Some(value) = let_expr.value.as_mut() {
                    self.expr(value);
                }
                self.expr(&mut let_expr.body);
            }
            Expr::Assign(assign_expr, _) => {
                self.expr(&mut assign_expr.target);
                self.expr(&mut assign_expr.value);
            }
            Expr::Match(match_expr, _) => {
                self.expr(&mut match_expr.scrutinee);
                for arm in &mut match_expr.arms {
                    if let Some(guard) = arm.guard.as_mut() {
                        self.expr(guard);
                    }
                    self.expr(&mut arm.body);
                }
            }
            Expr::Range { start, end, .. } => {
                if let Some(start) = start {
                    self.expr(start);
                }
                if let Some(end) = end {
                    self.expr(end);
                }
            }
            Expr::Join(join_expr, _) => {
                for branch in &mut join_expr.branches {
                    for annotation in &mut branch.annotations {
                        self.exprs(&mut annotation.args);
                    }
                    self.expr(&mut branch.expr);
                }
            }
            Expr::AsyncLet(async_let, _) => self.expr(&mut async_let.expr),
            Expr::Comptime(body, _) => self.statements(body),
            Expr::ComptimeFor(comptime_for, _) => {
                self.expr(&mut comptime_for.iterable);
                self.statements(&mut comptime_for.body);
            }
            Expr::FromQuery(query, _) => {
                self.expr(&mut query.source);
                for clause in &mut query.clauses {
                    match clause {
                        QueryClause::Where(condition) => self.expr(condition),
                        QueryClause::OrderBy(specs) => {
                            for spec in specs {
                                self.expr(&mut spec.key);
                            }
                        }
                        QueryClause::GroupBy { element, key, .. } => {
                            self.expr(element);
                            self.expr(key);
                        }
                        QueryClause::Join {
                            source,
                            left_key,
                            right_key,
                            ..
                        } => {
                            self.expr(source);
                            self.expr(left_key);
                            self.expr(right_key);
                        }
                        QueryClause::Let { value, .. } => self.expr(value),
                    }
                }
                self.expr(&mut query.select);
            }
            Expr::WindowExpr(window, _) => self.window_expr(window),
        }
    }

    fn window_expr(&mut self, window: &mut WindowExpr) {
        let WindowExpr { function, over } = window;
        match function {
            WindowFunction::Lag {
                expr,
                default,
                offset: _,
            }
            | WindowFunction::Lead {
                expr,
                default,
                offset: _,
            } => {
                self.expr(expr);
                if let Some(default) = default {
                    self.expr(default);
                }
            }
            WindowFunction::RowNumber
            | WindowFunction::Rank
            | WindowFunction::DenseRank
            | WindowFunction::Ntile(_) => {}
            WindowFunction::FirstValue(expr)
            | WindowFunction::LastValue(expr)
            | WindowFunction::NthValue(expr, _)
            | WindowFunction::Sum(expr)
            | WindowFunction::Avg(expr)
            | WindowFunction::Min(expr)
            | WindowFunction::Max(expr) => self.expr(expr),
            WindowFunction::Count(expr) => {
                if let Some(expr) = expr {
                    self.expr(expr);
                }
            }
        }
        let WindowSpec {
            partition_by,
            order_by,
            frame: _,
        } = over;
        self.exprs(partition_by);
        if let Some(order_by) = order_by {
            for (key, _direction) in &mut order_by.columns {
                self.expr(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{GeneratedNodeIssuer, Span};
    use crate::parse_program;

    fn origin() -> GeneratedNodeOrigin {
        GeneratedNodeIssuer::new().issue(
            (0x1122_3344_5566_7788, 0x0102_0304_0506_0708),
            vec!["extend:Job".to_string(), "method:read".to_string()],
            7,
            Span { start: 10, end: 40 },
            "Job.read".to_string(),
        )
    }

    fn body_of(source: &str) -> Vec<Statement> {
        let program = parse_program(source).expect("fixture must parse");
        for item in &program.items {
            if let crate::ast::Item::Function(func, _) = item
                && func.name == "generated"
            {
                return func.body.clone();
            }
        }
        panic!("fixture must declare `fn generated`");
    }

    /// TOTALITY PROOF (see module docs). The oracle is serde's derived
    /// traversal — total by construction, sharing no code with the walk — so a
    /// closure carrier this walk fails to recurse into shows up as a surviving
    /// `"generated_origin":null`.
    #[test]
    fn stamp_is_total_over_every_syntactic_closure_position() {
        let source = r#"
fn generated(n: int) -> int {
  let direct = || 1
  let nested = || { let inner = || 2; inner() }
  let in_call = apply(|| 3)
  let in_method = xs.map(|x| x + 1)
  let in_array = [|| 4, || 5]
  let in_object = { a: || 6 }
  let in_struct = Holder { f: || 7 }
  let in_binary = pick(|| 8) + pick(|| 9)
  let in_unary = -pick(|| 10)
  let in_index = table[pick(|| 11)]
  let in_conditional = if n > 0 { pick(|| 12) } else { pick(|| 13) }
  let in_named_arg = call(cb: || 14)
  let in_spread = [...pick(|| 15)]
  let in_range = pick(|| 16)..pick(|| 17)
  let in_try = pick(|| 18)?
  let in_await = await pick(|| 19)
  let in_ref = &pick(|| 20)
  let in_match = match n { 0 => pick(|| 21), _ => pick(|| 22) }
  let in_block = { let b = || 23; b() }
  let in_property = pick(|| 24).field
  let in_comprehension = [pick(|| 25) for y in ys if guard(|| 26)]
  let in_cast = pick(|| 27) as int
  for item in pick(|| 29) {
    let in_for_body = || 30
    in_for_body()
  }
  while guard(|| 31) {
    let in_while_body = || 32
    in_while_body()
  }
  if guard(|| 33) {
    let in_if_body = || 34
    in_if_body()
  } else {
    let in_else_body = || 35
    in_else_body()
  }
  let in_pipe = xs |> map(|x| x)
  n
}
"#;
        let mut body = body_of(source);
        let origin = origin();
        stamp_generated_closures(&mut body, &origin);

        let json = serde_json::to_string(&body).expect("body must serialize");
        assert!(
            json.contains("\"generated_origin\""),
            "fixture must actually contain closures"
        );
        let unstamped = json.matches("\"generated_origin\":null").count();
        assert_eq!(
            unstamped, 0,
            "the walk missed {unstamped} closure node(s) — an unrecursed AST carrier is a \
             hole in the Wave-46 capture gate (R4 totality)"
        );
    }

    /// Provenance DATA survives a serde round-trip, while its compiler-instance
    /// authority is deliberately erased (proved in `ast::provenance`). The
    /// compiler must re-stamp generated payloads before trusting them.
    #[test]
    fn stamp_data_survives_serde_round_trip() {
        let mut body = body_of("fn generated() -> int { let w = || 1; w() }");
        stamp_generated_closures(&mut body, &origin());
        let json = serde_json::to_string(&body).unwrap();
        let round_tripped: Vec<Statement> = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, body);
        assert_eq!(first_closure_origin(&round_tripped), Some(closure_path(0)));
    }

    /// The `__emit_extend` comptime directive ships its payload as JSON of an
    /// `ExtendStatement` (`comptime_builtins.rs`'s `serde_json::from_str`), so
    /// the diagnostic data has to survive at THAT type, not just at
    /// `Vec<Statement>`; authority is re-issued on compiler ingestion.
    #[test]
    fn stamp_survives_the_emit_extend_payload_type() {
        let program = parse_program(
            "extend Job { method read() -> int { let v = 41; let w = || v + 1; w() } }",
        )
        .expect("fixture must parse");
        let mut extend = program
            .items
            .into_iter()
            .find_map(|item| match item {
                crate::ast::Item::Extend(extend, _) => Some(extend),
                _ => None,
            })
            .expect("fixture must declare an extend block");
        for method in &mut extend.methods {
            stamp_generated_closures(&mut method.body, &origin());
        }

        let payload = serde_json::to_string(&extend).expect("payload must serialize");
        let round_tripped: crate::ast::types::ExtendStatement =
            serde_json::from_str(&payload).expect("payload must parse back");

        assert_eq!(
            first_closure_origin(&round_tripped.methods[0].body),
            Some(closure_path(0))
        );
    }

    /// An UNSTAMPED body deserializes with `generated_origin: None` — the
    /// `#[serde(default)]` back-compat leg (an old payload has no such key).
    #[test]
    fn absent_field_deserializes_as_ordinary_source() {
        let body = body_of("fn generated() -> int { let w = || 1; w() }");
        let json = serde_json::to_string(&body).unwrap();
        let stripped = json.replace(",\"generated_origin\":null", "");
        assert!(!stripped.contains("generated_origin"));
        let round_tripped: Vec<Statement> = serde_json::from_str(&stripped).unwrap();
        assert_eq!(first_closure_origin(&round_tripped), None);
    }

    #[test]
    fn nested_closures_extend_the_parent_path_and_siblings_are_indexed() {
        let mut body = body_of(
            "fn generated() -> int { let a = || 1; let b = || { let c = || 2; c() }; a() + b() }",
        );
        stamp_generated_closures(&mut body, &origin());
        let mut paths = Vec::new();
        collect_paths(&body, &mut paths);
        assert_eq!(
            paths,
            vec![
                vec![
                    "extend:Job".to_string(),
                    "method:read".to_string(),
                    "closure:0".to_string()
                ],
                vec![
                    "extend:Job".to_string(),
                    "method:read".to_string(),
                    "closure:1".to_string()
                ],
                vec![
                    "extend:Job".to_string(),
                    "method:read".to_string(),
                    "closure:1".to_string(),
                    "closure:0".to_string()
                ],
            ]
        );
    }

    #[test]
    fn stamping_is_idempotent() {
        let mut once = body_of(
            "fn generated() -> int { let a = || 1; let b = || { let c = || 2; c() }; a() + b() }",
        );
        stamp_generated_closures(&mut once, &origin());
        let mut twice = once.clone();
        stamp_generated_closures(&mut twice, &origin());
        assert_eq!(once, twice);
    }

    fn closure_path(index: u32) -> Vec<String> {
        vec![
            "extend:Job".to_string(),
            "method:read".to_string(),
            format!("closure:{index}"),
        ]
    }

    fn first_closure_origin(body: &[Statement]) -> Option<Vec<String>> {
        let mut paths = Vec::new();
        collect_paths(body, &mut paths);
        paths.into_iter().next()
    }

    fn collect_paths(body: &[Statement], out: &mut Vec<Vec<String>>) {
        // Deliberately independent of the walk: serde the tree and read the
        // stamps back out in document order.
        let value = serde_json::to_value(body).unwrap();
        fn visit(value: &serde_json::Value, out: &mut Vec<Vec<String>>) {
            match value {
                serde_json::Value::Object(map) => {
                    if let Some(path) = value.pointer("/FunctionExpr/generated_origin/node_path") {
                        out.push(
                            path.as_array()
                                .unwrap()
                                .iter()
                                .map(|s| s.as_str().unwrap().to_string())
                                .collect(),
                        );
                    }
                    for v in map.values() {
                        visit(v, out);
                    }
                }
                serde_json::Value::Array(items) => {
                    for v in items {
                        visit(v, out);
                    }
                }
                _ => {}
            }
        }
        visit(&value, out);
    }
}
