// Charter workload: numeric kernel — dense square matrix multiply (Node
// reference). Float64Array is the idiomatic contiguous-f64 counterpart to
// Shape's `Array<number>`; using it keeps the reference at its strongest.
function build(n, seed) {
    const out = new Float64Array(n * n);
    for (let i = 0; i < n * n; i++) out[i] = (i % 17) * seed + 1.0;
    return out;
}

function matmul(n) {
    const a = build(n, 0.5);
    const b = build(n, 0.25);
    const c = new Float64Array(n * n);
    for (let i = 0; i < n; i++) {
        for (let j = 0; j < n; j++) {
            let sum = 0.0;
            for (let k = 0; k < n; k++) sum = sum + a[i * n + k] * b[k * n + j];
            c[i * n + j] = sum;
        }
    }
    let trace = 0.0;
    for (let d = 0; d < n; d++) trace = trace + c[d * n + d];
    return trace;
}

console.log(Math.trunc(matmul(260)));
