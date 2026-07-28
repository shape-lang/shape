// Charter workload: numeric kernel — cubic B-spline evaluation (Node reference).
function controlPoints(n) {
    const cp = new Float64Array(n);
    for (let i = 0; i < n; i++) cp[i] = (i % 23) * 0.75 - 4.0;
    return cp;
}

function evalSpline(cp, segments, samples) {
    let acc = 0.0;
    for (let seg = 1; seg < segments; seg++) {
        for (let s = 0; s < samples; s++) {
            const t = s / samples;
            const t2 = t * t;
            const t3 = t2 * t;
            const b0 = (1.0 - 3.0 * t + 3.0 * t2 - t3) / 6.0;
            const b1 = (4.0 - 6.0 * t2 + 3.0 * t3) / 6.0;
            const b2 = (1.0 + 3.0 * t + 3.0 * t2 - 3.0 * t3) / 6.0;
            const b3 = t3 / 6.0;
            acc = acc + b0 * cp[seg - 1] + b1 * cp[seg] + b2 * cp[seg + 1] + b3 * cp[seg + 2];
        }
    }
    return acc;
}

const cp = controlPoints(1024);
console.log(Math.trunc(evalSpline(cp, 1021, 2200)));
