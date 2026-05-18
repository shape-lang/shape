//! Bounded mutation engine for the differential-fuzz corpus per W13 audit §4.2.
//!
//! The engine takes a base seed (a Shape source program) and applies a fixed
//! set of small, AST-aware textual rewrites. The output is a sequence of
//! derived programs that the harness runs the same way as base seeds.
//!
//! Strategy bindings (audit §4.2 + §1.3 refusals):
//!
//! - **Text-level edits over a regex'd token set.** Not random-byte mutation.
//!   Random bytes almost never form valid Shape programs (audit §1.3).
//! - **Bounded count per seed.** Default 5 derived seeds per base seed per
//!   nightly run; configurable via `MutationConfig::max_mutations`.
//! - **Deterministic for a given seed parameter.** A `u64` seed drives a
//!   tiny xorshift PRNG so a finding can be reproduced exactly.
//! - **Drops zero-effect edits.** Mutations that do not change the source
//!   string are not emitted — the harness would re-run an identical
//!   program for no signal.
//!
//! Strategies (5):
//!
//! 1. `NumericBoundShift` — replace one integer literal with `0`, `1`, `-1`,
//!    `i64::MAX`, or `i64::MIN`. Captures off-by-one and sign-flip surfaces.
//! 2. `CollectionSizeShift` — replace one `Vec<T>` literal with `[]`, a 1-elt
//!    form, or a 16-elt form. Exercises empty/1/many collection paths.
//! 3. `OperatorSwap` — swap one arithmetic operator (`+`/`-`/`*`/`/`) or one
//!    comparison (`<`/`<=`/`>`/`>=`/`==`/`!=`) for another in the same class.
//! 4. `CaptureMutation` — replace `|x|` with `|_x|` (drop-binding) or `|x, y|`
//!    with `|y, x|` (swap-binding) — mutates closure capture semantics.
//! 5. `TierUpWrap` — wrap the whole program body in `for _ in 0..N { ... }`
//!    for N in {100, 10000}, exercising the §2.2 tier boundary directly.
//!
//! Mutations are intentionally narrow per §4.2 ("intentionally NARROW —
//! coverage-guided mutation is explicitly OUT OF SCOPE"). Richer fuzzing
//! is a v0.4+ candidate.

use std::path::{Path, PathBuf};
use std::{fs, io};

/// Configuration for the mutation engine.
#[derive(Debug, Clone)]
pub struct MutationConfig {
    /// Cap on derived seeds per base seed per run. Audit §4.2 picks 5; CLI
    /// overrides via `--mutations-per-seed`.
    pub max_mutations: usize,
    /// PRNG seed for deterministic mutation selection. Two runs with the
    /// same `prng_seed` against the same base produce the same derived
    /// sequence — required for triage reproducibility per §4.2.
    pub prng_seed: u64,
}

impl Default for MutationConfig {
    fn default() -> Self {
        Self {
            max_mutations: 5,
            prng_seed: 0x5d2f_4f4e_8a23_91c1,
        }
    }
}

/// Identifier of a mutation strategy. Stable for findings filenames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    NumericBoundShift,
    CollectionSizeShift,
    OperatorSwap,
    CaptureMutation,
    TierUpWrap,
}

impl Strategy {
    pub fn name(self) -> &'static str {
        match self {
            Self::NumericBoundShift => "numeric-bound-shift",
            Self::CollectionSizeShift => "collection-size-shift",
            Self::OperatorSwap => "operator-swap",
            Self::CaptureMutation => "capture-mutation",
            Self::TierUpWrap => "tier-up-wrap",
        }
    }

    pub const ALL: &'static [Strategy] = &[
        Self::NumericBoundShift,
        Self::CollectionSizeShift,
        Self::OperatorSwap,
        Self::CaptureMutation,
        Self::TierUpWrap,
    ];
}

/// One derived seed produced from a base seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedSeed {
    /// Strategy that produced this derived program.
    pub strategy: Strategy,
    /// Strategy invocation index within the run (stable for filename ordering).
    pub index: usize,
    /// Full derived source text.
    pub source: String,
}

/// Tiny xorshift64 PRNG. Deterministic given the same seed; no `rand`
/// dependency, keeps the crate dependency surface bounded per the audit
/// "no new dev-tools dependency" line.
#[derive(Debug, Clone)]
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // Guard against the all-zero state which xorshift cannot escape.
        let state = if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed };
        Self { state }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_in_range(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next_u64() as usize) % n }
    }
}

