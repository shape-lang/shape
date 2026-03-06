#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SHAPE_ROOT="$(dirname "$SCRIPT_DIR")"
TIMEOUT=120  # seconds per benchmark
ENFORCE=false
COLLECT_JIT_METRICS=true
BUDGET_FILE="$SCRIPT_DIR/ci_jit_node_budget.tsv"
MAX_GEOMEAN=""
WRITE_RATIOS_FILE=""
WRITE_METRICS_FILE=""

BENCHMARKS=(
    "01_fib"
    "02_fib_iter"
    "03_sieve"
    "04_mandelbrot"
    "05_spectral"
    "06_ackermann"
    "07_sum_loop"
    "08_collatz"
    "09_matrix_mul"
    "10_primes_count"
)

# Optional per-benchmark repeat counts (applied uniformly across selected
# runtimes). Defaults to 1.
declare -A BENCH_REPEATS
for bench in "${BENCHMARKS[@]}"; do
    BENCH_REPEATS["$bench"]=1
done

# ── Runtime Selection ───────────────────────────────────────────────────
# Default: all runtimes. Override with --runtimes "rust node go jit"
# or use --fast for just Rust, Node, JIT (skips Python, Go, and VM)
RUNTIMES=(rust node python vm jit)

while [[ $# -gt 0 ]]; do
    case "$1" in
        --runtimes)
            IFS=' ' read -ra RUNTIMES <<< "$2"
            shift 2
            ;;
        --fast)
            RUNTIMES=(rust node jit)
            shift
            ;;
        --jit-only)
            RUNTIMES=(rust node jit)
            shift
            ;;
        --enforce)
            ENFORCE=true
            shift
            ;;
        --budget-file)
            BUDGET_FILE="$2"
            shift 2
            ;;
        --max-geomean)
            MAX_GEOMEAN="$2"
            shift 2
            ;;
        --write-ratios)
            WRITE_RATIOS_FILE="$2"
            shift 2
            ;;
        --write-metrics)
            WRITE_METRICS_FILE="$2"
            shift 2
            ;;
        --no-jit-metrics)
            COLLECT_JIT_METRICS=false
            shift
            ;;
        *)
            echo "Usage: $0 [--fast] [--runtimes 'rust node go python vm jit'] [--enforce] [--budget-file <path>] [--max-geomean <ratio>] [--write-ratios <path>] [--write-metrics <path>] [--no-jit-metrics]"
            exit 1
            ;;
    esac
done

has_runtime() { for r in "${RUNTIMES[@]}"; do [[ "$r" == "$1" ]] && return 0; done; return 1; }

# When running the fast profile (Rust/Node/JIT only), inflate the lighter
# workloads so Node timings are generally around ~1 second per benchmark.
# This keeps comparisons stable while avoiding huge default full-suite runs
# (which include slower Python/VM runtimes).
if has_runtime node && ! has_runtime python && ! has_runtime go && ! has_runtime vm; then
    BENCH_REPEATS["02_fib_iter"]=8
    BENCH_REPEATS["03_sieve"]=20
    BENCH_REPEATS["05_spectral"]=2
    BENCH_REPEATS["06_ackermann"]=8
    BENCH_REPEATS["07_sum_loop"]=2
    BENCH_REPEATS["09_matrix_mul"]=3
fi

