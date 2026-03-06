function mat_mul(n) {
    const a = new Float64Array(n*n);
    const b = new Float64Array(n*n);
    const c = new Float64Array(n*n);
    for (let i = 0; i < n*n; i++) { a[i] = i; b[i] = n*n - i; }
    for (let i = 0; i < n; i++)
        for (let j = 0; j < n; j++) {
            let s = 0;
            for (let k = 0; k < n; k++) s += a[i*n+k] * b[k*n+j];
            c[i*n+j] = s;
        }
    return c[0];
}
console.log(mat_mul(800));
