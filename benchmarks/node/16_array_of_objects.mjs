function array_of_objects(n) {
    const particles = [];
    for (let i = 0; i < n; i++) {
        particles.push({ x: i * 1.0, y: i * 2.0, mass: i * 0.5 });
    }
    let total_mass = 0;
    for (let i = 0; i < n; i++) {
        total_mass += particles[i].mass;
    }
    return total_mass;
}
console.log(array_of_objects(100000));
