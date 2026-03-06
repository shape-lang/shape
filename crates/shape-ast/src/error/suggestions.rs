//! Error suggestion and hint generation
//!
//! This module provides utilities for generating helpful error messages,
//! including "did you mean" suggestions and type conversion hints.

/// Find similar names using Levenshtein distance
pub fn find_similar<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
    max_distance: usize,
) -> Vec<&'a str> {
    let mut similar: Vec<(&str, usize)> = candidates
        .filter_map(|candidate| {
            let dist = levenshtein_distance(name, candidate);
            if dist <= max_distance && dist > 0 {
                Some((candidate, dist))
            } else {
                None
            }
        })
        .collect();
    similar.sort_by_key(|(_, d)| *d);
    similar.into_iter().map(|(s, _)| s).collect()
}

/// Simple Levenshtein distance implementation
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row = vec![0; b_len + 1];

    for (i, a_char) in a_chars.iter().enumerate() {
        curr_row[0] = i + 1;
        for (j, b_char) in b_chars.iter().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1)
                .min(curr_row[j] + 1)
                .min(prev_row[j] + cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Generate a "did you mean" hint if similar names exist
pub fn did_you_mean<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> Option<String> {
    let similar = find_similar(name, candidates, 3);
    match similar.len() {
        0 => None,
        1 => Some(format!("did you mean `{}`?", similar[0])),
        _ => Some(format!(
            "did you mean one of: {}?",
            similar
                .iter()
                .map(|s| format!("`{}`", s))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Generate hint for common type mismatches
pub fn type_conversion_hint(expected: &str, actual: &str) -> Option<String> {
    match (expected, actual) {
        ("number", "string") => {
            Some("try converting with `toNumber()` or `parseFloat()`".to_string())
        }
        ("string", "number") => {
            Some("try converting with `toString()` or string interpolation".to_string())
        }
        ("boolean", "number") => {
            Some("use a comparison like `x != 0` to convert to boolean".to_string())
        }
        ("array", other) => Some(format!("wrap the value in an array: `[{}]`", other)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suggestions_levenshtein() {
        let candidates = vec!["close", "open", "high", "low", "volume"];
        let similar = find_similar("clsoe", candidates.iter().copied(), 2);
        assert!(similar.contains(&"close"));
    }

    #[test]
    fn test_suggestions_did_you_mean() {
        // "closee" is only close to "close" (distance 1)
        let candidates = vec!["close", "momentum", "bollinger", "macdhistogram"];
        let hint = did_you_mean("closee", candidates.iter().copied());
        assert_eq!(hint, Some("did you mean `close`?".to_string()));

        // Test with no matches (all candidates too far)
        let hint2 = did_you_mean("xyz", candidates.iter().copied());
        assert_eq!(hint2, None);
    }
}
