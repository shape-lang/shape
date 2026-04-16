//! Generic pattern state machine for sequence detection
//!
//! Provides pattern matching over event streams:
//! - Sequential pattern matching
//! - Temporal constraints (WITHIN)
//! - Logical operators (AND, OR, NOT, FOLLOWED_BY)
//! - State-based pattern tracking
//!
//! This module is industry-agnostic and works with any events.

use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

use shape_ast::error::Result;
use shape_value::{ValueWord, ValueWordExt};

/// A condition for pattern matching
#[derive(Debug, Clone)]
pub struct PatternCondition {
    /// Unique name for this condition
    pub name: String,
    /// Field to evaluate
    pub field: String,
    /// Comparison operator
    pub operator: ComparisonOp,
    /// Value to compare against
    pub value: ValueWord,
}

/// Comparison operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Contains,
    StartsWith,
    EndsWith,
}

impl PatternCondition {
    /// Create a new pattern condition
    pub fn new(name: &str, field: &str, operator: ComparisonOp, value: ValueWord) -> Self {
        Self {
            name: name.to_string(),
            field: field.to_string(),
            operator,
            value,
        }
    }

    /// Evaluate this condition against event fields
    pub fn evaluate(&self, fields: &HashMap<String, ValueWord>) -> bool {
        let Some(field_value) = fields.get(&self.field) else {
            return false;
        };

        match self.operator {
            // Numeric comparisons
            ComparisonOp::Eq
            | ComparisonOp::Ne
            | ComparisonOp::Gt
            | ComparisonOp::Ge
            | ComparisonOp::Lt
            | ComparisonOp::Le => {
                let a_numeric = field_value.is_f64() || field_value.is_i64();
                let b_numeric = self.value.is_f64() || self.value.is_i64();
                if a_numeric && b_numeric {
                    if let (Some(a), Some(b)) = (field_value.as_f64(), self.value.as_f64()) {
                        match self.operator {
                            ComparisonOp::Eq => (a - b).abs() < f64::EPSILON,
                            ComparisonOp::Ne => (a - b).abs() >= f64::EPSILON,
                            ComparisonOp::Gt => a > b,
                            ComparisonOp::Ge => a >= b,
                            ComparisonOp::Lt => a < b,
                            ComparisonOp::Le => a <= b,
                            _ => false,
                        }
                    } else {
                        false
                    }
                } else if field_value.is_heap() && self.value.is_heap() {
                    if let (Some(a), Some(b)) = (field_value.as_str(), self.value.as_str()) {
                        match self.operator {
                            ComparisonOp::Eq => a == b,
                            ComparisonOp::Ne => a != b,
                            _ => false,
                        }
                    } else {
                        false
                    }
                } else if field_value.is_bool() && self.value.is_bool() {
                    match self.operator {
                        ComparisonOp::Eq => field_value.as_bool() == self.value.as_bool(),
                        ComparisonOp::Ne => field_value.as_bool() != self.value.as_bool(),
                        _ => false,
                    }
                } else {
                    false
                }
            }
            // String-specific operations
            ComparisonOp::Contains => {
                if let (Some(a), Some(b)) = (field_value.as_str(), self.value.as_str()) {
                    a.contains(b)
                } else {
                    false
                }
            }
            ComparisonOp::StartsWith => {
                if let (Some(a), Some(b)) = (field_value.as_str(), self.value.as_str()) {
                    a.starts_with(b)
                } else {
                    false
                }
            }
            ComparisonOp::EndsWith => {
                if let (Some(a), Some(b)) = (field_value.as_str(), self.value.as_str()) {
                    a.ends_with(b)
                } else {
                    false
                }
            }
        }
    }
}

/// Pattern sequence operators
#[derive(Debug, Clone)]
pub enum PatternSequence {
    /// Single condition
    Condition(PatternCondition),
    /// Sequence of patterns (must occur in order)
    Seq(Vec<PatternSequence>),
    /// Pattern must complete within duration
    Within(Box<PatternSequence>, Duration),
    /// One pattern followed by another
    FollowedBy(Box<PatternSequence>, Box<PatternSequence>),
    /// Pattern must NOT occur
    Not(Box<PatternSequence>),
    /// Any of these patterns
    Or(Vec<PatternSequence>),
    /// All of these patterns (any order)
    And(Vec<PatternSequence>),
    /// Pattern repeated N times
    Repeat(Box<PatternSequence>, usize),
}

impl PatternSequence {
    /// Create a single condition pattern
    pub fn condition(name: &str, field: &str, op: ComparisonOp, value: ValueWord) -> Self {
        PatternSequence::Condition(PatternCondition::new(name, field, op, value))
    }

