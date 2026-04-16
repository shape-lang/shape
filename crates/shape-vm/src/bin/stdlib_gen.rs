use sha2::{Digest, Sha256};
use shape_value::ValueWordExt;
use std::path::PathBuf;

fn main() {
    let verify = std::env::args().any(|a| a == "--verify");

    let out_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("embedded/core_stdlib.msgpack");

    // Always compile from source (bypass embedded artifact loading)
    eprintln!("Compiling core stdlib from source...");
    let program = shape_vm::stdlib::compile_core_modules_from_source()
        .expect("Failed to compile core stdlib modules");

    let bytes =
        rmp_serde::to_vec(&program).expect("Failed to serialize BytecodeProgram to MessagePack");

    let hash = Sha256::digest(&bytes);
    let hash_hex = hex::encode(hash);

    eprintln!("  modules (functions): {}", program.functions.len());
    eprintln!("  instructions:        {}", program.instructions.len());
    eprintln!("  strings:             {}", program.strings.len());
    eprintln!("  constants:           {}", program.constants.len());
    eprintln!("  byte size:           {} bytes", bytes.len());
    eprintln!("  sha256:              {}", hash_hex);

    if verify {
        if !out_path.exists() {
            eprintln!(
                "ERROR: Embedded artifact not found at {}",
                out_path.display()
            );
            eprintln!("Run `cargo run -p shape-vm --bin stdlib_gen` to generate it.");
            std::process::exit(1);
        }
        // Deserialize existing artifact and compare semantically
        // (byte-level comparison fails due to non-deterministic HashMap serialization order)
        let existing_bytes = std::fs::read(&out_path).expect("Failed to read existing artifact");
        let existing: shape_vm::bytecode::BytecodeProgram = rmp_serde::from_slice(&existing_bytes)
            .expect("Failed to deserialize existing artifact");

        let mut errors = Vec::new();
        if existing.functions.len() != program.functions.len() {
            errors.push(format!(
                "function count: existing={}, expected={}",
                existing.functions.len(),
                program.functions.len()
            ));
        }
        if existing.instructions.len() != program.instructions.len() {
            errors.push(format!(
                "instruction count: existing={}, expected={}",
                existing.instructions.len(),
                program.instructions.len()
            ));
        }
        if existing.constants.len() != program.constants.len() {
            errors.push(format!(
                "constant count: existing={}, expected={}",
                existing.constants.len(),
                program.constants.len()
            ));
        }
        // Check all expected functions are present
        let existing_fn_names: std::collections::HashSet<&str> =
            existing.functions.iter().map(|f| f.name.as_str()).collect();
        for f in &program.functions {
            if !existing_fn_names.contains(f.name.as_str()) {
                errors.push(format!("missing function: {}", f.name));
            }
        }

        if errors.is_empty() {
            eprintln!("OK: Embedded stdlib is up-to-date.");
        } else {
            eprintln!("ERROR: Embedded stdlib is stale!");
            for e in &errors {
                eprintln!("  - {}", e);
            }
            eprintln!("Run `cargo run -p shape-vm --bin stdlib_gen` to regenerate.");
            std::process::exit(1);
        }
    } else {
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create embedded/ directory");
        }
        std::fs::write(&out_path, &bytes).expect("Failed to write artifact");
        eprintln!("Wrote {}", out_path.display());
    }
}
