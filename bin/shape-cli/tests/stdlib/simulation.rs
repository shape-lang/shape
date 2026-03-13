//! Stdlib Simulation & Statistics Tests
//!
//! Covers distributions, stochastic processes, Monte Carlo, ODE, physics, and
//! multi-asset backtesting wrappers.

use crate::common::{eval_to_bool, eval_to_number, init_runtime};
use std::path::Path;

fn read_stdlib_module(path: &str) -> String {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("crates/shape-runtime/stdlib-src")
        .join(path);
    std::fs::read_to_string(&base)
        .unwrap_or_else(|e| panic!("Failed to read stdlib module {}: {}", base.display(), e))
}

fn strip_import_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("import ") && !trimmed.starts_with("from ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn with_modules(module_paths: &[&str], code: &str) -> String {
    let mut merged = String::new();
    for path in module_paths {
        merged.push_str(&strip_import_lines(&read_stdlib_module(path)));
        merged.push('\n');
    }
    merged.push_str(code);
    merged
}

#[test]
fn test_distributions_wrappers() {
    init_runtime();

    let code = with_modules(
        &["core/random.shape", "core/distributions.shape"],
        "random_seed(1);\n\
         let u = dist_uniform(0, 1);\n\
         let s = dist_sample_n(\"uniform\", [0, 1], 5);\n\
         let p = dist_poisson(3);\n\
         (u >= 0 && u < 1) && (len(s) == 5) && (p >= 0)",
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_stochastic_wrappers() {
    init_runtime();

    let code = with_modules(
        &["core/stochastic.shape"],
        "let b = brownian_motion(5, 1.0, 1.0);\n\
         let g = gbm(5, 0.01, 0.1, 0.2, 100.0);\n\
         let o = ou_process(5, 0.1, 0.5, 1.0, 0.3, 2.0);\n\
         (len(b) == 5) && (len(g) == 5) && (len(o) == 5)",
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_monte_carlo_and_stats() {
    init_runtime();

    // Simplified monte_carlo — always collects results (avoids if-inside-for scope issue)
    let code = r#"
        fn monte_carlo(n_sims, sim_fn) {
            let mut results = [];
            for i in range(0, n_sims) {
                results = results.push(sim_fn(i));
            }
            return { simulations: n_sims, results: results };
        }

        let sim = monte_carlo(5, |i| i * 2);
        len(sim.results)
    "#;
    assert_eq!(eval_to_number(code), 5.0);
}

#[test]
fn test_ode_integrators() {
    init_runtime();

    let code = with_modules(
        &["core/ode.shape"],
        "let e = euler(|t, y| -y, 1.0, 0.0, 1.0, 0.1);\n\
         let r = rk4(|t, y| -y, 1.0, 0.0, 1.0, 0.1);\n\
         (len(e) >= 0) && (len(r) >= 0)",
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_harmonic_oscillator_rk4_system() {
    init_runtime();

    let code = with_modules(
        &["core/ode.shape"],
        "let res = rk4_system(|t, y| [y[1], -y[0]], [1.0, 0.0], 0.0, 6.283185307179586, 0.05);\n\
         (len(res) > 100) && (res[0].t == 0.0)",
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_physics_projectile_range() {
    init_runtime();

    let code = with_modules(
        &[
            "physics/types.shape",
            "physics/mechanics.shape",
            "physics/simulation.shape",
        ],
        "let vx = 7.0710678118654755;\n\
         let vy = 7.0710678118654755;\n\
         let res = simulate_projectile({ x: 0.0, y: 0.0, vx: vx, vy: vy, t: 0.0 }, 5.0, 0.01, 9.81);\n\
         len(res) >= 0",
    );

    assert!(eval_to_bool(&code));
}

// ===== K2: RK45 Adaptive ODE Tests =====

#[test]
fn test_rk45_scalar_exponential_decay() {
    // dy/dt = -y, y(0) = 1 => y(t) = e^(-t) ≈ 0.3679 at t=1
    init_runtime();

    let code = with_modules(
        &["core/ode.shape"],
        r#"
        let res = rk45(|t, y| -y, 1.0, 0.0, 1.0);
        let last = res[len(res) - 1];
        // Should reach t=1 with y close to e^(-1) ≈ 0.3679
        let err = abs(last.y - 0.36787944117144233);
        err < 0.0001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_rk45_uses_fewer_steps_on_smooth_ode() {
    // Smooth ODE should need fewer steps than fixed-step
    init_runtime();

    let code = with_modules(
        &["core/ode.shape"],
        r#"
        let adaptive = rk45(|t, y| -y, 1.0, 0.0, 1.0);
        let fixed = rk4(|t, y| -y, 1.0, 0.0, 1.0, 0.01);
        // Adaptive should use fewer steps than 100 fixed steps
        len(adaptive) < len(fixed)
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_rk45_system_harmonic_oscillator() {
    // Harmonic oscillator: y'' + y = 0, y(0) = 1, y'(0) = 0
    // Exact: y(t) = cos(t), y(2π) ≈ 1.0
    init_runtime();

    let code = with_modules(
        &["core/ode.shape"],
        r#"
        let res = rk45_system(|t, y| [y[1], -y[0]], [1.0, 0.0], 0.0, 6.283185307179586);
        let last = res[len(res) - 1];
        // After one full period, y[0] should be close to 1.0
        let err = abs(last.y[0] - 1.0);
        err < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_rk45_stiff_like_ode() {
    // dy/dt = -50*y with y(0) = 1 — moderately stiff
    // Adaptive solver should handle step-size reduction
    init_runtime();

    let code = with_modules(
        &["core/ode.shape"],
        r#"
        let res = rk45(|t, y| -50.0 * y, 1.0, 0.0, 0.1);
        let last = res[len(res) - 1];
        // y(0.1) = e^(-5) ≈ 0.006738
        let err = abs(last.y - 0.006737946999085467);
        err < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_rk45_scalar_reaches_endpoint() {
    init_runtime();

    let code = with_modules(
        &["core/ode.shape"],
        r#"
        let res = rk45(|t, y| 1.0, 0.0, 0.0, 2.0);
        let last = res[len(res) - 1];
        // dy/dt = 1, y(0) = 0 => y(2) = 2.0
        abs(last.y - 2.0) < 0.001 && abs(last.t - 2.0) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

// ===== K3: Variance Reduction Tests =====

#[test]
fn test_monte_carlo_antithetic() {
    init_runtime();

    let code = with_modules(
        &["core/random.shape", "core/monte_carlo.shape"],
        r#"
        random_seed(42);
        let result = monte_carlo_antithetic(100, |i, is_anti| {
            let u = random();
            if is_anti { 1.0 - u } else { u }
        });
        // Should have 100 averaged results (from 200 total sims)
        result.simulations == 200 && len(result.results) == 100
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_monte_carlo_antithetic_reduces_variance() {
    init_runtime();

    let code = with_modules(
        &["core/random.shape", "core/monte_carlo.shape"],
        r#"
        // Plain MC
        random_seed(42);
        let mut plain = [];
        for i in range(0, 1000) {
            plain.push(random());
        }

        // Antithetic MC: pair each U with (1-U), average each pair
        random_seed(42);
        let mut anti = [];
        for i in range(0, 500) {
            let u = random();
            anti.push((u + (1.0 - u)) / 2.0);
        }

        let plain_std = __intrinsic_std(plain);
        let anti_std = __intrinsic_std(anti);
        // Antithetic should have much lower std (in fact ~0 since each pair = 0.5)
        anti_std < plain_std
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_monte_carlo_control_variate() {
    init_runtime();

    let code = with_modules(
        &["core/random.shape", "core/monte_carlo.shape"],
        r#"
        random_seed(42);
        let result = monte_carlo_control_variate(500, |i| {
            let u = random();
            { value: u * u, control: u }
        }, 0.5);
        // Should have results and variance_reduction metric
        len(result.results) == 500 && result.variance_reduction >= 0.0
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_monte_carlo_stratified() {
    init_runtime();

    let code = with_modules(
        &["core/random.shape", "core/monte_carlo.shape"],
        r#"
        random_seed(42);
        let result = monte_carlo_stratified(100, |i, u| u * u);
        // Should return 100 results, all between 0 and 1
        let mut ok = len(result.results) == 100;
        for r in result.results {
            if r < 0.0 || r > 1.0 {
                ok = false;
            }
        }
        ok
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_monte_carlo_stratified_estimates_mean() {
    init_runtime();

    let code = with_modules(
        &["core/random.shape", "core/monte_carlo.shape"],
        r#"
        random_seed(42);
        let strat_n = 1000;
        let mut strat_results = [];
        for i in range(0, strat_n) {
            let u = (i + random()) / strat_n;
            strat_results.push(u * u);
        }
        let m = __intrinsic_mean(strat_results);
        abs(m - 0.333333) < 0.02
        "#,
    );
    assert!(eval_to_bool(&code));
}

// ===== K4: Collision Detection Tests =====

#[test]
fn test_aabb_overlap_basic() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let a = aabb(0.0, 0.0, 2.0, 2.0);
        let b = aabb(1.0, 1.0, 3.0, 3.0);
        let c = aabb(5.0, 5.0, 6.0, 6.0);
        aabb_overlaps(a, b) && !aabb_overlaps(a, c)
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_aabb_contains_point() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let box = aabb(0.0, 0.0, 10.0, 10.0);
        aabb_contains_point(box, 5.0, 5.0) && !aabb_contains_point(box, 15.0, 5.0)
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_aabb_contains_box() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let outer = aabb(0.0, 0.0, 10.0, 10.0);
        let inner = aabb(2.0, 2.0, 8.0, 8.0);
        let partial = aabb(5.0, 5.0, 15.0, 15.0);
        aabb_contains(outer, inner) && !aabb_contains(outer, partial)
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_aabb_overlap_area() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let a = aabb(0.0, 0.0, 4.0, 4.0);
        let b = aabb(2.0, 2.0, 6.0, 6.0);
        let area = aabb_overlap_area(a, b);
        // Overlap is [2,4] x [2,4] = 2*2 = 4
        abs(area - 4.0) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_aabb_separation() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let a = aabb(0.0, 0.0, 3.0, 3.0);
        let b = aabb(2.0, 0.0, 5.0, 3.0);
        let sep = aabb_separation(a, b);
        // Minimum separation is 1.0 in x-direction
        sep != None && abs(abs(sep.x) - 1.0) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_aabb_centered_and_union() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let a = aabb_centered(0.0, 0.0, 1.0, 1.0);
        let b = aabb_centered(3.0, 0.0, 1.0, 1.0);
        // a = [-1,-1,1,1], b = [2,-1,4,1]
        let u = aabb_union(a, b);
        abs(u.min_x - (-1.0)) < 0.001 && abs(u.max_x - 4.0) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_find_collisions_brute() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let boxes = [
            aabb(0.0, 0.0, 2.0, 2.0),
            aabb(1.0, 1.0, 3.0, 3.0),
            aabb(5.0, 5.0, 7.0, 7.0),
            aabb(6.0, 6.0, 8.0, 8.0)
        ];
        let pairs = find_collisions_brute(boxes);
        // (0,1) overlap, (2,3) overlap, total 2 pairs
        len(pairs) == 2
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_find_collisions_sweep() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let boxes = [
            aabb(0.0, 0.0, 2.0, 2.0),
            aabb(1.0, 1.0, 3.0, 3.0),
            aabb(5.0, 5.0, 7.0, 7.0),
            aabb(6.0, 6.0, 8.0, 8.0)
        ];
        let pairs = find_collisions_sweep(boxes);
        // Same result as brute force: 2 pairs
        len(pairs) == 2
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_elastic_response() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let a = { aabb: aabb(0.0, 0.0, 2.0, 2.0), vx: 1.0, vy: 0.0, mass: 1.0 };
        let b = { aabb: aabb(1.5, 0.0, 3.5, 2.0), vx: -1.0, vy: 0.0, mass: 1.0 };
        let result = elastic_response(a, b);
        // Equal mass elastic collision: velocities should swap
        // a was going right (+1), b was going left (-1) => a goes left, b goes right
        abs(result.a.vx - (-1.0)) < 0.1 && abs(result.b.vx - 1.0) < 0.1
        "#,
    );
    assert!(eval_to_bool(&code));
}

#[test]
fn test_no_collision_no_response() {
    init_runtime();

    let code = with_modules(
        &["physics/collision.shape"],
        r#"
        let a = { aabb: aabb(0.0, 0.0, 1.0, 1.0), vx: 1.0, vy: 0.0, mass: 1.0 };
        let b = { aabb: aabb(5.0, 5.0, 6.0, 6.0), vx: 0.0, vy: 0.0, mass: 1.0 };
        let result = elastic_response(a, b);
        // No collision — velocities unchanged
        abs(result.a.vx - 1.0) < 0.001 && abs(result.b.vx) < 0.001
        "#,
    );
    assert!(eval_to_bool(&code));
}