declare -A BUDGET_RATIO_100X  # benchmark -> max ratio * 100
BUDGET_GEOMEAN_100X=""
if [ -f "$BUDGET_FILE" ]; then
    while read -r bench ratio; do
        if [ -z "${bench:-}" ] || [[ "$bench" =~ ^# ]]; then
            continue
        fi
        if [ "$bench" = "GEOMEAN_MAX" ]; then
            BUDGET_GEOMEAN_100X=$(awk "BEGIN{printf \"%.0f\", $ratio * 100}")
        else
            BUDGET_RATIO_100X["$bench"]=$(awk "BEGIN{printf \"%.0f\", $ratio * 100}")
        fi
    done < "$BUDGET_FILE"
fi
if [ -z "$MAX_GEOMEAN" ] && [ -n "$BUDGET_GEOMEAN_100X" ]; then
    MAX_GEOMEAN=$(awk "BEGIN{printf \"%.2f\", $BUDGET_GEOMEAN_100X / 100.0}")
fi

# ── Colors ───────────────────────────────────────────────────────────────
RST='\033[0m'
BOLD='\033[1m'
DIM='\033[2m'
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
MAGENTA='\033[0;35m'
CYAN='\033[0;36m'
WHITE='\033[0;37m'
BG_GREEN='\033[42;30m'   # green bg, black text
BG_RED='\033[41;37m'     # red bg, white text
BG_YELLOW='\033[43;30m'  # yellow bg, black text
BOLD_GREEN='\033[1;32m'
BOLD_RED='\033[1;31m'
BOLD_YELLOW='\033[1;33m'
BOLD_CYAN='\033[1;36m'
BOLD_WHITE='\033[1;37m'
BOLD_MAGENTA='\033[1;35m'
BOLD_BLUE='\033[1;34m'

# ── Box-drawing ──────────────────────────────────────────────────────────
H='─'
V='│'
TL='┌'; TR='┐'; BL='└'; BR='┘'
LT='├'; RT='┤'; TT='┬'; BT='┴'; CR='┼'

# Column widths
W_NAME=20
W_COL=12  # each runtime column
W_RATIO=10
NUM_COLS=${#RUNTIMES[@]}

# ── Helpers ──────────────────────────────────────────────────────────────
hline() {
    local l="$1" m="$2" r="$3" extra="${4:-}"
    printf '%s' "$l"
    printf '%*s' "$W_NAME" '' | tr ' ' "$H"
    for ((i=0; i<NUM_COLS; i++)); do
        printf '%s' "$m"
        printf '%*s' "$W_COL" '' | tr ' ' "$H"
    done
    if [ -n "$extra" ]; then
        printf '%s' "$m"
        printf '%*s' "$W_RATIO" '' | tr ' ' "$H"
    fi
    printf '%s\n' "$r"
}

fmt_ms() {
    local ms="$1"
    if [ "$ms" = "TIMEOUT" ]; then echo ">120s"
    elif [ "$ms" = "ERROR" ] || [ "$ms" = "ERR" ] || [ -z "$ms" ]; then echo "ERR"
    elif [ "$ms" -lt 1000 ]; then echo "${ms}ms"
    else
        local secs
        secs=$(awk "BEGIN{printf \"%.2f\", $ms/1000}")
        echo "${secs}s"
    fi
}

ms_to_num() {
    local ms="$1"
    if [ "$ms" = "TIMEOUT" ]; then echo "999999"
    elif [ "$ms" = "ERROR" ] || [ "$ms" = "ERR" ] || [ -z "$ms" ]; then echo "-1"
    else echo "$ms"
    fi
}

time_cmd() {
    local repeat="$1"
    shift
    if [ -z "${repeat:-}" ] || [ "$repeat" -lt 1 ] 2>/dev/null; then
        repeat=1
    fi
    local start end
    start=$(date +%s%N)
    local rc=0
    local iter=0
    while [ "$iter" -lt "$repeat" ]; do
        timeout "$TIMEOUT" "$@" > /tmp/bench_output 2>&1
        rc=$?
        if [ $rc -ne 0 ]; then
            break
        fi
        iter=$((iter + 1))
    done
    end=$(date +%s%N)
    if [ $rc -eq 124 ]; then
        echo "TIMEOUT"
    elif [ $rc -ne 0 ]; then
        echo "ERROR"
    else
        local ms=$(( (end - start) / 1000000 ))
        echo "$ms"
    fi
}

color_cell() {
    local val="$1" ms="$2" best="$3" worst="$4"
    local num
    num=$(ms_to_num "$ms")
    if [ "$num" = "-1" ]; then
        printf "${DIM}%${W_COL}s${RST}" "$val"
    elif [ "$num" -le "$best" ]; then
        printf "${BOLD_GREEN}%${W_COL}s${RST}" "$val"
    elif [ "$num" -ge "$worst" ]; then
        printf "${BOLD_RED}%${W_COL}s${RST}" "$val"
    else
        printf "%${W_COL}s" "$val"
    fi
}

ratio_cell() {
    local jit="$1" node="$2"
    local jn nn
    jn=$(ms_to_num "$jit")
    nn=$(ms_to_num "$node")
    if [ "$jn" = "-1" ] || [ "$nn" = "-1" ] || [ "$nn" = "0" ]; then
        printf "${DIM}%${W_RATIO}s${RST}" "—"
        return
    fi
    local ratio
    ratio=$(awk "BEGIN{printf \"%.1f\", $jn/$nn}")
    local ratio_100x
    ratio_100x=$(awk "BEGIN{printf \"%.0f\", $jn*100/$nn}")
    if [ "$ratio_100x" -le 100 ]; then
        printf "${BOLD_GREEN}%${W_RATIO}s${RST}" "${ratio}x"
    elif [ "$ratio_100x" -le 200 ]; then
        printf "${BOLD_YELLOW}%${W_RATIO}s${RST}" "${ratio}x"
    else
        printf "${BOLD_RED}%${W_RATIO}s${RST}" "${ratio}x"
    fi
}

# ── Build ────────────────────────────────────────────────────────────────
printf "\n${BOLD_CYAN}  Shape Benchmark Suite${RST}\n"
printf "${DIM}  %d benchmarks × %d runtimes (%s)${RST}\n\n" "${#BENCHMARKS[@]}" "${#RUNTIMES[@]}" "${RUNTIMES[*]}"

if has_runtime rust; then
    printf "  ${CYAN}Building Rust benchmarks...${RST}"
    (cd "$SCRIPT_DIR/rust" && cargo build --release -q 2>/dev/null)
    printf " ${GREEN}done${RST}\n"
fi

if has_runtime go; then
    which go &>/dev/null || { echo "go not found"; exit 1; }
fi

if has_runtime go; then
    printf "  ${CYAN}Building Go benchmarks...${RST}"
    (cd "$SCRIPT_DIR/go" && go build -o bench_all . >/dev/null 2>&1)
    printf " ${GREEN}done${RST}\n"
fi

if has_runtime vm || has_runtime jit; then
    printf "  ${CYAN}Building Shape (release)...${RST}"
    (cd "$SHAPE_ROOT" && cargo build --release --bin shape --features shape-cli/jit -q 2>/dev/null)
    printf " ${GREEN}done${RST}\n"
fi

SHAPE_BIN="$SHAPE_ROOT/../target/release/shape"
RUST_BIN="$SCRIPT_DIR/rust/target/release/bench_all"
GO_BIN="$SCRIPT_DIR/go/bench_all"

if has_runtime python; then
    which python3 &>/dev/null || { echo "python3 not found"; exit 1; }
fi
if has_runtime node; then
    which node &>/dev/null || { echo "node not found"; exit 1; }
fi

# ── Collect results ──────────────────────────────────────────────────────
printf "\n  ${DIM}Running benchmarks...${RST}\n\n"

repeat_profile_parts=()
for bench in "${BENCHMARKS[@]}"; do
    rep="${BENCH_REPEATS["$bench"]:-1}"
    if [ "$rep" -gt 1 ]; then
        repeat_profile_parts+=("${bench}x${rep}")
    fi
done
if [ "${#repeat_profile_parts[@]}" -gt 0 ]; then
    printf "  ${DIM}Repeat profile:${RST} ${DIM}%s${RST}\n\n" "${repeat_profile_parts[*]}"
fi

declare -A RESULTS  # RESULTS[runtime,bench_idx] = ms
declare -A JIT_TYPED_OPS      # JIT_TYPED_OPS[bench_idx] = typed opcode count
declare -A JIT_GENERIC_OPS    # JIT_GENERIC_OPS[bench_idx] = generic opcode count
declare -A JIT_BYTECODE_COMPILE_MS  # JIT bytecode compile time (ms)
declare -A JIT_COMPILE_MS           # JIT machine-code compile time (ms)
declare -A JIT_EXEC_MS              # JIT execution time (ms)
declare -A JIT_NODE_RATIO_100X  # JIT/Node ratio * 100 per benchmark
declare -A JIT_NODE_WARM_RATIO_100X  # JIT(exec-only)/Node ratio * 100 per benchmark

for i in "${!BENCHMARKS[@]}"; do
    bench="${BENCHMARKS[$i]}"
    repeat="${BENCH_REPEATS["$bench"]:-1}"
    printf "  ${DIM}[%2d/%d]${RST} %-20s" "$((i+1))" "${#BENCHMARKS[@]}" "$bench"

    for rt in "${RUNTIMES[@]}"; do
        case "$rt" in
            rust)
                ms=$(time_cmd "$repeat" "$RUST_BIN" "$bench")
                ;;
            node)
                ms=$(time_cmd "$repeat" node "$SCRIPT_DIR/node/${bench}.mjs")
                ;;
            go)
                ms=$(time_cmd "$repeat" "$GO_BIN" "$bench")
                ;;
            python)
                ms=$(time_cmd "$repeat" python3 "$SCRIPT_DIR/python/${bench}.py")
                ;;
            vm)
                ms=$(time_cmd "$repeat" "$SHAPE_BIN" "$SCRIPT_DIR/shape/${bench}.shape")
                ;;
            jit)
                if $COLLECT_JIT_METRICS; then
                    ms=$(time_cmd "$repeat" env SHAPE_JIT_METRICS=1 SHAPE_JIT_PHASE_METRICS=1 "$SHAPE_BIN" -m jit "$SCRIPT_DIR/shape/${bench}.shape")
                    metrics_line=$(grep -E '^\[shape-jit-metrics\]' /tmp/bench_output | tail -n 1 || true)
                    if [ -n "$metrics_line" ]; then
                        # Use first matches so `typed_numeric_ops`/`generic_numeric_ops`
                        # are not shadowed by `static_typed_numeric_ops` fields.
                        typed=$(echo "$metrics_line" | grep -oE 'typed_numeric_ops=[0-9]+' | head -n 1 | cut -d= -f2)
                        generic=$(echo "$metrics_line" | grep -oE 'generic_numeric_ops=[0-9]+' | head -n 1 | cut -d= -f2)
                        if [ -n "$typed" ] && [ -n "$generic" ]; then
                            JIT_TYPED_OPS["$i"]="$typed"
                            JIT_GENERIC_OPS["$i"]="$generic"
                        fi
                    fi
                    phase_line=$(grep -E '^\[shape-jit-phases\]' /tmp/bench_output | tail -n 1 || true)
                    if [ -n "$phase_line" ]; then
                        bytecode_ms=$(echo "$phase_line" | sed -n 's/.*bytecode_compile_ms=\([0-9][0-9]*\).*/\1/p')
                        jit_compile_ms=$(echo "$phase_line" | sed -n 's/.*jit_compile_ms=\([0-9][0-9]*\).*/\1/p')
                        jit_exec_ms=$(echo "$phase_line" | sed -n 's/.*jit_exec_ms=\([0-9][0-9]*\).*/\1/p')
                        if [ -n "$bytecode_ms" ] && [ -n "$jit_compile_ms" ] && [ -n "$jit_exec_ms" ]; then
                            if [ "$repeat" -gt 1 ]; then
                                bytecode_ms=$((bytecode_ms * repeat))
                                jit_compile_ms=$((jit_compile_ms * repeat))
                                jit_exec_ms=$((jit_exec_ms * repeat))
                            fi
                            JIT_BYTECODE_COMPILE_MS["$i"]="$bytecode_ms"
                            JIT_COMPILE_MS["$i"]="$jit_compile_ms"
                            JIT_EXEC_MS["$i"]="$jit_exec_ms"
                        fi
                    fi
                else
                    ms=$(time_cmd "$repeat" "$SHAPE_BIN" -m jit "$SCRIPT_DIR/shape/${bench}.shape")
                fi
                ;;
        esac
        RESULTS["$rt,$i"]="$ms"
        printf "."
    done
    printf " ${GREEN}done${RST}\n"
