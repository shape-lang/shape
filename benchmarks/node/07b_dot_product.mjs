function dot(n) {
    const a = new Float64Array(n);
    const b = new Float64Array(n);
    for (let i = 0; i < n; i++) {
        a[i] = i;
        b[i] = n - i;
    }
    let sum = 0;
    for (let k = 0; k < n; k++) sum += a[k] * b[k];
    return sum;
}
console.log(dot(10000000));
