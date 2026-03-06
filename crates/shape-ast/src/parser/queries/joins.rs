//! JOIN clause parser
//!
//! Parses JOIN clauses for ANALYZE queries:
//! - `INNER JOIN source ON condition`
//! - `LEFT JOIN source USING (col1, col2)`
//! - `JOIN source WITHIN 100ms` (temporal join)

use crate::ast::{JoinClause, JoinCondition, JoinSource, JoinType};
use crate::data::Timeframe;
use crate::error::{Result, ShapeError};
use crate::parser::{Rule, expressions, pair_location};
use pest::iterators::Pair;

/// Parse a JOIN clause
///
/// Grammar: `join_type? "join" join_source join_condition?`
pub fn parse_join_clause(pair: Pair<Rule>) -> Result<JoinClause> {
    let pair_loc = pair_location(&pair);
    let mut join_type = JoinType::Inner; // Default
    let mut join_source = None;
    let mut join_condition = JoinCondition::Natural; // Default for cross joins

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::join_type => {
                join_type = parse_join_type(inner)?;
            }
            Rule::join_source => {
                join_source = Some(parse_join_source(inner)?);
            }
            Rule::join_condition => {
                join_condition = parse_join_condition(inner)?;
            }
            _ => {}
        }
    }

    let right = join_source.ok_or_else(|| ShapeError::ParseError {
        message: "JOIN clause requires a source (table/symbol name or subquery)".to_string(),
        location: Some(
            pair_loc.with_hint("example: JOIN quotes ON trades.timestamp = quotes.timestamp"),
        ),
    })?;

    // Cross joins don't require a condition
    if matches!(join_type, JoinType::Cross) {
        return Ok(JoinClause {
            join_type,
            right,
            condition: JoinCondition::Natural,
        });
    }

    Ok(JoinClause {
        join_type,
        right,
        condition: join_condition,
    })
}

/// Parse JOIN type
///
/// Grammar: `"inner" | "left" "outer"? | "right" "outer"? | "full" "outer"? | "cross"`
fn parse_join_type(pair: Pair<Rule>) -> Result<JoinType> {
    let text = pair.as_str().to_lowercase();

    if text.starts_with("inner") {
        Ok(JoinType::Inner)
    } else if text.starts_with("left") {
        Ok(JoinType::Left)
    } else if text.starts_with("right") {
        Ok(JoinType::Right)
    } else if text.starts_with("full") {
        Ok(JoinType::Full)
    } else if text.starts_with("cross") {
        Ok(JoinType::Cross)
    } else {
        // Default to inner
        Ok(JoinType::Inner)
    }
}

/// Parse JOIN source
///
/// Grammar: `ident ("as" ident)? | "(" inner_query ")" ("as" ident)?`
pub fn parse_join_source(pair: Pair<Rule>) -> Result<JoinSource> {
    let pair_loc = pair_location(&pair);
    let mut inner_iter = pair.into_inner();

    let first = inner_iter.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected join source".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    match first.as_rule() {
        Rule::ident => {
            // Named source (optionally with alias)
            let name = first.as_str().to_string();
            // For now, we just use the name (aliases would require extending JoinSource)
            Ok(JoinSource::Named(name))
        }
        Rule::inner_query => {
            // Subquery
            let query = super::parse_inner_query(first)?;
            Ok(JoinSource::Subquery(Box::new(query)))
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected join source type: {:?}", first.as_rule()),
            location: Some(pair_location(&first)),
        }),
    }
}

