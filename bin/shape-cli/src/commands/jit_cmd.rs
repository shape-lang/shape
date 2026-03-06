use anyhow::Result;

#[cfg(feature = "jit")]
pub async fn run_jit_parity(builtins: bool, unsupported_only: bool) -> Result<()> {
    use shape_jit::{
        JitParityTarget, build_full_builtin_parity_matrix, build_full_opcode_parity_matrix,
    };

    let mut rows = build_full_opcode_parity_matrix();
    if builtins {
        rows.extend(build_full_builtin_parity_matrix());
        rows.sort_by_key(|entry| format!("{:?}", entry.target));
    }

    if unsupported_only {
        rows.retain(|entry| !entry.jit_supported);
    }

    println!("JIT parity matrix");
    println!("{:<8} {:<28} {:<3} {}", "kind", "target", "jit", "reason");
    println!("{:<8} {:<28} {:<3} {}", "----", "------", "---", "------");

    let mut opcode_total = 0usize;
    let mut opcode_supported = 0usize;
    let mut builtin_total = 0usize;
    let mut builtin_supported = 0usize;

    for row in &rows {
        let (kind, target_name) = match row.target {
            JitParityTarget::Opcode(opcode) => {
                opcode_total += 1;
                if row.jit_supported {
                    opcode_supported += 1;
                }
                ("opcode", format!("{opcode:?}"))
            }
            JitParityTarget::Builtin(builtin) => {
                builtin_total += 1;
                if row.jit_supported {
                    builtin_supported += 1;
                }
                ("builtin", format!("{builtin:?}"))
            }
        };

        let jit = if row.jit_supported { "yes" } else { "no" };
        println!("{:<8} {:<28} {:<3} {}", kind, target_name, jit, row.reason);
    }

    println!();
    println!(
        "Opcodes: {}/{} JIT-supported",
        opcode_supported, opcode_total
    );
    if builtins {
        println!(
            "Builtins: {}/{} JIT-supported",
            builtin_supported, builtin_total
        );
    }

    Ok(())
}

#[cfg(not(feature = "jit"))]
pub async fn run_jit_parity(_builtins: bool, _unsupported_only: bool) -> Result<()> {
    anyhow::bail!("JIT diagnostics require the 'jit' feature. Rebuild with --features jit");
}
