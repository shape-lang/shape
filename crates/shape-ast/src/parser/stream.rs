//! Parser for stream definitions

use crate::error::{Result, ShapeError};
use pest::iterators::Pair;

use crate::ast::{
    Span, Statement, StreamConfig, StreamDef, StreamOnError, StreamOnEvent, StreamOnWindow,
    Timeframe, VariableDecl,
};
use crate::parser::string_literals::parse_string_literal;
use crate::parser::{Rule, pair_span, parse_variable_decl, statements};

/// Parse a stream definition
pub fn parse_stream_def(pair: Pair<Rule>) -> Result<StreamDef> {
    let mut name = String::new();
    let mut name_span = Span::DUMMY;
    let mut config = StreamConfig {
        provider: String::new(),
        symbols: vec![],
        timeframes: vec![],
        buffer_size: None,
        reconnect: None,
        reconnect_delay: None,
    };
    let mut state = vec![];
    let mut on_connect = None;
    let mut on_disconnect = None;
    let mut on_event = None;
    let mut on_window = None;
    let mut on_error = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => {
                if name.is_empty() {
                    name = inner.as_str().to_string();
                    name_span = pair_span(&inner);
                }
            }
            Rule::stream_body => {
                for body_item in inner.into_inner() {
                    match body_item.as_rule() {
                        Rule::stream_config => {
                            config = parse_stream_config(body_item)?;
                        }
                        Rule::stream_state => {
                            state = parse_stream_state(body_item)?;
                        }
                        Rule::stream_on_connect => {
                            on_connect = Some(parse_stream_on_connect(body_item)?);
                        }
                        Rule::stream_on_disconnect => {
                            on_disconnect = Some(parse_stream_on_disconnect(body_item)?);
                        }
                        Rule::stream_on_event => {
                            on_event = Some(parse_stream_on_event(body_item)?);
                        }
                        Rule::stream_on_window => {
                            on_window = Some(parse_stream_on_window(body_item)?);
                        }
                        Rule::stream_on_error => {
                            on_error = Some(parse_stream_on_error(body_item)?);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    Ok(StreamDef {
        name,
        name_span,
        config,
        state,
        on_connect,
        on_disconnect,
        on_event,
        on_window,
        on_error,
    })
}

/// Parse stream configuration
fn parse_stream_config(pair: Pair<Rule>) -> Result<StreamConfig> {
    let mut config = StreamConfig {
        provider: String::new(),
        symbols: vec![],
        timeframes: vec![],
        buffer_size: None,
        reconnect: None,
        reconnect_delay: None,
    };

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::stream_config_list {
            for config_item in inner.into_inner() {
                if config_item.as_rule() == Rule::stream_config_item {
                    // The stream_config_item already contains the key:value structure
                    // We need to analyze its string content to determine which config it is
                    let config_str = config_item.as_str();
                    let inner_pairs = config_item.into_inner();

                    // Check which config item this is by looking at the string content
                    if config_str.starts_with("provider") {
                        // Skip to the string value
                        for pair in inner_pairs {
                            if pair.as_rule() == Rule::string {
                                config.provider = parse_string_literal(pair.as_str())?;
                            }
                        }
                    } else if config_str.starts_with("symbols") {
                        // Find the symbol_list
                        for pair in inner_pairs {
                            if pair.as_rule() == Rule::symbol_list {
                                config.symbols = parse_symbol_list(pair)?;
                            }
                        }
                    } else if config_str.starts_with("timeframes") {
                        // Collect timeframes
                        for pair in inner_pairs {
                            if pair.as_rule() == Rule::timeframe {
                                if let Some(tf) = Timeframe::parse(pair.as_str()) {
                                    config.timeframes.push(tf);
                                }
                            }
                        }
                    } else if config_str.starts_with("buffer_size") {
                        // Find the integer value
                        for pair in inner_pairs {
                            if pair.as_rule() == Rule::integer {
                                config.buffer_size = Some(pair.as_str().parse().map_err(|e| {
                                    ShapeError::ParseError {
                                        message: format!("Invalid buffer_size: {}", e),
                                        location: None,
                                    }
                                })?);
                            }
                        }
                    } else if config_str.starts_with("reconnect:")
                        && !config_str.starts_with("reconnect_delay")
                    {
                        // Find the boolean value
                        for pair in inner_pairs {
                            if pair.as_rule() == Rule::boolean {
                                config.reconnect = Some(pair.as_str() == "true");
                            }
                        }
                    } else if config_str.starts_with("reconnect_delay") {
                        // Find the number value
                        for pair in inner_pairs {
                            if pair.as_rule() == Rule::number {
                                config.reconnect_delay =
                                    Some(pair.as_str().parse().map_err(|e| {
                                        ShapeError::ParseError {
                                            message: format!("Invalid reconnect_delay: {}", e),
                                            location: None,
                                        }
                                    })?);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(config)
}

/// Parse a list of symbols
fn parse_symbol_list(pair: Pair<Rule>) -> Result<Vec<String>> {
    let mut symbols = vec![];

    if pair.as_rule() == Rule::symbol_list {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::ident {
                symbols.push(inner.as_str().to_string());
            }
        }
    }

    Ok(symbols)
}

/// Parse stream state declarations
fn parse_stream_state(pair: Pair<Rule>) -> Result<Vec<VariableDecl>> {
    let mut state = vec![];

    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::stream_state_list {
            for decl in inner.into_inner() {
                if decl.as_rule() == Rule::variable_decl {
                    state.push(parse_variable_decl(decl)?);
                }
            }
        }
    }

    Ok(state)
}

/// Parse on_connect handler
fn parse_stream_on_connect(pair: Pair<Rule>) -> Result<Vec<Statement>> {
    statements::parse_statements(pair.into_inner())
}

/// Parse on_disconnect handler
fn parse_stream_on_disconnect(pair: Pair<Rule>) -> Result<Vec<Statement>> {
    statements::parse_statements(pair.into_inner())
}

/// Parse on_event handler
fn parse_stream_on_event(pair: Pair<Rule>) -> Result<StreamOnEvent> {
    let mut inner_pairs = pair.into_inner();

    // First should be the parameter
    let event_param = inner_pairs
        .next()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();

    // Remaining pairs are the body statements
    let body = statements::parse_statements(inner_pairs)?;

    Ok(StreamOnEvent { event_param, body })
}

/// Parse on_window handler
fn parse_stream_on_window(pair: Pair<Rule>) -> Result<StreamOnWindow> {
    let mut inner_pairs = pair.into_inner();

    // First parameter - key
    let key_param = inner_pairs
        .next()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();

    // Second parameter - window
    let window_param = inner_pairs
        .next()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();

    // Remaining pairs are the body statements
    let body = statements::parse_statements(inner_pairs)?;

    Ok(StreamOnWindow {
        key_param,
        window_param,
        body,
    })
}

/// Parse on_error handler
fn parse_stream_on_error(pair: Pair<Rule>) -> Result<StreamOnError> {
    let mut inner_pairs = pair.into_inner();

    // First should be the parameter
    let error_param = inner_pairs
        .next()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();

    // Remaining pairs are the body statements
    let body = statements::parse_statements(inner_pairs)?;

    Ok(StreamOnError { error_param, body })
}
