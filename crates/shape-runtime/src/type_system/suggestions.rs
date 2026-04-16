//! Error Suggestions
//!
//! Provides "did you mean?" suggestions for type errors using Levenshtein distance
//! to find similar names when users make typos.

/// Calculate the Levenshtein edit distance between two strings.
///
/// This is the minimum number of single-character edits (insertions,
/// deletions, or substitutions) required to change one string into the other.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.chars().count();
    let b_len = b.chars().count();

    // Handle empty strings
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    // Use two rows for space efficiency
    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row = vec![0; b_len + 1];

    for (i, a_char) in a.chars().enumerate() {
        curr_row[0] = i + 1;

        for (j, b_char) in b.chars().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1) // deletion
                .min(curr_row[j] + 1) // insertion
                .min(prev_row[j] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Find similar strings from a list of candidates.
///
/// Returns candidates that are within `max_distance` edits of the target,
/// sorted by similarity (closest first).
pub fn find_similar<'a>(
    candidates: impl IntoIterator<Item = &'a str>,
    target: &str,
    max_distance: usize,
) -> Vec<&'a str> {
    let mut results: Vec<(&str, usize)> = candidates
        .into_iter()
        .filter_map(|candidate| {
            let distance = levenshtein_distance(candidate, target);
            if distance <= max_distance && distance > 0 {
                Some((candidate, distance))
            } else {
                None
            }
        })
        .collect();

    // Sort by distance (closest first)
    results.sort_by_key(|(_, d)| *d);

    // Return just the strings
    results.into_iter().map(|(s, _)| s).collect()
}

/// Calculate a reasonable max distance for suggestions based on target length.
///
/// Short names get stricter matching, long names allow more edits.
pub fn reasonable_max_distance(target: &str) -> usize {
    let len = target.len();
    if len <= 2 {
        1
    } else if len <= 5 {
        2
    } else {
        3
    }
}

/// Format a "did you mean?" suggestion.
pub fn format_suggestion(similar: &[&str]) -> Option<String> {
    match similar.len() {
        0 => None,
        1 => Some(format!("Did you mean '{}'?", similar[0])),
        2 => Some(format!(
            "Did you mean '{}' or '{}'?",
            similar[0], similar[1]
        )),
        _ => Some(format!(
            "Did you mean '{}', '{}', or '{}'?",
            similar[0], similar[1], similar[2]
        )),
    }
}

/// Suggestion for an undefined variable.
pub fn suggest_variable(
    undefined_name: &str,
    available_names: impl IntoIterator<Item = impl AsRef<str>>,
) -> Option<String> {
    let available: Vec<String> = available_names
        .into_iter()
        .map(|s| s.as_ref().to_string())
        .collect();
    let max_dist = reasonable_max_distance(undefined_name);
    let similar = find_similar(
        available.iter().map(|s| s.as_str()),
        undefined_name,
        max_dist,
    );
    format_suggestion(&similar)
}

/// Suggestion for an undefined function.
pub fn suggest_function(
    undefined_name: &str,
    available_functions: impl IntoIterator<Item = impl AsRef<str>>,
) -> Option<String> {
    let available: Vec<String> = available_functions
        .into_iter()
        .map(|s| s.as_ref().to_string())
        .collect();
    let max_dist = reasonable_max_distance(undefined_name);
    let similar = find_similar(
        available.iter().map(|s| s.as_str()),
        undefined_name,
        max_dist,
    );
    format_suggestion(&similar)
}

/// Suggestion for an unknown property.
pub fn suggest_property(
    unknown_prop: &str,
    available_props: impl IntoIterator<Item = impl AsRef<str>>,
) -> Option<String> {
    let available: Vec<String> = available_props
        .into_iter()
        .map(|s| s.as_ref().to_string())
        .collect();
    let max_dist = reasonable_max_distance(unknown_prop);
    let similar = find_similar(available.iter().map(|s| s.as_str()), unknown_prop, max_dist);
    format_suggestion(&similar)
}

#[cfg(test)]
mod tests {
    use super::*;
use shape_value::ValueWordExt;

    #[test]
    fn test_levenshtein_same_string() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_empty_strings() {
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("hello", ""), 5);
        assert_eq!(levenshtein_distance("", "world"), 5);
    }

    #[test]
    fn test_levenshtein_single_edit() {
        // Substitution
        assert_eq!(levenshtein_distance("hello", "hallo"), 1);
        // Insertion
        assert_eq!(levenshtein_distance("hello", "helloo"), 1);
        // Deletion
        assert_eq!(levenshtein_distance("hello", "helo"), 1);
    }

    #[test]
    fn test_levenshtein_multiple_edits() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("saturday", "sunday"), 3);
    }

    #[test]
    fn test_find_similar() {
        let candidates = vec!["count", "counter", "amount", "account", "mount"];
        let similar = find_similar(candidates.iter().copied(), "cont", 2);
        assert!(similar.contains(&"count"));
    }

    #[test]
    fn test_find_similar_no_matches() {
        let candidates = vec!["apple", "banana", "cherry"];
        let similar = find_similar(candidates.iter().copied(), "xyz", 2);
        assert!(similar.is_empty());
    }

    #[test]
    fn test_format_suggestion_single() {
        let similar = vec!["count"];
        assert_eq!(
            format_suggestion(&similar),
            Some("Did you mean 'count'?".to_string())
        );
    }

    #[test]
    fn test_format_suggestion_multiple() {
        let similar = vec!["count", "counter"];
        assert_eq!(
            format_suggestion(&similar),
            Some("Did you mean 'count' or 'counter'?".to_string())
        );
    }

    #[test]
    fn test_suggest_variable() {
        let available = vec!["count", "counter", "total"];
        let suggestion = suggest_variable("cont", available);
        assert!(suggestion.is_some());
        assert!(suggestion.unwrap().contains("count"));
    }

    #[test]
    fn test_reasonable_max_distance() {
        assert_eq!(reasonable_max_distance("a"), 1);
        assert_eq!(reasonable_max_distance("ab"), 1);
        assert_eq!(reasonable_max_distance("abc"), 2);
        assert_eq!(reasonable_max_distance("hello"), 2);
        assert_eq!(reasonable_max_distance("variable"), 3);
    }
}
