function sieve(n) {
    const flags = new Uint8Array(n + 1).fill(1);
    for (let p = 2; p * p <= n; p++) {
        if (flags[p]) {
            for (let j = p * p; j <= n; j += p) flags[j] = 0;
        }
    }
    let count = 0;
    for (let i = 2; i <= n; i++) if (flags[i]) count++;
    return count;
}
console.log(sieve(10000000));
