//! Alert Types
//!
//! Core types for the alert system.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Alert severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum AlertSeverity {
    /// Debug-level alert (lowest priority)
    Debug,
    /// Informational alert
    #[default]
    Info,
    /// Warning alert
    Warning,
    /// Error alert
    Error,
    /// Critical alert (highest priority)
    Critical,
}

impl std::fmt::Display for AlertSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertSeverity::Debug => write!(f, "DEBUG"),
            AlertSeverity::Info => write!(f, "INFO"),
            AlertSeverity::Warning => write!(f, "WARNING"),
            AlertSeverity::Error => write!(f, "ERROR"),
            AlertSeverity::Critical => write!(f, "CRITICAL"),
        }
    }
}

impl AlertSeverity {
    /// Parse severity from string
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "debug" => AlertSeverity::Debug,
            "info" => AlertSeverity::Info,
            "warning" | "warn" => AlertSeverity::Warning,
            "error" | "err" => AlertSeverity::Error,
            "critical" | "crit" => AlertSeverity::Critical,
            _ => AlertSeverity::Info,
        }
    }

    /// Get numeric priority (higher = more severe)
    pub fn priority(&self) -> u8 {
        match self {
            AlertSeverity::Debug => 0,
            AlertSeverity::Info => 1,
            AlertSeverity::Warning => 2,
            AlertSeverity::Error => 3,
            AlertSeverity::Critical => 4,
        }
    }
}

/// An alert to be sent through the pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    /// Unique identifier for this alert
    pub id: Uuid,

    /// Alert severity level
    pub severity: AlertSeverity,

    /// Alert title/subject (brief description)
    pub title: String,

    /// Detailed message
    pub message: String,

    /// Structured data associated with the alert
    #[serde(default)]
    pub data: HashMap<String, serde_json::Value>,

    /// Tags for routing (e.g., ["price", "btc", "urgent"])
    #[serde(default)]
    pub tags: Vec<String>,

    /// Timestamp when the alert was created
    pub timestamp: DateTime<Utc>,

    /// Source of the alert (e.g., script name, strategy name)
    #[serde(default)]
    pub source: Option<String>,
}

impl Alert {
    /// Create a new alert with the given title and message
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            severity: AlertSeverity::Info,
            title: title.into(),
            message: message.into(),
            data: HashMap::new(),
            tags: Vec::new(),
            timestamp: Utc::now(),
            source: None,
        }
    }

    /// Set the severity level
    pub fn with_severity(mut self, severity: AlertSeverity) -> Self {
        self.severity = severity;
        self
    }

    /// Add a tag
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Add multiple tags
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.tags.extend(tags.into_iter().map(|t| t.into()));
        self
    }

    /// Add structured data
    pub fn with_data(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.data.insert(key.into(), value);
        self
    }

    /// Set the source
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Check if alert has a specific tag
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Check if alert has any of the specified tags
    pub fn has_any_tag(&self, tags: &[String]) -> bool {
        tags.iter().any(|t| self.has_tag(t))
    }
}

impl Default for Alert {
    fn default() -> Self {
        Self::new("", "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_creation() {
        let alert = Alert::new("Test Alert", "This is a test message")
            .with_severity(AlertSeverity::Warning)
            .with_tag("test")
            .with_data("key", serde_json::json!("value"));

        assert_eq!(alert.title, "Test Alert");
        assert_eq!(alert.message, "This is a test message");
        assert_eq!(alert.severity, AlertSeverity::Warning);
        assert!(alert.has_tag("test"));
        assert_eq!(alert.data.get("key"), Some(&serde_json::json!("value")));
    }

    #[test]
    fn test_severity_priority() {
        assert!(AlertSeverity::Critical.priority() > AlertSeverity::Error.priority());
        assert!(AlertSeverity::Error.priority() > AlertSeverity::Warning.priority());
        assert!(AlertSeverity::Warning.priority() > AlertSeverity::Info.priority());
        assert!(AlertSeverity::Info.priority() > AlertSeverity::Debug.priority());
    }

    #[test]
    fn test_severity_from_str() {
        assert_eq!(AlertSeverity::from_str("debug"), AlertSeverity::Debug);
        assert_eq!(AlertSeverity::from_str("WARNING"), AlertSeverity::Warning);
        assert_eq!(AlertSeverity::from_str("crit"), AlertSeverity::Critical);
        assert_eq!(AlertSeverity::from_str("unknown"), AlertSeverity::Info);
    }
}
