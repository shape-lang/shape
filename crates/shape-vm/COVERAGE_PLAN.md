# Shape VM Feature Coverage Plan

**Current Coverage**: 41.9% (119/284 grammar rules)
**Target**: 100% coverage of testable grammar rules

## Rules to Skip (Internal/Helper Rules)

These rules are internal implementation details and don't need direct tests:

- `COMMENT`, `WHITESPACE` - Lexer rules
- `program`, `item` - Top-level structural rules
- `*_no_range` variants - Internal disambiguation rules (and_expr_no_range, etc.)
- `balanced_ternary`, `ternary_lookahead` - Parser lookahead helpers
- `number_part` - Internal number parsing

## Category 1: Type System (12 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `type_alias_def` | `type Point = { x: number; y: number };` | High |
| `interface_def` | `interface Shape { area(): number; }` | High |
| `interface_body`, `interface_member` | (covered by interface_def) | - |
| `enum_def`, `enum_member`, `enum_members` | `enum Color { Red, Green, Blue }` | High |
| `function_type` | `type Handler = (x: number) => string;` | Medium |
| `union_type` | `type Result = Success \| Error;` | Medium |
| `optional_type` | `let x: number?;` | Medium |
| `type_param`, `type_param_name`, `type_params` | `function map<T, U>(arr: T[]): U[]` | Medium |
| `extends_clause` | `interface Square extends Shape {}` | Medium |
| `non_array_type` | (internal helper) | Skip |

## Category 2: Module System (8 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `import_stmt` | `import { foo } from "./module";` | High |
| `import_spec`, `import_item`, `import_item_list` | (covered by import_stmt) | - |
| `export_item` | `export function foo() {}` | High |
| `export_spec`, `export_spec_list` | `export { foo, bar as baz };` | Medium |
| `module_decl` | `module Utils { ... }` | Low |

## Category 3: Pattern Definitions (12 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `pattern_def` | `@pattern function hammer(c) { ... }` | High |
| `pattern_body`, `pattern_statement`, `pattern_statement_list` | (covered by pattern_def) | - |
| `pattern_param`, `pattern_param_list`, `pattern_params` | (covered by pattern_def) | - |
| `pattern_ref` | `find(hammer)` | Medium |
| `inline_pattern` | `find({ c => c.close > c.open })` | Medium |
| `pattern_identifier`, `pattern_literal`, `pattern_wildcard` | (match patterns) | Medium |
| `pattern_constructor_name` | `Ok(x)`, `Some(y)` - already tested | Done |

## Category 4: Query/SQL Features (25 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `query` | Top-level query wrapper | Skip |
| `alert_query`, `alert_options` | `alert when price > 100 message "..."` | Low |
| `with_query` | `WITH cte AS (...) SELECT ...` | Low |
| `cte_def`, `cte_list`, `cte_columns` | (covered by with_query) | - |
| `inner_query`, `recursive_keyword` | (CTE internals) | - |
| `join_clause`, `join_type`, `join_source`, `join_condition` | `data JOIN other ON ...` | Medium |
| `group_by_clause`, `group_by_list`, `group_by_expr` | `.group(x => x.category)` | Medium |
| `having_clause` | `.having(count > 10)` | Medium |
| `order_by_clause`, `order_by_list`, `order_by_item`, `sort_direction` | `.orderBy("name", "desc")` | Medium |
| `limit_clause` | `.limit(10)` | Medium |
| `where_clause` | `.where(x > 5)` | Medium |
| `on_clause` | `on(1h) { ... }` | Medium |

## Category 5: Window Functions (12 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `window_function_call` | `row_number() OVER (...)` | Medium |
| `window_function_name`, `window_function_args` | (covered above) | - |
| `over_clause`, `window_spec` | `OVER (PARTITION BY x ORDER BY y)` | Medium |
| `partition_by_clause` | (covered above) | - |
| `window_frame_clause`, `frame_type`, `frame_extent`, `frame_bound` | `ROWS BETWEEN ...` | Low |
| `window_args`, `window_range` | (internal) | Skip |

## Category 6: Time Windows (8 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `time_window` | `last(5, "days")` | Medium |
| `last_window` | `last 1 year` | Medium |
| `between_window` | `between @"2020-01-01" and @"2021-01-01"` | Medium |
| `session_window` | `session(30m)` | Low |
| `time_interval`, `time_unit` | (internal) | Skip |
| `timeframe_spec` | `data(5m)[0]` | Medium |

