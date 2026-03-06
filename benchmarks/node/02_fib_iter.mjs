function fib_iter(n) {
    let a = 0, b = 1;
    for (let i = 0; i < n; i++) { [a, b] = [b, a + b]; }
    return a;
}
console.log(fib_iter(100000000));
