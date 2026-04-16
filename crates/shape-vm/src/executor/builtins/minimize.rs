//! L-BFGS optimizer intrinsic
//!
//! `__intrinsic_minimize(objective_fn, x0)` — minimizes a Shape closure using
//! L-BFGS with forward-difference gradients and Wolfe line search.
//!
//! Returns the optimized parameter array.

use crate::executor::VirtualMachine;
use shape_runtime::context::ExecutionContext;
use shape_value::{VMError, ValueWord, ValueWordExt};
use std::collections::VecDeque;

impl VirtualMachine {
    /// Builtin: L-BFGS minimizer with callback-based objective function.
    ///
    /// Args: [objective_closure, x0_array]
    /// - objective_closure: (Array<number>) => number
    /// - x0_array: Array<number> — initial point
    ///
    /// Returns: Array<number> — optimized point
    pub(in crate::executor) fn builtin_minimize(
        &mut self,
        args: Vec<ValueWord>,
        mut ctx: Option<&mut ExecutionContext>,
    ) -> Result<ValueWord, VMError> {
        if args.len() < 2 {
            return Err(VMError::RuntimeError(
                "minimize() requires at least 2 arguments (objective, x0)".to_string(),
            ));
        }

        let objective = args[0].clone();

        // Extract x0
        let x0_view = args[1].as_any_array().ok_or_else(|| {
            VMError::RuntimeError("minimize() x0 must be an array".to_string())
        })?;
        let mut x: Vec<f64> = if let Some(s) = x0_view.as_f64_slice() {
            s.to_vec()
        } else {
            x0_view
                .to_generic()
                .iter()
                .map(|v| v.as_number_coerce().unwrap_or(0.0))
                .collect()
        };

        let n = x.len();
        if n == 0 {
            return Ok(args[1].clone());
        }

        // Configuration
        let max_iter = 200;
        let gtol = 1e-10;
        let ftol = 1e-10;
        let h = 1e-7; // forward difference step
        let m = 10.min(n); // L-BFGS history size

        // Helper: evaluate objective function
        let eval_obj =
            |vm: &mut VirtualMachine,
             x: &[f64],
             ctx: &mut Option<&mut ExecutionContext>|
             -> Result<f64, VMError> {
                let x_vw = f64_vec_to_vw_array(x);
                let result = vm.call_value_immediate_nb(
                    &objective,
                    &[x_vw],
                    ctx.as_deref_mut(),
                )?;
                result.as_number_coerce().ok_or_else(|| {
                    VMError::RuntimeError(
                        "minimize: objective function must return a number".to_string(),
                    )
                })
            };

        // Initial function value
        let mut f = eval_obj(self, &x, &mut ctx)?;

        // Forward-difference gradient (n evaluations, reuses f(x))
        let mut g = forward_gradient(self, &objective, &x, f, h, &mut ctx)?;

        // L-BFGS history
        let mut s_hist: VecDeque<Vec<f64>> = VecDeque::with_capacity(m);
        let mut y_hist: VecDeque<Vec<f64>> = VecDeque::with_capacity(m);
        let mut rho_hist: VecDeque<f64> = VecDeque::with_capacity(m);

        let mut x_scratch = vec![0.0; n];

        for _iter in 0..max_iter {
            // Check gradient convergence
            let gnorm = norm(&g);
            if gnorm < gtol {
                break;
            }

            // L-BFGS two-loop recursion: compute search direction d = -H*g
            let d = lbfgs_direction(&g, &s_hist, &y_hist, &rho_hist);

            // Wolfe line search
            let dg = dot(&d, &g);
            if dg >= 0.0 {
                // Not a descent direction — reset history
                break;
            }

            let (alpha, f_new) =
                wolfe_line_search(self, &objective, &x, &d, f, dg, &mut ctx, &mut x_scratch)?;

            // Compute step s = alpha * d
            let s: Vec<f64> = d.iter().map(|&di| alpha * di).collect();

            // Update x
            for i in 0..n {
                x[i] += s[i];
            }

            // Check function convergence
            if (f - f_new).abs() < ftol {
                f = f_new;
                break;
            }

            // New gradient (reuses f_new)
            let g_new = forward_gradient(self, &objective, &x, f_new, h, &mut ctx)?;

            // y = g_new - g
            let y: Vec<f64> = g_new.iter().zip(g.iter()).map(|(&a, &b)| a - b).collect();
            let sy = dot(&s, &y);

            // Update L-BFGS history
            if sy > 1e-16 {
                if s_hist.len() >= m {
                    s_hist.pop_front();
                    y_hist.pop_front();
                    rho_hist.pop_front();
                }
                rho_hist.push_back(1.0 / sy);
                s_hist.push_back(s);
                y_hist.push_back(y);
            }

            f = f_new;
            g = g_new;
        }

        Ok(f64_vec_to_vw_array(&x))
    }
}

