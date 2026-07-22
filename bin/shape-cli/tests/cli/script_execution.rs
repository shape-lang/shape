//! CLI binary tests using assert_cmd.
//!
//! Tests that the `shape` binary can execute scripts correctly.

use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;
use tempfile::tempdir;

fn shape_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("shape"))
}

#[test]
fn test_script_arithmetic() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("test.shape");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "let x = 1 + 2").unwrap();
        writeln!(f, "print(x)").unwrap();
    }

    shape_cmd()
        .arg(script.to_str().unwrap())
        .assert()
        .success()
        .stdout(predicate::str::contains("3"));
}

#[test]
fn test_script_nonexistent_file() {
    shape_cmd()
        .arg("/tmp/nonexistent_shape_test_file.shape")
        .assert()
        .failure();
}

#[test]
fn test_script_function_definition() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("func.shape");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "function double(x) {{ return x * 2 }}").unwrap();
        writeln!(f, "print(double(21))").unwrap();
    }

    shape_cmd()
        .arg(script.to_str().unwrap())
        .assert()
        .success()
        .stdout(predicate::str::contains("42"));
}

#[test]
fn test_expand_comptime_summary_lists_generated_methods() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("expand.shape");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "annotation add_sum() on type {{").unwrap();
        writeln!(f, "  comptime post(target, ctx) {{").unwrap();
        writeln!(f, "    extend target {{").unwrap();
        writeln!(f, "      method sum() {{ self.x + self.y }}").unwrap();
        writeln!(f, "    }}").unwrap();
        writeln!(f, "  }}").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "@add_sum()").unwrap();
        writeln!(f, "type Point {{ x: int, y: int }}").unwrap();
    }

    shape_cmd()
        .arg("expand-comptime")
        .arg(script.to_str().unwrap())
        .assert()
        .success()
        .stdout(predicate::str::contains("Comptime expansion report:"))
        .stdout(predicate::str::contains("Functions (post-comptime):"))
        .stdout(predicate::str::contains("extend Point:"))
        .stdout(predicate::str::contains("method sum"));
}

#[test]
fn test_expand_comptime_shorthand_flag_works() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("expand_short.shape");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "annotation add_sum() on type {{").unwrap();
        writeln!(f, "  comptime post(target, ctx) {{").unwrap();
        writeln!(f, "    extend target {{").unwrap();
        writeln!(f, "      method sum() {{ self.x + self.y }}").unwrap();
        writeln!(f, "    }}").unwrap();
        writeln!(f, "  }}").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "@add_sum()").unwrap();
        writeln!(f, "type Point {{ x: int, y: int }}").unwrap();
    }

    shape_cmd()
        .arg(script.to_str().unwrap())
        .arg("--expand")
        .assert()
        .success()
        .stdout(predicate::str::contains("Comptime expansion report:"))
        .stdout(predicate::str::contains("extend Point:"))
        .stdout(predicate::str::contains("method sum"));
}

#[test]
fn test_expand_comptime_function_filter() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("expand_filter.shape");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "annotation add_methods() on type {{").unwrap();
        writeln!(f, "  comptime post(target, ctx) {{").unwrap();
        writeln!(f, "    extend target {{").unwrap();
        writeln!(f, "      method sum() {{ self.x + self.y }}").unwrap();
        writeln!(f, "      method diff() {{ self.x - self.y }}").unwrap();
        writeln!(f, "    }}").unwrap();
        writeln!(f, "  }}").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "@add_methods()").unwrap();
        writeln!(f, "type Point {{ x: int, y: int }}").unwrap();
    }

    shape_cmd()
        .arg("expand-comptime")
        .arg(script.to_str().unwrap())
        .arg("--function")
        .arg("sum")
        .assert()
        .success()
        .stdout(predicate::str::contains("method sum"))
        .stdout(predicate::str::contains("Generated extends: 1"))
        .stdout(predicate::str::contains("filter function: sum"));
}

/// Issue #14 finalize: `xml::stringify` on a node whose `children` is an empty
/// array `[]` and whose `attributes` is an empty object `{}` must run GREEN in
/// BOTH `--mode vm` and `--mode jit` and produce identical output (`<root/>`).
///
/// The empty `children: []` field carries no element type of its own and flows
/// into the polymorphic marshal sink (`stringify(value: _)`). The fix
/// canonical-instantiates it to a monomorphic `TypedArray<int>` (a real typed
/// carrier that marshals to an empty child list) rather than SURFACEing the
/// untyped `op_new_array(0)` placeholder. The shape-cli stdlib integration
/// suite already exercises the VM path via `eval_to_string`; this test adds the
/// end-to-end binary parity guard across both execution backends so a JIT-side
/// regression (or a VM/JIT output divergence) is caught automatically.
fn run_xml_empty_children_fixture(mode: &str) -> String {
    let dir = tempdir().unwrap();
    let script = dir.path().join("xml_empty_children.shape");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "use std::core::xml").unwrap();
        writeln!(
            f,
            "let r = xml::stringify({{ name: \"root\", attributes: {{}}, children: [] }})"
        )
        .unwrap();
        writeln!(f, "match r {{").unwrap();
        writeln!(f, "    Ok(s) => print(s)").unwrap();
        writeln!(f, "    Err(e) => print(f\"stringify error: {{e}}\")").unwrap();
        writeln!(f, "}}").unwrap();
    }

    let assertion = shape_cmd()
        .args(["run", "--mode", mode])
        .arg(script.to_str().unwrap())
        .assert()
        .success();
    let output = assertion.get_output();
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn test_xml_stringify_empty_children_parity_vm_jit() {
    let vm = run_xml_empty_children_fixture("vm");
    let jit = run_xml_empty_children_fixture("jit");

    assert!(
        vm.contains("<root/>"),
        "VM mode should serialize the empty-children node to `<root/>`; got: {vm}"
    );
    assert!(
        jit.contains("<root/>"),
        "JIT mode should serialize the empty-children node to `<root/>` \
         (typed empty-array carrier, no op_new_array(0) SURFACE); got: {jit}"
    );
    assert_eq!(
        vm.trim(),
        jit.trim(),
        "xml::stringify empty-children output must be identical under VM and JIT"
    );
}

/// Issue #14 finalize: a genuinely-unresolvable empty array (`let xs = []`,
/// never annotated and never pushed to) must be a CLEAN compile error — not a
/// crash, not an untyped/any carrier, not a silent success — and it must reject
/// identically under both execution backends. Guards against the
/// canonical-instantiate escape hatch leaking into non-sink positions.
fn run_unresolvable_empty_array_fixture(mode: &str) -> (Option<i32>, String) {
    let dir = tempdir().unwrap();
    let script = dir.path().join("unresolvable_empty.shape");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "let xs = []").unwrap();
        writeln!(f, "print(xs.len())").unwrap();
    }

    let output = shape_cmd()
        .args(["run", "--mode", mode])
        .arg(script.to_str().unwrap())
        .assert()
        .get_output()
        .clone();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), combined)
}

#[test]
fn test_unresolvable_empty_array_is_clean_compile_error_both_modes() {
    for mode in ["vm", "jit"] {
        let (code, combined) = run_unresolvable_empty_array_fixture(mode);
        assert_eq!(
            code,
            Some(1),
            "unresolvable empty array must fail cleanly (exit 1), not crash, in --mode {mode}; \
             output={combined}"
        );
        assert!(
            combined.contains("un-resolvable element type"),
            "unresolvable empty array should surface the strict-typing compile error \
             in --mode {mode}; output={combined}"
        );
    }
}
