function sum_to(n) {
    let s = 0;
    for (let i = 0; i < n; i++) s += i;
    return s;
}
console.log(sum_to(1000000000));
