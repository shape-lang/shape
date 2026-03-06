function build_tree(depth) {
    if (depth === 0) return { value: 0, left: null, right: null };
    const left = build_tree(depth - 1);
    const right = build_tree(depth - 1);
    return { value: left.value + right.value + 1, left, right };
}

function checksum(node) {
    if (node === null) return 0;
    return node.value + checksum(node.left) + checksum(node.right);
}

function gc_pressure_tree(depth, iterations) {
    let total = 0;
    for (let i = 0; i < iterations; i++) {
        const tree = build_tree(depth);
        total += tree.value;
    }
    return total;
}
console.log(gc_pressure_tree(20, 5));