done

# ── Render table ─────────────────────────────────────────────────────────
printf "\n"
has_both_jit_node=false
if has_runtime jit && has_runtime node; then has_both_jit_node=true; fi

if $has_both_jit_node; then
    hline "$TL" "$TT" "$TR" "extra"
else
    hline "$TL" "$TT" "$TR"
fi

# Header
printf "${V}${BOLD_WHITE}%-${W_NAME}s${RST}" "  Benchmark"
for rt in "${RUNTIMES[@]}"; do
    case "$rt" in
        rust)   printf "${V}${BOLD_MAGENTA}%${W_COL}s${RST}" "Rust " ;;
        node)   printf "${V}${BOLD_CYAN}%${W_COL}s${RST}" "Node " ;;
        go)     printf "${V}${BOLD_BLUE}%${W_COL}s${RST}" "Go " ;;
        python) printf "${V}${BOLD_YELLOW}%${W_COL}s${RST}" "Python " ;;
        vm)     printf "${V}${BOLD_WHITE}%${W_COL}s${RST}" "Shape VM " ;;
        jit)    printf "${V}${BOLD_GREEN}%${W_COL}s${RST}" "Shape JIT " ;;
    esac
done
if $has_both_jit_node; then
    printf "${V}${BOLD_WHITE}%${W_RATIO}s${RST}" "JIT/Node "
