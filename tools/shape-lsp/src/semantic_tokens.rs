//! Semantic token support for Shape LSP
//!
//! Provides accurate syntax highlighting based on the actual AST.
//! Uses the Visitor pattern for consistent AST traversal.

use crate::type_inference::unified_metadata;
use crate::util::{offset_to_line_col, parser_source};
use shape_ast::ast::{
    BlockItem, Expr, FunctionDef, InterpolationMode, Item, Literal, Pattern, Span, Spanned,
    Statement, TypeAnnotation, VarKind,
};
use shape_ast::interpolation::split_expression_and_format_spec;
use shape_ast::parser::{parse_expression_str, parse_program};
use shape_runtime::visitor::{Visitor, walk_expr, walk_program};
use tower_lsp_server::ls_types::{
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokens, SemanticTokensLegend,
};

/// Standard semantic token types used by Shape
pub const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::NAMESPACE,   // 0 - module names
    SemanticTokenType::TYPE,        // 1 - type names
    SemanticTokenType::CLASS,       // 2 - pattern names
    SemanticTokenType::ENUM,        // 3 - enum names
    SemanticTokenType::FUNCTION,    // 4 - function names
    SemanticTokenType::VARIABLE,    // 5 - variables
    SemanticTokenType::PARAMETER,   // 6 - function parameters
    SemanticTokenType::PROPERTY,    // 7 - object properties
    SemanticTokenType::KEYWORD,     // 8 - keywords
    SemanticTokenType::STRING,      // 9 - strings
    SemanticTokenType::NUMBER,      // 10 - numbers
    SemanticTokenType::OPERATOR,    // 11 - operators
    SemanticTokenType::COMMENT,     // 12 - comments
    SemanticTokenType::MACRO,       // 13 - annotations
    SemanticTokenType::DECORATOR,   // 14 - decorators (@strategy, @warmup, etc.)
    SemanticTokenType::INTERFACE,   // 15 - trait names
    SemanticTokenType::ENUM_MEMBER, // 16 - enum variants
    SemanticTokenType::METHOD,      // 17 - method calls (distinct from free functions)
];

/// Semantic token modifiers
pub const TOKEN_MODIFIERS: &[SemanticTokenModifier] = &[
    SemanticTokenModifier::DECLARATION, // 0 (bit 0 = 1) - definition site
    SemanticTokenModifier::DEFINITION,  // 1 (bit 1 = 2) - definition
    SemanticTokenModifier::READONLY,    // 2 (bit 2 = 4) - const/let
    SemanticTokenModifier::STATIC,      // 3 (bit 3 = 8) - static/module-level
    SemanticTokenModifier::DEPRECATED,  // 4 (bit 4 = 16) - deprecated
    SemanticTokenModifier::DEFAULT_LIBRARY, // 5 (bit 5 = 32) - built-in
    SemanticTokenModifier::MODIFICATION, // 6 (bit 6 = 64) - mutable variable (var)
];

/// Get the semantic tokens legend for capability registration
pub fn get_legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: TOKEN_MODIFIERS.to_vec(),
    }
}

/// A token to be highlighted
#[derive(Debug, Clone)]
struct TokenInfo {
    line: u32,
    start_char: u32,
    length: u32,
    token_type: u32,
    modifiers: u32,
}

/// Collect semantic tokens from source code
pub fn get_semantic_tokens(source: &str) -> Option<SemanticTokens> {
    let mut collector = TokenCollector::new(source);
    let parse_source = parser_source(source);
    let parse_source = parse_source.as_ref();

    // Collect comment tokens from raw source (parser strips them)
    collector.collect_comment_tokens();

    // Prefer AST-driven tokens when parse succeeds, but keep lexical keyword
    // highlighting available while the user is typing incomplete code.
    // With strict parsing, parse_program fails on recovery nodes. Use resilient
    // parsing to keep valid items tokenized while preserving fallback keyword
    // highlighting in broken regions.
    let partial = shape_ast::parse_program_resilient(parse_source);

    if partial.is_complete() {
        // Clean parse — full AST-driven tokens
        if let Ok(program) = parse_program(parse_source) {
            walk_program(&mut collector, &program);
        }
    } else if !partial.items.is_empty() {
        // Partial parse — walk valid items + fallback for broken regions
        let program = partial.into_program();
        walk_program(&mut collector, &program);
        collector.collect_keyword_tokens_fallback();
    } else {
        // Complete failure — lexical fallback only
        collector.collect_keyword_tokens_fallback();
    }

    Some(SemanticTokens {
        result_id: None,
        data: collector.to_semantic_tokens(),
    })
}

/// Collects tokens while walking the AST
struct TokenCollector<'a> {
    source: &'a str,
    lines: Vec<&'a str>,
    tokens: Vec<TokenInfo>,
    /// Track positions already claimed by tokens to avoid duplicates
    used_positions: std::collections::HashSet<(u32, u32)>,
}