/// Apply the bounded mutation set to `source`, returning up to
/// `cfg.max_mutations` derived programs.
///
/// The output is deterministic given `cfg.prng_seed` + `source`. Empty if
/// every candidate edit is a no-op against the source (e.g. the seed has no
/// integer literal so `NumericBoundShift` cannot fire).
pub fn mutate_seed(source: &str, cfg: &MutationConfig) -> Vec<DerivedSeed> {
    let mut prng = Xorshift64::new(cfg.prng_seed);
    let mut out = Vec::new();
    let mut attempts = 0usize;
    let cap = cfg.max_mutations.saturating_mul(4); // bounded retry budget for no-op strategies

    while out.len() < cfg.max_mutations && attempts < cap {
        attempts += 1;
        let strategy = Strategy::ALL[prng.next_in_range(Strategy::ALL.len())];
        let derived = match strategy {
            Strategy::NumericBoundShift => apply_numeric_bound_shift(source, &mut prng),
            Strategy::CollectionSizeShift => apply_collection_size_shift(source, &mut prng),
            Strategy::OperatorSwap => apply_operator_swap(source, &mut prng),
            Strategy::CaptureMutation => apply_capture_mutation(source, &mut prng),
            Strategy::TierUpWrap => apply_tier_up_wrap(source, &mut prng),
        };
        if let Some(new_source) = derived {
            if new_source != source && !out.iter().any(|s: &DerivedSeed| s.source == new_source) {
                out.push(DerivedSeed {
                    strategy,
                    index: out.len(),
                    source: new_source,
                });
            }
        }
    }

    out
}

/// Apply `mutate_seed` to a base seed file, write the derived programs into
/// `out_dir/<stem>__<index>__<strategy>.shape`, and return the list of
/// derived paths.
pub fn mutate_seed_to_dir(
    base: &Path,
    out_dir: &Path,
    cfg: &MutationConfig,
) -> io::Result<Vec<PathBuf>> {
    let source = fs::read_to_string(base)?;
    fs::create_dir_all(out_dir)?;
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("seed");

    let derived = mutate_seed(&source, cfg);
    let mut paths = Vec::with_capacity(derived.len());
    for d in derived {
        let filename = format!("{stem}__{:02}__{}.shape", d.index, d.strategy.name());
        let path = out_dir.join(filename);
        fs::write(&path, &d.source)?;
        paths.push(path);
    }
    Ok(paths)
}

// ---- Strategy 1: NumericBoundShift ----------------------------------------

const NUMERIC_REPLACEMENTS: &[&str] = &[
    "0",
    "1",
    "-1",
    "9223372036854775807",  // i64::MAX
    "-9223372036854775808", // i64::MIN
];

fn apply_numeric_bound_shift(source: &str, prng: &mut Xorshift64) -> Option<String> {
    let spans = find_integer_literal_spans(source);
    if spans.is_empty() {
        return None;
    }
    let pick = prng.next_in_range(spans.len());
    let (start, end) = spans[pick];
    let replacement = NUMERIC_REPLACEMENTS[prng.next_in_range(NUMERIC_REPLACEMENTS.len())];
    let mut out = String::with_capacity(source.len() + replacement.len());
    out.push_str(&source[..start]);
    out.push_str(replacement);
    out.push_str(&source[end..]);
    Some(out)
}

/// Find spans of integer literals — sequences of decimal digits not adjacent
/// to alpha/underscore/dot. Skips floats (`3.14`), identifiers (`x1`), and
/// non-decimal literals (`0x...`, `0b...`).
fn find_integer_literal_spans(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // Don't claim a digit immediately preceded by alpha/_/digit (identifier tail)
        // or by `.` (potential float tail).
        if i > 0 {
            let b = bytes[i - 1];
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'.' {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                continue;
            }
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let end = i;
        // Reject if followed by `.` (float), `b`/`x`/`o` (0b/0x/0o prefix
        // already matched the 0 only — skip this run), or alpha/_ (identifier).
        if end < bytes.len() {
            let b = bytes[end];
            if b == b'.' || b == b'b' || b == b'x' || b == b'o' || b.is_ascii_alphabetic() || b == b'_' {
                continue;
            }
        }
        spans.push((start, end));
    }
    spans
}

// ---- Strategy 2: CollectionSizeShift ---------------------------------------

