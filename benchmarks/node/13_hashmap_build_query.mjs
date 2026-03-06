function hashmap_build_query(n) {
    const m = new Map();
    for (let i = 0; i < n; i++) {
        m.set(i, i * 2.5);
    }
    let sum = 0;
    for (let i = 0; i < n; i++) {
        const v = m.get(i);
        if (v !== undefined) {
            sum += v;
        }
    }
    return sum;
}
console.log(hashmap_build_query(100000));
