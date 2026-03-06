package main

import (
    "fmt"
    "os"
    "strconv"
    "time"
)

func fib(n int64) int64 {
    if n < 2 {
        return n
    }
    return fib(n-1) + fib(n-2)
}

func fibIter(n int64) int64 {
    var a int64 = 0
    var b int64 = 1
    for i := int64(0); i < n; i++ {
        t := a + b
        a = b
        b = t
    }
    return a
}

func sieve(n int) int {
    flags := make([]bool, n+1)
    for i := range flags {
        flags[i] = true
    }
    p := 2
    for p*p <= n {
        if flags[p] {
            j := p * p
            for j <= n {
                flags[j] = false
                j += p
            }
        }
        p++
    }
    count := 0
    for i := 2; i <= n; i++ {
        if flags[i] {
            count++
        }
    }
    return count
}

func mandelbrot(size int) int {
    count := 0
    sizeF := float64(size)
    for y := 0; y < size; y++ {
        for x := 0; x < size; x++ {
            cr := 2.0*float64(x)/sizeF - 1.5
            ci := 2.0*float64(y)/sizeF - 1.0
            zr := 0.0
            zi := 0.0
            i := 0
            for i < 50 {
                tr := zr*zr - zi*zi + cr
                zi = 2.0*zr*zi + ci
                zr = tr
                if zr*zr+zi*zi > 4.0 {
                    break
                }
                i++
            }
            if i == 50 {
                count++
            }
        }
    }
    return count
}

func spectral(n int) float64 {
    u := make([]float64, n)
    v := make([]float64, n)
    for i := range u {
        u[i] = 1.0
    }
    for iter := 0; iter < 10; iter++ {
        for i := 0; i < n; i++ {
            s := 0.0
            for j := 0; j < n; j++ {
                denom := float64((i+j)*(i+j+1)/2 + i + 1)
                s += u[j] / denom
            }
            v[i] = s
        }
        for i := 0; i < n; i++ {
            s := 0.0
            for j := 0; j < n; j++ {
                denom := float64((i+j)*(i+j+1)/2 + i + 1)
                s += v[j] / denom
            }
            u[i] = s
        }
    }
    return u[0]
}

func ack(m int64, n int64) int64 {
    if m == 0 {
        return n + 1
    }
    if n == 0 {
        return ack(m-1, 1)
    }
    return ack(m-1, ack(m, n-1))
}

func sumTo(n int64) int64 {
    var s int64 = 0
    var i int64 = 0
    for i < n {
        s += i
        i++
    }
    return s
}

func collatzLen(x int64) int64 {
    var count int64 = 0
    for x != 1 {
        if x%2 == 0 {
            x /= 2
        } else {
            x = 3*x + 1
        }
        count++
    }
    return count
}

func longestCollatz(limit int64) int64 {
    var best int64 = 0
    for n := int64(2); n < limit; n++ {
        l := collatzLen(n)
        if l > best {
            best = l
        }
    }
    return best
}

func matMul(n int) float64 {
    size := n * n
    a := make([]float64, size)
    b := make([]float64, size)
    c := make([]float64, size)
    for i := 0; i < size; i++ {
        a[i] = float64(i)
        b[i] = float64(size - i)
    }
    for i := 0; i < n; i++ {
        for j := 0; j < n; j++ {
            s := 0.0
            rowOffset := i * n
            for k := 0; k < n; k++ {
                s += a[rowOffset+k] * b[k*n+j]
            }
            c[rowOffset+j] = s
        }
    }
    return c[0]
}

func isPrime(n int64) bool {
    if n < 2 {
        return false
    }
    if n < 4 {
        return true
    }
    if n%2 == 0 {
        return false
    }
    d := int64(3)
    for d*d <= n {
        if n%d == 0 {
            return false
        }
        d += 2
    }
    return true
}

func countPrimes(limit int64) int64 {
    var count int64 = 0
    for n := int64(2); n < limit; n++ {
        if isPrime(n) {
            count++
        }
    }
    return count
}

type benchmark struct {
    name string
    run  func() string
}

func main() {
    bench := "all"
    if len(os.Args) > 1 {
        bench = os.Args[1]
    }

    benchmarks := []benchmark{
        {"01_fib", func() string { return strconv.FormatInt(fib(40), 10) }},
        {"02_fib_iter", func() string { return strconv.FormatInt(fibIter(100000000), 10) }},
        {"03_sieve", func() string { return strconv.Itoa(sieve(10000000)) }},
        {"04_mandelbrot", func() string { return strconv.Itoa(mandelbrot(4000)) }},
        {"05_spectral", func() string { return strconv.FormatFloat(spectral(5000), 'f', -1, 64) }},
        {"06_ackermann", func() string { return strconv.FormatInt(ack(3, 10), 10) }},
        {"07_sum_loop", func() string { return strconv.FormatInt(sumTo(1000000000), 10) }},
        {"08_collatz", func() string { return strconv.FormatInt(longestCollatz(1000000), 10) }},
        {"09_matrix_mul", func() string { return strconv.FormatFloat(matMul(800), 'f', -1, 64) }},
        {"10_primes_count", func() string { return strconv.FormatInt(countPrimes(10000000), 10) }},
    }

    for _, b := range benchmarks {
        if bench != "all" && b.name != bench {
            continue
        }
        start := time.Now()
        result := b.run()
        elapsed := time.Since(start).Seconds()
        fmt.Printf("%s|%s|%.4f\n", b.name, result, elapsed)
    }
}