## Category 7: Datetime Operations (5 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `datetime_arithmetic` | `@today + 5d` | Medium |
| `datetime_op`, `datetime_primary` | (internal) | Skip |
| `quoted_time`, `relative_time`, `timezone` | `"2020-01-01"`, `"EST"` | Low |

## Category 8: Testing Framework (15 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `test_def` | `test "description" { ... }` | Medium |
| `test_body`, `test_case`, `test_statements`, `test_statement` | (covered above) | - |
| `test_setup`, `test_teardown`, `test_fixture_statement` | `setup { ... }` | Low |
| `test_tag`, `test_tags` | `@slow test "..." { }` | Low |
| `assert_statement` | `assert x == 5;` | Medium |
| `expect_statement`, `expectation_matcher` | `expect(x).toBe(5);` | Low |
| `should_statement`, `should_matcher` | `x should equal 5;` | Low |
| `test_match_option`, `test_match_options` | (internal) | Skip |

## Category 9: Streams (12 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `stream_def` | `stream MyStream { ... }` | Low |
| `stream_body`, `stream_config`, `stream_config_item`, `stream_config_list` | (covered above) | - |
| `stream_state`, `stream_state_list` | `state { count: 0 }` | Low |
| `stream_on_*` | `on_bar { ... }`, `on_tick { ... }` | Low |

## Category 10: Annotations (6 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `annotation` | `@deprecated` | Medium |
| `annotations` | `@deprecated @inline` | Medium |
| `annotation_args` | `@timeout(5000)` | Medium |
| `annotation_def`, `annotation_def_params` | `annotation @custom(x) { }` | Low |
| `annotation_name` | (internal) | Skip |

## Category 11: Miscellaneous (20 rules)

| Rule | Test Code | Priority |
|------|-----------|----------|
| `pipe_expr` | `data \|> filter(x => x > 0) \|> map(x => x * 2)` | High |
| `aggregation_expr`, `aggregation_function` | `sum(x)`, `avg(y)` | Medium |
| `calculate_clause`, `calculate_expr`, `calculate_list` | (internal) | Skip |
| `some_expr` | `Some(42)` - already tested | Done |
| `try_operator` | `risky_fn()?` | Medium |
| `named_arg` | `foo(x: 1, y: 2)` | Medium |
| `argument` | (internal) | Skip |
| `method_def` | `extend Array { method sum() { } }` | Low |
| `extend_statement` | `extend String { ... }` | Low |
| `optimize_statement` | `@optimize(simd) function ...` | Low |
| `metric_expr`, `metric_list` | (internal) | Skip |
| `param`, `param_list`, `param_range` | (internal query params) | Skip |
| `condition`, `condition_list`, `when_clause` | (pattern internals) | Skip |
| `analysis_target`, `market_keyword`, `symbol_list` | (query targets) | Skip |
| `threshold`, `weight` | (internal) | Skip |
| `block_statement` | `{ stmt1; stmt2; }` | Low |
| `destructure_ident_pattern` | `let { x }` shorthand | Medium |
| `range_op` | (internal) | Skip |
| `object_type_member_list` | (internal) | Skip |
| `compound_duration` | `5h30m` | Medium |

---

## Implementation Priority

### Phase 1: High Priority (Estimated: 15 tests)
Core language features that are commonly used:

1. Type aliases, interfaces, enums
2. Import/export statements
3. Pattern definitions with @pattern
4. Pipe expressions

### Phase 2: Medium Priority (Estimated: 25 tests)
Important but less common features:

1. Query clauses (group_by, order_by, where, having)
2. Window functions
3. Time windows and datetime operations
4. Testing framework basics
5. Annotations
6. Named arguments, try operator

### Phase 3: Low Priority (Estimated: 15 tests)
Advanced/specialized features:

1. Streams
2. Advanced testing (setup/teardown)
3. Module declarations
4. Type extensions
5. Window frame clauses

### Skip (~40 rules)
Internal helper rules that don't need direct tests.

---

## Expected Final Coverage

- **Testable rules**: ~200 (excluding ~84 internal/helper rules)
- **Currently covered**: 119
- **To add**: ~80 new tests
- **Target**: 95%+ of testable rules