fi
printf "${V}\n"

if $has_both_jit_node; then
    hline "$LT" "$CR" "$RT" "extra"
else
    hline "$LT" "$CR" "$RT"
fi

# Data rows
jit_wins=0
jit_losses=0
for i in "${!BENCHMARKS[@]}"; do
    bench="${BENCHMARKS[$i]}"

    # Find min/max among all runtimes (only valid values)
    min_ms=999999
    max_ms=0
    for rt in "${RUNTIMES[@]}"; do
        ms="${RESULTS["$rt,$i"]}"
        num=$(ms_to_num "$ms")
        if [ "$num" = "-1" ]; then continue; fi
        if [ "$num" -lt "$min_ms" ]; then min_ms=$num; fi
        if [ "$num" -gt "$max_ms" ]; then max_ms=$num; fi
    done

    # Track JIT vs Node wins
    if $has_both_jit_node; then
        jn=$(ms_to_num "${RESULTS["jit,$i"]}")
        nn=$(ms_to_num "${RESULTS["node,$i"]}")
        if [ "$jn" != "-1" ] && [ "$nn" != "-1" ]; then
            if [ "$nn" != "0" ]; then
                ratio_100x=$(awk "BEGIN{printf \"%.0f\", $jn*100/$nn}")
                JIT_NODE_RATIO_100X["$bench"]="$ratio_100x"
            fi
            if [ "$jn" -le "$nn" ]; then
                jit_wins=$((jit_wins+1))
            else
                jit_losses=$((jit_losses+1))
            fi
        fi
    fi

    # Print row
    printf "${V}  %-$((W_NAME-2))s" "$bench"
    for rt in "${RUNTIMES[@]}"; do
        ms="${RESULTS["$rt,$i"]}"
        printf "${V}"; color_cell "$(fmt_ms "$ms") " "$ms" "$min_ms" "$max_ms"
    done
    if $has_both_jit_node; then
        printf "${V}"; ratio_cell "${RESULTS["jit,$i"]}" "${RESULTS["node,$i"]}"
    fi
    printf "${V}\n"
