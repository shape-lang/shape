use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::tempdir;

fn shape_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("shape"))
}

#[test]
fn tree_prints_transitive_path_dependencies() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("root");
    let dep_a = dir.path().join("dep-a");
    let dep_b = dir.path().join("dep-b");

    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&dep_a).unwrap();
    fs::create_dir_all(&dep_b).unwrap();

    fs::write(
        root.join("shape.toml"),
        r#"
[project]
name = "root"
version = "0.1.0"
entry = "main.shape"

[dependencies]
dep-a = { path = "../dep-a" }
"#,
    )
    .unwrap();
    fs::write(root.join("main.shape"), "print(\"root\")\n").unwrap();

    fs::write(
        dep_a.join("shape.toml"),
        r#"
[project]
name = "dep-a"
version = "1.2.3"
entry = "main.shape"

[dependencies]
dep-b = { path = "../dep-b" }
"#,
    )
    .unwrap();
    fs::write(dep_a.join("main.shape"), "print(\"dep-a\")\n").unwrap();

    fs::write(
        dep_b.join("shape.toml"),
        r#"
[project]
name = "dep-b"
version = "0.9.0"
entry = "main.shape"
"#,
    )
    .unwrap();
    fs::write(dep_b.join("main.shape"), "print(\"dep-b\")\n").unwrap();

    shape_cmd()
        .current_dir(&root)
        .arg("tree")
        .assert()
        .success()
        .stdout(predicate::str::contains("root@0.1.0"))
        .stdout(predicate::str::contains("dep-a@1.2.3 [source]"))
        .stdout(predicate::str::contains("dep-b@0.9.0 [source]"));
}

#[test]
fn tree_requires_project_context() {
    let dir = tempdir().unwrap();

    shape_cmd()
        .current_dir(dir.path())
        .arg("tree")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "No shape.toml found. Run `shape tree` from within a Shape project.",
        ));
}

#[test]
fn tree_help_hides_unrelated_global_flags() {
    shape_cmd()
        .arg("tree")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Print dependency tree for the current project",
        ))
        .stdout(predicate::str::contains("--native"))
        .stdout(predicate::str::contains("--mode").not())
        .stdout(predicate::str::contains("--resume").not())
        .stdout(predicate::str::contains("--extension").not())
        .stdout(predicate::str::contains("--expand").not())
        .stdout(predicate::str::contains("--module").not())
        .stdout(predicate::str::contains("--function").not());
}