/// Parse JOIN condition
///
/// Grammar: `"on" expression | "using" "(" ident ("," ident)* ")" | "within" duration`
fn parse_join_condition(pair: Pair<Rule>) -> Result<JoinCondition> {
    let pair_loc = pair_location(&pair);
    let mut inner_iter = pair.into_inner();

    let first = inner_iter.next().ok_or_else(|| ShapeError::ParseError {
        message: "expected join condition".to_string(),
        location: Some(pair_loc.clone()),
    })?;

    match first.as_rule() {
        Rule::expression => {
            // ON condition
            let expr = expressions::parse_expression(first)?;
            Ok(JoinCondition::On(expr))
        }
        Rule::ident => {
            // USING clause - first identifier already parsed
            let mut columns = vec![first.as_str().to_string()];
            for col in inner_iter {
                if col.as_rule() == Rule::ident {
                    columns.push(col.as_str().to_string());
                }
            }
            Ok(JoinCondition::Using(columns))
        }
        Rule::duration => {
            // WITHIN clause for temporal join
            let timeframe = parse_duration_as_timeframe(first)?;
            Ok(JoinCondition::Temporal {
                left_time: "timestamp".to_string(),
                right_time: "timestamp".to_string(),
                within: timeframe,
            })
        }
        _ => Err(ShapeError::ParseError {
            message: format!("unexpected join condition type: {:?}", first.as_rule()),
            location: Some(pair_location(&first)),
        }),
    }
}

/// Parse duration to Timeframe for temporal joins
fn parse_duration_as_timeframe(pair: Pair<Rule>) -> Result<Timeframe> {
    use crate::data::TimeframeUnit;

    let text = pair.as_str().to_lowercase();
    let pair_loc = pair_location(&pair);

    // Parse duration like "100ms", "1s", "5m", etc.
    let (num_str, unit_str) = extract_duration_parts(&text);

    let value = num_str.parse::<u32>().map_err(|_| ShapeError::ParseError {
        message: format!("invalid duration value: '{}'", num_str),
        location: Some(pair_loc.clone()),
    })?;

    let unit = match unit_str {
        "s" | "seconds" => TimeframeUnit::Second,
        "m" | "minutes" => TimeframeUnit::Minute,
        "h" | "hours" => TimeframeUnit::Hour,
        "d" | "days" => TimeframeUnit::Day,
        "w" | "weeks" => TimeframeUnit::Week,
        "ms" => {
            // Convert milliseconds to seconds with fractional handling
            // For simplicity, treat ms as 1 second minimum
            return Ok(Timeframe::new(1, TimeframeUnit::Second));
        }
        _ => {
            return Err(ShapeError::ParseError {
                message: format!("unknown duration unit: '{}'", unit_str),
                location: Some(pair_loc.with_hint("valid units: s, m, h, d, w, ms")),
            });
        }
    };

    Ok(Timeframe::new(value, unit))
}

/// Extract numeric and unit parts from duration string
fn extract_duration_parts(s: &str) -> (&str, &str) {
    let idx = s
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(s.len());
    (&s[..idx], &s[idx..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pest::Parser;

    fn parse_join(input: &str) -> Result<JoinClause> {
        let pairs = crate::parser::ShapeParser::parse(Rule::join_clause, input).map_err(|e| {
            ShapeError::ParseError {
                message: format!("parse error: {}", e),
                location: None,
            }
        })?;
        let pair = pairs.into_iter().next().unwrap();
        parse_join_clause(pair)
    }

    #[test]
    fn test_inner_join_on() {
        let result = parse_join("join quotes on trades.id = quotes.id");
        assert!(result.is_ok());
        let join = result.unwrap();
        assert!(matches!(join.join_type, JoinType::Inner));
        assert!(matches!(join.condition, JoinCondition::On(_)));
    }

    #[test]
    fn test_left_join_using() {
        let result = parse_join("left join orders using (symbol, timestamp)");
        assert!(result.is_ok());
        let join = result.unwrap();
        assert!(matches!(join.join_type, JoinType::Left));
        if let JoinCondition::Using(cols) = &join.condition {
            assert_eq!(cols.len(), 2);
            assert_eq!(cols[0], "symbol");
            assert_eq!(cols[1], "timestamp");
        } else {
            panic!("Expected Using condition");
        }
    }

    #[test]
    fn test_temporal_join() {
        let result = parse_join("join executions within 100s");
        assert!(result.is_ok());
        let join = result.unwrap();
        assert!(matches!(join.condition, JoinCondition::Temporal { .. }));
    }
}
