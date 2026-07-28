// Charter workload: closure-heavy dispatch — closure called through a
// variable, a parameter, and a returned value (Node reference).
function apply(f, x) {
    return f(x);
}

function makeScaler(k) {
    return (x) => x * k + 1.0;
}

function run(iterations) {
    const direct = (x) => x * 2.0 + 1.0;
    const returned = makeScaler(2.0);
    let acc = 0.0;
    for (let i = 0; i < iterations; i++) {
        const v = i % 64;
        acc = acc + direct(v);
        acc = acc + apply(direct, v);
        acc = acc + returned(v);
    }
    return acc;
}

console.log(Math.trunc(run(6000000)));