done

if $has_both_jit_node; then
    hline "$BL" "$BT" "$BR" "extra"
else
    hline "$BL" "$BT" "$BR"
fi

# ── Summary ──────────────────────────────────────────────────────────────
printf "\n"
printf "  ${BOLD_WHITE}Legend:${RST}  ${BOLD_GREEN}■ fastest${RST}    ${BOLD_RED}■ slowest${RST}    "
if $has_both_jit_node; then
    printf "${DIM}JIT/Node: ${BOLD_GREEN}<1x${RST}${DIM}=JIT wins  ${BOLD_YELLOW}1-2x${RST}${DIM}=close  ${BOLD_RED}>2x${RST}${DIM}=JIT behind${RST}"
fi
printf "\n\n"

if $has_both_jit_node; then
    geo_mean=""
    geo_count=0
    printf "  ${BOLD_WHITE}Shape JIT vs Node:${RST}  "
    printf "${BOLD_GREEN}%d wins${RST}  ${BOLD_RED}%d losses${RST}  " "$jit_wins" "$jit_losses"
    if [ "$jit_wins" -gt "$jit_losses" ]; then
        printf "${BOLD_GREEN}JIT leads!${RST}\n"
    elif [ "$jit_wins" -eq "$jit_losses" ]; then
        printf "${BOLD_YELLOW}Tied${RST}\n"
    else
        printf "${BOLD_RED}Node leads${RST}\n"
    fi

    # Compute geometric mean of JIT/Node ratios
    geo_product="1"
    for i in "${!BENCHMARKS[@]}"; do
        jn=$(ms_to_num "${RESULTS["jit,$i"]}")
        nn=$(ms_to_num "${RESULTS["node,$i"]}")
        if [ "$jn" != "-1" ] && [ "$nn" != "-1" ] && [ "$nn" != "0" ]; then
            geo_product=$(awk "BEGIN{printf \"%.10f\", $geo_product * ($jn/$nn)}")
            geo_count=$((geo_count+1))
        fi
    done
    if [ "$geo_count" -gt 0 ]; then
        geo_mean=$(awk "BEGIN{printf \"%.2f\", exp(log($geo_product)/$geo_count)}")
        printf "  ${BOLD_WHITE}Geometric mean JIT/Node:${RST}  "
        gm_100=$(awk "BEGIN{printf \"%.0f\", $geo_mean * 100}")
        if [ "$gm_100" -le 100 ]; then
            printf "${BOLD_GREEN}${geo_mean}x${RST} ${DIM}(JIT is faster on average)${RST}\n"
        else
            printf "${BOLD_RED}${geo_mean}x${RST} ${DIM}(JIT is slower on average)${RST}\n"
        fi
    fi