const COLLECTION_REPLACEMENTS: &[&str] = &[
    "[]",
    "[1]",
    "[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]",
];

fn apply_collection_size_shift(source: &str, prng: &mut Xorshift64) -> Option<String> {
    let spans = find_array_literal_spans(source);
    if spans.is_empty() {
        return None;
    }
    let pick = prng.next_in_range(spans.len());
    let (start, end) = spans[pick];
    let replacement = COLLECTION_REPLACEMENTS[prng.next_in_range(COLLECTION_REPLACEMENTS.len())];
    let mut out = String::with_capacity(source.len() + replacement.len());
    out.push_str(&source[..start]);
    out.push_str(replacement);
    out.push_str(&source[end..]);
    Some(out)
}

/// Find spans of balanced bracket-delimited array literals `[...]`. Skips
/// brackets that look like index access (`v[i]`) by requiring the bracket to
/// be preceded by an operator-class char (`=`, `(`, `,`, ` `, `\n`, `\t`).
fn find_array_literal_spans(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        let prev = if i == 0 { b' ' } else { bytes[i - 1] };
        let is_array_literal = matches!(prev, b'=' | b'(' | b',' | b' ' | b'\n' | b'\t' | b'>' | b'{');
        if !is_array_literal {
            i += 1;
            continue;
        }
        // Find matching `]` (bracket-balanced; no string-aware parsing — best-effort).
        let mut depth = 1;
        let mut j = i + 1;
        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        if depth == 0 {
            spans.push((i, j));
            i = j;
        } else {
            i += 1;
        }
    }
    spans
}

// ---- Strategy 3: OperatorSwap ----------------------------------------------

const ARITH_OPS: &[&str] = &["+", "-", "*", "/"];
const CMP_OPS: &[&str] = &["<", "<=", ">", ">=", "==", "!="];

fn apply_operator_swap(source: &str, prng: &mut Xorshift64) -> Option<String> {
    let mut candidates: Vec<(usize, usize, &'static [&'static str])> = Vec::new();
    candidates.extend(
        find_operator_spans(source, ARITH_OPS).into_iter().map(|(s, e)| (s, e, ARITH_OPS)),
    );
    candidates.extend(
        find_operator_spans(source, CMP_OPS).into_iter().map(|(s, e)| (s, e, CMP_OPS)),
    );
    if candidates.is_empty() {
        return None;
    }
    let pick = prng.next_in_range(candidates.len());
    let (start, end, set) = candidates[pick];
    let existing = &source[start..end];
    // Pick a different operator from the same class.
    let other: Vec<&&str> = set.iter().filter(|o| **o != existing).collect();
    if other.is_empty() {
        return None;
    }
    let replacement = *other[prng.next_in_range(other.len())];
    let mut out = String::with_capacity(source.len());
    out.push_str(&source[..start]);
    out.push_str(replacement);
    out.push_str(&source[end..]);
    Some(out)
}

/// Find spans of two-char or one-char operators surrounded by whitespace on
/// at least one side (avoids matching inside identifiers / pipes / closures).
fn find_operator_spans(source: &str, set: &[&str]) -> Vec<(usize, usize)> {
    let mut sorted: Vec<&&str> = set.iter().collect();
    sorted.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut spans = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    'outer: while i < bytes.len() {
        for op in &sorted {
            let op_bytes = op.as_bytes();
            if i + op_bytes.len() > bytes.len() {
                continue;
            }
            if &bytes[i..i + op_bytes.len()] != op_bytes {
                continue;
            }
            // Whitespace flank to avoid `|x|`, `=>`, `->`, `<T`, `>=`, etc.
            let prev = if i == 0 { b' ' } else { bytes[i - 1] };
            let next = if i + op_bytes.len() >= bytes.len() {
                b' '
            } else {
                bytes[i + op_bytes.len()]
            };
            let ws = |b: u8| matches!(b, b' ' | b'\t' | b'\n');
            if ws(prev) && ws(next) {
                spans.push((i, i + op_bytes.len()));
                i += op_bytes.len();
                continue 'outer;
            }
        }
        i += 1;
    }
    spans
}

// ---- Strategy 4: CaptureMutation -------------------------------------------