impl<'a> TokenCollector<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            lines: source.lines().collect(),
            tokens: Vec::new(),
            used_positions: std::collections::HashSet::new(),
        }
    }

    /// Convert collected tokens to LSP semantic token format (delta-encoded)
    fn to_semantic_tokens(&mut self) -> Vec<SemanticToken> {
        // Sort by position
        self.tokens
            .sort_by(|a, b| a.line.cmp(&b.line).then(a.start_char.cmp(&b.start_char)));

        let mut result = Vec::new();
        let mut prev_line = 0u32;
        let mut prev_char = 0u32;

        for token in &self.tokens {
            let delta_line = token.line - prev_line;
            let delta_start = if delta_line == 0 {
                token.start_char - prev_char
            } else {
                token.start_char
            };

            result.push(SemanticToken {
                delta_line,
                delta_start,
                length: token.length,
                token_type: token.token_type,
                token_modifiers_bitset: token.modifiers,
            });

            prev_line = token.line;
            prev_char = token.start_char;
        }

        result
    }

    /// Add a token using span-based positioning
    fn add_token_from_span(&mut self, span: Span, token_type: u32, modifiers: u32) {
        if span.is_empty() {
            return;
        }

        let Some(text) = self.source.get(span.start..span.end) else {
            return;
        };

        if !text.contains('\n') {
            let (line, col) = offset_to_line_col(self.source, span.start);
            self.add_token(line, col, span.len() as u32, token_type, modifiers);
            return;
        }

        // Split multiline spans into one token per line.
        // This keeps triple-quoted strings highlighted across all lines.
        let mut offset = span.start;
        for segment in text.split('\n') {
            let seg_len = segment.len();
            if seg_len > 0 {
                let (line, col) = offset_to_line_col(self.source, offset);
                self.add_token(line, col, seg_len as u32, token_type, modifiers);
            }
            offset = offset.saturating_add(seg_len);
            if offset < span.end {
                offset = offset.saturating_add(1);
            }
        }
    }

    /// Add a keyword token at the start of a span
    /// Keywords are at the beginning of statements/items, so span.start IS the keyword position
    fn add_keyword_token(&mut self, keyword: &str, span: Span) {
        let (line, col) = offset_to_line_col(self.source, span.start);
        self.add_token(line, col, keyword.len() as u32, 8, 0); // 8 = keyword type
    }

    /// Add a keyword token by locating the keyword within the item's span.
    fn add_keyword_token_in_span(&mut self, keyword: &str, span: Span) {
        let Some(source) = self.source.get(span.start..span.end) else {
            return;
        };
        let Some(rel_offset) = find_keyword_offset(source, keyword) else {
            return;
        };

        let absolute_offset = span.start + rel_offset;
        let (line, col) = offset_to_line_col(self.source, absolute_offset);
        self.add_token(line, col, keyword.len() as u32, 8, 0);
    }

    /// Find a name that appears after a keyword within the span and emit a token for it.
    /// `token_type`: 1 = TYPE, 4 = FUNCTION, 5 = VARIABLE, etc.
    fn add_name_token_after_keyword(
        &mut self,
        keyword: &str,
        name: &str,
        span: Span,
        token_type: u32,
    ) {
        let Some(source) = self.source.get(span.start..span.end) else {
            return;
        };
        // Find the keyword first
        let Some(kw_offset) = find_keyword_offset(source, keyword) else {
            return;
        };
        // Search for the name after the keyword
        let after_kw = &source[kw_offset + keyword.len()..];
        let Some(name_rel) = find_keyword_offset(after_kw, name) else {
            return;
        };
        let absolute_offset = span.start + kw_offset + keyword.len() + name_rel;
        let (line, col) = offset_to_line_col(self.source, absolute_offset);
        self.add_token(line, col, name.len() as u32, token_type, 0);
    }

    /// Emit KEYWORD ("method") and FUNCTION tokens for each method in an impl block.
    /// Scans the source text sequentially so multiple methods are handled correctly.
    fn add_impl_method_tokens(&mut self, impl_block: &shape_ast::ast::ImplBlock, span: Span) {
        let Some(source) = self.source.get(span.start..span.end) else {
            return;
        };
        let mut search_from = 0;
        for method in &impl_block.methods {
            // Find the "method" keyword after search_from
            let remaining = &source[search_from..];
            if let Some(kw_rel) = find_keyword_offset(remaining, "method") {
                let kw_abs = span.start + search_from + kw_rel;
                let (kw_line, kw_col) = offset_to_line_col(self.source, kw_abs);
                self.add_token(kw_line, kw_col, "method".len() as u32, 8, 0); // KEYWORD

                // Find the method name after the "method" keyword
                let after_kw = &remaining[kw_rel + "method".len()..];
                if let Some(name_rel) = find_keyword_offset(after_kw, &method.name) {
                    let name_abs = kw_abs + "method".len() + name_rel;
                    let (name_line, name_col) = offset_to_line_col(self.source, name_abs);
                    self.add_token(name_line, name_col, method.name.len() as u32, 17, 1); // METHOD + DECLARATION
                    // Advance past self method for next iteration
                    search_from += kw_rel + "method".len() + name_rel + method.name.len();
                } else {
                    search_from += kw_rel + "method".len();
                }
            }
        }
    }

    /// Emit TYPE tokens for a function's explicit return type annotation.
    ///
    /// Type annotations currently do not carry dedicated spans in the AST.
    /// We therefore constrain text search to the function signature range
    /// (`fn ... -> ... {`) and highlight type identifiers in-order.
    fn add_function_return_type_tokens(&mut self, func: &FunctionDef, span: Span) {
        let Some(return_type) = &func.return_type else {
            return;
        };

        let Some(item_source) = self.source.get(span.start..span.end) else {
            return;
        };
        let signature_end_rel = item_source.find('{').unwrap_or(item_source.len());
        let signature = &item_source[..signature_end_rel];
        let Some(arrow_rel) = signature.rfind("->") else {
            return;
        };

        let search_start = span.start + arrow_rel + 2; // skip `->`
        let search_end = span.start + signature_end_rel;
        self.add_type_annotation_tokens_in_range(return_type, search_start, search_end);
    }

    fn add_type_annotation_tokens_in_range(
        &mut self,
        annotation: &TypeAnnotation,
        search_start: usize,
        search_end: usize,
    ) {
        self.add_type_annotations_tokens_in_range(
            std::iter::once(annotation),
            search_start,
            search_end,
        );
    }

    fn add_type_annotations_tokens_in_range<'b, I>(
        &mut self,
        annotations: I,
        search_start: usize,
        search_end: usize,
    ) where
        I: IntoIterator<Item = &'b TypeAnnotation>,
    {
        if search_start >= search_end || search_end > self.source.len() {
            return;
        }

        let mut cursor = search_start;
        for annotation in annotations {
            let mut names = Vec::new();
            Self::collect_type_annotation_identifiers(annotation, &mut names);
            if names.is_empty() {
                continue;
            }

            for name in names {
                if name.is_empty() || cursor >= search_end {
                    continue;
                }
                let Some(haystack) = self.source.get(cursor..search_end) else {
                    break;
                };
                let Some(rel) = find_keyword_offset(haystack, name) else {
                    continue;
                };

                let abs = cursor + rel;
                let (line, col) = offset_to_line_col(self.source, abs);
                self.add_token(line, col, name.len() as u32, 1, 0); // TYPE
                cursor = abs + name.len();
            }
        }
    }

    fn collect_type_annotation_identifiers<'b>(
        annotation: &'b TypeAnnotation,
        out: &mut Vec<&'b str>,
    ) {
        match annotation {
            TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => {
                out.push(name.as_str());
            }
            TypeAnnotation::Generic { name, args } => {
                out.push(name.as_str());
                for arg in args {
                    Self::collect_type_annotation_identifiers(arg, out);
                }
            }
            TypeAnnotation::Array(inner) => {
                out.push("Array");
                Self::collect_type_annotation_identifiers(inner, out);
            }
            TypeAnnotation::Optional(inner) => {
                Self::collect_type_annotation_identifiers(inner, out);
            }
            TypeAnnotation::Tuple(items)
            | TypeAnnotation::Union(items)
            | TypeAnnotation::Intersection(items) => {
                for item in items {
                    Self::collect_type_annotation_identifiers(item, out);
                }
            }
            TypeAnnotation::Object(fields) => {
                for field in fields {
                    Self::collect_type_annotation_identifiers(&field.type_annotation, out);
                }
            }
            TypeAnnotation::Function { params, returns } => {
                for param in params {
                    Self::collect_type_annotation_identifiers(&param.type_annotation, out);
                }
                Self::collect_type_annotation_identifiers(returns, out);
            }
            TypeAnnotation::Dyn(traits) => {
                for trait_name in traits {
                    out.push(trait_name.as_str());
                }
            }
            TypeAnnotation::Void
            | TypeAnnotation::Never
            | TypeAnnotation::Null
            | TypeAnnotation::Undefined => {}
        }
    }

    /// Emit KEYWORD tokens for "comptime" in struct field definitions.
    fn add_comptime_field_tokens(
        &mut self,
        struct_def: &shape_ast::ast::StructTypeDef,
        span: Span,
    ) {
        let Some(source) = self.source.get(span.start..span.end) else {
            return;
        };
        let mut search_from = 0;
        for field in &struct_def.fields {
            if !field.is_comptime {
                continue;
            }
            let remaining = &source[search_from..];
            if let Some(kw_rel) = find_keyword_offset(remaining, "comptime") {
                let kw_abs = span.start + search_from + kw_rel;
                let (kw_line, kw_col) = offset_to_line_col(self.source, kw_abs);
                self.add_token(kw_line, kw_col, "comptime".len() as u32, 8, 0); // 8 = KEYWORD
                search_from += kw_rel + "comptime".len();
            }
        }
    }

    /// Add a token at a specific position and mark it as used
    fn add_token(
        &mut self,
        line: u32,
        start_char: u32,
        length: u32,
        token_type: u32,
        modifiers: u32,
    ) {
        if self.used_positions.contains(&(line, start_char)) {
            return;
        }
        self.used_positions.insert((line, start_char));
        self.tokens.push(TokenInfo {
            line,
            start_char,
            length,
            token_type,
            modifiers,
        });
    }

    /// Find position (line, column) from a string in source, skipping already-used positions
    fn find_position(&self, needle: &str, after_line: u32) -> Option<(u32, u32)> {
        for (line_idx, line) in self.lines.iter().enumerate().skip(after_line as usize) {
            let mut search_start = 0;
            while let Some(col) = line[search_start..].find(needle) {
                let actual_col = search_start + col;
                let pos = (line_idx as u32, actual_col as u32);
                if !self.used_positions.contains(&pos) {
                    return Some(pos);
                }
                search_start = actual_col + 1;
            }
        }
        None
    }

    /// Add a token for an identifier using text search with line hint
    fn add_ident_token(&mut self, name: &str, token_type: u32, modifiers: u32, hint_line: u32) {
        if let Some((line, col)) = self.find_position(name, hint_line) {
            self.add_token(line, col, name.len() as u32, token_type, modifiers);
        }
    }

    fn highlight_match_arm_pattern(&mut self, pattern: &Pattern, pattern_span: Option<Span>) {
        let Some(pattern_span) = pattern_span else {
            return;
        };
        if pattern_span.is_dummy() {
            return;
        }
        let Some(pattern_src) = self.source.get(pattern_span.start..pattern_span.end) else {
            return;
        };

        match pattern {
            Pattern::Identifier(name) => {
                if let Some(rel) = pattern_src.find(name) {
                    let start = pattern_span.start + rel;
                    let (line, col) = offset_to_line_col(self.source, start);
                    self.add_token(line, col, name.len() as u32, 2, 0);
                }
            }
            Pattern::Typed {
                name,
                type_annotation,
            } => {
                if let Some(rel) = pattern_src.find(name) {
                    let start = pattern_span.start + rel;
                    let (line, col) = offset_to_line_col(self.source, start);
                    self.add_token(line, col, name.len() as u32, 2, 0);
                }

                let type_name = match type_annotation {
                    TypeAnnotation::Basic(name) | TypeAnnotation::Reference(name) => {
                        Some(name.as_str())
                    }
                    _ => None,
                };
                if let Some(type_name) = type_name {
                    if let Some(rel) = pattern_src.find(type_name) {
                        let start = pattern_span.start + rel;
                        let (line, col) = offset_to_line_col(self.source, start);
                        self.add_token(line, col, type_name.len() as u32, 1, 0);
                    }
                }
            }
            Pattern::Constructor {
                enum_name, variant, ..
            } => {
                if let Some(enum_name) = enum_name {
                    if let Some(rel) = pattern_src.find(enum_name) {
                        let start = pattern_span.start + rel;
                        let (line, col) = offset_to_line_col(self.source, start);
                        self.add_token(line, col, enum_name.len() as u32, 3, 0);
                    }
                }
                if let Some(rel) = pattern_src.find(variant) {
                    let start = pattern_span.start + rel;
                    let (line, col) = offset_to_line_col(self.source, start);
                    self.add_token(line, col, variant.len() as u32, 16, 0);
                }
            }
            _ => {}
        }
    }

    /// Scan raw source text for comments and add COMMENT semantic tokens.
    /// This is needed because the parser strips comments from the AST.
    fn collect_comment_tokens(&mut self) {
        let bytes = self.source.as_bytes();
        let len = bytes.len();
        let mut i = 0;
        let mut line = 0u32;
        let mut col = 0u32;

        while i < len {
            if bytes[i] == b'\n' {
                line += 1;
                col = 0;
                i += 1;
                continue;
            }

            // Check for string literals to avoid false comment matches inside strings
            if bytes[i] == b'"' {
                i += 1;
                col += 1;
                while i < len && bytes[i] != b'"' {
                    if bytes[i] == b'\n' {
                        line += 1;
                        col = 0;
                    } else {
                        col += 1;
                    }
                    i += 1;
                }
                if i < len {
                    i += 1; // skip closing quote
                    col += 1;
                }
                continue;
            }

            // Line comment: // (including /// doc comments)
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                let start_col = col;
                let start_i = i;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                let comment_len = (i - start_i) as u32;
                self.add_token(line, start_col, comment_len, 12, 0); // 12 = COMMENT
                // Don't increment line here — the \n will be handled next iteration
                continue;
            }

            // Block comment: /* */ (including /** doc comments), with nesting
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                let start_line = line;
                let start_col = col;
                i += 2;
                col += 2;
                let mut depth = 1u32;
                while i < len && depth > 0 {
                    if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                        col += 2;
                    } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                        col += 2;
                    } else if bytes[i] == b'\n' {
                        line += 1;
                        col = 0;
                        i += 1;
                    } else {
                        col += 1;
                        i += 1;
                    }
                }
                // For multiline block comments, emit one token per line
                if start_line == line {
                    // Single-line block comment
                    let end_col = col;
                    self.add_token(start_line, start_col, end_col - start_col, 12, 0);
                } else {
                    // Multiline — highlight each line separately
                    let comment_lines: Vec<&str> = self.source[..i].lines().collect();
                    let first_line_idx = start_line as usize;
                    for (idx, cline) in comment_lines.iter().enumerate().skip(first_line_idx) {
                        if idx > line as usize {
                            break;
                        }
                        let c = if idx == first_line_idx { start_col } else { 0 };
                        let l = if idx == first_line_idx {
                            cline.len() as u32 - start_col
                        } else {
                            cline.len() as u32
                        };
                        if l > 0 {
                            self.add_token(idx as u32, c, l, 12, 0);
                        }
                    }
                }
                continue;
            }

            col += 1;
            i += 1;
        }
    }

    /// Fallback lexical keyword scan used when full parsing fails.
    /// Keeps semantic highlighting responsive for incomplete declarations.
    fn collect_keyword_tokens_fallback(&mut self) {
        let bytes = self.source.as_bytes();
        let len = bytes.len();
        let mut i = 0usize;

        while i < len {
            // Strings: "...", """...""", f"...", f"""..."""
            if bytes[i] == b'"' || (bytes[i] == b'f' && i + 1 < len && bytes[i + 1] == b'"') {
                i = skip_string_literal(bytes, i);
                continue;
            }

            // Line comment: //...
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'/' {
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }

            // Block comment: /* ... */ with nesting
            if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                i += 2;
                let mut depth = 1u32;
                while i < len && depth > 0 {
                    if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                        depth += 1;
                        i += 2;
                    } else if i + 1 < len && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }

            if is_ident_start_byte(bytes[i]) {
                let start = i;
                i += 1;
                while i < len && is_ident_continue_byte(bytes[i]) {
                    i += 1;
                }

                if let Some(ident) = self.source.get(start..i) {
                    if is_fallback_keyword(ident) {
                        let (line, col) = offset_to_line_col(self.source, start);
                        self.add_token(line, col, ident.len() as u32, 8, 0);

                        // For declaration keywords, also highlight the following name
                        let name_token_type = match ident {
                            "enum" => Some(3u32),            // ENUM
                            "type" | "interface" => Some(1), // TYPE
                            "trait" => Some(15),             // INTERFACE
                            "fn" | "function" => Some(4),    // FUNCTION
                            _ => None,
                        };
                        if let Some(tt) = name_token_type {
                            // Skip whitespace after keyword to find the name
                            let mut j = i;
                            while j < len && bytes[j].is_ascii_whitespace() {
                                j += 1;
                            }
                            if j < len && is_ident_start_byte(bytes[j]) {
                                let name_start = j;
                                j += 1;
                                while j < len && is_ident_continue_byte(bytes[j]) {
                                    j += 1;
                                }
                                let name_len = (j - name_start) as u32;
                                let (name_line, name_col) =
                                    offset_to_line_col(self.source, name_start);
                                let modifier = 1; // DECLARATION
                                self.add_token(name_line, name_col, name_len, tt, modifier);
                            }
                        }
                    }
                }
                continue;
            }

            i += 1;
        }
    }

    /// Emit non-overlapping semantic tokens for a formatted string literal.
    ///
    /// Instead of one STRING token covering the entire f-string (which suppresses
    /// code highlighting and autocomplete inside `{expr}`), self emits:
    /// 1. STRING token for the prefix (`f"`, `f$"`, `f#"`, and triple variants)
    /// 2. STRING tokens for text segments between interpolations
    /// 3. Code tokens for expressions inside `{expr}` via `InterpolationExprTokenCollector`
    /// 4. STRING token for the suffix (`"` or `"""`)
    fn add_formatted_string_tokens(&mut self, span: Span, mode: InterpolationMode) {
        let literal_source = match self.source.get(span.start..span.end) {
            Some(src) => src,
            None => return,
        };

        let prefix = mode.prefix();
        let triple_prefix = format!(r#"{}"""#, prefix);
        let simple_prefix = format!(r#"{}""#, prefix);

        let (body, body_offset, prefix_len, suffix_len) = if literal_source
            .starts_with(&triple_prefix)
            && literal_source.ends_with("\"\"\"")
            && literal_source.len() >= triple_prefix.len() + 3
        {
            (
                &literal_source[triple_prefix.len()..literal_source.len() - 3],
                triple_prefix.len(),
                triple_prefix.len(),
                3usize,
            )
        } else if literal_source.starts_with(&simple_prefix)
            && literal_source.ends_with('"')
            && literal_source.len() >= simple_prefix.len() + 1
        {
            (
                &literal_source[simple_prefix.len()..literal_source.len() - 1],
                simple_prefix.len(),
                simple_prefix.len(),
                1usize,
            )
        } else {
            // Fallback: treat entire span as single string token
            self.add_token_from_span(span, 9, 0);
            return;
        };

        // Emit STRING token for prefix (f" or f""")
        self.add_token_from_span(Span::new(span.start, span.start + prefix_len), 9, 0);

        let segments = find_interpolation_segments(body, mode);
        let mut last_end = 0; // position in body after last `}`

        for (expr_start, expr_end) in &segments {
            // Text segment before the `{` of self interpolation.
            // `expr_start` points right after the opening `{` token.
            // In sigil modes (`${` / `#{`) there are two opener bytes.
            let opener_len = if mode == InterpolationMode::Braces {
                1
            } else {
                2
            };
            let brace_open_pos = expr_start.saturating_sub(opener_len);
            if brace_open_pos > last_end {
                let text_abs_start = span.start + body_offset + last_end;
                let text_abs_end = span.start + body_offset + brace_open_pos;
                self.add_token_from_span(Span::new(text_abs_start, text_abs_end), 9, 0);
            }

            // Emit code tokens for the expression content inside {expr}
            let raw_expr = &body[*expr_start..*expr_end];
            let trimmed_expr = raw_expr.trim();
            if !trimmed_expr.is_empty() {
                let leading_ws = raw_expr.len().saturating_sub(raw_expr.trim_start().len());
                let base_offset = span.start + body_offset + expr_start + leading_ws;
                let expr_for_tokens = if let Ok((expr_only, _spec)) =
                    split_expression_and_format_spec(trimmed_expr)
                {
                    expr_only
                } else {
                    trimmed_expr.to_string()
                };

                if let Ok(parsed) = parse_expression_str(&expr_for_tokens) {
                    let mut nested = InterpolationExprTokenCollector::new(self, base_offset);
                    walk_expr(&mut nested, &parsed);
                }
            }

            last_end = expr_end + 1; // right after `}`
        }

        // Text segment after the last `}` to end of body
        if last_end < body.len() {
            let text_abs_start = span.start + body_offset + last_end;
            let text_abs_end = span.start + body_offset + body.len();
            self.add_token_from_span(Span::new(text_abs_start, text_abs_end), 9, 0);
        }

        // Emit STRING token for suffix (" or """)
        self.add_token_from_span(Span::new(span.end - suffix_len, span.end), 9, 0);
    }

    /// Add semantic tokens for a content string literal (`c"..."`, `c$"..."`, `c#"..."`).
    ///
    /// The `c` prefix is highlighted as a KEYWORD token to distinguish content strings
    /// from formatted strings. The rest uses the same interpolation logic as f-strings.
    fn add_content_string_tokens(&mut self, span: Span, mode: InterpolationMode) {
        let literal_source = match self.source.get(span.start..span.end) {
            Some(src) => src,
            None => return,
        };

        // Content strings use c/c$/c# prefix
        let c_prefix = match mode {
            InterpolationMode::Braces => "c",
            InterpolationMode::Dollar => "c$",
            InterpolationMode::Hash => "c#",
        };
        let triple_prefix = format!(r#"{}"""#, c_prefix);
        let simple_prefix = format!(r#"{}""#, c_prefix);

        let (body, body_offset, prefix_len, suffix_len) = if literal_source
            .starts_with(&triple_prefix)
            && literal_source.ends_with("\"\"\"")
            && literal_source.len() >= triple_prefix.len() + 3
        {
            (
                &literal_source[triple_prefix.len()..literal_source.len() - 3],
                triple_prefix.len(),
                triple_prefix.len(),
                3usize,
            )
        } else if literal_source.starts_with(&simple_prefix)
            && literal_source.ends_with('"')
            && literal_source.len() >= simple_prefix.len() + 1
        {
            (
                &literal_source[simple_prefix.len()..literal_source.len() - 1],
                simple_prefix.len(),
                simple_prefix.len(),
                1usize,
            )
        } else {
            // Fallback: treat entire span as single string token
            self.add_token_from_span(span, 9, 0);
            return;
        };

        // Emit KEYWORD token for the `c` prefix character to distinguish from f-strings
        let c_char_len = 1; // just the 'c' character
        self.add_token_from_span(Span::new(span.start, span.start + c_char_len), 8, 0);

        // Emit STRING token for the rest of the prefix (the quote(s))
        self.add_token_from_span(
            Span::new(span.start + c_char_len, span.start + prefix_len),
            9,
            0,
        );

        let segments = find_interpolation_segments(body, mode);
        let mut last_end = 0;

        for (expr_start, expr_end) in &segments {
            let opener_len = if mode == InterpolationMode::Braces {
                1
            } else {
                2
            };
            let brace_open_pos = expr_start.saturating_sub(opener_len);
            if brace_open_pos > last_end {
                let text_abs_start = span.start + body_offset + last_end;
                let text_abs_end = span.start + body_offset + brace_open_pos;
                self.add_token_from_span(Span::new(text_abs_start, text_abs_end), 9, 0);
            }

            let raw_expr = &body[*expr_start..*expr_end];
            let trimmed_expr = raw_expr.trim();
            if !trimmed_expr.is_empty() {
                let leading_ws = raw_expr.len().saturating_sub(raw_expr.trim_start().len());
                let base_offset = span.start + body_offset + expr_start + leading_ws;
                let expr_for_tokens = if let Ok((expr_only, _spec)) =
                    split_expression_and_format_spec(trimmed_expr)
                {
                    expr_only
                } else {
                    trimmed_expr.to_string()
                };

                if let Ok(parsed) = parse_expression_str(&expr_for_tokens) {
                    let mut nested = InterpolationExprTokenCollector::new(self, base_offset);
                    walk_expr(&mut nested, &parsed);
                }
            }

            last_end = expr_end + 1;
        }

        if last_end < body.len() {
            let text_abs_start = span.start + body_offset + last_end;
            let text_abs_end = span.start + body_offset + body.len();
            self.add_token_from_span(Span::new(text_abs_start, text_abs_end), 9, 0);
        }

        // Emit STRING token for suffix (" or """)
        self.add_token_from_span(Span::new(span.end - suffix_len, span.end), 9, 0);
    }
}

/// Find interpolation expression segments in a formatted string body.
///
/// Returned byte ranges exclude the braces themselves.
fn find_interpolation_segments(body: &str, mode: InterpolationMode) -> Vec<(usize, usize)> {
    let mut segments = Vec::new();
    let mut chars = body.char_indices().peekable();

    while let Some((idx, ch)) = chars.next() {
        if mode != InterpolationMode::Braces && ch == mode.sigil().unwrap_or_default() {
            // Escaped opener: $${ or ##{ should remain literal text.
            if let Some((_, next)) = chars.peek() {
                if *next == ch {
                    let mut probe = chars.clone();
                    let _ = probe.next(); // second sigil
                    if matches!(probe.next(), Some((_, '{'))) {
                        let _ = chars.next(); // second sigil
                        let _ = chars.next(); // '{'
                        continue;
                    }
                }
            }
        }

        let is_open = match mode {
            InterpolationMode::Braces => ch == '{',
            InterpolationMode::Dollar => ch == '$' && matches!(chars.peek(), Some((_, '{'))),
            InterpolationMode::Hash => ch == '#' && matches!(chars.peek(), Some((_, '{'))),
        };
        if !is_open {
            continue;
        }

        if mode == InterpolationMode::Braces {
            // Escaped open brace `{{` -> literal `{`
            if matches!(chars.peek(), Some((_, '{'))) {
                chars.next();
                continue;
            }
        } else {
            // Consume the `{` from `${` / `#{`
            chars.next();
        }

        let expr_start = if mode == InterpolationMode::Braces {
            idx + ch.len_utf8()
        } else {
            idx + ch.len_utf8() + 1
        };
        let mut depth = 1usize;
        let mut in_string: Option<char> = None;
        let mut escaped = false;
        let mut expr_end = None;

        while let Some((inner_idx, inner_ch)) = chars.next() {
            if let Some(quote) = in_string {
                if escaped {
                    escaped = false;
                    continue;
                }
                if inner_ch == '\\' {
                    escaped = true;
                    continue;
                }
                if inner_ch == quote {
                    in_string = None;
                }
                continue;
            }

            match inner_ch {
                '"' | '\'' => in_string = Some(inner_ch),
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        expr_end = Some(inner_idx);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(end) = expr_end {
            segments.push((expr_start, end));
        } else {
            break;
        }
    }

    segments
}

fn find_keyword_offset(text: &str, keyword: &str) -> Option<usize> {
    text.match_indices(keyword).find_map(|(idx, _)| {
        let before_ok = idx == 0
            || !text[..idx]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let end = idx + keyword.len();
        let after_ok = end >= text.len()
            || !text[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');

        if before_ok && after_ok {
            Some(idx)
        } else {
            None
        }
    })
}

fn is_ident_start_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_ident_continue_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

fn skip_string_literal(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start;

    // Optional formatted-string prefix: f"...", f$"...", f#"..."
    if bytes[i] == b'f' {
        if i + 2 < len && (bytes[i + 1] == b'$' || bytes[i + 1] == b'#') {
            if bytes[i + 2] != b'"' {
                return (start + 1).min(len);
            }
            i += 2;
        } else {
            if i + 1 >= len || bytes[i + 1] != b'"' {
                return (start + 1).min(len);
            }
            i += 1;
        }
    }

    // Triple-quoted string
    if i + 2 < len && bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
        i += 3;
        while i + 2 < len {
            if bytes[i] == b'"' && bytes[i + 1] == b'"' && bytes[i + 2] == b'"' {
                return i + 3;
            }
            i += 1;
        }
        return len;
    }

    // Simple quoted string
    if bytes[i] != b'"' {
        return (start + 1).min(len);
    }

    i += 1;
    while i < len {
        if bytes[i] == b'\\' && i + 1 < len {
            i += 2;
            continue;
        }
        if bytes[i] == b'"' {
            return i + 1;
        }
        i += 1;
    }

    len
}

fn is_fallback_keyword(word: &str) -> bool {
    matches!(
        word,
        "pub"
            | "from"
            | "use"
            | "as"
            | "default"
            | "let"
            | "var"
            | "const"
            | "function"
            | "fn"
            | "async"
            | "await"
            | "if"
            | "else"
            | "for"
            | "while"
            | "return"
            | "break"
            | "continue"
            | "loop"
            | "match"
            | "true"
            | "false"
            | "None"
            | "Some"
            | "and"
            | "or"
            | "not"
            | "in"
            | "find"
            | "all"
            | "analyze"
            | "scan"
            | "type"
            | "interface"
            | "enum"
            | "extend"
            | "trait"
            | "impl"
            | "method"
            | "when"
            | "self"
            | "on"
            | "comptime"
            | "datasource"
            | "query"
            | "stream"
            | "test"
            | "optimize"
            | "backtest"
            | "alert"
            | "with"
            | "select"
            | "order"
            | "by"
            | "asc"
            | "desc"
            | "group"
            | "into"
            | "join"
            | "race"
            | "settle"
            | "equals"
            | "dyn"
            | "where"
            | "extends"
    )
}

/// Nested token collector used for expression AST parsed from interpolation body text.
struct InterpolationExprTokenCollector<'t, 'src> {
    tokens: &'t mut TokenCollector<'src>,
    base_offset: usize,
}

impl<'t, 'src> InterpolationExprTokenCollector<'t, 'src> {
    fn new(tokens: &'t mut TokenCollector<'src>, base_offset: usize) -> Self {
        Self {
            tokens,
            base_offset,
        }
    }

    fn add_shifted_span_token(&mut self, span: Span, token_type: u32, modifiers: u32) {
        let shifted = Span::new(
            span.start.saturating_add(self.base_offset),
            span.end.saturating_add(self.base_offset),
        );
        self.tokens
            .add_token_from_span(shifted, token_type, modifiers);
    }
}

impl Visitor for InterpolationExprTokenCollector<'_, '_> {
    fn visit_expr(&mut self, expr: &Expr) -> bool {
        match expr {
            Expr::Identifier(_, span) => {
                self.add_shifted_span_token(*span, 5, 0); // variable
            }
            Expr::Literal(lit, span) => {
                let token_type = match lit {
                    Literal::Int(_)
                    | Literal::UInt(_)
                    | Literal::TypedInt(_, _)
                    | Literal::Number(_)
                    | Literal::Decimal(_) => 10,
                    Literal::String(_)
                    | Literal::FormattedString { .. }
                    | Literal::ContentString { .. } => 9,
                    Literal::Bool(_) | Literal::None | Literal::Unit => 8,
                    Literal::Timeframe(_) => 10,
                };
                self.add_shifted_span_token(*span, token_type, 0);
            }
            Expr::FunctionCall { name, span, .. } => {
                // Emit FUNCTION token for the function name (at the start of the call span)
                let name_span = Span::new(span.start, span.start + name.len());
                self.add_shifted_span_token(name_span, 4, 0); // function
                // Walker will recurse into arguments
            }
            Expr::PropertyAccess { property, span, .. } => {
                // The property name is at the end of the span: `obj.property`
                let prop_start = span.end.saturating_sub(property.len());
                let prop_span = Span::new(prop_start, span.end);
                self.add_shifted_span_token(prop_span, 7, 0); // property
                // Walker will recurse into the object expression
            }
            Expr::MethodCall {
                receiver, method, ..
            } => {
                // Method name is right after `receiver.` in the source.
                // Compute method name position from receiver span end + 1 (for the dot).
                let receiver_span = receiver.span();
                let method_start = receiver_span.end + 1; // +1 for the `.`
                let method_span = Span::new(method_start, method_start + method.len());
                self.add_shifted_span_token(method_span, 17, 0); // METHOD type
                // Walker will recurse into receiver and arguments
            }
            // BinaryOp, UnaryOp, Array, Object, etc. - the walker recurses into children
            // which will be caught by the above handlers.
            _ => {}
        }
        true // Always recurse into children
    }
}

/// Implement the Visitor trait for TokenCollector
impl<'a> Visitor for TokenCollector<'a> {
    fn visit_item(&mut self, item: &Item) -> bool {
        match item {
            Item::Function(func, span) => {
                // Support both `fn` and legacy `function`.
                let keyword = self
                    .source
                    .get(span.start..span.end)
                    .and_then(|src| {
                        if find_keyword_offset(src, "fn").is_some() {
                            Some("fn")
                        } else if find_keyword_offset(src, "function").is_some() {
                            Some("function")
                        } else {
                            None
                        }
                    })
                    .unwrap_or("function");
                self.add_keyword_token_in_span(keyword, *span);
                // Function name - use name_span directly
                self.add_token_from_span(func.name_span, 4, 1); // function, declaration
                // Highlight parameters - use their name_span
                for param in &func.params {
                    self.add_token_from_span(param.span(), 6, 0); // parameter
                }
                if let Some(item_source) = self.source.get(span.start..span.end) {
                    let signature_end_rel = item_source.find('{').unwrap_or(item_source.len());
                    let signature_start = span.start;
                    let signature_end = span.start + signature_end_rel;
                    let param_types = func
                        .params
                        .iter()
                        .filter_map(|p| p.type_annotation.as_ref());
                    self.add_type_annotations_tokens_in_range(
                        param_types,
                        signature_start,
                        signature_end,
                    );
                }
                self.add_function_return_type_tokens(func, *span);
            }
            Item::VariableDecl(decl, span) => {
                let keyword = match decl.kind {
                    VarKind::Let => "let",
                    VarKind::Const => "const",
                    VarKind::Var => "var",
                };
                self.add_keyword_token(keyword, *span);
                if let Some(name) = decl.pattern.as_identifier() {
                    let modifiers = match decl.kind {
                        VarKind::Const => 1 | 4, // DECLARATION | READONLY
                        VarKind::Let => 1 | 4,   // DECLARATION | READONLY
                        VarKind::Var => 1 | 64,  // DECLARATION | MODIFICATION (mutable)
                    };
                    let (line, _) = offset_to_line_col(self.source, span.start);
                    self.add_ident_token(name, 5, modifiers, line);
                }
            }
            Item::Import(import_stmt, span) => {
                // Highlight "from" keyword if this is a from-use (Named import)
                if matches!(import_stmt.items, shape_ast::ast::ImportItems::Named(_)) {
                    self.add_keyword_token("from", *span);
                    self.add_keyword_token_in_span("use", *span);
                } else {
                    // Namespace import variant: `use module.path`.
                    self.add_keyword_token("use", *span);
                }
            }
            Item::Export(_, span) => {
                // "pub" keyword at span start
                self.add_keyword_token("pub", *span);
            }
            Item::Module(module_def, span) => {
                self.add_keyword_token("mod", *span);
                self.add_name_token_after_keyword("mod", &module_def.name, *span, 8);
                // namespace
            }
            Item::Extend(_, span) => {
                // "extend" keyword at span start
                self.add_keyword_token("extend", *span);
            }
            Item::Query(query, span) => {
                // Query keyword at span start - match on the query variant
                let keyword = match query {
                    shape_ast::ast::Query::Backtest(_) => "backtest",
                    shape_ast::ast::Query::Alert(_) => "alert",
                    shape_ast::ast::Query::With(_) => "with",
                };
                self.add_keyword_token(keyword, *span);
            }
            Item::TypeAlias(type_alias, span) => {
                self.add_keyword_token("type", *span);
                self.add_name_token_after_keyword("type", &type_alias.name, *span, 1); // TYPE
                self.add_type_annotation_tokens_in_range(
                    &type_alias.type_annotation,
                    span.start,
                    span.end,
                );
            }
            Item::Interface(interface_def, span) => {
                self.add_keyword_token("interface", *span);
                self.add_name_token_after_keyword("interface", &interface_def.name, *span, 15);
                // INTERFACE
            }
            Item::Trait(trait_def, span) => {
                self.add_keyword_token("trait", *span);
                // Emit INTERFACE token for the trait name
                self.add_name_token_after_keyword("trait", &trait_def.name, *span, 15);
            }
            Item::Impl(impl_block, span) => {
                self.add_keyword_token("impl", *span);
                // Emit INTERFACE token for the trait name after "impl"
                let trait_name = match &impl_block.trait_name {
                    shape_ast::ast::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::TypeName::Generic { name, .. } => name.as_str(),
                };
                self.add_name_token_after_keyword("impl", trait_name, *span, 15); // INTERFACE
                // Emit KEYWORD token for "for"
                self.add_keyword_token_in_span("for", *span);
                // Emit TYPE token for the target type after "for"
                let target_name = match &impl_block.target_type {
                    shape_ast::ast::TypeName::Simple(n) => n.as_str(),
                    shape_ast::ast::TypeName::Generic { name, .. } => name.as_str(),
                };
                self.add_name_token_after_keyword("for", target_name, *span, 1); // TYPE
                if let Some(impl_name) = &impl_block.impl_name {
                    self.add_keyword_token_in_span("as", *span);
                    self.add_name_token_after_keyword("as", impl_name, *span, 1);
                    // TYPE
                }
                // Emit KEYWORD + METHOD tokens for each method in the impl block
                self.add_impl_method_tokens(impl_block, *span);
            }
            Item::Enum(enum_def, span) => {
                self.add_keyword_token("enum", *span);
                self.add_name_token_after_keyword("enum", &enum_def.name, *span, 3);
                // ENUM
            }
            Item::Stream(_, span) => {
                self.add_keyword_token("stream", *span);
            }
            Item::Test(_, span) => {
                self.add_keyword_token("test", *span);
            }
            Item::Optimize(_, span) => {
                self.add_keyword_token("optimize", *span);
            }
            Item::StructType(struct_def, span) => {
                self.add_keyword_token("type", *span);
                self.add_name_token_after_keyword("type", &struct_def.name, *span, 1); // TYPE
                self.add_type_annotations_tokens_in_range(
                    struct_def.fields.iter().map(|field| &field.type_annotation),
                    span.start,
                    span.end,
                );
                // Emit "comptime" keyword tokens for comptime fields
                self.add_comptime_field_tokens(struct_def, *span);
            }
            Item::DataSource(_, span) => {
                self.add_keyword_token("datasource", *span);
            }
            Item::QueryDecl(_, span) => {
                self.add_keyword_token("query", *span);
            }
            Item::BuiltinTypeDecl(type_decl, span) => {
                self.add_keyword_token("builtin", *span);
                self.add_keyword_token_in_span("type", *span);
                self.add_token_from_span(type_decl.name_span, 1, 1); // type, declaration
            }
            Item::BuiltinFunctionDecl(func_decl, span) => {
                self.add_keyword_token("builtin", *span);
                let keyword = self
                    .source
                    .get(span.start..span.end)
                    .and_then(|src| {
                        if find_keyword_offset(src, "fn").is_some() {
                            Some("fn")
                        } else if find_keyword_offset(src, "function").is_some() {
                            Some("function")
                        } else {
                            None
                        }
                    })
                    .unwrap_or("fn");
                self.add_keyword_token_in_span(keyword, *span);
                self.add_token_from_span(func_decl.name_span, 4, 1); // function, declaration
                for param in &func_decl.params {
                    self.add_token_from_span(param.span(), 6, 0); // parameter
                }
            }
            Item::ForeignFunction(foreign_fn, span) => {
                if foreign_fn.is_async {
                    self.add_keyword_token_in_span("async", *span);
                }
                // "fn" keyword
                self.add_keyword_token_in_span("fn", *span);
                // Language identifier as a keyword token
                self.add_token_from_span(foreign_fn.language_span, 8, 0); // keyword
                // Function name as a function declaration token
                self.add_token_from_span(foreign_fn.name_span, 4, 1); // function, declaration
                // Parameters
                for param in &foreign_fn.params {
                    self.add_token_from_span(param.span(), 6, 0); // parameter
                }
                if let Some(item_source) = self.source.get(span.start..span.end) {
                    let signature_end_rel = item_source.find('{').unwrap_or(item_source.len());
                    let signature_start = span.start;
                    let signature_end = span.start + signature_end_rel;
                    let param_types = foreign_fn
                        .params
                        .iter()
                        .filter_map(|param| param.type_annotation.as_ref());
                    self.add_type_annotations_tokens_in_range(
                        param_types,
                        signature_start,
                        signature_end,
                    );
                    if let Some(return_type) = &foreign_fn.return_type {
                        self.add_type_annotation_tokens_in_range(
                            return_type,
                            signature_start,
                            signature_end,
                        );
                    }
                }
                // Do not force-tokenize foreign body as STRING.
                // This avoids painting the whole block as a string in editors and
                // leaves room for foreign-language tooling to provide richer UX.
            }
            Item::Assignment(_, _)
            | Item::Expression(_, _)
            | Item::Statement(_, _)
            | Item::AnnotationDef(_, _) => {
                // These are handled by walking their children
            }
            Item::Comptime(_, span) => {
                self.add_keyword_token("comptime", *span);
            }
        }
        true // Continue visiting children
    }

    fn visit_stmt(&mut self, stmt: &Statement) -> bool {
        match stmt {
            Statement::VariableDecl(decl, span) => {
                // Highlight the keyword (let/const/var) at span start
                let keyword = match decl.kind {
                    VarKind::Let => "let",
                    VarKind::Const => "const",
                    VarKind::Var => "var",
                };
                self.add_keyword_token(keyword, *span);
                // Highlight variable name with appropriate modifiers
                if let Some(name) = decl.pattern.as_identifier() {
                    let modifiers = match decl.kind {
                        VarKind::Const => 1 | 4, // DECLARATION | READONLY
                        VarKind::Let => 1 | 4,   // DECLARATION | READONLY
                        VarKind::Var => 1 | 64,  // DECLARATION | MODIFICATION (mutable)
                    };
                    let (line, _) = offset_to_line_col(self.source, span.start);
                    self.add_ident_token(name, 5, modifiers, line);
                }
                if let Some(type_annotation) = &decl.type_annotation {
                    let statement_end = self
                        .source
                        .get(span.start..span.end)
                        .and_then(|src| src.find('='))
                        .map(|rel| span.start + rel)
                        .unwrap_or(span.end);
                    self.add_type_annotation_tokens_in_range(
                        type_annotation,
                        span.start,
                        statement_end,
                    );
                }
            }
            Statement::Assignment(assign, span) => {
                if let Some(name) = assign.pattern.as_identifier() {
                    let (line, _) = offset_to_line_col(self.source, span.start);
                    self.add_ident_token(name, 5, 0, line); // variable
                }
            }
            Statement::Return(_, span) => {
                self.add_keyword_token("return", *span);
            }
            Statement::If(_, span) => {
                self.add_keyword_token("if", *span);
            }
            Statement::For(for_loop, span) => {
                self.add_keyword_token("for", *span);
                // Highlight "await" keyword for `for await` loops
                if for_loop.is_async {
                    self.add_keyword_token_in_span("await", *span);
                }
            }
            Statement::While(_, span) => {
                self.add_keyword_token("while", *span);
            }
            Statement::Break(span) => {
                self.add_keyword_token("break", *span);
            }
            Statement::Continue(span) => {
                self.add_keyword_token("continue", *span);
            }
            Statement::Expression(_, _) => {
                // Expressions are handled by visit_expr
            }
            Statement::Extend(_, span) => {
                self.add_keyword_token("extend", *span);
            }
            Statement::RemoveTarget(span) => {
                self.add_keyword_token("remove", *span);
                self.add_keyword_token_in_span("target", *span);
            }
            Statement::SetParamType { span, .. } => {
                self.add_keyword_token("set", *span);
                self.add_keyword_token_in_span("param", *span);
            }
            Statement::SetReturnType { span, .. } => {
                self.add_keyword_token("set", *span);
                self.add_keyword_token_in_span("return", *span);
            }
            Statement::SetReturnExpr { span, .. } => {
                self.add_keyword_token("set", *span);
                self.add_keyword_token_in_span("return", *span);
            }
            Statement::ReplaceBodyExpr { span, .. } => {
                self.add_keyword_token("replace", *span);
                self.add_keyword_token_in_span("body", *span);
            }
            Statement::ReplaceBody { span, .. } => {
                self.add_keyword_token("replace", *span);
                self.add_keyword_token_in_span("body", *span);
            }
            Statement::ReplaceModuleExpr { span, .. } => {
                self.add_keyword_token("replace", *span);
                self.add_keyword_token_in_span("module", *span);
            }
        }
        true // Continue visiting children
    }

    fn visit_expr(&mut self, expr: &Expr) -> bool {
        match expr {
            Expr::Identifier(name, span) => {
                // If the identifier text is a known keyword (e.g. "fn"), emit as
                // KEYWORD.  This handles resilient-parse artefacts where incomplete
                // code like "fn compute(" is parsed with "fn" as an identifier.
                if is_fallback_keyword(name) {
                    self.add_token_from_span(*span, 8, 0); // KEYWORD
                    // For declaration keywords, also highlight the following name
                    // using a scan-ahead in source text (mirrors fallback scanner).
                    let name_token_type = match name.as_str() {
                        "enum" => Some(3u32),            // ENUM
                        "type" | "interface" => Some(1), // TYPE
                        "trait" => Some(15),             // INTERFACE
                        "fn" | "function" => Some(4),    // FUNCTION
                        _ => None,
                    };
                    if let Some(tt) = name_token_type {
                        let bytes = self.source.as_bytes();
                        let mut j = span.end;
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if j < bytes.len() && is_ident_start_byte(bytes[j]) {
                            let name_start = j;
                            j += 1;
                            while j < bytes.len() && is_ident_continue_byte(bytes[j]) {
                                j += 1;
                            }
                            let (name_line, name_col) = offset_to_line_col(self.source, name_start);
                            self.add_token(
                                name_line,
                                name_col,
                                (j - name_start) as u32,
                                tt,
                                1, // DECLARATION modifier
                            );
                        }
                    }
                } else {
                    // Distinguish function types using unified metadata
                    let metadata = unified_metadata();
                    let (token_type, modifiers) = if let Some(_func) = metadata.get_function(name) {
                        // It's a known function - check if it's Rust builtin or stdlib
                        let is_rust_builtin = metadata
                            .rust_builtins()
                            .iter()
                            .any(|f| f.name == name.as_str());
                        let is_stdlib = metadata
                            .stdlib_functions()
                            .iter()
                            .any(|f| f.name == name.as_str());

                        let modifier = if is_rust_builtin {
                            32 // DEFAULT_LIBRARY modifier (bit 5)
                        } else if is_stdlib {
                            8 // STATIC modifier (bit 3)
                        } else {
                            0 // User-defined
                        };
                        (4, modifier) // function token type
                    } else {
                        (5, 0) // variable token type, no modifier
                    };
                    self.add_token_from_span(*span, token_type, modifiers);
                }
            }
            Expr::FunctionCall {
                name, args, span, ..
            } => {
                // Distinguish function types using unified metadata
                let metadata = unified_metadata();
                let modifiers = if let Some(_func) = metadata.get_function(name) {
                    // Check if it's Rust builtin or stdlib
                    let is_rust_builtin = metadata
                        .rust_builtins()
                        .iter()
                        .any(|f| f.name == name.as_str());
                    let is_stdlib = metadata
                        .stdlib_functions()
                        .iter()
                        .any(|f| f.name == name.as_str());

                    if is_rust_builtin {
                        32 // DEFAULT_LIBRARY modifier (bit 5)
                    } else if is_stdlib {
                        8 // STATIC modifier (bit 3)
                    } else {
                        0 // User-defined
                    }
                } else {
                    0 // Unknown function, no modifier
                };

                // Function name is at the span start, use text search with line hint
                let (line, _) = offset_to_line_col(self.source, span.start);
                self.add_ident_token(name, 4, modifiers, line);

                // Special case: Highlight data loader names for data() function
                if name == "data" && !args.is_empty() {
                    // First argument should be the data loader name (a string literal)
                    if let Expr::Literal(Literal::String(_loader_name), loader_span) = &args[0] {
                        // Highlight the loader name with NAMESPACE token type (0)
                        self.add_token_from_span(*loader_span, 0, 0);
                    }
                }
            }
            Expr::EnumConstructor {
                enum_name,
                variant,
                span,
                ..
            } => {
                let (line, _) = offset_to_line_col(self.source, span.start);
                self.add_ident_token(enum_name, 3, 0, line); // ENUM type
                self.add_ident_token(variant, 16, 0, line); // ENUM_MEMBER type
            }
            Expr::MethodCall { method, span, .. } => {
                let (line, _) = offset_to_line_col(self.source, span.start);
                self.add_ident_token(method, 17, 0, line); // METHOD type (distinct from free functions)
            }
            Expr::PropertyAccess { property, span, .. } => {
                let (line, _) = offset_to_line_col(self.source, span.start);
                self.add_ident_token(property, 7, 0, line);
            }
            Expr::Object(entries, span) => {
                use shape_ast::ast::ObjectEntry;
                let (line, _) = offset_to_line_col(self.source, span.start);
                for entry in entries {
                    if let ObjectEntry::Field { key, .. } = entry {
                        self.add_ident_token(key, 7, 0, line); // property
                    }
                }
            }
            Expr::FunctionExpr { params, .. } => {
                // Highlight parameters - use their name_span directly
                for param in params {
                    self.add_token_from_span(param.span(), 6, 0); // parameter
                }
            }
            Expr::If(_, span) => {
                self.add_keyword_token("if", *span);
            }
            Expr::While(_, span) => {
                self.add_keyword_token("while", *span);
            }
            Expr::For(for_expr, span) => {
                self.add_keyword_token("for", *span);
                // Highlight "await" keyword for `for await` loops
                if for_expr.is_async {
                    self.add_keyword_token_in_span("await", *span);
                }
            }
            Expr::Loop(_, span) => {
                self.add_keyword_token("loop", *span);
            }
            Expr::Match(match_expr, span) => {
                self.add_keyword_token("match", *span);
                for arm in &match_expr.arms {
                    self.highlight_match_arm_pattern(&arm.pattern, arm.pattern_span);
                }
            }
            Expr::Return(_, span) => {
                self.add_keyword_token("return", *span);
            }
            Expr::Break(_, span) => {
                self.add_keyword_token("break", *span);
            }
            Expr::Continue(span) => {
                self.add_keyword_token("continue", *span);
            }
            Expr::Let(_, span) => {
                self.add_keyword_token("let", *span);
            }
            Expr::TryOperator(_, _) => {
                // The ? operator - no special highlighting needed
            }
            Expr::UsingImpl { span, .. } => {
                self.add_keyword_token("using", *span);
            }
            Expr::Literal(lit, span) => {
                match lit {
                    Literal::FormattedString { mode, .. } => {
                        // Split f-string into non-overlapping segments:
                        // STRING tokens for text parts, code tokens for {expr} parts.
                        self.add_formatted_string_tokens(*span, *mode);
                    }
                    Literal::ContentString { mode, .. } => {
                        // Content strings use c/c$/c# prefix instead of f/f$/f#.
                        self.add_content_string_tokens(*span, *mode);
                    }
                    _ => {
                        let token_type = match lit {
                            Literal::Int(_) | Literal::UInt(_) | Literal::TypedInt(_, _) => 10, // number
                            Literal::Number(_) => 10,  // number
                            Literal::Decimal(_) => 10, // number (decimal)
                            Literal::String(_) => 9,   // string
                            Literal::Bool(_) | Literal::None | Literal::Unit => 8, // keyword
                            Literal::Timeframe(_) => 10, // number-like
                            Literal::FormattedString { .. } | Literal::ContentString { .. } => 9, // unreachable in self branch
                        };
                        self.add_token_from_span(*span, token_type, 0);
                    }
                }
            }
            // FromQuery - highlight keywords
            Expr::FromQuery(_, span) => {
                self.add_keyword_token("from", *span);
            }
            // These expression types are handled by their children or don't need tokens
            Expr::StructLiteral {
                type_name, span, ..
            } => {
                let (line, _) = offset_to_line_col(self.source, span.start);
                self.add_ident_token(type_name, 1, 0, line); // type token
            }
            Expr::Await(_, span) => {
                self.add_keyword_token("await", *span);
            }
            Expr::Join(join_expr, span) => {
                // Emit "join" keyword token
                self.add_keyword_token_in_span("join", *span);
                // Emit the strategy keyword (all/race/any/settle)
                let strategy = match join_expr.kind {
                    shape_ast::ast::JoinKind::All => "all",
                    shape_ast::ast::JoinKind::Race => "race",
                    shape_ast::ast::JoinKind::Any => "any",
                    shape_ast::ast::JoinKind::Settle => "settle",
                };
                self.add_keyword_token_in_span(strategy, *span);
                // Emit label tokens for named branches
                let (line, _) = offset_to_line_col(self.source, span.start);
                for branch in &join_expr.branches {
                    if let Some(label) = &branch.label {
                        self.add_ident_token(label, 5, 0, line); // variable token for label
                    }
                }
            }
            Expr::Annotated { annotation, .. } => {
                // Emit DECORATOR token for the annotation
                self.add_token_from_span(annotation.span, 14, 0); // DECORATOR type
            }
            Expr::BinaryOp { .. }
            | Expr::FuzzyComparison { .. }
            | Expr::UnaryOp { .. }
            | Expr::IndexAccess { .. }
            | Expr::Array(_, _)
            | Expr::Conditional { .. }
            | Expr::Block(_, _)
            | Expr::DataRef(_, _)
            | Expr::DataDateTimeRef(_, _)
            | Expr::DataRelativeAccess { .. }
            | Expr::TimeRef(_, _)
            | Expr::DateTime(_, _)
            | Expr::PatternRef(_, _)
            | Expr::ListComprehension(_, _)
            | Expr::TypeAssertion { .. }
            | Expr::InstanceOf { .. }
            | Expr::Duration(_, _)
            | Expr::Spread(_, _)
            | Expr::Assign(_, _)
            | Expr::Unit(_)
            | Expr::Range { .. }
            | Expr::TimeframeContext { .. }
            | Expr::WindowExpr(_, _)
            | Expr::SimulationCall { .. } => {}
            Expr::AsyncLet(async_let, span) => {
                // Highlight "async" keyword at span start
                self.add_keyword_token("async", *span);
                // Highlight "let" keyword within the span
                self.add_keyword_token_in_span("let", *span);
                // Highlight the variable name as a variable declaration
                let (line, _) = offset_to_line_col(self.source, span.start);
                self.add_ident_token(&async_let.name, 5, 1, line); // variable, declaration modifier
            }
            Expr::AsyncScope(_, span) => {
                // Highlight "async" keyword at span start
                self.add_keyword_token("async", *span);
                // Highlight "scope" keyword within the span
                self.add_keyword_token_in_span("scope", *span);
            }
            Expr::Comptime(_, span) => {
                self.add_keyword_token("comptime", *span);
            }
            Expr::ComptimeFor(comptime_for, span) => {
                self.add_keyword_token("comptime", *span);
                self.add_keyword_token_in_span("for", *span);
                // Highlight the loop variable as a variable declaration
                let (line, _) = offset_to_line_col(self.source, span.start);
                self.add_ident_token(&comptime_for.variable, 5, 1, line); // variable, declaration modifier
            }
            Expr::Reference { .. } => {
                // Reference expressions (&expr) - no special token highlighting needed
            }
            Expr::TableRows(..) => {
                // Table row literals — child expressions visited by the walker
            }
        }
        true // Continue visiting children
    }

    fn visit_literal(&mut self, _lit: &Literal) -> bool {
        // Literals are now handled in visit_expr with span information
        // This is called by the visitor but we've already processed in visit_expr
        true
    }

    fn visit_function(&mut self, func: &FunctionDef) -> bool {
        // Highlight annotations as DECORATOR semantic tokens
        for annotation in &func.annotations {
            // The annotation span covers the entire `@name(args)` syntax
            self.add_token_from_span(annotation.span, 14, 0); // DECORATOR type
        }

        true
    }

    fn visit_block(&mut self, block: &shape_ast::ast::BlockExpr) -> bool {
        // Handle block items - expressions are handled by visit_expr through children walk
        // Variable decls and assignments in blocks need identifier highlighting
        for item in &block.items {
            match item {
                BlockItem::VariableDecl(decl) => {
                    // Variable names are highlighted when we visit the expressions
                    if let Some(name) = decl.pattern.as_identifier() {
                        let modifiers = match decl.kind {
                            VarKind::Const => 1 | 4, // DECLARATION | READONLY
                            VarKind::Let => 1 | 4,   // DECLARATION | READONLY
                            VarKind::Var => 1 | 64,  // DECLARATION | MODIFICATION (mutable)
                        };
                        self.add_ident_token(name, 5, modifiers, 0);
                    }
                }
                BlockItem::Assignment(assign) => {
                    if let Some(name) = assign.pattern.as_identifier() {
                        self.add_ident_token(name, 5, 0, 0);
                    }
                }
                BlockItem::Expression(_) => {
                    // Expressions are handled by visit_expr
                }
                BlockItem::Statement(_) => {
                    // Statements are handled by walk_children
                }
            }
        }
        true
    }
}

/// Check if a name is a language-level built-in function
///
/// These are functions provided by the VM runtime, not from stdlib.
/// Stdlib functions are discovered dynamically via annotation/import discovery.
#[allow(dead_code)]
fn is_builtin_function(name: &str) -> bool {
    matches!(
        name,
        "print"
            | "len"
            | "count"
            | "sum"
            | "max"
            | "min"
            | "abs"
            | "sqrt"
            | "ln"
            | "stddev"
            | "highest"
            | "lowest"
            | "first"
            | "last"
            | "range"
            | "push"
            | "where"
            | "shift"
            | "resample"
            | "slice"
            | "fold"
            | "cumsum"
            | "floor"
            | "ceil"
            | "round"
            | "pow"
            | "log"
            | "exp"
            | "sin"
            | "cos"
            | "tan"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn decode_tokens(tokens: &[SemanticToken]) -> Vec<(u32, u32, u32, u32)> {
        let mut decoded = Vec::new();
        let mut line = 0u32;
        let mut col = 0u32;

        for token in tokens {
            line += token.delta_line;
            if token.delta_line == 0 {
                col += token.delta_start;
            } else {
                col = token.delta_start;
            }
            decoded.push((line, col, token.length, token.token_type));
        }

        decoded
    }

    /// Decode tokens with modifiers: (line, col, length, token_type, modifiers)
    fn decode_tokens_full(tokens: &[SemanticToken]) -> Vec<(u32, u32, u32, u32, u32)> {
        let mut decoded = Vec::new();
        let mut line = 0u32;
        let mut col = 0u32;

        for token in tokens {
            line += token.delta_line;
            if token.delta_line == 0 {
                col += token.delta_start;
            } else {
                col = token.delta_start;
            }
            decoded.push((
                line,
                col,
                token.length,
                token.token_type,
                token.token_modifiers_bitset,
            ));
        }

        decoded
    }

    fn token_lexeme(source: &str, token: (u32, u32, u32, u32)) -> Option<String> {
        let (line, col, len, _) = token;
        let line_text = source.lines().nth(line as usize)?;
        let start = col as usize;
        let end = start + len as usize;
        line_text.get(start..end).map(|s| s.to_string())
    }

    fn token_lines_by_type(tokens: &[SemanticToken], wanted_type: u32) -> HashSet<u32> {
        let mut lines = HashSet::new();
        let mut line = 0u32;

        for token in tokens {
            line += token.delta_line;
            if token.token_type == wanted_type && token.length > 0 {
                lines.insert(line);
            }
        }

        lines
    }

    #[test]
    fn test_get_legend() {
        let legend = get_legend();
        assert!(!legend.token_types.is_empty());
        assert!(!legend.token_modifiers.is_empty());
    }

    #[test]
    fn test_simple_tokens() {
        let source = r#"let x = 42;
print("hello");
"#;
        let tokens = get_semantic_tokens(source);
        assert!(tokens.is_some());
        let tokens = tokens.unwrap();
        assert!(!tokens.data.is_empty());
    }

    #[test]
    fn test_function_tokens() {
        let source = r#"function foo(a, b) {
    return a + b;
}
"#;
        let tokens = get_semantic_tokens(source);
        assert!(tokens.is_some());
    }

    #[test]
    fn test_fn_keyword_tokens() {
        let source = r#"fn foo(a, b) {
    return a + b;
}
"#;
        let tokens = get_semantic_tokens(source);
        assert!(tokens.is_some());
    }

    #[test]
    fn test_formatted_string_literal_is_tokenized_as_string() {
        let source = r#"let msg = f"value: {x}";"#;
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        assert!(!tokens.data.is_empty());

        let has_string_token = tokens.data.iter().any(|token| token.token_type == 9);
        assert!(has_string_token, "expected at least one string token");

        let has_variable_token = tokens.data.iter().any(|token| token.token_type == 5);
        assert!(
            has_variable_token,
            "expected variable token for interpolation expression"
        );
    }

    #[test]
    fn test_dollar_formatted_string_literal_is_tokenized_with_expression_tokens() {
        let source = r#"let msg = f$"json: {\"name\": ${user.name}}";"#;
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        assert!(!tokens.data.is_empty());

        let has_string_token = tokens.data.iter().any(|token| token.token_type == 9);
        assert!(has_string_token, "expected at least one string token");

        let has_variable_token = tokens.data.iter().any(|token| token.token_type == 5);
        assert!(
            has_variable_token,
            "expected variable token for interpolation expression in f$ string"
        );
    }

    #[test]
    fn test_fstring_splits_into_segments_no_single_string_token() {
        // The f-string should NOT produce a single STRING token covering the entire literal.
        // Instead, it should split into prefix, text, expression, text, suffix tokens.
        let source = r#"let s = f"value: {x}""#;
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        // Find all string tokens (type 9) on line 0
        let string_tokens: Vec<_> = decoded
            .iter()
            .filter(|&&(line, _col, _len, ty)| line == 0 && ty == 9)
            .collect();

        // Should have multiple string tokens (prefix, text segment, suffix),
        // NOT a single one covering the whole f-string
        assert!(
            string_tokens.len() >= 2,
            "f-string should produce multiple STRING tokens, got {} tokens: {:?}",
            string_tokens.len(),
            string_tokens
        );

        // No single string token should span the entire f-string (f"value: {x}" = 14 chars)
        let has_oversized = string_tokens.iter().any(|&&(_, _, len, _)| len >= 14);
        assert!(
            !has_oversized,
            "no STRING token should cover the entire f-string"
        );
    }

    #[test]
    fn test_fstring_variable_gets_variable_token_not_string() {
        // The variable `x` inside f"...{x}..." should get a VARIABLE token (5), not STRING (9)
        let source = r#"let x = 42
let s = f"val: {x}""#;
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        // Check that there's a variable token (type 5) on line 1 (the f-string line)
        // that corresponds to `x` inside the interpolation
        let var_tokens_line1: Vec<_> = decoded
            .iter()
            .filter(|&&(line, _col, len, ty)| line == 1 && ty == 5 && len == 1)
            .collect();

        assert!(
            !var_tokens_line1.is_empty(),
            "expected variable token for `x` in f-string interpolation on line 1, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_fstring_function_call_gets_function_token() {
        // f"result: {foo(x)}" — `foo` should get a FUNCTION token (4)
        let source = "fn foo(a) { return a }\nlet s = f\"result: {foo(1)}\"";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        // Check for a function token (type 4) on line 1 with length 3 (for "foo")
        let func_tokens_line1: Vec<_> = decoded
            .iter()
            .filter(|&&(line, _col, len, ty)| line == 1 && ty == 4 && len == 3)
            .collect();

        assert!(
            !func_tokens_line1.is_empty(),
            "expected function token for `foo` in f-string interpolation on line 1, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_fstring_property_access_gets_property_token() {
        // f"val: {obj.x}" — `x` should get a PROPERTY token (7)
        let source = "let obj = { x: 1 }\nlet s = f\"val: {obj.x}\"";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        // Check for a property token (type 7) on line 1 with length 1 (for "x")
        let prop_tokens_line1: Vec<_> = decoded
            .iter()
            .filter(|&&(line, _col, len, ty)| line == 1 && ty == 7 && len == 1)
            .collect();

        assert!(
            !prop_tokens_line1.is_empty(),
            "expected property token for `x` in f-string interpolation on line 1, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_fstring_with_format_spec_keeps_expression_tokens() {
        // f"{price:fixed(2)}" should still tokenize `price` as a variable.
        let source = "let price = 12.3\nlet s = f\"price={price:fixed(2)}\"";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        let var_tokens_line1: Vec<_> = decoded
            .iter()
            .filter(|&&(line, _col, len, ty)| line == 1 && ty == 5 && len == 5)
            .collect();

        assert!(
            !var_tokens_line1.is_empty(),
            "expected variable token for `price` in format-spec interpolation, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_triple_string_literal_tokenized_on_all_lines() {
        let source = "let s = \"\"\"\nline1\nline2\n\"\"\";";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let string_lines = token_lines_by_type(&tokens.data, 9);

        assert!(string_lines.contains(&0), "opening line should be string");
        assert!(string_lines.contains(&1), "line1 should be string");
        assert!(string_lines.contains(&2), "line2 should be string");
        assert!(
            string_lines.contains(&3),
            "closing quote line should be string"
        );
    }

    #[test]
    fn test_formatted_triple_string_literal_tokenized_on_all_lines() {
        let source = "let s = f\"\"\"\nvalue: {x}\ndone\n\"\"\";";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let string_lines = token_lines_by_type(&tokens.data, 9);

        assert!(string_lines.contains(&0), "opening line should be string");
        assert!(
            string_lines.contains(&1),
            "interpolation line should be string"
        );
        assert!(string_lines.contains(&2), "middle line should be string");
        assert!(
            string_lines.contains(&3),
            "closing quote line should be string"
        );
    }

    #[test]
    fn test_incomplete_fn_still_highlights_keyword() {
        let source = "fn foo(";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 0 && len == 2 && ty == 8),
            "expected fallback keyword token for `fn`"
        );
    }

    #[test]
    fn test_incomplete_enum_still_highlights_keyword() {
        let source = "enum Signal";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 0 && len == 4 && ty == 8),
            "expected fallback keyword token for `enum`"
        );
    }

    #[test]
    fn test_use_namespace_highlights_use_keyword() {
        let source = "use duckdb";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 0 && len == 3 && ty == 8),
            "expected keyword token for `use`, got: {:?}",
            decoded
        );
    }

    #[test]
    fn test_fallback_keyword_scan_skips_comments_and_strings() {
        let source = "// fn enum\nlet s = \"enum\";\nenum Signal";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);
        let keyword_positions: Vec<(u32, u32, u32)> = decoded
            .iter()
            .filter(|&&(_, _, _, ty)| ty == 8)
            .map(|&(line, col, len, _)| (line, col, len))
            .collect();

        assert!(
            keyword_positions.contains(&(1, 0, 3)),
            "expected `let` keyword token"
        );
        assert!(
            keyword_positions.contains(&(2, 0, 4)),
            "expected `enum` keyword token"
        );
        assert!(
            !keyword_positions.contains(&(0, 3, 2)),
            "did not expect `fn` inside comment to be highlighted as keyword"
        );
        assert!(
            !keyword_positions.contains(&(1, 9, 4)),
            "did not expect `enum` inside string to be highlighted as keyword"
        );
    }

    #[test]
    fn test_offset_to_line_col() {
        let source = "let x = 1;\nlet y = 2;";
        assert_eq!(offset_to_line_col(source, 0), (0, 0));
        assert_eq!(offset_to_line_col(source, 4), (0, 4));
        assert_eq!(offset_to_line_col(source, 11), (1, 0)); // Start of second line
        assert_eq!(offset_to_line_col(source, 15), (1, 4));
    }

    #[test]
    fn test_join_keywords_highlighted() {
        // await join all { ... } should highlight "await", "join", and "all" as keywords
        let source = "async fn foo() {\n  let x = await join all {\n    1,\n    2\n  }\n}";
        let tokens = get_semantic_tokens(source).expect("tokens should be produced");
        let decoded = decode_tokens(&tokens.data);

        let keyword_tokens: Vec<(u32, u32, u32)> = decoded
            .iter()
            .filter(|&&(_, _, _, ty)| ty == 8) // keyword type
            .map(|&(line, col, len, _)| (line, col, len))
            .collect();

        // "await" (5 chars), "join" (4 chars), "all" (3 chars) on line 1
        assert!(
            keyword_tokens.iter().any(|&(l, _, len)| l == 1 && len == 5),
            "expected 'await' keyword token on line 1, got: {:?}",
            keyword_tokens
        );
        assert!(
            keyword_tokens.iter().any(|&(l, _, len)| l == 1 && len == 4),
            "expected 'join' keyword token on line 1, got: {:?}",
            keyword_tokens
        );
        assert!(
            keyword_tokens.iter().any(|&(l, _, len)| l == 1 && len == 3),
            "expected 'all' keyword token on line 1, got: {:?}",
            keyword_tokens
        );
    }

    #[test]
    fn test_fallback_keywords_include_race_and_settle() {
        assert!(is_fallback_keyword("race"));
        assert!(is_fallback_keyword("settle"));
        assert!(is_fallback_keyword("join"));
        assert!(is_fallback_keyword("await"));
        assert!(is_fallback_keyword("async"));
    }

    #[test]
    fn test_mutable_var_gets_modification_modifier() {
        let source = "var x = 1;\nlet y = 2;\nconst z = 3;";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens_full(&tokens.data);

        // Find variable tokens (type 5) for x, y, z
        let var_tokens: Vec<_> = decoded
            .iter()
            .filter(|t| t.3 == 5) // VARIABLE type
            .collect();

        assert!(
            var_tokens.len() >= 3,
            "expected at least 3 variable tokens, got {:?}",
            var_tokens
        );

        // x (var) should have MODIFICATION modifier (bit 6 = 64) + DECLARATION (bit 0 = 1)
        let x_token = var_tokens.iter().find(|t| t.0 == 0 && t.2 == 1);
        assert!(x_token.is_some(), "expected variable token for 'x'");
        assert_eq!(
            x_token.unwrap().4 & 64,
            64,
            "var x should have MODIFICATION modifier"
        );
        assert_eq!(
            x_token.unwrap().4 & 1,
            1,
            "var x should have DECLARATION modifier"
        );

        // y (let) should have READONLY modifier (bit 2 = 4) + DECLARATION (bit 0 = 1)
        let y_token = var_tokens.iter().find(|t| t.0 == 1 && t.2 == 1);
        assert!(y_token.is_some(), "expected variable token for 'y'");
        assert_eq!(
            y_token.unwrap().4 & 4,
            4,
            "let y should have READONLY modifier"
        );
        assert_eq!(
            y_token.unwrap().4 & 1,
            1,
            "let y should have DECLARATION modifier"
        );

        // z (const) should have READONLY modifier (bit 2 = 4) + DECLARATION (bit 0 = 1)
        let z_token = var_tokens.iter().find(|t| t.0 == 2 && t.2 == 1);
        assert!(z_token.is_some(), "expected variable token for 'z'");
        assert_eq!(
            z_token.unwrap().4 & 4,
            4,
            "const z should have READONLY modifier"
        );
    }

    #[test]
    fn test_function_def_gets_declaration_modifier() {
        let source = "fn add(a, b) { return a + b; }";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens_full(&tokens.data);

        // Function name "add" should have FUNCTION type (4) + DECLARATION modifier (bit 0 = 1)
        let func_tokens: Vec<_> = decoded
            .iter()
            .filter(|t| t.3 == 4 && t.2 == 3) // FUNCTION type, length 3 (for "add")
            .collect();
        assert!(
            !func_tokens.is_empty(),
            "expected function token for 'add', decoded: {:?}",
            decoded
        );
        assert_eq!(
            func_tokens[0].4 & 1,
            1,
            "'add' should have DECLARATION modifier"
        );
    }

    #[test]
    fn test_method_call_gets_method_token_type() {
        let source = "let x = [1, 2, 3];\nlet y = x.length();";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        // "length" method call should get METHOD token type (17)
        let method_tokens: Vec<_> = decoded.iter().filter(|t| t.3 == 17).collect();
        assert!(
            !method_tokens.is_empty(),
            "expected METHOD token type for method call, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_trait_gets_interface_token_type() {
        let source = "trait Display {\n  method to_string() { return \"\"; }\n}";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        // "Display" should get INTERFACE token type (15)
        let interface_tokens: Vec<_> = decoded
            .iter()
            .filter(|t| t.3 == 15) // INTERFACE
            .collect();
        assert!(
            !interface_tokens.is_empty(),
            "expected INTERFACE token for trait name, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_struct_type_name_gets_type_token() {
        let source = "type User { name: String }\n";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 5 && len == 4 && ty == 1),
            "expected TYPE token for `User` in type declaration, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_function_return_generic_type_annotation_gets_type_tokens() {
        let source = "fn test() -> Result<int> {\n  return Err(\"x\")\n}\n";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 13 && len == 6 && ty == 1),
            "expected TYPE token for `Result` in return annotation, decoded: {:?}",
            decoded
        );
        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 20 && len == 3 && ty == 1),
            "expected TYPE token for `int` in return annotation, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_type_annotations_highlight_builtin_and_named_types() {
        let source = "type Measurement {\n  value: number,\n}\nfn compute(values: Array<Measurement>) -> Table<Measurement> {\n  let bucket: int = 1\n  return values\n}\n";
        let program = shape_ast::parser::parse_program(source).expect("program should parse");
        let func = match &program.items[1] {
            shape_ast::ast::Item::Function(func, _) => func,
            other => panic!("expected second item to be function, got {:?}", other),
        };
        assert!(
            func.return_type.is_some(),
            "expected function return type to parse"
        );

        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        let type_tokens: Vec<(u32, u32, u32, String)> = decoded
            .iter()
            .filter(|t| t.3 == 1)
            .filter_map(|t| token_lexeme(source, *t).map(|lex| (t.0, t.1, t.2, lex)))
            .collect();
        let type_lexemes: HashSet<String> = decoded
            .iter()
            .filter(|t| t.3 == 1)
            .filter_map(|t| token_lexeme(source, *t))
            .collect();

        for expected in ["number", "Array", "Measurement", "Table", "int"] {
            assert!(
                type_lexemes.contains(expected),
                "expected TYPE token lexeme `{}` in {:?}; type tokens: {:?}",
                expected,
                type_lexemes,
                type_tokens
            );
        }
    }

    #[test]
    fn test_named_impl_highlights_as_keyword_and_impl_name() {
        let source = "impl Display for User as JsonDisplay {\n  method display() { \"x\" }\n}\n";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 22 && len == 2 && ty == 8),
            "expected KEYWORD token for `as`, decoded: {:?}",
            decoded
        );
        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 25 && len == 11 && ty == 1),
            "expected TYPE token for impl name `JsonDisplay`, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_fallback_highlights_name_after_declaration_keyword() {
        // Incomplete enum declaration — fallback scanner should still highlight both "enum" and "Signal"
        let source = "enum Signal";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        // "enum" keyword (type 8)
        assert!(
            decoded
                .iter()
                .any(|&(l, c, len, ty)| l == 0 && c == 0 && len == 4 && ty == 8),
            "expected 'enum' keyword token"
        );

        // "Signal" as ENUM type (type 3) with DECLARATION modifier
        let enum_name_tokens: Vec<_> = decoded
            .iter()
            .filter(|&&(l, _, len, ty)| l == 0 && len == 6 && ty == 3) // ENUM type, length 6
            .collect();
        assert!(
            !enum_name_tokens.is_empty(),
            "expected ENUM token for 'Signal' in fallback mode, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_fallback_highlights_fn_name() {
        let source = "fn compute(";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        // "fn" keyword
        assert!(
            decoded
                .iter()
                .any(|&(l, c, len, ty)| l == 0 && c == 0 && len == 2 && ty == 8),
            "expected 'fn' keyword token"
        );

        // "compute" as FUNCTION type (4)
        let fn_name_tokens: Vec<_> = decoded
            .iter()
            .filter(|&&(_, _, len, ty)| len == 7 && ty == 4) // FUNCTION type, length 7
            .collect();
        assert!(
            !fn_name_tokens.is_empty(),
            "expected FUNCTION token for 'compute' in fallback mode, decoded: {:?}",
            decoded
        );
    }

    #[test]
    fn test_malformed_from_use_keeps_from_keyword_span_precise() {
        let source = "from std.core.snapshot duse { Snapshot }\nlet x = 1\n";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(l, c, len, ty)| l == 0 && c == 0 && len == 4 && ty == 8),
            "expected exact 'from' keyword token (len=4), got: {:?}",
            decoded
        );
        assert!(
            !decoded
                .iter()
                .any(|&(l, c, len, ty)| l == 0 && c == 0 && len > 4 && ty == 8),
            "unexpected oversized keyword token at line start: {:?}",
            decoded
        );
    }

    #[test]
    fn test_fallback_dyn_and_where_keywords() {
        assert!(is_fallback_keyword("dyn"));
        assert!(is_fallback_keyword("where"));
        assert!(is_fallback_keyword("extends"));
    }

    #[test]
    fn test_match_patterns_emit_pattern_and_enum_tokens() {
        let source = "match value {\n  c: int => c + 1\n  Snapshot::Hash(id) => 0\n  _ => 1\n}\n";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(line, _col, len, ty)| line == 1 && len == 1 && ty == 2),
            "expected typed-pattern variable token (CLASS) for `c`, got {:?}",
            decoded
        );
        assert!(
            decoded
                .iter()
                .any(|&(line, _col, len, ty)| line == 1 && len == 3 && ty == 1),
            "expected TYPE token for `int` in typed pattern, got {:?}",
            decoded
        );
        assert!(
            decoded
                .iter()
                .any(|&(line, _col, len, ty)| line == 2 && len == 8 && ty == 3),
            "expected ENUM token for `Snapshot` pattern, got {:?}",
            decoded
        );
        assert!(
            decoded
                .iter()
                .any(|&(line, _col, len, ty)| line == 2 && len == 4 && ty == 16),
            "expected ENUM_MEMBER token for `Hash` pattern, got {:?}",
            decoded
        );
    }

    #[test]
    fn test_parameters_get_parameter_token_type() {
        let source = "fn greet(name, age) { return name; }";
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        // Parameters should get PARAMETER type (6)
        let param_tokens: Vec<_> = decoded.iter().filter(|t| t.3 == 6).collect();
        assert!(
            param_tokens.len() >= 2,
            "expected at least 2 parameter tokens for 'name' and 'age', got {:?}",
            param_tokens
        );
    }

    #[test]
    fn test_content_string_prefix_is_keyword_token() {
        let source = r#"let x = c"hello {name}""#;
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        // The 'c' prefix character should be tagged as KEYWORD (8)
        assert!(
            decoded
                .iter()
                .any(|&(line, _col, len, ty)| line == 0 && len == 1 && ty == 8),
            "expected KEYWORD token for 'c' prefix in content string, got {:?}",
            decoded
        );
        // There should be STRING tokens (9) for the quoted parts
        let string_tokens: Vec<_> = decoded.iter().filter(|t| t.3 == 9).collect();
        assert!(
            !string_tokens.is_empty(),
            "expected STRING tokens in content string"
        );
    }

    #[test]
    fn test_foreign_function_body_is_not_forced_to_string_token() {
        let source = r#"fn python percentile(values: Array<number>, pct: number) -> number {
    sorted_v = sorted(values)
    k = (len(sorted_v) - 1) * (pct / 100.0)
    return k
}"#;
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);

        assert!(
            decoded
                .iter()
                .any(|&(line, col, len, ty)| line == 0 && col == 0 && len == 2 && ty == 8),
            "expected fn keyword token on declaration line"
        );
        assert!(
            !decoded
                .iter()
                .any(|&(line, _, _, ty)| (line == 1 || line == 2 || line == 3) && ty == 9),
            "foreign body lines should not be tagged as STRING tokens, got {:?}",
            decoded
        );
    }

    #[test]
    fn test_frontmatter_foreign_function_keeps_shape_tokens() {
        let source = r#"---
[[extensions]]
name = "python"
path = "./extensions/libshape_ext_python.so"
---
fn python percentile(values: Array<number>, pct: number) -> number {
  return 1
}
"#;
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);
        let lexemes: Vec<(u32, String, u32)> = decoded
            .iter()
            .filter_map(|t| token_lexeme(source, *t).map(|lex| (t.0, lex, t.3)))
            .collect();

        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 5 && lex == "fn" && *ty == 8),
            "expected `fn` keyword token on declaration line, got {:?}",
            lexemes
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 5 && lex == "python" && *ty == 8),
            "expected `python` language token on declaration line, got {:?}",
            lexemes
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 5 && lex == "Array" && *ty == 1),
            "expected `Array` type token on declaration line, got {:?}",
            lexemes
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 5 && lex == "number" && *ty == 1),
            "expected `number` type token on declaration line, got {:?}",
            lexemes
        );
    }

    #[test]
    fn test_async_foreign_function_highlights_async_and_fn_keywords() {
        let source = r#"async fn python fetch_json(url: string) -> Array<number> {
  return []
}
"#;
        let tokens = get_semantic_tokens(source).expect("tokens");
        let decoded = decode_tokens(&tokens.data);
        let lexemes: Vec<(u32, String, u32)> = decoded
            .iter()
            .filter_map(|t| token_lexeme(source, *t).map(|lex| (t.0, lex, t.3)))
            .collect();

        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 0 && lex == "async" && *ty == 8),
            "expected `async` keyword token on declaration line, got {:?}",
            lexemes
        );
        assert!(
            lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 0 && lex == "fn" && *ty == 8),
            "expected `fn` keyword token on declaration line, got {:?}",
            lexemes
        );
        assert!(
            !lexemes
                .iter()
                .any(|(line, lex, ty)| *line == 0 && lex == "as" && *ty == 8),
            "unexpected partial keyword token `as` on declaration line, got {:?}",
            lexemes
        );
    }
}