fi

if has_runtime jit && $COLLECT_JIT_METRICS; then
    typed_total=0
    generic_total=0
    metrics_rows=0
    printf "\n  ${BOLD_WHITE}JIT Numeric Coverage (Effective, with speculation):${RST}\n"
    for i in "${!BENCHMARKS[@]}"; do
        typed="${JIT_TYPED_OPS["$i"]:-}"
        generic="${JIT_GENERIC_OPS["$i"]:-}"
        if [ -z "$typed" ] || [ -z "$generic" ]; then
            continue
        fi
        total=$((typed + generic))
        if [ "$total" -eq 0 ]; then
            cov="100.00"
        else
            cov=$(awk "BEGIN{printf \"%.2f\", $typed*100/$total}")
        fi
        bench="${BENCHMARKS[$i]}"
        printf "    %-18s  typed=%-4s generic=%-4s coverage=%s%%\n" "$bench" "$typed" "$generic" "$cov"
        typed_total=$((typed_total + typed))
        generic_total=$((generic_total + generic))
        metrics_rows=$((metrics_rows + 1))
    done
    if [ "$metrics_rows" -gt 0 ]; then
        sum_total=$((typed_total + generic_total))
        if [ "$sum_total" -eq 0 ]; then
            avg_cov="100.00"
        else
            avg_cov=$(awk "BEGIN{printf \"%.2f\", $typed_total*100/$sum_total}")
        fi
        printf "    ${DIM}Overall effective coverage: %s%% (%s typed / %s generic)${RST}\n" "$avg_cov" "$typed_total" "$generic_total"
    fi
fi