fn apply_capture_mutation(source: &str, prng: &mut Xorshift64) -> Option<String> {
    // Find `|<ident>|` (single param) and rewrite to `|_<ident>|` (kill the
    // binding). Skips |a, b| forms — captured-by-value vs swap is risky on
    // text-level rewrites and the audit's intent is closure capture shape
    // mutation, which the underscore-bind covers.
    let bytes = source.as_bytes();
    let mut spans: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] != b'|' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        // Skip the simplest case: |ident|
        let id_start = j;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        if j > id_start && j < bytes.len() && bytes[j] == b'|' {
            let end = j + 1;
            let ident = std::str::from_utf8(&bytes[id_start..j]).unwrap_or("x");
            // Skip underscored already.
            if !ident.starts_with('_') && ident != "self" {
                spans.push((start, end, format!("|_{ident}|")));
            }
        }
        i = j + 1;
    }
    if spans.is_empty() {
        return None;
    }
    let pick = prng.next_in_range(spans.len());
    let (start, end, repl) = &spans[pick];
    let mut out = String::with_capacity(source.len() + repl.len());
    out.push_str(&source[..*start]);
    out.push_str(repl);
    out.push_str(&source[*end..]);
    Some(out)
}

// ---- Strategy 5: TierUpWrap ------------------------------------------------

const TIER_UP_N: &[u32] = &[100, 10000];