    /// Create a sequence of patterns
    pub fn seq(patterns: Vec<PatternSequence>) -> Self {
        PatternSequence::Seq(patterns)
    }

    /// Add a time constraint
    pub fn within(self, duration: Duration) -> Self {
        PatternSequence::Within(Box::new(self), duration)
    }

    /// Create a followed-by pattern
    pub fn followed_by(self, next: PatternSequence) -> Self {
        PatternSequence::FollowedBy(Box::new(self), Box::new(next))
    }

    /// Negate a pattern
    pub fn not(self) -> Self {
        PatternSequence::Not(Box::new(self))
    }

    /// Create an OR of patterns
    pub fn or(patterns: Vec<PatternSequence>) -> Self {
        PatternSequence::Or(patterns)
    }

    /// Create an AND of patterns
    pub fn and(patterns: Vec<PatternSequence>) -> Self {
        PatternSequence::And(patterns)
    }

    /// Repeat pattern N times
    pub fn repeat(self, times: usize) -> Self {
        PatternSequence::Repeat(Box::new(self), times)
    }
}

/// State of a pattern match in progress
#[derive(Debug, Clone)]
struct MatchState {
    /// Pattern being matched
    pattern_id: usize,
    /// Current position in sequence
    position: usize,
    /// When matching started
    start_time: DateTime<Utc>,
    /// Deadline for WITHIN constraints
    deadline: Option<DateTime<Utc>>,
    /// Events matched so far
    matched_events: Vec<MatchedEvent>,
}

/// A matched event in a pattern
#[derive(Debug, Clone)]
pub struct MatchedEvent {
    pub timestamp: DateTime<Utc>,
    pub condition_name: String,
    pub fields: HashMap<String, ValueWord>,
}

/// A completed pattern match
#[derive(Debug, Clone)]
pub struct PatternMatch {
    /// Pattern name
    pub pattern_name: String,
    /// When the match started
    pub start_time: DateTime<Utc>,
    /// When the match completed
    pub end_time: DateTime<Utc>,
    /// Events that made up the match
    pub events: Vec<MatchedEvent>,
}

/// Pattern definition with name
#[derive(Debug, Clone)]
pub struct PatternDef {
    pub name: String,
    pub sequence: PatternSequence,
}

/// Generic pattern state machine for event sequence detection
pub struct PatternStateMachine {
    /// Registered patterns
    patterns: Vec<PatternDef>,
    /// Active match states
    active_states: Vec<MatchState>,
    /// Completed matches
    completed_matches: Vec<PatternMatch>,
}

