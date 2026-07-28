// Charter workload: collection pipeline — hash-map build and query traffic
// (Node reference). `Map` with string keys is the counterpart to Shape's
// `HashMap<string, int>`.
function build(n) {
    const m = new Map();
    for (let i = 0; i < n; i++) m.set(`key-${i}`, i * 3);
    return m;
}

function query(m, n, rounds) {
    let found = 0;
    for (let r = 0; r < rounds; r++) {
        for (let i = 0; i < n; i++) {
            const v = m.get(`key-${i}`);
            if (v === undefined) found = found - 1;
            else found = found + v;
        }
    }
    return found;
}

const m = build(4000);
console.log(query(m, 4000, 80));
