use std::env;
use std::time::Instant;

fn fib(n: i64) -> i64 {
    if n < 2 { return n; }
    fib(n - 1) + fib(n - 2)
}

fn fib_iter(n: i64) -> i64 {
    let (mut a, mut b) = (0i64, 1i64);
    for _ in 0..n { let t = a + b; a = b; b = t; }
    a
}

fn sieve(n: usize) -> usize {
    let mut flags = vec![true; n + 1];
    let mut p = 2;
    while p * p <= n {
        if flags[p] { let mut j = p * p; while j <= n { flags[j] = false; j += p; } }
        p += 1;
    }
    (2..=n).filter(|&i| flags[i]).count()
}

fn mandelbrot(size: usize) -> usize {
    let mut count = 0;
    for y in 0..size {
        for x in 0..size {
            let cr = 2.0 * x as f64 / size as f64 - 1.5;
            let ci = 2.0 * y as f64 / size as f64 - 1.0;
            let (mut zr, mut zi) = (0.0, 0.0);
            let mut i = 0;
            while i < 50 {
                let tr = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = tr;
                if zr * zr + zi * zi > 4.0 { break; }
                i += 1;
            }
            if i == 50 { count += 1; }
        }
    }
    count
}

fn spectral(n: usize) -> f64 {
    let mut u = vec![1.0f64; n];
    let mut v = vec![0.0f64; n];
    for _ in 0..10 {
        for i in 0..n {
            let mut s = 0.0;
            for j in 0..n { s += u[j] / ((i+j)*(i+j+1)/2 + i + 1) as f64; }
            v[i] = s;
        }
        for i in 0..n {
            let mut s = 0.0;
            for j in 0..n { s += v[j] / ((i+j)*(i+j+1)/2 + i + 1) as f64; }
            u[i] = s;
        }
    }
    u[0]
}

fn ack(m: i64, n: i64) -> i64 {
    if m == 0 { return n + 1; }
    if n == 0 { return ack(m - 1, 1); }
    ack(m - 1, ack(m, n - 1))
}

fn sum_to(n: i64) -> i64 {
    let mut s: i64 = 0;
    let mut i: i64 = 0;
    while i < n { s += i; i += 1; }
    s
}

fn collatz_len(mut x: i64) -> i64 {
    let mut count = 0;
    while x != 1 {
        x = if x % 2 == 0 { x / 2 } else { 3 * x + 1 };
        count += 1;
    }
    count
}

fn longest_collatz(limit: i64) -> i64 {
    let mut best = 0;
    for n in 2..limit {
        let l = collatz_len(n);
        if l > best { best = l; }
    }
    best
}

fn mat_mul(n: usize) -> f64 {
    let mut a = vec![0.0f64; n * n];
    let mut b = vec![0.0f64; n * n];
    let mut c = vec![0.0f64; n * n];
    for i in 0..n*n { a[i] = i as f64; b[i] = (n*n - i) as f64; }
    for i in 0..n {
        for j in 0..n {
            let mut s = 0.0;
            for k in 0..n { s += a[i*n+k] * b[k*n+j]; }
            c[i*n+j] = s;
        }
    }
    c[0]
}

fn is_prime(n: i64) -> bool {
    if n < 2 { return false; }
    if n < 4 { return true; }
    if n % 2 == 0 { return false; }
    let mut d = 3;
    while d * d <= n { if n % d == 0 { return false; } d += 2; }
    true
}

fn count_primes(limit: i64) -> i64 {
    (2..limit).filter(|&n| is_prime(n)).count() as i64
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let bench = if args.len() > 1 { &args[1] } else { "all" };

    let benchmarks: Vec<(&str, Box<dyn Fn() -> String>)> = vec![
        ("01_fib",         Box::new(|| format!("{}", fib(40)))),
        ("02_fib_iter",    Box::new(|| format!("{}", fib_iter(100000000)))),
        ("03_sieve",       Box::new(|| format!("{}", sieve(10000000)))),
        ("04_mandelbrot",  Box::new(|| format!("{}", mandelbrot(4000)))),
        ("05_spectral",    Box::new(|| format!("{}", spectral(5000)))),
        ("06_ackermann",   Box::new(|| format!("{}", ack(3, 10)))),
        ("07_sum_loop",    Box::new(|| format!("{}", sum_to(1000000000)))),
        ("08_collatz",     Box::new(|| format!("{}", longest_collatz(1000000)))),
        ("09_matrix_mul",  Box::new(|| format!("{}", mat_mul(800)))),
        ("10_primes_count",Box::new(|| format!("{}", count_primes(10000000)))),
    ];

    for (name, f) in &benchmarks {
        if bench != "all" && *name != bench { continue; }
        let start = Instant::now();
        let result = f();
        let elapsed = start.elapsed();
        println!("{}|{}|{:.4}", name, result, elapsed.as_secs_f64());
    }
}