impl Default for PatternStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl PatternStateMachine {
    /// Create a new pattern state machine
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
            active_states: Vec::new(),
            completed_matches: Vec::new(),
        }
    }

    /// Register a pattern
    pub fn register(&mut self, name: &str, sequence: PatternSequence) -> &mut Self {
        self.patterns.push(PatternDef {
            name: name.to_string(),
            sequence,
        });
        self
    }

    /// Process an event
    pub fn process(
        &mut self,
        timestamp: DateTime<Utc>,
        fields: HashMap<String, ValueWord>,
    ) -> Result<()> {
        // Remove expired states
        self.active_states
            .retain(|state| state.deadline.map(|d| timestamp <= d).unwrap_or(true));

        // Try to advance existing states
        let mut new_states = Vec::new();
        let mut completed = Vec::new();

        for state in &self.active_states {
            if let Some((new_state, is_complete)) = self.advance_state(state, timestamp, &fields)? {
                if is_complete {
                    // Pattern completed
                    let pattern = &self.patterns[state.pattern_id];
                    completed.push(PatternMatch {
                        pattern_name: pattern.name.clone(),
                        start_time: state.start_time,
                        end_time: timestamp,
                        events: new_state.matched_events,
                    });
                } else {
                    new_states.push(new_state);
                }
            }
        }

        // Try to start new pattern matches
        for (pattern_id, pattern) in self.patterns.iter().enumerate() {
            if let Some(state) =
                self.try_start_match(pattern_id, &pattern.sequence, timestamp, &fields)?
            {
                // Check if it's already complete (single condition pattern)
                if self.is_pattern_complete(&pattern.sequence, &state) {
                    completed.push(PatternMatch {
                        pattern_name: pattern.name.clone(),
                        start_time: timestamp,
                        end_time: timestamp,
                        events: state.matched_events,
                    });
                } else {
                    new_states.push(state);
                }
            }
        }

        // Update states
        self.active_states = new_states;
        self.completed_matches.extend(completed);

        Ok(())
    }

    /// Try to start a new pattern match
    fn try_start_match(
        &self,
        pattern_id: usize,
        sequence: &PatternSequence,
        timestamp: DateTime<Utc>,
        fields: &HashMap<String, ValueWord>,
    ) -> Result<Option<MatchState>> {
        match sequence {
            PatternSequence::Condition(cond) => {
                if cond.evaluate(fields) {
                    Ok(Some(MatchState {
                        pattern_id,
                        position: 1, // Completed first (and only) condition
                        start_time: timestamp,
                        deadline: None,
                        matched_events: vec![MatchedEvent {
                            timestamp,
                            condition_name: cond.name.clone(),
                            fields: fields.clone(),
                        }],
                    }))
                } else {
                    Ok(None)
                }
            }
            PatternSequence::Seq(patterns) if !patterns.is_empty() => {
                // Try to match first pattern in sequence
                self.try_start_match(pattern_id, &patterns[0], timestamp, fields)
            }
            PatternSequence::Within(inner, duration) => {
                if let Some(mut state) =
                    self.try_start_match(pattern_id, inner, timestamp, fields)?
                {
                    state.deadline = Some(timestamp + *duration);
                    Ok(Some(state))
                } else {
                    Ok(None)
                }
            }
            PatternSequence::Or(patterns) => {
                for pattern in patterns {
                    if let Some(state) =
                        self.try_start_match(pattern_id, pattern, timestamp, fields)?
                    {
                        return Ok(Some(state));
                    }
                }
                Ok(None)
            }
            PatternSequence::And(patterns) => {
                // For AND, all conditions must match the same event
                let mut all_matched = true;
                let mut matched_events = Vec::new();

                for pattern in patterns {
                    if let Some(state) =
                        self.try_start_match(pattern_id, pattern, timestamp, fields)?
                    {
                        matched_events.extend(state.matched_events);
                    } else {
                        all_matched = false;
                        break;
                    }
                }

                if all_matched && !matched_events.is_empty() {
                    Ok(Some(MatchState {
                        pattern_id,
                        position: 1,
                        start_time: timestamp,
                        deadline: None,
                        matched_events,
                    }))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Advance an existing match state
    fn advance_state(
        &self,
        state: &MatchState,
        timestamp: DateTime<Utc>,
        fields: &HashMap<String, ValueWord>,
    ) -> Result<Option<(MatchState, bool)>> {
        let pattern = &self.patterns[state.pattern_id];

        match &pattern.sequence {
            PatternSequence::Seq(patterns) => {
                if state.position < patterns.len() {
                    // Try to match next pattern in sequence
                    if let PatternSequence::Condition(cond) = &patterns[state.position] {
                        if cond.evaluate(fields) {
                            let mut new_state = state.clone();
                            new_state.position += 1;
                            new_state.matched_events.push(MatchedEvent {
                                timestamp,
                                condition_name: cond.name.clone(),
                                fields: fields.clone(),
                            });

                            let is_complete = new_state.position >= patterns.len();
                            return Ok(Some((new_state, is_complete)));
                        }
                    }
                }
            }
            PatternSequence::FollowedBy(_, second) => {
                // If we're past the first pattern, try matching the second
                if state.position == 1 {
                    if let PatternSequence::Condition(cond) = second.as_ref() {
                        if cond.evaluate(fields) {
                            let mut new_state = state.clone();
                            new_state.position = 2;
                            new_state.matched_events.push(MatchedEvent {
                                timestamp,
                                condition_name: cond.name.clone(),
                                fields: fields.clone(),
                            });
                            return Ok(Some((new_state, true)));
                        }
                    }
                }
            }
            PatternSequence::Repeat(inner, times) => {
                if state.position < *times {
                    if let Some(new_inner_state) =
                        self.try_start_match(state.pattern_id, inner, timestamp, fields)?
                    {
                        let mut new_state = state.clone();
                        new_state.position += 1;
                        new_state
                            .matched_events
                            .extend(new_inner_state.matched_events);

                        let is_complete = new_state.position >= *times;
                        return Ok(Some((new_state, is_complete)));
                    }
                }
            }
            _ => {}
        }

        // No advancement, keep current state
        Ok(Some((state.clone(), false)))
    }

    /// Check if a pattern is complete
    fn is_pattern_complete(&self, sequence: &PatternSequence, state: &MatchState) -> bool {
        match sequence {
            PatternSequence::Condition(_) => state.position >= 1,
            PatternSequence::Seq(patterns) => state.position >= patterns.len(),
            PatternSequence::Within(inner, _) => self.is_pattern_complete(inner, state),
            PatternSequence::Repeat(_, times) => state.position >= *times,
            PatternSequence::And(_) | PatternSequence::Or(_) => state.position >= 1,
            _ => false,
        }
    }

    /// Take completed matches
    pub fn take_matches(&mut self) -> Vec<PatternMatch> {
        std::mem::take(&mut self.completed_matches)
    }

    /// Get count of active match states
    pub fn active_count(&self) -> usize {
        self.active_states.len()
    }

    /// Reset all state
    pub fn reset(&mut self) {
        self.active_states.clear();
        self.completed_matches.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_event(value: f64, status: &str) -> HashMap<String, ValueWord> {
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), ValueWord::from_f64(value));
        fields.insert(
            "status".to_string(),
            ValueWord::from_string(Arc::new(status.to_string())),
        );
        fields
    }

    #[test]
    fn test_single_condition() {
        let mut psm = PatternStateMachine::new();

        psm.register(
            "high_value",
            PatternSequence::condition(
                "high",
                "value",
                ComparisonOp::Gt,
                ValueWord::from_f64(100.0),
            ),
        );

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();

        // Should not match
        psm.process(base, make_event(50.0, "ok")).unwrap();
        assert!(psm.take_matches().is_empty());

        // Should match
        psm.process(base + Duration::seconds(1), make_event(150.0, "ok"))
            .unwrap();
        let matches = psm.take_matches();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].pattern_name, "high_value");
    }

    #[test]
    fn test_sequence_pattern() {
        let mut psm = PatternStateMachine::new();

        // Pattern: value goes from low to high
        psm.register(
            "spike",
            PatternSequence::seq(vec![
                PatternSequence::condition(
                    "low",
                    "value",
                    ComparisonOp::Lt,
                    ValueWord::from_f64(50.0),
                ),
                PatternSequence::condition(
                    "high",
                    "value",
                    ComparisonOp::Gt,
                    ValueWord::from_f64(150.0),
                ),
            ]),
        );

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();

        // Start with low value
        psm.process(base, make_event(30.0, "ok")).unwrap();
        assert!(psm.take_matches().is_empty());
        assert_eq!(psm.active_count(), 1); // Active state waiting for high

        // High value completes the pattern
        psm.process(base + Duration::seconds(1), make_event(200.0, "ok"))
            .unwrap();
        let matches = psm.take_matches();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].events.len(), 2);
    }

    #[test]
    fn test_within_constraint() {
        let mut psm = PatternStateMachine::new();

        // Pattern must complete within 5 seconds
        psm.register(
            "fast_spike",
            PatternSequence::seq(vec![
                PatternSequence::condition(
                    "low",
                    "value",
                    ComparisonOp::Lt,
                    ValueWord::from_f64(50.0),
                ),
                PatternSequence::condition(
                    "high",
                    "value",
                    ComparisonOp::Gt,
                    ValueWord::from_f64(150.0),
                ),
            ])
            .within(Duration::seconds(5)),
        );

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();

        // Start with low value
        psm.process(base, make_event(30.0, "ok")).unwrap();
        assert_eq!(psm.active_count(), 1);

        // High value comes too late (10 seconds later)
        psm.process(base + Duration::seconds(10), make_event(200.0, "ok"))
            .unwrap();

        // State should have expired
        let matches = psm.take_matches();
        assert!(matches.is_empty());
    }

    #[test]
    fn test_or_pattern() {
        let mut psm = PatternStateMachine::new();

        // Pattern: either high value OR status is "alert"
        psm.register(
            "alert_condition",
            PatternSequence::or(vec![
                PatternSequence::condition(
                    "high_val",
                    "value",
                    ComparisonOp::Gt,
                    ValueWord::from_f64(100.0),
                ),
                PatternSequence::condition(
                    "alert_status",
                    "status",
                    ComparisonOp::Eq,
                    ValueWord::from_string(Arc::new("alert".to_string())),
                ),
            ]),
        );

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();

        // Match via value
        psm.process(base, make_event(150.0, "ok")).unwrap();
        assert_eq!(psm.take_matches().len(), 1);

        // Match via status
        psm.process(base + Duration::seconds(1), make_event(50.0, "alert"))
            .unwrap();
        assert_eq!(psm.take_matches().len(), 1);
    }

    #[test]
    fn test_string_conditions() {
        let mut psm = PatternStateMachine::new();

        psm.register(
            "status_check",
            PatternSequence::condition(
                "starts_err",
                "status",
                ComparisonOp::StartsWith,
                ValueWord::from_string(Arc::new("err".to_string())),
            ),
        );

        let base = DateTime::from_timestamp(1000000000, 0).unwrap();

        // Should not match
        psm.process(base, make_event(0.0, "ok")).unwrap();
        assert!(psm.take_matches().is_empty());

        // Should match
        let mut fields = HashMap::new();
        fields.insert("value".to_string(), ValueWord::from_f64(0.0));
        fields.insert(
            "status".to_string(),
            ValueWord::from_string(Arc::new("error: connection failed".to_string())),
        );
        psm.process(base + Duration::seconds(1), fields).unwrap();
        assert_eq!(psm.take_matches().len(), 1);
    }
}
