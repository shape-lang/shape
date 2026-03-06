function spectral(n) {
    let u = new Float64Array(n).fill(1.0);
    let v = new Float64Array(n);
    for (let iter = 0; iter < 10; iter++) {
        for (let i = 0; i < n; i++) {
            let s = 0;
            for (let j = 0; j < n; j++) s += u[j] / ((i+j)*(i+j+1)/2 + i + 1);
            v[i] = s;
        }
        for (let i = 0; i < n; i++) {
            let s = 0;
            for (let j = 0; j < n; j++) s += v[j] / ((i+j)*(i+j+1)/2 + i + 1);
            u[i] = s;
        }
    }
    console.log(u[0]);
}
spectral(5000);
