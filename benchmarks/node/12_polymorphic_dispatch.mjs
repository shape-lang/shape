class Linear {
    constructor(slope, intercept) { this.slope = slope; this.intercept = intercept; }
}
class Quadratic {
    constructor(a, b, c) { this.a = a; this.b = b; this.c = c; }
}
class Cubic {
    constructor(a, b, c, d) { this.a = a; this.b = b; this.c = c; this.d = d; }
}

function polymorphic_dispatch(n) {
    const lin = new Linear(2.0, 1.0);
    const quad = new Quadratic(1.0, 2.0, 3.0);
    const cub = new Cubic(0.5, 1.0, 1.5, 2.0);
    let sum = 0;
    for (let i = 0; i < n; i++) {
        const x = (i % 100) * 1.0;
        const r = i % 3;
        if (r === 0) {
            sum += lin.slope * x + lin.intercept;
        } else if (r === 1) {
            sum += quad.a * x * x + quad.b * x + quad.c;
        } else {
            sum += cub.a * x * x * x + cub.b * x * x + cub.c * x + cub.d;
        }
    }
    return sum;
}
console.log(polymorphic_dispatch(5000000));
