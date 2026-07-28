// Charter workload: allocation-heavy — recursive build-and-discard of a
// complete binary tree as an index-linked node array (Node reference).
function buildTree(depth) {
    const nodes = [];
    const total = (1 << depth) - 1;
    for (let i = 0; i < total; i++) {
        const l = 2 * i + 1;
        const r = 2 * i + 2;
        nodes.push({ depth: depth, left: l < total ? l : -1, right: r < total ? r : -1 });
    }
    return nodes;
}

function sumTree(nodes) {
    let acc = 0;
    for (let i = 0; i < nodes.length; i++) {
        const n = nodes[i];
        acc = acc + n.depth;
        if (n.left >= 0) acc = acc + 1;
        if (n.right >= 0) acc = acc + 1;
    }
    return acc;
}

function churn(rounds, depth) {
    let total = 0;
    for (let r = 0; r < rounds; r++) {
        const t = buildTree(depth);
        total = total + sumTree(t);
    }
    return total;
}

console.log(churn(160, 14));
