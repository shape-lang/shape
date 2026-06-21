#include <stdint.h>
#include <stddef.h>
#include <string.h>

/* ------------------------------------------------------------------ *
 * Fixed C stub backing the native-c book-acceptance programs.        *
 * Pure, deterministic integer/float arithmetic. No global state      *
 * except the deterministic LCG seed below.                           *
 * ------------------------------------------------------------------ */

/* scalar arithmetic */
int64_t stub_add_i64(int64_t a, int64_t b) { return a + b; }
int64_t stub_sub_i64(int64_t a, int64_t b) { return a - b; }
int64_t stub_mul_i64(int64_t a, int64_t b) { return a * b; }
double  stub_add_f64(double a, double b)   { return a + b; }
double  stub_mul_f64(double a, double b)   { return a * b; }
int32_t stub_mul_i32(int32_t a, int32_t b) { return a * b; }

/* out-parameters (book "Out Parameters") */
int32_t stub_divmod(int64_t a, int64_t b, int64_t* out_rem) {
    if (b == 0) { return 0; }
    *out_rem = a % b;
    return (int32_t)(a / b);
}
int32_t stub_product(int64_t a, int64_t b, int64_t* out_prod) {
    *out_prod = a * b;
    return 0;
}

/* slice (Vec<T> -> cslice<T>) reductions, copy-in */
int64_t stub_sum_i64(const int64_t* data, size_t len) {
    int64_t acc = 0;
    for (size_t i = 0; i < len; i++) acc += data[i];
    return acc;
}
int64_t stub_max_i64(const int64_t* data, size_t len) {
    if (len == 0) return 0;
    int64_t m = data[0];
    for (size_t i = 1; i < len; i++) if (data[i] > m) m = data[i];
    return m;
}
double stub_dot_f64(const double* a, const double* b, size_t len) {
    double acc = 0.0;
    for (size_t i = 0; i < len; i++) acc += a[i] * b[i];
    return acc;
}

/* mutable slice writeback (cmut_slice<T>, copy-in/copy-out) */
void stub_scale_i64(int64_t* data, size_t len, int64_t factor) {
    for (size_t i = 0; i < len; i++) data[i] = data[i] * factor;
}

/* deterministic LCG (seeded each call; no hidden state) */
uint64_t stub_lcg_next(uint64_t state) {
    /* Numerical Recipes LCG constants */
    return state * 6364136223846793005ULL + 1442695040888963407ULL;
}
