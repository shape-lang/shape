function is_prime(n) {
    if (n < 2) return false;
    if (n < 4) return true;
    if (n % 2 === 0) return false;
    for (let d = 3; d * d <= n; d += 2) if (n % d === 0) return false;
    return true;
}
function count_primes(limit) {
    let count = 0;
    for (let n = 2; n < limit; n++) if (is_prime(n)) count++;
    return count;
}
console.log(count_primes(10000000));
