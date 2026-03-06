function object_property_loop(n) {
    const p = { x: 1.0, y: 2.0, z: 3.0, w: 4.0, v: 5.0 };
    let sum = 0;
    for (let i = 0; i < n; i++) {
        sum += p.x + p.y + p.z + p.w + p.v;
    }
    return sum;
}
console.log(object_property_loop(10000000));
