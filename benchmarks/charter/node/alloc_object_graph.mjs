// Charter workload: allocation-heavy — object-graph churn (Node reference).
function buildGraph(n) {
    const g = [];
    for (let i = 0; i < n; i++) {
        g.push({ id: i, next: (i * 7 + 3) % n, weight: (i % 31) * 0.5 });
    }
    return g;
}

function walk(g, steps) {
    let acc = 0.0;
    let cursor = 0;
    for (let s = 0; s < steps; s++) {
        const node = g[cursor];
        acc = acc + node.weight;
        cursor = node.next;
    }
    return acc;
}

function churn(rounds, n, steps) {
    let total = 0.0;
    for (let r = 0; r < rounds; r++) {
        const g = buildGraph(n);
        total = total + walk(g, steps);
    }
    return total;
}

console.log(Math.trunc(churn(240, 5000, 20000)));
