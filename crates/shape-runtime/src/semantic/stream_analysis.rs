//! Stream definition analysis
//!
//! This module handles analysis of stream definitions and their handlers.

use shape_ast::ast::StreamDef;
use shape_ast::error::Result;

use super::types;

/// Implementation of stream analysis methods for SemanticAnalyzer
impl super::SemanticAnalyzer {
    /// Analyze a stream definition
    pub(super) fn analyze_stream(&mut self, stream: &StreamDef) -> Result<()> {
        // Create a scope for the stream
        self.symbol_table.push_scope();

        // Analyze state variables
        for state_var in &stream.state {
            self.analyze_variable_decl(state_var)?;
        }

        // Analyze on_connect handler
        if let Some(on_connect) = &stream.on_connect {
            self.symbol_table.push_scope();
            for stmt in on_connect {
                self.analyze_statement(stmt)?;
            }
            self.symbol_table.pop_scope();
        }

        // Analyze on_disconnect handler
        if let Some(on_disconnect) = &stream.on_disconnect {
            self.symbol_table.push_scope();
            for stmt in on_disconnect {
                self.analyze_statement(stmt)?;
            }
            self.symbol_table.pop_scope();
        }

        // Analyze on_event handler
        if let Some(on_event) = &stream.on_event {
            self.symbol_table.push_scope();
            // Register the event parameter
            self.symbol_table.define_variable(
                &on_event.event_param,
                types::Type::Unknown, // Event type
                shape_ast::ast::VarKind::Const,
                true,
            )?;
            for stmt in &on_event.body {
                self.analyze_statement(stmt)?;
            }
            self.symbol_table.pop_scope();
        }

        // Analyze on_window handler
        if let Some(on_window) = &stream.on_window {
            self.symbol_table.push_scope();
            // Register the key and window parameters
            self.symbol_table.define_variable(
                &on_window.key_param,
                types::Type::String,
                shape_ast::ast::VarKind::Const,
                true,
            )?;
            self.symbol_table.define_variable(
                &on_window.window_param,
                types::Type::Object(vec![]),
                shape_ast::ast::VarKind::Const,
                true,
            )?;
            for stmt in &on_window.body {
                self.analyze_statement(stmt)?;
            }
            self.symbol_table.pop_scope();
        }

        // Analyze on_error handler
        if let Some(on_error) = &stream.on_error {
            self.symbol_table.push_scope();
            // Register the error parameter
            self.symbol_table.define_variable(
                &on_error.error_param,
                types::Type::String, // Error type
                shape_ast::ast::VarKind::Const,
                true,
            )?;
            for stmt in &on_error.body {
                self.analyze_statement(stmt)?;
            }
            self.symbol_table.pop_scope();
        }

        self.symbol_table.pop_scope();

        // Register the stream name
        self.symbol_table.define_variable(
            &stream.name,
            types::Type::Unknown, // Stream type
            shape_ast::ast::VarKind::Const,
            true,
        )?;

        Ok(())
    }
}
