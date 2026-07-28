// Charter workload: collection pipeline — map/filter/reduce with closures
// (Node reference).
function source(n) {
    const xs = [];
    for (let i = 0; i < n; i++) xs.push(i % 97);
    return xs;
}

function pipeline(xs) {
    return xs
        .map((x) => x * 1.5 + 1.0)
        .filter((x) => x > 20.0)
        .map((x) => x - 0.5)
        .reduce((acc, x) => acc + x, 0.0);
}

const xs = source(20000);
let total = 0.0;
for (let r = 0; r < 60; r++) total = total + pipeline(xs);
console.log(Math.trunc(total));
