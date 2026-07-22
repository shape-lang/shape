//! Tests for module system features
use super::{FeatureCategory, FeatureTest};

pub const TESTS: &[FeatureTest] = &[
    // === Module Use Statement ===
    FeatureTest {
        name: "import_named",
        covers: &["import_stmt", "import_item_list", "import_item"],
        code: r#"
from mylib::module use { foo }
function test() {
    return 42;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_named_multiple",
        covers: &["import_stmt", "import_item_list", "import_item"],
        code: r#"
from mylib::module use { foo, bar, baz }
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_with_alias",
        covers: &["import_stmt", "import_item"],
        code: r#"
from mylib::module use { foo as myFoo }
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_module_alias",
        covers: &["import_stmt"],
        code: r#"
use utils as u
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_simple",
        covers: &["import_stmt"],
        code: r#"
use utils
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "import_from_syntax",
        covers: &[
            "import_stmt",
            "import_item_list",
            "import_item",
            "module_path",
        ],
        code: r#"
from std::core::math use { sum, avg }
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Pub Item ===
    FeatureTest {
        name: "pub_function",
        covers: &["pub_item", "function_def"],
        code: r#"
pub fn add(a, b) {
    return a + b;
}
function test() {
    return add(1, 2);
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_variable",
        covers: &["pub_item", "variable_decl"],
        code: r#"
pub let PI = 3.14159;
function test() {
    return PI;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_enum",
        covers: &["pub_item", "enum_def"],
        code: r#"
pub enum Color {
    Red,
    Green,
    Blue
}
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_struct",
        covers: &["pub_item", "struct_type_def"],
        code: r#"
pub type Point {
    x: number,
    y: number
}
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_type_alias",
        covers: &["pub_item", "type_alias_def"],
        code: r#"
pub type ID = number;
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_trait",
        covers: &["pub_item", "trait_def"],
        code: r#"
pub trait Printable {
    fn toString(): string;
}
function test() {
    return 1;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    // === Export Spec ===
    FeatureTest {
        name: "pub_spec_basic",
        covers: &["pub_item", "export_spec_list", "export_spec"],
        code: r#"
let foo = 1;
let bar = 2;
pub { foo, bar };
function test() {
    return foo + bar;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_spec_with_alias",
        covers: &["pub_item", "export_spec_list", "export_spec"],
        code: r#"
let internalName = 42;
pub { internalName as publicName };
function test() {
    return internalName;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
    FeatureTest {
        name: "pub_spec_mixed_aliases",
        covers: &["export_spec_list", "export_spec"],
        code: r#"
let a = 1;
let b = 2;
let c = 3;
pub { a, b as renamed, c };
function test() {
    return a + b + c;
}
"#,
        function: "test",
        category: FeatureCategory::Module,
        requires_data: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn test_module_tests_defined() {
        assert!(!TESTS.is_empty());
        // All tests should be in Module category
        for test in TESTS {
            assert_eq!(test.category, FeatureCategory::Module);
        }
    }

    #[test]
    fn test_covers_module_grammar_rules() {
        let covered: BTreeSet<_> = TESTS
            .iter()
            .flat_map(|t| t.covers.iter().copied())
            .collect();
        // Verify key grammar rules are covered
        assert!(covered.contains(&"import_stmt"));
        assert!(covered.contains(&"pub_item"));
        assert!(covered.contains(&"export_spec"));
    }

    // ── STAGE Modules: imported-symbol resolution at every use position ──
    //
    // End-to-end file-module compile+run regression for the three resolution
    // gaps fixed in STAGE Modules:
    //   (1) imported call in a let-INITIALIZER (`let m = imax(3, 9)`),
    //   (2) namespace-qualified call (`numbers::imax(3, 9)`),
    //   (3) imported `pub type` constructed + field-read in the consumer.
    // Before the fix, (1) raised "Undefined function" and (2) raised
    // "module namespace ... is not typed" at the strict type-checker / compiler.

    fn run_modules_program_to_i64(numbers_src: &str, main_src: &str) -> i64 {
        use crate::executor::{VMConfig, VirtualMachine};

        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("calc")).unwrap();
        std::fs::write(dir.path().join("calc/numbers.shape"), numbers_src).unwrap();

        let program = shape_ast::parser::parse_program(main_src).expect("parse failed");

        let mut loader = shape_runtime::module_loader::ModuleLoader::new();
        loader.add_module_path(dir.path().to_path_buf());

        let (graph, stdlib_names, prelude_imports) =
            crate::module_resolution::build_graph_and_stdlib_names(&program, &mut loader, &[])
                .expect("graph build failed");
        let mut compiler = crate::compiler::BytecodeCompiler::new();
        compiler.stdlib_function_names = stdlib_names;
        let bytecode = compiler
            .compile_with_graph_and_prelude(&program, graph, &prelude_imports)
            .expect("compile failed");

        let mut vm = VirtualMachine::new(VMConfig::default());
        vm.load_program(bytecode);
        let slot = vm.execute(None).expect("execution failed");
        slot.as_i64().expect("expected i64 program result")
    }

    const NUMBERS_SRC: &str = r#"
pub fn imax(a: int, b: int) -> int {
    if a > b { a } else { b }
}
pub fn imin(a: int, b: int) -> int {
    if a < b { a } else { b }
}
"#;

    #[test]
    fn imported_call_resolves_in_let_initializer() {
        // `let m = imax(3, 9)` — previously "Undefined function: imax".
        let main = r#"
from calc::numbers use { imax }
let m = imax(3, 9)
m
"#;
        assert_eq!(run_modules_program_to_i64(NUMBERS_SRC, main), 9);
    }

    #[test]
    fn imported_call_resolves_nested_in_let_initializer() {
        let main = r#"
from calc::numbers use { imax, imin }
let n = imin(imax(2, 5), 7)
n
"#;
        assert_eq!(run_modules_program_to_i64(NUMBERS_SRC, main), 5);
    }

    #[test]
    fn namespace_qualified_call_resolves() {
        // `numbers::imax(3, 9)` — previously "module namespace not typed".
        let main = r#"
use calc::numbers
let q = numbers::imax(3, 9)
q
"#;
        assert_eq!(run_modules_program_to_i64(NUMBERS_SRC, main), 9);
    }

    #[test]
    fn imported_pub_type_constructs_and_reads_field() {
        let numbers = r#"
pub type Rect { w: int, h: int }
pub fn area(r: Rect) -> int { r.w * r.h }
"#;
        let main = r#"
from calc::numbers use { Rect, area }
let r = Rect { w: 4, h: 6 }
let a = area(r)
a + r.w
"#;
        // area(4,6)=24 ; r.w=4 ; 24+4 = 28
        assert_eq!(run_modules_program_to_i64(numbers, main), 28);
    }

    // ── ADR-009 §4.1 review round 1 (findings 1+2): comptime sites inside
    // imported modules must obtain the semantic-freeze handle. ──
    //
    // Graph-driven compilation (`compile_with_graph_and_prelude`, the standard
    // pipeline via `execution.rs`) compiles dependency modules FIRST in
    // Phase 1 (`compile_module_from_graph` → `compile_item_with_context`).
    // With the freeze barrier only inside `compile()` (Phase 2, root), every
    // comptime site inside an imported module hit `comptime_freeze_overlay()`
    // while `semantic_freeze` was still `None` and failed with the row-3
    // NO_FREEZE_HANDLE diagnostic — a regression of the imports × comptime
    // composition (the deleted per-site builder worked at any phase). The
    // barrier now runs at the graph entry point BEFORE Phase 1, over
    // predeclared freeze inputs of every graph module + the root.

    #[test]
    fn imported_module_fn_body_comptime_expression_gets_freeze_handle() {
        // `Expr::Comptime` in an imported fn body (expressions/mod.rs site).
        let numbers = r#"
pub fn scaled(a: int) -> int {
    let k = comptime { 2 }
    a * k
}
"#;
        let main = r#"
from calc::numbers use { scaled }
scaled(21)
"#;
        assert_eq!(run_modules_program_to_i64(numbers, main), 42);
    }

    #[test]
    fn imported_module_top_level_comptime_block_gets_freeze_handle() {
        // Top-level `comptime { }` item in an imported module (passes
        // `qualify_module_item`'s default arm, hits the Item::Comptime
        // freeze site in statements.rs).
        let numbers = r#"
comptime {
    let sanity = 1 + 2
}
pub fn seven() -> int { 7 }
"#;
        let main = r#"
from calc::numbers use { seven }
seven()
"#;
        assert_eq!(run_modules_program_to_i64(numbers, main), 7);
    }

    #[test]
    fn imported_module_comptime_reflection_consumes_a_populated_freeze() {
        // The handle an imported-module comptime site obtains is the REAL
        // populated per-unit freeze (the reflection trio resolves against
        // it), not merely "some handle exists". Reflection uses a primitive
        // here: resolving a module's OWN nominal type by its bare name from
        // inside its own comptime block is a pre-existing name-resolution
        // gap (dep structs freeze under their qualified names, and the
        // deleted per-site builder on main read the same qualified tables),
        // independent of the freeze-barrier ordering this test pins.
        let numbers = r#"
pub fn int_category_code() -> int {
    let code = comptime {
        match type_category(type_ref(int)) {
            FrozenTypeCategory::Primitive => 1
            _ => 0
        }
    }
    code
}
"#;
        let main = r#"
from calc::numbers use { int_category_code }
int_category_code()
"#;
        assert_eq!(run_modules_program_to_i64(numbers, main), 1);
    }

    #[test]
    fn imported_module_annotation_comptime_handler_gets_freeze_handle() {
        // Third comptime-site family: an annotation comptime handler on an
        // item inside an imported module executes during Phase 1
        // (`execute_comptime_annotation_handler` → `comptime_freeze_overlay`)
        // and must obtain the pre-Phase-1 unit freeze — its body uses frozen
        // reflection to prove the handle is populated.
        let numbers = r#"
annotation reflected() on type {
  comptime post(target, ctx) {
    let flag = match type_category(type_ref(int)) {
      FrozenTypeCategory::Primitive => 1
      _ => 0
    }
  }
}

@reflected()
type Config { id: int }

pub fn five() -> int { 5 }
"#;
        let main = r#"
from calc::numbers use { five }
five()
"#;
        assert_eq!(run_modules_program_to_i64(numbers, main), 5);
    }
}