/// Forward-difference gradient: g[i] = (f(x + h*e_i) - f_x) / h
fn forward_gradient(
    vm: &mut VirtualMachine,
    objective: &ValueWord,
    x: &[f64],
    f_x: f64,
    h: f64,
    ctx: &mut Option<&mut ExecutionContext>,
) -> Result<Vec<f64>, VMError> {
    let n = x.len();
    let inv_h = 1.0 / h;
    let mut g = Vec::with_capacity(n);
    let mut x_pert = x.to_vec();

    for i in 0..n {
        let orig = x_pert[i];
        x_pert[i] = orig + h;
        let x_vw = f64_vec_to_vw_array(&x_pert);
        let f_pert = vm
            .call_value_immediate_nb(objective, &[x_vw], ctx.as_deref_mut())?
            .as_number_coerce()
            .ok_or_else(|| {
                VMError::RuntimeError("minimize: objective must return a number".to_string())
            })?;
        g.push((f_pert - f_x) * inv_h);
        x_pert[i] = orig;
    }
    Ok(g)
}

/// L-BFGS two-loop recursion: compute d = -H*g without forming H explicitly.
fn lbfgs_direction(
    g: &[f64],
    s_hist: &VecDeque<Vec<f64>>,
    y_hist: &VecDeque<Vec<f64>>,
    rho_hist: &VecDeque<f64>,
) -> Vec<f64> {
    let k = s_hist.len();
    let mut q: Vec<f64> = g.to_vec();
    let mut alpha_hist = vec![0.0; k];

    // First loop (backward)
    for i in (0..k).rev() {
        alpha_hist[i] = rho_hist[i] * dot(&s_hist[i], &q);
        for j in 0..q.len() {
            q[j] -= alpha_hist[i] * y_hist[i][j];
        }
    }

    // Scale by initial Hessian approximation: H0 = (s'y / y'y) * I
    if k > 0 {
        let sy = dot(&s_hist[k - 1], &y_hist[k - 1]);
        let yy = dot(&y_hist[k - 1], &y_hist[k - 1]);
        if yy > 0.0 {
            let scale = sy / yy;
            for qi in q.iter_mut() {
                *qi *= scale;
            }
        }
    }

    // Second loop (forward)
    for i in 0..k {
        let beta = rho_hist[i] * dot(&y_hist[i], &q);
        let diff = alpha_hist[i] - beta;
        for j in 0..q.len() {
            q[j] += diff * s_hist[i][j];
        }
    }

    // Negate: d = -H*g
    for qi in q.iter_mut() {
        *qi = -*qi;
    }
    q
}

/// Backtracking line search with Wolfe conditions.
/// Returns (alpha, f(x + alpha*d)).
fn wolfe_line_search(
    vm: &mut VirtualMachine,
    objective: &ValueWord,
    x: &[f64],
    d: &[f64],
    f_x: f64,
    dg: f64, // d . g (directional derivative at x)
    ctx: &mut Option<&mut ExecutionContext>,
    x_scratch: &mut [f64],
) -> Result<(f64, f64), VMError> {
    let c1 = 1e-4; // sufficient decrease
    let n = x.len();
    let mut alpha = 1.0;

    for _ in 0..40 {
        // x_new = x + alpha * d
        for i in 0..n {
            x_scratch[i] = x[i] + alpha * d[i];
        }
        let x_vw = f64_vec_to_vw_array(x_scratch);
        let f_new = vm
            .call_value_immediate_nb(objective, &[x_vw], ctx.as_deref_mut())?
            .as_number_coerce()
            .ok_or_else(|| {
                VMError::RuntimeError("minimize: objective must return a number".to_string())
            })?;

        // Armijo (sufficient decrease) condition
        if f_new <= f_x + c1 * alpha * dg {
            return Ok((alpha, f_new));
        }

        alpha *= 0.5;
    }

    // Return best effort
    for i in 0..n {
        x_scratch[i] = x[i] + alpha * d[i];
    }
    let x_vw = f64_vec_to_vw_array(x_scratch);
    let f_new = vm
        .call_value_immediate_nb(objective, &[x_vw], ctx.as_deref_mut())?
        .as_number_coerce()
        .ok_or_else(|| {
            VMError::RuntimeError("minimize: objective must return a number".to_string())
        })?;
    Ok((alpha, f_new))
}

#[inline]
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(&x, &y)| x * y).sum()
}

#[inline]
fn norm(a: &[f64]) -> f64 {
    dot(a, a).sqrt()
}

/// Build a ValueWord array from a &[f64] slice.
fn f64_vec_to_vw_array(data: &[f64]) -> ValueWord {
    ValueWord::from_array(std::sync::Arc::new(
        data.iter().map(|&v| ValueWord::from_f64(v)).collect(),
    ))
}
