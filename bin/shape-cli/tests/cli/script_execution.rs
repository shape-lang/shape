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
        writeln!(f, "annotation add_sum() {{").unwrap();
        writeln!(f, "  targets: [type]").unwrap();
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
        writeln!(f, "annotation add_sum() {{").unwrap();
        writeln!(f, "  targets: [type]").unwrap();
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
        writeln!(f, "annotation add_methods() {{").unwrap();
        writeln!(f, "  targets: [type]").unwrap();
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