if has_runtime jit && $COLLECT_JIT_METRICS; then
    phase_rows=0
    bytecode_total=0
    jit_compile_total=0
    jit_exec_total=0
    printf "\n  ${BOLD_WHITE}JIT Phase Timing (Reported by JIT):${RST}\n"
    for i in "${!BENCHMARKS[@]}"; do
        b_ms="${JIT_BYTECODE_COMPILE_MS["$i"]:-}"
        c_ms="${JIT_COMPILE_MS["$i"]:-}"
        e_ms="${JIT_EXEC_MS["$i"]:-}"
        if [ -z "$b_ms" ] || [ -z "$c_ms" ] || [ -z "$e_ms" ]; then
            continue
        fi
        phase_total=$((b_ms + c_ms + e_ms))
        if [ "$phase_total" -eq 0 ]; then
            compile_share="0.0"
        else
            compile_share=$(awk "BEGIN{printf \"%.1f\", ($b_ms+$c_ms)*100/$phase_total}")
        fi
        bench="${BENCHMARKS[$i]}"
        printf "    %-18s  bytecode=%-4sms jit-compile=%-4sms exec=%-5sms compile-share=%s%%\n" \
            "$bench" "$b_ms" "$c_ms" "$e_ms" "$compile_share"
        bytecode_total=$((bytecode_total + b_ms))
        jit_compile_total=$((jit_compile_total + c_ms))
        jit_exec_total=$((jit_exec_total + e_ms))
        phase_rows=$((phase_rows + 1))
    done
    if [ "$phase_rows" -gt 0 ]; then
        avg_bytecode=$(awk "BEGIN{printf \"%.1f\", $bytecode_total/$phase_rows}")
        avg_compile=$(awk "BEGIN{printf \"%.1f\", $jit_compile_total/$phase_rows}")
        avg_exec=$(awk "BEGIN{printf \"%.1f\", $jit_exec_total/$phase_rows}")
        avg_total=$(awk "BEGIN{printf \"%.1f\", ($bytecode_total+$jit_compile_total+$jit_exec_total)/$phase_rows}")
        if [ "$(awk "BEGIN{printf \"%.0f\", $avg_total}")" -eq 0 ]; then
            avg_compile_share="0.0"
        else
            avg_compile_share=$(awk "BEGIN{printf \"%.1f\", ($avg_bytecode+$avg_compile)*100/$avg_total}")
        fi
        printf "    ${DIM}Average phases: bytecode=%sms jit-compile=%sms exec=%sms total=%sms compile-share=%s%%${RST}\n" \
            "$avg_bytecode" "$avg_compile" "$avg_exec" "$avg_total" "$avg_compile_share"
    fi
fi

warm_geo_mean=""
if $has_both_jit_node && has_runtime jit && $COLLECT_JIT_METRICS; then
    warm_rows=0
    warm_product="1"
    printf "\n  ${BOLD_WHITE}JIT vs Node Warm Runtime (exec-only):${RST}\n"
    for i in "${!BENCHMARKS[@]}"; do
        bench="${BENCHMARKS[$i]}"
        e_ms="${JIT_EXEC_MS["$i"]:-}"
        nn=$(ms_to_num "${RESULTS["node,$i"]}")
        if [ -z "$e_ms" ] || [ "$nn" = "-1" ] || [ "$nn" = "0" ]; then
            continue
        fi
        warm_100x=$(awk "BEGIN{printf \"%.0f\", $e_ms*100/$nn}")
        JIT_NODE_WARM_RATIO_100X["$bench"]="$warm_100x"
        warm_ratio=$(awk "BEGIN{printf \"%.2f\", $warm_100x/100.0}")
        cold_100x="${JIT_NODE_RATIO_100X["$bench"]:-}"
        if [ -n "$cold_100x" ]; then
            cold_ratio=$(awk "BEGIN{printf \"%.2f\", $cold_100x/100.0}")
            printf "    %-18s  cold=%sx  warm=%sx\n" "$bench" "$cold_ratio" "$warm_ratio"
        else
            printf "    %-18s  warm=%sx\n" "$bench" "$warm_ratio"
        fi
        warm_product=$(awk "BEGIN{printf \"%.10f\", $warm_product * ($e_ms/$nn)}")
        warm_rows=$((warm_rows + 1))
    done
    if [ "$warm_rows" -gt 0 ]; then
        warm_geo_mean=$(awk "BEGIN{printf \"%.2f\", exp(log($warm_product)/$warm_rows)}")
        printf "    ${DIM}Warm geomean JIT(exec)/Node: %sx${RST}\n" "$warm_geo_mean"
    fi
fi

if $has_both_jit_node && [ -n "$WRITE_RATIOS_FILE" ]; then
    {
        echo "# benchmark jit_node_ratio"
        for bench in "${BENCHMARKS[@]}"; do
            ratio_100x="${JIT_NODE_RATIO_100X["$bench"]:-}"
            if [ -n "$ratio_100x" ]; then
                ratio=$(awk "BEGIN{printf \"%.2f\", $ratio_100x/100.0}")
                echo "$bench $ratio"
            fi
        done
        if [ -n "$geo_mean" ]; then
            echo "GEOMEAN $geo_mean"
        fi
        if [ -n "$warm_geo_mean" ]; then
            echo "GEOMEAN_WARM $warm_geo_mean"
        fi
    } > "$WRITE_RATIOS_FILE"
    printf "\n  ${DIM}Wrote JIT/Node ratios to %s${RST}\n" "$WRITE_RATIOS_FILE"
fi

if [ -n "$WRITE_METRICS_FILE" ]; then
    {
        echo -e "benchmark\trust_ms\tnode_ms\tgo_ms\tpython_ms\tvm_ms\tjit_ms\tjit_node_ratio\tjit_node_warm_ratio\tjit_node_ratio_100x\tjit_node_warm_ratio_100x\ttyped_numeric_ops\tgeneric_numeric_ops\tbytecode_compile_ms\tjit_compile_ms\tjit_exec_ms\tgeomean_jit_node\twarm_geomean_jit_exec_node"
        for i in "${!BENCHMARKS[@]}"; do
            bench="${BENCHMARKS[$i]}"
            rust_ms="${RESULTS["rust,$i"]:-}"
            node_ms="${RESULTS["node,$i"]:-}"
            go_ms="${RESULTS["go,$i"]:-}"
            python_ms="${RESULTS["python,$i"]:-}"
            vm_ms="${RESULTS["vm,$i"]:-}"
            jit_ms="${RESULTS["jit,$i"]:-}"
            ratio_100x="${JIT_NODE_RATIO_100X["$bench"]:-}"
            warm_ratio_100x="${JIT_NODE_WARM_RATIO_100X["$bench"]:-}"
            typed="${JIT_TYPED_OPS["$i"]:-}"
            generic="${JIT_GENERIC_OPS["$i"]:-}"
            bytecode_ms="${JIT_BYTECODE_COMPILE_MS["$i"]:-}"
            jit_compile_ms="${JIT_COMPILE_MS["$i"]:-}"
            jit_exec_ms="${JIT_EXEC_MS["$i"]:-}"

            ratio=""
            warm_ratio=""
            if [ -n "$ratio_100x" ]; then
                ratio=$(awk "BEGIN{printf \"%.2f\", $ratio_100x/100.0}")
            fi
            if [ -n "$warm_ratio_100x" ]; then
                warm_ratio=$(awk "BEGIN{printf \"%.2f\", $warm_ratio_100x/100.0}")
            fi

            echo -e "${bench}\t${rust_ms}\t${node_ms}\t${go_ms}\t${python_ms}\t${vm_ms}\t${jit_ms}\t${ratio}\t${warm_ratio}\t${ratio_100x}\t${warm_ratio_100x}\t${typed}\t${generic}\t${bytecode_ms}\t${jit_compile_ms}\t${jit_exec_ms}\t\t"
        done
        echo -e "__summary__\t\t\t\t\t\t\t\t\t\t\t\t\t\t\t${geo_mean:-}\t${warm_geo_mean:-}"
    } > "$WRITE_METRICS_FILE"
    printf "  ${DIM}Wrote benchmark metrics to %s${RST}\n" "$WRITE_METRICS_FILE"
fi

if $ENFORCE; then
    if ! $has_both_jit_node; then
        printf "\n  ${BOLD_RED}CI guardrails require both 'jit' and 'node' runtimes.${RST}\n"
        exit 1
    fi

    if [ "${#BUDGET_RATIO_100X[@]}" -eq 0 ] && [ -z "$MAX_GEOMEAN" ]; then
        printf "\n  ${BOLD_RED}No guardrail thresholds found. Provide --budget-file and/or --max-geomean.${RST}\n"
        exit 1
    fi

    failures=0
    printf "\n  ${BOLD_WHITE}CI Guardrails:${RST}\n"

    for bench in "${BENCHMARKS[@]}"; do
        budget_100x="${BUDGET_RATIO_100X["$bench"]:-}"
        if [ -z "$budget_100x" ]; then
            continue
        fi
        observed_100x="${JIT_NODE_RATIO_100X["$bench"]:-}"
        if [ -z "$observed_100x" ]; then
            printf "    ${BOLD_RED}%s${RST} missing JIT/Node ratio\n" "$bench"
            failures=$((failures + 1))
            continue
        fi
        observed=$(awk "BEGIN{printf \"%.2f\", $observed_100x/100.0}")
        budget=$(awk "BEGIN{printf \"%.2f\", $budget_100x/100.0}")
        if [ "$observed_100x" -le "$budget_100x" ]; then
            printf "    ${BOLD_GREEN}%s${RST} %sx <= %sx\n" "$bench" "$observed" "$budget"
        else
            printf "    ${BOLD_RED}%s${RST} %sx > %sx\n" "$bench" "$observed" "$budget"
            failures=$((failures + 1))
        fi
    done

    if [ -n "$MAX_GEOMEAN" ] && [ -n "$geo_mean" ]; then
        gm_100x=$(awk "BEGIN{printf \"%.0f\", $geo_mean * 100}")
        max_gm_100x=$(awk "BEGIN{printf \"%.0f\", $MAX_GEOMEAN * 100}")
        if [ "$gm_100x" -le "$max_gm_100x" ]; then
            printf "    ${BOLD_GREEN}GEOMEAN${RST} %sx <= %sx\n" "$geo_mean" "$MAX_GEOMEAN"
        else
            printf "    ${BOLD_RED}GEOMEAN${RST} %sx > %sx\n" "$geo_mean" "$MAX_GEOMEAN"
            failures=$((failures + 1))
        fi
    fi

    if [ "$failures" -gt 0 ]; then
        printf "\n  ${BOLD_RED}Guardrail check failed: %d violation(s).${RST}\n" "$failures"
        exit 1
    fi
    printf "\n  ${BOLD_GREEN}Guardrail check passed.${RST}\n"
fi

printf "\n"
