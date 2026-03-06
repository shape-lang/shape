//! TUI UI components for the Shape REPL

mod status_bar;

pub use status_bar::{ProgressDisplay, StatusBar};

use ratatui::prelude::*;

/// Common UI styles
pub struct Styles;

impl Styles {
    /// Style for keywords in syntax highlighting
    pub fn keyword() -> Style {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for strings
    pub fn string() -> Style {
        Style::default().fg(Color::Green)
    }

    /// Style for numbers
    pub fn number() -> Style {
        Style::default().fg(Color::Yellow)
    }

    /// Style for comments
    pub fn comment() -> Style {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC)
    }

    /// Style for operators
    pub fn operator() -> Style {
        Style::default().fg(Color::Magenta)
    }

    /// Style for function names
    pub fn function() -> Style {
        Style::default().fg(Color::Blue)
    }

    /// Style for types
    pub fn type_name() -> Style {
        Style::default().fg(Color::LightCyan)
    }

    /// Style for errors
    pub fn error() -> Style {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    }

    /// Style for success messages
    pub fn success() -> Style {
        Style::default().fg(Color::Green)
    }

    /// Style for muted/secondary text
    pub fn muted() -> Style {
        Style::default().fg(Color::DarkGray)
    }

    /// Style for selected items
    pub fn selected() -> Style {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    }

    /// Style for the prompt
    pub fn prompt() -> Style {
        Style::default().fg(Color::Cyan)
    }
}
