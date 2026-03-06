function collatz_len(n) {
    let count = 0, x = n;
    while (x !== 1) {
        x = x % 2 === 0 ? x / 2 : 3 * x + 1;
        count++;
    }
    return count;
}
function longest_collatz(limit) {
    let best = 0;
    for (let n = 2; n < limit; n++) {
        const l = collatz_len(n);
        if (l > best) best = l;
    }
    return best;
}
console.log(longest_collatz(1000000));