fn apply_tier_up_wrap(source: &str, prng: &mut Xorshift64) -> Option<String> {
    // Wrap the entire program in `for _ in 0..N { <source> }`. Only applies
    // if the source has no top-level `fn` / `enum` / `type` / `trait` / `impl`
    // / `extend` / `extern` / `async fn` declarations — wrapping declarations
    // in a for-loop is illegal Shape.
    let head_keywords = [
        "fn ", "enum ", "type ", "trait ", "impl ", "extend ", "extern ",
        "async ", "var ", "use ", "import ", "mod ", "pub ",
    ];
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if head_keywords.iter().any(|kw| trimmed.starts_with(kw)) {
            return None;
        }
    }
    let n = TIER_UP_N[prng.next_in_range(TIER_UP_N.len())];
    // Indent every body line by two spaces — Shape doesn't care about
    // indentation but it makes the derived source readable in triage.
    let indented = source
        .lines()
        .map(|l| if l.is_empty() { String::new() } else { format!("  {l}") })
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("for _ in 0..{n} {{\n{indented}\n}}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(seed: u64, max: usize) -> MutationConfig {
        MutationConfig { max_mutations: max, prng_seed: seed }
    }

    #[test]
    fn xorshift_is_deterministic_for_a_given_seed() {
        let mut a = Xorshift64::new(0x1234);
        let mut b = Xorshift64::new(0x1234);
        for _ in 0..16 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn xorshift_handles_zero_seed_without_lockup() {
        let mut p = Xorshift64::new(0);
        // Different invocations must produce non-equal outputs.
        let v0 = p.next_u64();
        let v1 = p.next_u64();
        assert_ne!(v0, v1);
        assert_ne!(v0, 0);
    }

    #[test]
    fn find_integer_literal_spans_skips_floats_and_identifiers() {
        let s = "let x1 = 2 + 3.14\nlet y = 0xff\nprint(42)\n";
        let spans = find_integer_literal_spans(s);
        // Expect spans for `2` and `42` only. `1` in `x1` is alpha-adjacent;
        // `3` in `3.14` is followed by `.`; `0` in `0xff` is followed by `x`.
        let tokens: Vec<&str> = spans.iter().map(|(a, b)| &s[*a..*b]).collect();
        assert_eq!(tokens, vec!["2", "42"]);
    }

    #[test]
    fn find_array_literal_spans_balances_brackets() {
        let s = "let v = [1, [2, 3], 4]\nlet w = v[0]\n";
        let spans = find_array_literal_spans(s);
        // Expect one span for the outer literal; `v[0]` not flanked by an
        // operator-class char so it's not classified as a literal.
        assert_eq!(spans.len(), 1);
        let (a, b) = spans[0];
        assert_eq!(&s[a..b], "[1, [2, 3], 4]");
    }

    #[test]
    fn numeric_bound_shift_replaces_an_integer_literal() {
        let src = "print(2 + 3)\n";
        let c = cfg(1, 5);
        let derived = mutate_seed(src, &c);
        assert!(!derived.is_empty(), "should produce at least one derived seed");
        for d in &derived {
            assert_ne!(d.source, src);
        }
    }

    #[test]
    fn mutate_seed_is_deterministic_for_a_given_prng_seed() {
        let src = "let v: Vec<int> = [1, 2, 3, 4, 5]\nprint(v.map(|x| x * 2).sum())\n";
        let c = cfg(0xabc, 5);
        let a = mutate_seed(src, &c);
        let b = mutate_seed(src, &c);
        assert_eq!(a, b);
    }

    #[test]
    fn mutate_seed_respects_max_mutations_cap() {
        let src = "let v: Vec<int> = [1, 2, 3]\nprint(v.sum())\n";
        for max in [0usize, 1, 2, 5, 10] {
            let c = cfg(99, max);
            let derived = mutate_seed(src, &c);
            assert!(
                derived.len() <= max,
                "derived={} exceeded cap={}",
                derived.len(),
                max
            );
        }
    }

    #[test]
    fn mutate_seed_returns_empty_when_source_has_no_mutatable_token() {
        // No integer literals, no array literals, no whitespace-flanked
        // operators, no `|ident|` closure, and the source has a `fn` keyword
        // so TierUpWrap declines.
        let src = "fn main() {}\n";
        let derived = mutate_seed(src, &cfg(42, 5));
        assert!(derived.is_empty(), "got unexpected mutations: {:?}", derived);
    }

    #[test]
    fn operator_swap_swaps_arith_only_when_flanked_by_whitespace() {
        let src = "print(2 + 3)\n";
        let c = MutationConfig { max_mutations: 20, prng_seed: 7 };
        let derived = mutate_seed(src, &c);
        // At least one mutation should be an operator swap; verify by finding
        // a derived form whose `+` is replaced by `-`/`*`/`/`.
        let any_op_swap = derived.iter().any(|d| {
            d.source.contains("- 3") || d.source.contains("* 3") || d.source.contains("/ 3")
        });
        assert!(any_op_swap, "no OperatorSwap fired: {:?}", derived);
    }

    #[test]
    fn capture_mutation_underscore_prefixes_a_closure_param() {
        let src = "let v: Vec<int> = [1,2,3]\nprint(v.map(|x| x * 2).sum())\n";
        let c = MutationConfig { max_mutations: 20, prng_seed: 11 };
        let derived = mutate_seed(src, &c);
        let any_capture = derived.iter().any(|d| d.source.contains("|_x|"));
        assert!(any_capture, "no CaptureMutation fired: {:?}", derived);
    }

    #[test]
    fn tier_up_wrap_refuses_when_program_has_top_level_fn() {
        let src = "fn f() -> int { 1 }\nprint(f())\n";
        let mut prng = Xorshift64::new(0);
        let out = apply_tier_up_wrap(src, &mut prng);
        assert!(out.is_none(), "tier-up-wrap must decline programs with fn");
    }

    #[test]
    fn tier_up_wrap_applies_when_program_is_top_level_only() {
        let src = "let x = 5\nprint(x)\n";
        let mut prng = Xorshift64::new(0);
        let out = apply_tier_up_wrap(src, &mut prng).expect("should wrap");
        assert!(out.starts_with("for _ in 0..") || out.starts_with("for _ in 0.."), "{out}");
        assert!(out.contains("  let x = 5"), "body should be indented; got: {out}");
        assert!(out.contains("  print(x)"));
    }

    #[test]
    fn collection_size_shift_replaces_one_array_literal() {
        let src = "let v: Vec<int> = [1, 2, 3, 4, 5]\nprint(v.len())\n";
        let c = MutationConfig { max_mutations: 20, prng_seed: 3 };
        let derived = mutate_seed(src, &c);
        let any_size = derived.iter().any(|d| {
            d.source.contains("= []") || d.source.contains("= [1]") || d.source.contains("[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]")
        });
        assert!(any_size, "no CollectionSizeShift fired: {:?}", derived);
    }

    #[test]
    fn mutate_seed_to_dir_writes_files_with_stable_names() {
        let dir = std::env::temp_dir().join(format!(
            "shape-fuzz-mut-dir-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let base = dir.join("seed.shape");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&base, "let v: Vec<int> = [1, 2, 3]\nprint(v.sum())\n").unwrap();
        let out_dir = dir.join("mutated");
        let paths = mutate_seed_to_dir(&base, &out_dir, &cfg(13, 3)).unwrap();
        assert!(!paths.is_empty());
        for p in &paths {
            let name = p.file_name().unwrap().to_str().unwrap();
            assert!(name.starts_with("seed__"), "name format wrong: {name}");
            assert!(name.ends_with(".shape"));
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
