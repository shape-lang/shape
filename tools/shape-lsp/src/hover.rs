//! Hover information provider for Shape
//!
//! Provides type information and documentation when hovering over symbols.

use crate::annotation_discovery::AnnotationDiscovery;
use crate::context::{CompletionContext, analyze_context, is_inside_interpolation_expression};
use crate::module_cache::ModuleCache;
use crate::scope::ScopeTree;
use crate::symbols::{SymbolKind, extract_symbols};
use crate::trait_lookup::resolve_trait_definition;
use crate::type_inference::{
    FunctionTypeInfo, ParamReferenceMode, extract_struct_fields,
    infer_block_return_type_via_engine, infer_function_signatures, infer_program_types,
    infer_variable_type, infer_variable_type_for_display, infer_variable_visible_type_at_offset,
    parse_object_shape_fields, resolve_struct_field_type, type_annotation_to_string,
    unified_metadata,
};
use crate::util::{get_word_at_position, offset_to_line_col, parser_source, position_to_offset};
use shape_ast::ast::{Expr, Item, JoinKind, Pattern, Program, Span, Statement, TypeName, VarKind};
use shape_ast::parser::parse_program;
use shape_runtime::metadata::LanguageMetadata;
use shape_runtime::visitor::{Visitor, walk_program};
use std::path::Path;
use tower_lsp_server::ls_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

// Thread-local storage for the cached program fallback.
// This avoids threading the parameter through every internal helper.
std::thread_local! {
    static CACHED_PROGRAM: std::cell::RefCell<Option<Program>> = const { std::cell::RefCell::new(None) };
}

/// Try to parse text, falling back to the thread-local cached program.
fn parse_with_fallback(text: &str) -> Option<Program> {
    let parse_src = parser_source(text);
    let parse_src = parse_src.as_ref();

    match parse_program(parse_src) {
        Ok(p) => Some(p),
        Err(_) => {
            // Try cached program first
            let cached = CACHED_PROGRAM.with(|c| c.borrow().clone());
            if cached.is_some() {
                return cached;
            }
            // Fall back to resilient parser — always succeeds with partial results
            let partial = shape_ast::parser::resilient::parse_program_resilient(parse_src);
            if !partial.items.is_empty() {
                Some(partial.into_program())
            } else {
                None
            }
        }
    }
}

/// Get hover information for a position in the document.
///
/// When `cached_program` is provided, it is used as a fallback AST when
/// the current source text fails to parse (e.g., user is mid-edit).
pub fn get_hover(
    text: &str,
    position: Position,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
    cached_program: Option<&Program>,
) -> Option<Hover> {
    // Set the cached program as fallback for internal helpers
    CACHED_PROGRAM.with(|c| {
        *c.borrow_mut() = cached_program.cloned();
    });

    let result = get_hover_inner(text, position, module_cache, current_file);

    // Clear the cache
    CACHED_PROGRAM.with(|c| {
        *c.borrow_mut() = None;
    });

    result
}

fn get_hover_inner(
    text: &str,
    position: Position,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
) -> Option<Hover> {
    // Get the word at the cursor position
    let word = get_word_at_position(text, position)?;

    // First, check if we're hovering on a property access (e.g., instr.symbol)
    if let Some(hover) = get_property_access_hover(text, &word, position) {
        return Some(hover);
    }
    if let Some(hover) = get_interpolation_self_property_hover(text, &word, position) {
        return Some(hover);
    }

    // Try to find hover information for self word
    if let Some(hover) = get_hover_for_word(text, &word, position, module_cache, current_file) {
        return Some(hover);
    }

    // Check imported symbols via module cache
    if let (Some(cache), Some(file_path)) = (module_cache, current_file) {
        if let Some(hover) = get_imported_symbol_hover(text, &word, cache, file_path) {
            return Some(hover);
        }
    }

    None
}

/// Get hover information for a specific word
fn get_hover_for_word(
    text: &str,
    word: &str,
    position: Position,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
) -> Option<Hover> {
    // Check interpolation format-spec docs first when inside `f"{expr:spec}"`.
    if let Some(hover) = get_interpolation_format_spec_hover(text, word, position) {
        return Some(hover);
    }

    // Check annotations (local and imported via module resolution).
    if let Some(hover) = get_annotation_hover(text, word, position, module_cache, current_file) {
        return Some(hover);
    }

    // Check if we're hovering on a join strategy keyword — show resolved return type
    if matches!(word, "all" | "race" | "any" | "settle") {
        if let Some(hover) = get_join_expression_hover(text, word, position) {
            return Some(hover);
        }
    }

    // Check if hovering on `async` in `async let` or `async scope` context
    if word == "async" {
        if let Some(hover) = get_async_structured_hover(text, position) {
            return Some(hover);
        }
    }

    // Check if hovering on `scope` in `async scope` context
    if word == "scope" {
        if let Some(hover) = get_async_scope_keyword_hover(text, position) {
            return Some(hover);
        }
    }

    // Check if hovering on `comptime` as a block/expression keyword
    if word == "comptime" {
        if let Some(hover) = get_comptime_block_hover(text, position) {
            return Some(hover);
        }
    }

    // Check if hovering on a comptime builtin.
    if let Some(hover) = get_comptime_builtin_hover(word) {
        return Some(hover);
    }

    // `self` inside impl/extend method bodies is an implicit receiver binding.
    if let Some(hover) = get_self_receiver_hover(text, word, position) {
        return Some(hover);
    }

    // Hovering trait name in `impl Trait for Type` should show trait context,
    // even when the trait is not defined in the current file.
    if let Some(hover) =
        get_impl_header_trait_hover(text, word, position, module_cache, current_file)
    {
        return Some(hover);
    }

    // Check Content API namespaces (Content, Color, Border, ChartType, Align)
    if let Some(hover) = get_content_api_hover(word) {
        return Some(hover);
    }

    // Check DateTime / io / time namespaces
    if let Some(hover) = get_namespace_api_hover(word) {
        return Some(hover);
    }

    // Check if it's a keyword
    if let Some(hover) = get_keyword_hover(word) {
        return Some(hover);
    }

    // Check if it's a built-in function
    if let Some(hover) = get_builtin_function_hover(word) {
        return Some(hover);
    }

    // Check if it's a module namespace (extension or local `mod`)
    if let Some(hover) = get_module_hover(text, word) {
        return Some(hover);
    }

    // Check if it's a comptime field (in struct def or type alias override)
    if let Some(hover) = get_comptime_field_hover(text, word, position) {
        return Some(hover);
    }

    // Check if it's a bounded type parameter — show required traits
    if let Some(hover) = get_type_param_hover(text, word, position) {
        return Some(hover);
    }

    // Check if it's a method name inside an impl block — show trait method signature
    if let Some(hover) = get_impl_method_hover(text, word, position, module_cache, current_file) {
        return Some(hover);
    }

    // Check user-defined symbols BEFORE builtin types — prevents false matches
    // from type aliases (e.g., "double" → "float", "record" → "object")
    if let Some(hover) = get_typed_match_pattern_hover(text, word, position) {
        return Some(hover);
    }

    if let Some(hover) = get_user_symbol_hover_at(text, word, position) {
        return Some(hover);
    }

    // Check if it's a built-in type (after user symbols, to avoid alias collisions)
    if let Some(hover) = get_type_hover(word) {
        return Some(hover);
    }

    None
}

fn get_interpolation_format_spec_hover(
    text: &str,
    word: &str,
    position: Position,
) -> Option<Hover> {
    if !matches!(
        analyze_context(text, position),
        CompletionContext::InterpolationFormatSpec { .. }
    ) {
        return None;
    }

    let doc = match word {
        "fixed" => {
            "**Interpolation Spec**: `fixed(precision)`\n\n\
             Formats numeric values using fixed decimal precision.\n\n\
             Example: `f\"price={p:fixed(2)}\"`"
        }
        "table" => {
            "**Interpolation Spec**: `table(...)`\n\n\
             Renders table values with typed configuration.\n\n\
             Supported keys: `max_rows`, `align`, `precision`, `color`, `border`.\n\n\
             Example: `f\"{rows:table(max_rows=20, align=right, precision=2, border=on)}\"`"
        }
        "max_rows" => {
            "**Table Format Key**: `max_rows`\n\n\
             Maximum number of rendered rows.\n\n\
             Example: `table(max_rows=10)`"
        }
        "align" => {
            "**Table Format Key**: `align`\n\n\
             Global cell alignment (`left`, `center`, `right`)."
        }
        "precision" => {
            "**Table Format Key**: `precision`\n\n\
             Numeric precision for floating-point columns."
        }
        "color" => {
            "**Table Format Key**: `color`\n\n\
             Optional color hint (`default`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`)."
        }
        "border" => {
            "**Table Format Key**: `border`\n\n\
             Border mode (`on` or `off`)."
        }
        "left" | "center" | "right" => {
            "**Table Align Enum**\n\n\
             Alignment enum value used by `align=`."
        }
        "default" | "red" | "green" | "yellow" | "blue" | "magenta" | "cyan" | "white" => {
            "**Table Color Enum**\n\n\
             Color enum value used by `color=`."
        }
        "on" | "off" => {
            "**Table Border Enum**\n\n\
             Border toggle value used by `border=`."
        }
        _ => return None,
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.to_string(),
        }),
        range: None,
    })
}

fn span_contains_offset(span: Span, offset: usize) -> bool {
    if span.is_dummy() || span.is_empty() {
        return false;
    }
    offset >= span.start && offset < span.end
}

#[derive(Debug, Clone)]
struct TypedMatchPatternInfo {
    name: String,
    def_span: (usize, usize),
    type_name: String,
}

struct TypedMatchPatternCollector {
    patterns: Vec<TypedMatchPatternInfo>,
}

impl Visitor for TypedMatchPatternCollector {
    fn visit_expr(&mut self, expr: &Expr) -> bool {
        if let Expr::Match(match_expr, _) = expr {
            for arm in &match_expr.arms {
                let Pattern::Typed {
                    name,
                    type_annotation,
                } = &arm.pattern
                else {
                    continue;
                };
                let Some(pattern_span) = arm.pattern_span else {
                    continue;
                };
                if pattern_span.is_dummy() {
                    continue;
                }
                let Some(type_name) = type_annotation_to_string(type_annotation) else {
                    continue;
                };
                let start = pattern_span.start;
                let end = start.saturating_add(name.len());
                self.patterns.push(TypedMatchPatternInfo {
                    name: name.clone(),
                    def_span: (start, end),
                    type_name,
                });
            }
        }
        true
    }
}

fn collect_typed_match_patterns(program: &Program) -> Vec<TypedMatchPatternInfo> {
    let mut collector = TypedMatchPatternCollector {
        patterns: Vec::new(),
    };
    walk_program(&mut collector, program);
    collector.patterns
}

fn get_typed_match_pattern_hover(text: &str, word: &str, position: Position) -> Option<Hover> {
    let mut program = parse_with_fallback(text)?;
    shape_ast::transform::desugar_program(&mut program);

    let patterns = collect_typed_match_patterns(&program);
    if patterns.is_empty() {
        return None;
    }

    let offset = position_to_offset(text, position)?;

    if let Some(info) = patterns
        .iter()
        .find(|p| p.name == word && offset >= p.def_span.0 && offset < p.def_span.1)
    {
        return Some(build_typed_match_pattern_hover(info));
    }

    // For references inside match-arm bodies, resolve lexical binding first.
    let scope_tree = ScopeTree::build(&program, text);
    let binding = scope_tree.binding_at(offset)?;
    if binding.name != word {
        return None;
    }

    let info = patterns.iter().find(|p| p.def_span == binding.def_span)?;
    Some(build_typed_match_pattern_hover(info))
}

fn build_typed_match_pattern_hover(info: &TypedMatchPatternInfo) -> Hover {
    let content = format!(
        "**Variable**: `{}`\n\n**Type:** `{}`",
        info.name, info.type_name
    );

    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    }
}

/// Get hover information for annotation names (`@name`) from local/imported definitions.
fn get_annotation_hover(
    text: &str,
    word: &str,
    position: Position,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
) -> Option<Hover> {
    let offset = position_to_offset(text, position)?;
    let program = parse_with_fallback(text)?;

    let is_definition_name = program.items.iter().any(|item| match item {
        Item::AnnotationDef(annotation_def, _) => {
            annotation_def.name == word && span_contains_offset(annotation_def.name_span, offset)
        }
        _ => false,
    });

    let is_usage_name = is_annotation_word_at_position(text, position);
    if !is_definition_name && !is_usage_name {
        return None;
    }

    let mut discovery = AnnotationDiscovery::new();
    discovery.discover_from_program(&program);
    if let (Some(cache), Some(file_path)) = (module_cache, current_file) {
        discovery.discover_from_imports_with_cache(&program, file_path, cache, None);
    } else {
        discovery.discover_from_imports(&program);
    }

    let info = discovery.get(word)?;
    let mut content = format!("**Annotation**: `@{}`", info.name);
    if !info.params.is_empty() {
        content.push_str(&format!("\n\n**Parameters:** `{}`", info.params.join(", ")));
    }
    if !info.description.is_empty() {
        content.push_str(&format!("\n\n{}", info.description));
    }
    if let Some(source_file) = &info.source_file {
        content.push_str(&format!("\n\n**Defined in:** `{}`", source_file.display()));
    } else {
        content.push_str("\n\n**Defined in:** current file");
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

fn is_annotation_word_at_position(text: &str, position: Position) -> bool {
    let Some(offset) = position_to_offset(text, position) else {
        return false;
    };
    let mut start = offset.min(text.len());

    while start > 0 {
        let ch = text[..start]
            .chars()
            .next_back()
            .expect("slice is non-empty when start > 0");
        if ch.is_ascii_alphanumeric() || ch == '_' {
            start -= ch.len_utf8();
        } else {
            break;
        }
    }

    text[..start].chars().next_back() == Some('@')
}

/// Get hover for Content API namespaces (Content, Color, Border, ChartType, Align)
fn get_content_api_hover(word: &str) -> Option<Hover> {
    let doc = match word {
        "Content" => {
            "**Content API**\n\n\
             Static constructors for building rich content nodes.\n\n\
             **Methods:**\n\
             - `Content.text(string)` — Create a plain text content node\n\
             - `Content.table(data)` — Create a table from a collection\n\
             - `Content.chart(type, data)` — Create a chart\n\
             - `Content.fragment(parts)` — Compose multiple content nodes\n\
             - `Content.code(language, source)` — Create a code block\n\
             - `Content.kv(pairs)` — Create key-value content\n\n\
             Content strings (`c\"...\"`) produce `ContentNode` values that can be \
             styled and composed using the Content API."
        }
        "Color" => {
            "**Color Enum**\n\n\
             Terminal color values for styling content strings.\n\n\
             **Values:**\n\
             - `Color.red`, `Color.green`, `Color.blue`, `Color.yellow`\n\
             - `Color.magenta`, `Color.cyan`, `Color.white`, `Color.default`\n\
             - `Color.rgb(r, g, b)` — Custom RGB color (0-255 per channel)"
        }
        "Border" => {
            "**Border Enum**\n\n\
             Border styles for content tables and panels.\n\n\
             **Values:**\n\
             - `Border.rounded` — Rounded corners (default)\n\
             - `Border.sharp` — Sharp 90-degree corners\n\
             - `Border.heavy` — Thick border lines\n\
             - `Border.double` — Double-line border\n\
             - `Border.minimal` — Minimal separator lines\n\
             - `Border.none` — No border"
        }
        "ChartType" => {
            "**ChartType Enum**\n\n\
             Chart type selectors for `Content.chart()`.\n\n\
             **Values:**\n\
             - `ChartType.line` — Line chart\n\
             - `ChartType.bar` — Bar chart\n\
             - `ChartType.scatter` — Scatter plot\n\
             - `ChartType.area` — Area chart\n\
             - `ChartType.candlestick` — Candlestick chart\n\
             - `ChartType.histogram` — Histogram"
        }
        "Align" => {
            "**Align Enum**\n\n\
             Text alignment for content layout.\n\n\
             **Values:**\n\
             - `Align.left` — Left-aligned (default)\n\
             - `Align.center` — Center-aligned\n\
             - `Align.right` — Right-aligned"
        }
        _ => return None,
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.to_string(),
        }),
        range: None,
    })
}

/// Get hover for Content API member access (e.g., Content.text, Color.red, Border.rounded)
fn get_content_member_hover(object: &str, member: &str) -> Option<Hover> {
    let doc = match (object, member) {
        // Content constructors
        ("Content", "text") => {
            "**Content.text**(string): ContentNode\n\nCreate a plain text content node.\n\n```shape\nContent.text(\"Hello world\")\n```"
        }
        ("Content", "table") => {
            "**Content.table**(data): ContentNode\n\nCreate a table from a collection or array of objects.\n\n```shape\nContent.table(my_data)\n```"
        }
        ("Content", "chart") => {
            "**Content.chart**(type, data): ContentNode\n\nCreate a chart visualization.\n\n```shape\nContent.chart(ChartType.line, series)\n```"
        }
        ("Content", "fragment") => {
            "**Content.fragment**(parts): ContentNode\n\nCompose multiple content nodes into a single fragment.\n\n```shape\nContent.fragment([header, body, footer])\n```"
        }
        ("Content", "code") => {
            "**Content.code**(language, source): ContentNode\n\nCreate a syntax-highlighted code block.\n\n```shape\nContent.code(\"shape\", \"let x = 42\")\n```"
        }
        ("Content", "kv") => {
            "**Content.kv**(pairs): ContentNode\n\nCreate a key-value display from an object.\n\n```shape\nContent.kv({ name: \"test\", value: 42 })\n```"
        }

        // Color values
        ("Color", "red") => "**Color.red**: Color\n\nRed terminal color.",
        ("Color", "green") => "**Color.green**: Color\n\nGreen terminal color.",
        ("Color", "blue") => "**Color.blue**: Color\n\nBlue terminal color.",
        ("Color", "yellow") => "**Color.yellow**: Color\n\nYellow terminal color.",
        ("Color", "magenta") => "**Color.magenta**: Color\n\nMagenta terminal color.",
        ("Color", "cyan") => "**Color.cyan**: Color\n\nCyan terminal color.",
        ("Color", "white") => "**Color.white**: Color\n\nWhite terminal color.",
        ("Color", "default") => {
            "**Color.default**: Color\n\nDefault terminal color (inherits from parent)."
        }
        ("Color", "rgb") => {
            "**Color.rgb**(r, g, b): Color\n\nCustom RGB color. Each component must be 0-255.\n\n```shape\nColor.rgb(255, 128, 0)\n```"
        }

        // Border styles
        ("Border", "rounded") => {
            "**Border.rounded**: Border\n\nRounded corners border style (default).\n```\n\u{256d}\u{2500}\u{2500}\u{2500}\u{256e}\n\u{2502}   \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256f}\n```"
        }
        ("Border", "sharp") => {
            "**Border.sharp**: Border\n\nSharp 90-degree corners.\n```\n\u{250c}\u{2500}\u{2500}\u{2500}\u{2510}\n\u{2502}   \u{2502}\n\u{2514}\u{2500}\u{2500}\u{2500}\u{2518}\n```"
        }
        ("Border", "heavy") => {
            "**Border.heavy**: Border\n\nThick border lines.\n```\n\u{250f}\u{2501}\u{2501}\u{2501}\u{2513}\n\u{2503}   \u{2503}\n\u{2517}\u{2501}\u{2501}\u{2501}\u{251b}\n```"
        }
        ("Border", "double") => {
            "**Border.double**: Border\n\nDouble-line border.\n```\n\u{2554}\u{2550}\u{2550}\u{2550}\u{2557}\n\u{2551}   \u{2551}\n\u{255a}\u{2550}\u{2550}\u{2550}\u{255d}\n```"
        }
        ("Border", "minimal") => "**Border.minimal**: Border\n\nMinimal separator lines only.",
        ("Border", "none") => "**Border.none**: Border\n\nNo border.",

        // ChartType values
        ("ChartType", "line") => {
            "**ChartType.line**: ChartType\n\nLine chart — connects data points with lines."
        }
        ("ChartType", "bar") => {
            "**ChartType.bar**: ChartType\n\nBar chart — vertical bars for each data point."
        }
        ("ChartType", "scatter") => {
            "**ChartType.scatter**: ChartType\n\nScatter plot — individual data points."
        }
        ("ChartType", "area") => {
            "**ChartType.area**: ChartType\n\nArea chart — filled area under a line."
        }
        ("ChartType", "candlestick") => {
            "**ChartType.candlestick**: ChartType\n\nCandlestick chart — OHLC financial data."
        }
        ("ChartType", "histogram") => {
            "**ChartType.histogram**: ChartType\n\nHistogram — frequency distribution of values."
        }

        // Align values
        ("Align", "left") => "**Align.left**: Align\n\nLeft-aligned text (default).",
        ("Align", "center") => "**Align.center**: Align\n\nCenter-aligned text.",
        ("Align", "right") => "**Align.right**: Align\n\nRight-aligned text.",

        _ => return None,
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.to_string(),
        }),
        range: None,
    })
}

/// Get hover for keywords
fn get_keyword_hover(word: &str) -> Option<Hover> {
    let keywords = LanguageMetadata::keywords();
    let keyword = keywords.iter().find(|k| k.keyword == word)?;

    let content = format!(
        "**Keyword**: `{}`\n\n{}",
        keyword.keyword, keyword.description
    );

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

/// Get hover for built-in functions (using unified metadata)
fn get_builtin_function_hover(word: &str) -> Option<Hover> {
    let function = unified_metadata().get_function(word)?;

    let mut content = format!(
        "**Function**: `{}`\n\n{}\n\n**Signature:**\n```shape\n{}\n```",
        function.name, function.description, function.signature
    );

    if !function.parameters.is_empty() {
        content.push_str("\n\n**Parameters:**\n");
        for param in &function.parameters {
            content.push_str(&format!(
                "- `{}`: `{}` - {}\n",
                param.name, param.param_type, param.description
            ));
        }
    }

    content.push_str(&format!("\n**Returns:** `{}`", function.return_type));

    if let Some(example) = &function.example {
        content.push_str(&format!("\n\n**Example:**\n```shape\n{}\n```", example));
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

/// Get hover for types
fn get_type_hover(word: &str) -> Option<Hover> {
    let word = word.trim();
    let types = LanguageMetadata::builtin_types();
    let type_info = types
        .iter()
        .find(|t| t.name == word)
        .or_else(|| types.iter().find(|t| t.name.eq_ignore_ascii_case(word)));

    let (type_name, type_description) = if let Some(info) = type_info {
        (info.name.clone(), info.description.clone())
    } else {
        let (name, description) = fallback_builtin_type_hover(word)?;
        (name.to_string(), description.to_string())
    };

    let content = format!("**Type**: `{}`\n\n{}", type_name, type_description);

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

fn fallback_builtin_type_hover(word: &str) -> Option<(&'static str, &'static str)> {
    match word.to_ascii_lowercase().as_str() {
        "int" | "integer" => Some(("int", "Integer numeric type")),
        "float" | "double" => Some(("float", "Floating-point numeric type")),
        "number" => Some(("number", "Numeric type (integer or floating-point)")),
        "string" | "str" => Some(("string", "String type")),
        "bool" | "boolean" => Some(("bool", "Boolean type (true or false)")),
        "array" => Some(("Array", "Array type")),
        "table" => Some((
            "Table",
            "Typed table container for row-oriented and relational operations",
        )),
        "object" | "record" => Some(("object", "Object type")),
        "datetime" => Some(("DateTime", "Date/time value")),
        "result" => Some(("Result", "Result type - Ok(value) or Err(AnyError)")),
        "option" => Some(("Option", "Option type - Some(value) or None")),
        "anyerror" => Some(("AnyError", "Universal runtime error type used by Result<T>")),
        _ => None,
    }
}

/// Get hover for `async` when used in `async let` or `async scope` context.
fn get_async_structured_hover(text: &str, position: Position) -> Option<Hover> {
    let offset = position_to_offset(text, position)?;
    let program = parse_with_fallback(text)?;

    #[derive(Clone, Copy)]
    enum AsyncHoverKind {
        AsyncLet,
        AsyncScope,
    }

    struct AsyncContextFinder {
        offset: usize,
        best: Option<(usize, AsyncHoverKind)>,
    }

    impl Visitor for AsyncContextFinder {
        fn visit_expr(&mut self, expr: &Expr) -> bool {
            let (kind, span) = match expr {
                Expr::AsyncLet(_, span) => (Some(AsyncHoverKind::AsyncLet), *span),
                Expr::AsyncScope(_, span) => (Some(AsyncHoverKind::AsyncScope), *span),
                _ => (None, Span::DUMMY),
            };

            if let Some(kind) = kind {
                if span_contains_offset(span, self.offset) {
                    let len = span.len();
                    if self
                        .best
                        .map(|(best_len, _)| len < best_len)
                        .unwrap_or(true)
                    {
                        self.best = Some((len, kind));
                    }
                }
            }

            true
        }
    }

    let mut finder = AsyncContextFinder { offset, best: None };
    walk_program(&mut finder, &program);

    match finder.best.map(|(_, kind)| kind) {
        Some(AsyncHoverKind::AsyncLet) => {
            let content = "**Async Let**: `async let name = expr`\n\n\
                Spawns an asynchronous task and binds a future handle to a local variable.\n\n\
                The task begins executing immediately. Use `await name` to retrieve the result.\n\n\
                **Requirements:** Must be used inside an `async` function.\n\n\
                **Example:**\n\
                ```shape\nasync fn fetch_data() {\n  async let a = fetch(\"url1\")\n  async let b = fetch(\"url2\")\n  let results = (await a, await b)\n}\n```";
            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content.to_string(),
                }),
                range: None,
            })
        }
        Some(AsyncHoverKind::AsyncScope) => {
            let content = "**Async Scope**: `async scope { ... }`\n\n\
                Creates a structured concurrency boundary. All tasks spawned inside the scope \
                are automatically cancelled (in LIFO order) when the scope exits.\n\n\
                **Requirements:** Must be used inside an `async` function.\n\n\
                **Example:**\n\
                ```shape\nasync fn process() {\n  async scope {\n    async let a = task1()\n    async let b = task2()\n    await a + await b\n  }\n  // a and b are guaranteed complete or cancelled here\n}\n```";
            Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content.to_string(),
                }),
                range: None,
            })
        }
        None => None,
    }
}

/// Get hover for `scope` keyword when used in `async scope` context.
fn get_async_scope_keyword_hover(text: &str, position: Position) -> Option<Hover> {
    let offset = position_to_offset(text, position)?;
    let program = parse_with_fallback(text)?;

    struct AsyncScopeFinder {
        offset: usize,
        found: bool,
    }

    impl Visitor for AsyncScopeFinder {
        fn visit_expr(&mut self, expr: &Expr) -> bool {
            if let Expr::AsyncScope(_, span) = expr {
                if span_contains_offset(*span, self.offset) {
                    self.found = true;
                }
            }
            true
        }
    }

    let mut finder = AsyncScopeFinder {
        offset,
        found: false,
    };
    walk_program(&mut finder, &program);

    if !finder.found {
        return None;
    }

    let content = "**Scope** (structured concurrency)\n\n\
        The `scope` keyword after `async` creates a structured concurrency boundary.\n\
        All spawned tasks within the scope are tracked and automatically cancelled \
        when the scope exits, ensuring no dangling tasks.";
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content.to_string(),
        }),
        range: None,
    })
}

/// Get hover for `comptime` when used as a block or expression keyword.
///
/// Shows compile-time block info with available builtins when hovering on `comptime`
/// followed by `{` (block context), as opposed to struct field context.
fn get_comptime_block_hover(text: &str, position: Position) -> Option<Hover> {
    let offset = position_to_offset(text, position)?;
    let program = parse_with_fallback(text)?;

    struct ComptimeContextFinder {
        offset: usize,
        found: bool,
    }

    impl Visitor for ComptimeContextFinder {
        fn visit_expr(&mut self, expr: &Expr) -> bool {
            if let Expr::Comptime(_, span) = expr {
                if span_contains_offset(*span, self.offset) {
                    self.found = true;
                }
            }
            true
        }

        fn visit_item(&mut self, item: &Item) -> bool {
            if let Item::Comptime(_, span) = item {
                if span_contains_offset(*span, self.offset) {
                    self.found = true;
                }
            }
            true
        }
    }

    let mut finder = ComptimeContextFinder {
        offset,
        found: false,
    };
    walk_program(&mut finder, &program);

    if !finder.found {
        return None;
    }

    let comptime_builtins: Vec<_> = unified_metadata()
        .all_functions()
        .into_iter()
        .filter(|f| f.comptime_only)
        .collect();
    let builtins_list = if comptime_builtins.is_empty() {
        "- (no comptime intrinsics discovered)".to_string()
    } else {
        comptime_builtins
            .iter()
            .map(|f| format!("- `{}`", f.signature))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let example = comptime_builtins
        .iter()
        .find_map(|f| f.example.as_deref())
        .unwrap_or("let version = comptime { build_config().version }");

    let content = "**Compile-Time Block**: `comptime { }`\n\n\
        Evaluates the enclosed expression at compile time. The result is \
        embedded as a constant in the compiled output.\n\n\
        **Available builtins:**\n\
"
    .to_string()
        + &builtins_list
        + "\n\n\
        **Example:**\n\
        ```shape\n"
        + example
        + "\n```";

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

/// Get hover for comptime builtin functions.
fn get_comptime_builtin_hover(word: &str) -> Option<Hover> {
    let function = unified_metadata()
        .all_functions()
        .into_iter()
        .find(|f| f.comptime_only && f.name == word)?;
    let mut doc = format!(
        "**`{}`**\n\n{}\n\n*Only available inside `comptime {{ }}` blocks.*",
        function.signature, function.description
    );
    if !function.parameters.is_empty() {
        doc.push_str("\n\n**Parameters:**\n");
        for param in &function.parameters {
            doc.push_str(&format!(
                "- `{}`: `{}` - {}\n",
                param.name, param.param_type, param.description
            ));
        }
    }
    if let Some(example) = &function.example {
        doc.push_str(&format!("\n**Example:**\n```shape\n{}\n```", example));
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc,
        }),
        range: None,
    })
}

/// Get hover for a comptime field name.
///
/// Shows the comptime field's type, default value, and resolved value when inside a type alias
/// override (e.g., `type EUR = Currency { symbol: "EUR" }`).
fn get_comptime_field_hover(text: &str, word: &str, position: Position) -> Option<Hover> {
    let program = parse_with_fallback(text)?;
    let offset = position_to_offset(text, position)?;

    // Check if cursor is inside a type alias override:
    // `type EUR = Currency { symbol: ... }`
    for item in &program.items {
        let Item::TypeAlias(alias_def, alias_span) = item else {
            continue;
        };
        if !span_contains_offset(*alias_span, offset) {
            continue;
        }
        let shape_ast::ast::TypeAnnotation::Basic(base_type) = &alias_def.type_annotation else {
            continue;
        };

        for item in &program.items {
            if let Item::StructType(struct_def, _) = item {
                if struct_def.name == *base_type {
                    for field in &struct_def.fields {
                        if field.name == word && field.is_comptime {
                            let type_str = type_annotation_to_string(&field.type_annotation)
                                .unwrap_or_else(|| "unknown".to_string());
                            let default_str = field
                                .default_value
                                .as_ref()
                                .map(format_expr_short)
                                .unwrap_or_else(|| "none".to_string());

                            let content = format!(
                                "**Comptime Field**: `{}`\n\n**Type:** `{}`\n**Default:** `{}`\n\nCompile-time constant field of type `{}`",
                                word, type_str, default_str, base_type
                            );
                            return Some(Hover {
                                contents: HoverContents::Markup(MarkupContent {
                                    kind: MarkupKind::Markdown,
                                    value: content,
                                }),
                                range: None,
                            });
                        }
                    }
                }
            }
        }
    }

    // Check if cursor is on a comptime field inside a struct type definition
    for item in &program.items {
        if let Item::StructType(struct_def, span) = item {
            if span_contains_offset(*span, offset) {
                for field in &struct_def.fields {
                    if field.name == word && field.is_comptime {
                        let type_str = type_annotation_to_string(&field.type_annotation)
                            .unwrap_or_else(|| "unknown".to_string());
                        let default_str = field
                            .default_value
                            .as_ref()
                            .map(format_expr_short)
                            .unwrap_or_else(|| "none".to_string());

                        let content = format!(
                            "**Comptime Field**: `{}`\n\n**Type:** `{}`\n**Default:** `{}`\n\nCompile-time constant field of type `{}`. Resolved at compile time — zero runtime cost.",
                            word, type_str, default_str, struct_def.name
                        );
                        return Some(Hover {
                            contents: HoverContents::Markup(MarkupContent {
                                kind: MarkupKind::Markdown,
                                value: content,
                            }),
                            range: None,
                        });
                    }
                }
            }
        }
    }

    None
}

/// Format an expression as a short string for display in hover
fn format_expr_short(expr: &Expr) -> String {
    match expr {
        Expr::Literal(lit, _) => match lit {
            shape_ast::ast::Literal::String(s) => format!("\"{}\"", s),
            shape_ast::ast::Literal::Number(n) => format!("{}", n),
            shape_ast::ast::Literal::Int(n) => format!("{}", n),
            shape_ast::ast::Literal::Decimal(d) => format!("{}D", d),
            shape_ast::ast::Literal::Bool(b) => format!("{}", b),
            shape_ast::ast::Literal::None => "None".to_string(),
            _ => "...".to_string(),
        },
        _ => "...".to_string(),
    }
}

/// Get hover for a bounded type parameter.
///
/// When the cursor is on a type parameter name (e.g., `T` in `fn foo<T: Comparable>`),
/// shows the required trait bounds.
fn get_type_param_hover(text: &str, word: &str, position: Position) -> Option<Hover> {
    let offset = position_to_offset(text, position)?;
    let program = parse_with_fallback(text)?;

    for item in &program.items {
        let (type_params, span) = match item {
            Item::Function(func, span) => (func.type_params.as_ref(), *span),
            Item::Trait(trait_def, span) => (trait_def.type_params.as_ref(), *span),
            _ => (None, Span::DUMMY),
        };

        if !span_contains_offset(span, offset) {
            continue;
        }

        if let Some(params) = type_params {
            for tp in params {
                if tp.name == word && !tp.trait_bounds.is_empty() {
                    let bounds_str = tp.trait_bounds.join(" + ");
                    let content = format!(
                        "**Type Parameter**: `{}`\n\n**Bounds:** `{}: {}`\n\nMust implement: {}",
                        word,
                        word,
                        bounds_str,
                        tp.trait_bounds
                            .iter()
                            .map(|b| format!("`{}`", b))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    return Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: content,
                        }),
                        range: None,
                    });
                }
            }
        }
    }

    None
}

/// Get hover for a method name inside an impl block.
///
/// When the cursor is on a method name within `impl Trait for Type { method foo(...) { ... } }`,
/// self shows the trait method signature.
fn get_impl_method_hover(
    text: &str,
    word: &str,
    position: Position,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
) -> Option<Hover> {
    use crate::type_inference::type_annotation_to_string;

    let offset = position_to_offset(text, position)?;
    let program = parse_with_fallback(text)?;

    let mut selected_impl: Option<(&shape_ast::ast::ImplBlock, Span)> = None;
    for item in &program.items {
        let Item::Impl(impl_block, span) = item else {
            continue;
        };
        if !span_contains_offset(*span, offset) {
            continue;
        }
        let is_method_name = impl_block.methods.iter().any(|method| method.name == word);
        if !is_method_name {
            continue;
        }

        if selected_impl
            .map(|(_, current_span)| span.len() < current_span.len())
            .unwrap_or(true)
        {
            selected_impl = Some((impl_block, *span));
        }
    }

    let (impl_block, _) = selected_impl?;
    let trait_name = type_name_base_name(&impl_block.trait_name);
    let target_type = type_name_base_name(&impl_block.target_type);
    if trait_name.is_empty() {
        return None;
    }

    if let Some(resolved_trait) =
        resolve_trait_definition(&program, &trait_name, module_cache, current_file, None)
    {
        for member in &resolved_trait.trait_def.members {
            match member {
                shape_ast::ast::TraitMember::Required(
                    shape_ast::ast::InterfaceMember::Method {
                        name,
                        params,
                        return_type,
                        ..
                    },
                ) if name == word => {
                    let param_names: Vec<String> = params
                        .iter()
                        .map(|p| {
                            let pname = p.name.clone().unwrap_or_else(|| "_".to_string());
                            let ptype = type_annotation_to_string(&p.type_annotation)
                                .unwrap_or_else(|| "any".to_string());
                            format!("{}: {}", pname, ptype)
                        })
                        .collect();
                    let return_type_str =
                        type_annotation_to_string(return_type).unwrap_or_else(|| "any".to_string());
                    let signature =
                        format!("{}({}): {}", name, param_names.join(", "), return_type_str);
                    let mut content = format!(
                        "**Trait Method**: `{}`\n\n**Trait:** `{}`\n**Target:** `{}`\n\n**Signature:**\n```shape\n{}\n```",
                        name, trait_name, target_type, signature
                    );
                    if let Some(impl_name) = &impl_block.impl_name {
                        content.push_str(&format!("\n\n**Implementation:** `{}`", impl_name));
                    }
                    return Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: content,
                        }),
                        range: None,
                    });
                }
                shape_ast::ast::TraitMember::Default(method_def) if method_def.name == word => {
                    let param_names: Vec<String> = method_def
                        .params
                        .iter()
                        .map(|p| p.simple_name().unwrap_or("_").to_string())
                        .collect();

                    let return_type_str = method_def
                        .return_type
                        .as_ref()
                        .and_then(type_annotation_to_string)
                        .unwrap_or_else(|| "any".to_string());

                    let signature = format!(
                        "{}({}): {}",
                        method_def.name,
                        param_names.join(", "),
                        return_type_str
                    );

                    let mut content = format!(
                        "**Trait Method** (default): `{}`\n\n**Trait:** `{}`\n**Target:** `{}`\n\nThis method has a default implementation and does not need to be overridden.\n\n**Signature:**\n```shape\n{}\n```",
                        method_def.name, trait_name, target_type, signature
                    );
                    if let Some(impl_name) = &impl_block.impl_name {
                        content.push_str(&format!("\n\n**Implementation:** `{}`", impl_name));
                    }

                    return Some(Hover {
                        contents: HoverContents::Markup(MarkupContent {
                            kind: MarkupKind::Markdown,
                            value: content,
                        }),
                        range: None,
                    });
                }
                _ => {}
            }
        }
    }

    // Fallback: trait definition may live in another module; still provide
    // method-level hover from the impl body itself.
    if let Some(method_def) = impl_block.methods.iter().find(|method| method.name == word) {
        let param_names: Vec<String> = method_def
            .params
            .iter()
            .map(|p| {
                let pname = p.simple_name().unwrap_or("_").to_string();
                let ptype = p
                    .type_annotation
                    .as_ref()
                    .and_then(type_annotation_to_string);
                match ptype {
                    Some(t) => format!("{}: {}", pname, t),
                    None => pname,
                }
            })
            .collect();

        let return_type_str = method_def
            .return_type
            .as_ref()
            .and_then(type_annotation_to_string)
            .or_else(|| infer_block_return_type_via_engine(&method_def.body))
            .unwrap_or_else(|| "unknown".to_string());

        let signature = format!(
            "{}({}): {}",
            method_def.name,
            param_names.join(", "),
            return_type_str
        );

        let mut content = format!(
            "**Method**: `{}`\n\n**Trait:** `{}`\n**Target:** `{}`\n\n**Signature:**\n```shape\n{}\n```",
            method_def.name, trait_name, target_type, signature
        );
        if let Some(impl_name) = &impl_block.impl_name {
            content.push_str(&format!("\n\n**Implementation:** `{}`", impl_name));
        }

        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        });
    }

    None
}

fn method_body_contains_offset(method: &shape_ast::ast::MethodDef, offset: usize) -> bool {
    method
        .body
        .iter()
        .any(|stmt| statement_contains_offset(stmt, offset))
}

fn statement_contains_offset(stmt: &Statement, offset: usize) -> bool {
    match stmt {
        Statement::Return(_, span)
        | Statement::Break(span)
        | Statement::Continue(span)
        | Statement::VariableDecl(_, span)
        | Statement::Assignment(_, span)
        | Statement::Expression(_, span)
        | Statement::Extend(_, span)
        | Statement::RemoveTarget(span)
        | Statement::SetParamType { span, .. }
        | Statement::SetReturnType { span, .. } => span_contains_offset(*span, offset),
        Statement::SetReturnExpr { span, .. } => span_contains_offset(*span, offset),
        Statement::ReplaceModuleExpr { span, .. } => span_contains_offset(*span, offset),
        Statement::ReplaceBodyExpr { span, .. } => span_contains_offset(*span, offset),
        Statement::ReplaceBody { body, span } => {
            span_contains_offset(*span, offset)
                || body
                    .iter()
                    .any(|nested| statement_contains_offset(nested, offset))
        }
        Statement::For(for_stmt, span) => {
            span_contains_offset(*span, offset)
                || for_stmt
                    .body
                    .iter()
                    .any(|nested| statement_contains_offset(nested, offset))
        }
        Statement::While(while_stmt, span) => {
            span_contains_offset(*span, offset)
                || while_stmt
                    .body
                    .iter()
                    .any(|nested| statement_contains_offset(nested, offset))
        }
        Statement::If(if_stmt, span) => {
            span_contains_offset(*span, offset)
                || if_stmt
                    .then_body
                    .iter()
                    .any(|nested| statement_contains_offset(nested, offset))
                || if_stmt.else_body.as_ref().is_some_and(|else_body| {
                    else_body
                        .iter()
                        .any(|nested| statement_contains_offset(nested, offset))
                })
        }
    }
}

fn receiver_type_at_offset(program: &Program, offset: usize) -> Option<String> {
    let mut best: Option<(usize, String)> = None;

    for item in &program.items {
        match item {
            Item::Impl(impl_block, span) if span_contains_offset(*span, offset) => {
                if !impl_block
                    .methods
                    .iter()
                    .any(|method| method_body_contains_offset(method, offset))
                {
                    continue;
                }
                let target_type = type_name_base_name(&impl_block.target_type);
                if target_type.is_empty() {
                    continue;
                }
                let len = span.len();
                if best
                    .as_ref()
                    .map(|(best_len, _)| len < *best_len)
                    .unwrap_or(true)
                {
                    best = Some((len, target_type));
                }
            }
            Item::Extend(extend_stmt, span) if span_contains_offset(*span, offset) => {
                if !extend_stmt
                    .methods
                    .iter()
                    .any(|method| method_body_contains_offset(method, offset))
                {
                    continue;
                }
                let target_type = type_name_base_name(&extend_stmt.type_name);
                if target_type.is_empty() {
                    continue;
                }
                let len = span.len();
                if best
                    .as_ref()
                    .map(|(best_len, _)| len < *best_len)
                    .unwrap_or(true)
                {
                    best = Some((len, target_type));
                }
            }
            _ => {}
        }
    }

    best.map(|(_, ty)| ty)
}

fn get_self_receiver_hover(text: &str, word: &str, position: Position) -> Option<Hover> {
    if word != "self" {
        return None;
    }

    let offset = position_to_offset(text, position)?;
    let mut program = parse_with_fallback(text)?;
    shape_ast::transform::desugar_program(&mut program);

    struct SelfUseFinder {
        offset: usize,
        found: bool,
    }

    impl Visitor for SelfUseFinder {
        fn visit_expr(&mut self, expr: &Expr) -> bool {
            if let Expr::Identifier(name, span) = expr {
                if name == "self" && span_contains_offset(*span, self.offset) {
                    self.found = true;
                }
            }
            true
        }
    }

    let mut finder = SelfUseFinder {
        offset,
        found: false,
    };
    walk_program(&mut finder, &program);
    if !finder.found && !is_inside_interpolation_expression(text, position) {
        return None;
    }

    let receiver_type = receiver_type_at_offset(&program, offset)?;
    let content = format!(
        "**Variable**: `self`\n\n**Type:** `{}`\n\nImplicit method receiver.",
        receiver_type
    );

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

fn get_interpolation_self_property_hover(
    text: &str,
    hovered_word: &str,
    position: Position,
) -> Option<Hover> {
    if !is_inside_interpolation_expression(text, position) {
        return None;
    }

    let offset = position_to_offset(text, position)?;
    if !is_hovering_self_property(text, offset, hovered_word) {
        return None;
    }

    let mut program = parse_with_fallback(text)?;
    shape_ast::transform::desugar_program(&mut program);

    let receiver_type = receiver_type_at_offset(&program, offset)?;
    let field_type = extract_struct_fields(&program)
        .get(&receiver_type)
        .and_then(|fields| {
            fields
                .iter()
                .find(|(name, _)| name == hovered_word)
                .map(|(_, ty)| ty.clone())
        })
        .unwrap_or_else(|| "unknown".to_string());

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!(
                "**Property**: `{}`\n\n**Type:** `{}`\n\n**Receiver:** `{}`",
                hovered_word, field_type, receiver_type
            ),
        }),
        range: None,
    })
}

fn is_hovering_self_property(text: &str, offset: usize, hovered_word: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() || offset > bytes.len() {
        return false;
    }

    let mut start = offset;
    while start > 0 {
        let ch = bytes[start - 1];
        if (ch as char).is_ascii_alphanumeric() || ch == b'_' {
            start -= 1;
        } else {
            break;
        }
    }

    let mut end = offset;
    while end < bytes.len() {
        let ch = bytes[end];
        if (ch as char).is_ascii_alphanumeric() || ch == b'_' {
            end += 1;
        } else {
            break;
        }
    }

    if start >= end {
        return false;
    }

    if text.get(start..end) != Some(hovered_word) {
        return false;
    }

    if start < 5 {
        return false;
    }

    let self_start = start - 5;
    if text.get(self_start..start) != Some("self.") {
        return false;
    }

    if self_start == 0 {
        return true;
    }

    let prev = bytes[self_start - 1];
    !((prev as char).is_ascii_alphanumeric() || prev == b'_')
}

fn get_impl_header_trait_hover(
    text: &str,
    word: &str,
    position: Position,
    module_cache: Option<&ModuleCache>,
    current_file: Option<&Path>,
) -> Option<Hover> {
    let offset = position_to_offset(text, position)?;
    let program = parse_with_fallback(text)?;

    let mut selected_impl: Option<(&shape_ast::ast::ImplBlock, Span)> = None;
    for item in &program.items {
        let Item::Impl(impl_block, span) = item else {
            continue;
        };
        if !span_contains_offset(*span, offset) {
            continue;
        }

        let trait_name = type_name_base_name(&impl_block.trait_name);
        if trait_name != word {
            continue;
        }

        if selected_impl
            .map(|(_, current_span)| span.len() < current_span.len())
            .unwrap_or(true)
        {
            selected_impl = Some((impl_block, *span));
        }
    }

    let (impl_block, _) = selected_impl?;
    let trait_name = type_name_base_name(&impl_block.trait_name);
    let target_type = type_name_base_name(&impl_block.target_type);

    let resolved =
        resolve_trait_definition(&program, &trait_name, module_cache, current_file, None);

    let mut content = format!(
        "**Trait**: `{}`\n\n**Target:** `{}`",
        trait_name, target_type
    );
    if let Some(resolved_trait) = resolved {
        if let Some(source) = &resolved_trait.source_text {
            if let Some((line, _)) = (!resolved_trait.span.is_dummy())
                .then(|| offset_to_line_col(source, resolved_trait.span.start))
            {
                if let Some(doc) = extract_comment_block(source, line as usize) {
                    content.push_str(&format!("\n\n{}", doc));
                }
            }
        }

        if let Some(import_path) = &resolved_trait.import_path {
            content.push_str(&format!("\n\n**Resolved from:** `{}`", import_path));
        } else {
            content.push_str("\n\nResolved from current file.");
        }

        let signatures = trait_member_signatures(&resolved_trait.trait_def);
        if !signatures.is_empty() {
            content.push_str("\n\n**Members:**\n```shape\n");
            for sig in signatures {
                content.push_str(&sig);
                content.push('\n');
            }
            content.push_str("```");
        }
    } else {
        content.push_str("\n\nTrait definition not found in current module context.");
    }

    if let Some(impl_name) = &impl_block.impl_name {
        content.push_str(&format!("\n\n**Implementation:** `{}`", impl_name));
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

fn trait_member_signatures(trait_def: &shape_ast::ast::TraitDef) -> Vec<String> {
    let mut signatures = Vec::new();

    for member in &trait_def.members {
        match member {
            shape_ast::ast::TraitMember::Required(shape_ast::ast::InterfaceMember::Method {
                name,
                params,
                return_type,
                ..
            }) => {
                let param_names: Vec<String> = params
                    .iter()
                    .map(|p| {
                        let pname = p.name.clone().unwrap_or_else(|| "_".to_string());
                        let ptype = type_annotation_to_string(&p.type_annotation)
                            .unwrap_or_else(|| "unknown".to_string());
                        format!("{}: {}", pname, ptype)
                    })
                    .collect();
                let return_type_str =
                    type_annotation_to_string(return_type).unwrap_or_else(|| "unknown".to_string());
                signatures.push(format!(
                    "{}({}): {}",
                    name,
                    param_names.join(", "),
                    return_type_str
                ));
            }
            shape_ast::ast::TraitMember::Default(method_def) => {
                let param_names: Vec<String> = method_def
                    .params
                    .iter()
                    .map(|p| p.simple_name().unwrap_or("_").to_string())
                    .collect();
                let return_type_str = method_def
                    .return_type
                    .as_ref()
                    .and_then(type_annotation_to_string)
                    .unwrap_or_else(|| "unknown".to_string());
                signatures.push(format!(
                    "{}({}): {}",
                    method_def.name,
                    param_names.join(", "),
                    return_type_str
                ));
            }
            _ => {}
        }
    }

    signatures
}

fn extract_comment_block(source: &str, target_line: usize) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    if target_line == 0 || target_line > lines.len() {
        return None;
    }

    let mut comment_lines = Vec::new();
    let mut i = target_line.saturating_sub(1);
    loop {
        let trimmed = lines[i].trim();
        if trimmed.starts_with("///") || trimmed.starts_with("//") {
            let content = trimmed
                .trim_start_matches("///")
                .trim_start_matches("//")
                .trim();
            comment_lines.push(content.to_string());
        } else if trimmed.is_empty() {
            if i == 0 {
                break;
            }
            i -= 1;
            continue;
        } else {
            break;
        }

        if i == 0 {
            break;
        }
        i -= 1;
    }

    if comment_lines.is_empty() {
        return None;
    }
    comment_lines.reverse();
    Some(comment_lines.join("\n"))
}

/// Extract doc comments (`///` or `/** */`) above a given line in the source text.
///
/// Returns the doc comment body with leading `///` stripped and trimmed,
/// or `None` if no doc comment is found.
pub fn extract_doc_comment(source: &str, target_line: usize) -> Option<String> {
    let lines: Vec<&str> = source.lines().collect();
    if target_line == 0 || target_line > lines.len() {
        return None;
    }

    let mut doc_lines = Vec::new();

    // Walk backwards from the line above target_line
    let mut i = target_line.saturating_sub(1);
    loop {
        let trimmed = lines[i].trim();

        if trimmed.starts_with("///") {
            // Line doc comment — strip the `///` prefix
            let content = trimmed.strip_prefix("///").unwrap_or("");
            let content = content.strip_prefix(' ').unwrap_or(content);
            doc_lines.push(content.to_string());
        } else if trimmed.starts_with("/**") && trimmed.ends_with("*/") {
            // Single-line block doc comment
            let content = trimmed
                .strip_prefix("/**")
                .unwrap_or("")
                .strip_suffix("*/")
                .unwrap_or("")
                .trim();
            if !content.is_empty() {
                doc_lines.push(content.to_string());
            }
        } else if trimmed.ends_with("*/") {
            // End of a multiline block doc comment — scan upward for `/**`
            let end = i;
            while i > 0 {
                i -= 1;
                let t = lines[i].trim();
                if t.starts_with("/**") {
                    // Collect all lines between /** and */
                    for j in i..=end {
                        let line_text = lines[j].trim();
                        let line_text = line_text.strip_prefix("/**").unwrap_or(line_text);
                        let line_text = line_text.strip_suffix("*/").unwrap_or(line_text);
                        let line_text = line_text
                            .strip_prefix("* ")
                            .unwrap_or(line_text.strip_prefix('*').unwrap_or(line_text));
                        let line_text = line_text.trim();
                        if !line_text.is_empty() {
                            doc_lines.push(line_text.to_string());
                        }
                    }
                    break;
                }
            }
        } else if trimmed.is_empty() {
            // Skip blank lines between doc comment and definition
            if i == 0 {
                break;
            }
            i -= 1;
            continue;
        } else {
            // Hit non-comment content — stop
            break;
        }

        if i == 0 {
            break;
        }
        i -= 1;
    }

    if doc_lines.is_empty() {
        return None;
    }

    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Find the 0-based line number where a symbol is defined in the source text.
fn type_name_base_name(type_name: &TypeName) -> String {
    match type_name {
        TypeName::Simple(name) => name.clone(),
        TypeName::Generic { name, .. } => name.clone(),
    }
}

fn binding_span_for_symbol(
    pattern: &shape_ast::ast::DestructurePattern,
    symbol_name: &str,
) -> Option<Span> {
    pattern.get_bindings().into_iter().find_map(|(name, span)| {
        if name == symbol_name {
            Some(span)
        } else {
            None
        }
    })
}

fn find_definition_line_in_program(
    program: &Program,
    source: &str,
    symbol_name: &str,
    kind: &SymbolKind,
) -> Option<usize> {
    for item in &program.items {
        match item {
            Item::Function(func, item_span)
                if *kind == SymbolKind::Function && func.name == symbol_name =>
            {
                let span = if !func.name_span.is_dummy() {
                    func.name_span
                } else {
                    *item_span
                };
                return Some(offset_to_line_col(source, span.start).0 as usize);
            }
            Item::VariableDecl(decl, item_span)
                if matches!(kind, SymbolKind::Variable | SymbolKind::Constant) =>
            {
                let Some(binding_span) = binding_span_for_symbol(&decl.pattern, symbol_name) else {
                    continue;
                };
                let kind_matches = match kind {
                    SymbolKind::Variable => matches!(decl.kind, VarKind::Let | VarKind::Var),
                    SymbolKind::Constant => matches!(decl.kind, VarKind::Const),
                    _ => false,
                };
                if !kind_matches {
                    continue;
                }
                let span = if binding_span.is_dummy() {
                    *item_span
                } else {
                    binding_span
                };
                return Some(offset_to_line_col(source, span.start).0 as usize);
            }
            Item::Statement(Statement::VariableDecl(decl, stmt_span), item_span)
                if matches!(kind, SymbolKind::Variable | SymbolKind::Constant) =>
            {
                let Some(binding_span) = binding_span_for_symbol(&decl.pattern, symbol_name) else {
                    continue;
                };
                let kind_matches = match kind {
                    SymbolKind::Variable => matches!(decl.kind, VarKind::Let | VarKind::Var),
                    SymbolKind::Constant => matches!(decl.kind, VarKind::Const),
                    _ => false,
                };
                if !kind_matches {
                    continue;
                }
                let span = if binding_span.is_dummy() {
                    if stmt_span.is_dummy() {
                        *item_span
                    } else {
                        *stmt_span
                    }
                } else {
                    binding_span
                };
                return Some(offset_to_line_col(source, span.start).0 as usize);
            }
            Item::TypeAlias(alias, span)
                if *kind == SymbolKind::Type && alias.name == symbol_name =>
            {
                return Some(offset_to_line_col(source, span.start).0 as usize);
            }
            Item::StructType(struct_def, span)
                if *kind == SymbolKind::Type && struct_def.name == symbol_name =>
            {
                return Some(offset_to_line_col(source, span.start).0 as usize);
            }
            Item::Enum(enum_def, span)
                if *kind == SymbolKind::Type && enum_def.name == symbol_name =>
            {
                return Some(offset_to_line_col(source, span.start).0 as usize);
            }
            Item::Interface(interface_def, span)
                if *kind == SymbolKind::Type && interface_def.name == symbol_name =>
            {
                return Some(offset_to_line_col(source, span.start).0 as usize);
            }
            Item::Trait(trait_def, span)
                if *kind == SymbolKind::Type && trait_def.name == symbol_name =>
            {
                return Some(offset_to_line_col(source, span.start).0 as usize);
            }
            Item::ForeignFunction(foreign_fn, span)
                if *kind == SymbolKind::Function && foreign_fn.name == symbol_name =>
            {
                let name_span = if !foreign_fn.name_span.is_dummy() {
                    foreign_fn.name_span
                } else {
                    *span
                };
                return Some(offset_to_line_col(source, name_span.start).0 as usize);
            }
            _ => {}
        }
    }

    None
}

/// Get hover for user-defined symbols
#[cfg(test)]
fn get_user_symbol_hover(text: &str, word: &str) -> Option<Hover> {
    let mut program = parse_with_fallback(text)?;
    // Desugar query syntax before analysis
    shape_ast::transform::desugar_program(&mut program);
    get_user_symbol_hover_from_program(text, &program, word, None)
}

/// Get hover for user-defined symbols with scope-aware resolution at cursor position.
fn get_user_symbol_hover_at(text: &str, word: &str, position: Position) -> Option<Hover> {
    let mut program = parse_with_fallback(text)?;
    // Desugar query syntax before analysis
    shape_ast::transform::desugar_program(&mut program);

    let offset = position_to_offset(text, position)?;
    if let Some(hover) = get_scoped_binding_hover(&program, text, word, offset) {
        return Some(hover);
    }

    get_user_symbol_hover_from_program(text, &program, word, Some(offset))
}

fn get_scoped_binding_hover(
    program: &Program,
    text: &str,
    word: &str,
    offset: usize,
) -> Option<Hover> {
    let scope_tree = ScopeTree::build(program, text);
    let binding = scope_tree.binding_at(offset)?;
    if binding.name != word {
        return None;
    }
    get_function_param_hover(program, binding.def_span, &binding.name)
}

fn get_function_param_hover(
    program: &Program,
    def_span: (usize, usize),
    name: &str,
) -> Option<Hover> {
    let function_sigs = infer_function_signatures(program);
    for item in &program.items {
        let (params, func_name): (&[shape_ast::ast::FunctionParameter], &str) = match item {
            Item::Function(func, _) => (&func.params, &func.name),
            Item::ForeignFunction(foreign_fn, _) => (&foreign_fn.params, &foreign_fn.name),
            _ => continue,
        };

        for param in params {
            let param_span = param.span();
            if param_span.is_dummy()
                || param_span.start != def_span.0
                || param_span.end != def_span.1
            {
                continue;
            }

            let Some(param_name) = param.simple_name() else {
                continue;
            };
            if param_name != name {
                continue;
            }

            let type_name = param
                .type_annotation
                .as_ref()
                .and_then(type_annotation_to_string)
                .or_else(|| {
                    function_sigs.get(func_name).and_then(|info| {
                        info.param_types
                            .iter()
                            .find(|(param, _)| param == param_name)
                            .map(|(_, ty)| ty.clone())
                    })
                });
            let ref_mode = function_sigs
                .get(func_name)
                .and_then(|info| info.param_ref_modes.get(param_name));

            let mut content = format!("**Variable**: `{}`", param_name);
            if let Some(type_name) = type_name {
                let display_type = format_reference_aware_type(&type_name, ref_mode);
                content.push_str(&format!("\n\n**Type:** `{}`", display_type));
            }

            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            });
        }
    }

    None
}

fn get_user_symbol_hover_from_program(
    text: &str,
    program: &Program,
    word: &str,
    cursor_offset: Option<usize>,
) -> Option<Hover> {
    // Parse the document to extract symbols
    let symbols = extract_symbols(program);

    // Find the symbol
    let symbol = symbols.iter().find(|s| s.name == word)?;

    let kind_name = match symbol.kind {
        SymbolKind::Variable => "Variable",
        SymbolKind::Constant => "Constant",
        SymbolKind::Function => "Function",
        SymbolKind::Type => "Type",
    };

    let mut content = format!("**{}**: `{}`", kind_name, symbol.name);

    // Run program-level type inference for best results
    let program_types = infer_program_types(program);
    let function_sigs = infer_function_signatures(program);

    // Show type annotation for variables/constants
    // Priority: explicit annotation > engine-inferred > heuristic
    let type_str = if let Some(type_ann) = &symbol.type_annotation {
        Some(type_ann.clone())
    } else if matches!(symbol.kind, SymbolKind::Variable | SymbolKind::Constant) {
        if let Some(offset) = cursor_offset {
            infer_variable_type_for_display(program, word, offset).or_else(|| {
                choose_best_variable_type(
                    program_types.get(word).cloned(),
                    infer_variable_type(program, word),
                )
            })
        } else {
            choose_best_variable_type(
                program_types.get(word).cloned(),
                infer_variable_type(program, word),
            )
        }
    } else {
        None
    };

    if let Some(type_ann) = type_str {
        content.push_str(&format!("\n\n**Type:** `{}`", type_ann));
    }

    if symbol.kind == SymbolKind::Type {
        let struct_fields = extract_struct_fields(program);
        if let Some(fields) = struct_fields.get(word) {
            if !fields.is_empty() {
                let shape = fields
                    .iter()
                    .map(|(name, ty)| format!("{}: {}", name, ty))
                    .collect::<Vec<_>>()
                    .join(", ");
                content.push_str(&format!("\n\n**Shape:** `{{ {} }}`", shape));
            }
        }
    }

    // Show annotations generically
    if !symbol.annotations.is_empty() {
        content.push_str("\n\n**Annotations:**\n");
        for ann in &symbol.annotations {
            content.push_str(&format!("- `@{}`\n", ann));
        }
    }

    // Show signature for functions with inferred types
    if matches!(symbol.kind, SymbolKind::Function) {
        if let Some(sig_info) = function_sigs.get(word) {
            // Build an enhanced signature with inferred return type
            let sig = build_function_signature_from_inference(
                program,
                word,
                sig_info,
                symbol.detail.as_deref(),
            );
            if let Some(sig) = sig {
                content.push_str(&format!("\n\n**Signature:**\n```shape\n{}\n```", sig));
            }
        } else if let Some(detail) = &symbol.detail {
            content.push_str(&format!("\n\n**Signature:**\n```shape\n{}\n```", detail));
        }
    }

    // Show doc comments extracted from source text
    let doc = symbol.documentation.clone().or_else(|| {
        let def_line = find_definition_line_in_program(program, text, word, &symbol.kind)?;
        extract_doc_comment(text, def_line)
    });
    if let Some(doc) = doc {
        content.push_str(&format!("\n\n---\n\n{}", doc));
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

fn choose_best_variable_type(primary: Option<String>, secondary: Option<String>) -> Option<String> {
    match (primary, secondary) {
        (Some(primary), Some(secondary)) => {
            if should_prefer_secondary_type(&primary, &secondary) {
                Some(secondary)
            } else {
                Some(primary)
            }
        }
        (Some(primary), None) => Some(primary),
        (None, secondary) => secondary,
    }
}

fn should_prefer_secondary_type(primary: &str, secondary: &str) -> bool {
    let primary = primary.trim();
    let secondary = secondary.trim();
    if (primary.eq_ignore_ascii_case("object") && secondary.starts_with('{'))
        || primary == "unknown"
    {
        return true;
    }

    if primary.starts_with('{') && secondary.starts_with('{') {
        let primary_len = parse_object_shape_fields(primary)
            .map(|fields| fields.len())
            .unwrap_or(0);
        let secondary_len = parse_object_shape_fields(secondary)
            .map(|fields| fields.len())
            .unwrap_or(0);
        return secondary_len > primary_len;
    }

    false
}

fn is_primitive_value_type_name(name: &str) -> bool {
    let normalized = name.trim().trim_end_matches('?');
    matches!(
        normalized,
        "int"
            | "integer"
            | "i64"
            | "number"
            | "float"
            | "f64"
            | "decimal"
            | "bool"
            | "boolean"
            | "()"
            | "void"
            | "unit"
            | "none"
            | "null"
            | "undefined"
            | "never"
    )
}

fn split_top_level_union(type_str: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut angle_depth = 0usize;

    for (idx, ch) in type_str.char_indices() {
        match ch {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            _ => {}
        }

        if ch == '|'
            && paren_depth == 0
            && bracket_depth == 0
            && brace_depth == 0
            && angle_depth == 0
        {
            parts.push(type_str[start..idx].trim().to_string());
            start = idx + ch.len_utf8();
        }
    }

    parts.push(type_str[start..].trim().to_string());
    parts.into_iter().filter(|part| !part.is_empty()).collect()
}

fn apply_ref_prefix(type_str: &str, mode: &ParamReferenceMode) -> String {
    let trimmed = type_str.trim();
    if trimmed.starts_with('&') {
        trimmed.to_string()
    } else {
        format!("{}{}", mode.prefix(), trimmed)
    }
}

fn format_reference_aware_type(type_str: &str, mode: Option<&ParamReferenceMode>) -> String {
    let Some(mode) = mode else {
        return type_str.to_string();
    };

    let union_parts = split_top_level_union(type_str);
    if union_parts.len() <= 1 {
        return apply_ref_prefix(type_str, mode);
    }

    union_parts
        .into_iter()
        .map(|part| {
            if is_primitive_value_type_name(&part) {
                part
            } else {
                apply_ref_prefix(&part, mode)
            }
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Build an enhanced function signature using inferred type information.
///
/// Combines the function's AST definition with engine-inferred parameter/return types.
fn build_function_signature_from_inference(
    program: &Program,
    func_name: &str,
    sig_info: &FunctionTypeInfo,
    _fallback_detail: Option<&str>,
) -> Option<String> {
    // Find the function definition in the AST — regular or foreign
    enum FuncKind<'a> {
        Regular(&'a shape_ast::ast::FunctionDef),
        Foreign(&'a shape_ast::ast::ForeignFunctionDef),
    }

    let func_kind = program.items.iter().find_map(|item| match item {
        Item::Function(f, _) if f.name == func_name => Some(FuncKind::Regular(f)),
        Item::ForeignFunction(f, _) if f.name == func_name => Some(FuncKind::Foreign(f)),
        _ => None,
    })?;

    let ast_params = match &func_kind {
        FuncKind::Regular(f) => f.params.as_slice(),
        FuncKind::Foreign(f) => f.params.as_slice(),
    };

    // Build parameter list with inferred types where available
    let params: Vec<String> = ast_params
        .iter()
        .map(|p| {
            let name = p.simple_name().unwrap_or("_");
            let ref_mode = sig_info.param_ref_modes.get(name);
            if let Some(type_ann) = &p.type_annotation {
                let type_str =
                    type_annotation_to_string(type_ann).unwrap_or_else(|| "any".to_string());
                let display_type = format_reference_aware_type(&type_str, ref_mode);
                format!("{}: {}", name, display_type)
            } else if let Some((_, inferred)) = sig_info.param_types.iter().find(|(n, _)| n == name)
            {
                let display_type = format_reference_aware_type(inferred, ref_mode);
                format!("{}: {}", name, display_type)
            } else if let Some(ref_mode) = ref_mode {
                format!("{}: {}unknown", name, ref_mode.prefix())
            } else {
                name.to_string()
            }
        })
        .collect();

    // Build return type string
    let return_str = match &func_kind {
        FuncKind::Foreign(f) => f.return_type.as_ref().and_then(type_annotation_to_string),
        FuncKind::Regular(f) => {
            if let Some(ref rt) = f.return_type {
                type_annotation_to_string(rt)
            } else {
                sig_info.return_type.clone()
            }
        }
    };

    let mut sig = match &func_kind {
        FuncKind::Regular(_) => format!("fn {}({})", func_name, params.join(", ")),
        FuncKind::Foreign(f) => format!("fn {} {}({})", f.language, func_name, params.join(", ")),
    };
    if let Some(ret) = return_str {
        let display = crate::type_inference::simplify_result_type(&ret);
        sig.push_str(&format!(" -> {}", display));
    }

    Some(sig)
}

/// Get hover for symbols imported from other modules
fn get_imported_symbol_hover(
    text: &str,
    word: &str,
    module_cache: &ModuleCache,
    current_file: &Path,
) -> Option<Hover> {
    use crate::module_cache::SymbolKind as ModSymbolKind;
    use shape_ast::ast::{EnumMemberKind, ExportItem};

    // Parse the current file to find import statements
    let program = parse_with_fallback(text)?;

    for item in &program.items {
        if let Item::Import(import_stmt, _) = item {
            let resolved = module_cache.resolve_import(&import_stmt.from, current_file, None)?;
            let module_info =
                module_cache.load_module_with_context(&resolved, current_file, None)?;

            // Check if the word matches any exported symbol from self module
            for export in &module_info.exports {
                if export.exported_name() != word {
                    continue;
                }

                // Found a match — build hover content based on kind
                let content = match export.kind {
                    ModSymbolKind::Enum => {
                        // Find the enum definition in the module's AST for details
                        let mut detail = format!(
                            "**Enum**: `{}`\n\n*Imported from `{}`*",
                            word, import_stmt.from
                        );
                        for module_item in module_info.program.items.iter() {
                            let enum_def = match module_item {
                                Item::Export(e, _) => {
                                    if let ExportItem::Enum(ed) = &e.item {
                                        Some(ed)
                                    } else {
                                        None
                                    }
                                }
                                Item::Enum(ed, _) => Some(ed),
                                _ => None,
                            };
                            if let Some(ed) = enum_def {
                                if ed.name == word {
                                    detail.push_str("\n\n**Variants:**\n```shape\nenum ");
                                    detail.push_str(&ed.name);
                                    detail.push_str(" {\n");
                                    for m in &ed.members {
                                        detail.push_str("    ");
                                        detail.push_str(&m.name);
                                        match &m.kind {
                                            EnumMemberKind::Unit { .. } => {}
                                            EnumMemberKind::Tuple(types) => {
                                                detail.push('(');
                                                let type_strs: Vec<String> = types
                                                    .iter()
                                                    .map(|t| format!("{:?}", t))
                                                    .collect();
                                                detail.push_str(&type_strs.join(", "));
                                                detail.push(')');
                                            }
                                            EnumMemberKind::Struct(fields) => {
                                                detail.push_str(" { ");
                                                let field_strs: Vec<String> = fields
                                                    .iter()
                                                    .map(|f| {
                                                        format!(
                                                            "{}: {:?}",
                                                            f.name, f.type_annotation
                                                        )
                                                    })
                                                    .collect();
                                                detail.push_str(&field_strs.join(", "));
                                                detail.push_str(" }");
                                            }
                                        }
                                        detail.push_str(",\n");
                                    }
                                    detail.push_str("}\n```");
                                    break;
                                }
                            }
                        }
                        detail
                    }
                    ModSymbolKind::Function => {
                        // Find function signature
                        let mut detail = format!(
                            "**Function**: `{}`\n\n*Imported from `{}`*",
                            word, import_stmt.from
                        );
                        for module_item in module_info.program.items.iter() {
                            let func_def = match module_item {
                                Item::Export(e, _) => {
                                    if let ExportItem::Function(fd) = &e.item {
                                        Some(fd)
                                    } else {
                                        None
                                    }
                                }
                                Item::Function(fd, _) => Some(fd),
                                _ => None,
                            };
                            if let Some(fd) = func_def {
                                if fd.name == word {
                                    let params: Vec<String> = fd
                                        .params
                                        .iter()
                                        .map(|p| {
                                            let name = p.simple_name().unwrap_or("_");
                                            if let Some(ref ty) = p.type_annotation {
                                                format!("{}: {:?}", name, ty)
                                            } else {
                                                name.to_string()
                                            }
                                        })
                                        .collect();
                                    detail.push_str(&format!(
                                        "\n\n**Signature:**\n```shape\nfn {}({})",
                                        word,
                                        params.join(", ")
                                    ));
                                    if let Some(ref rt) = fd.return_type {
                                        detail.push_str(&format!(": {:?}", rt));
                                    }
                                    detail.push_str("\n```");
                                    break;
                                }
                            }
                        }
                        detail
                    }
                    _ => {
                        format!(
                            "**{}**: `{}`\n\n*Imported from `{}`*",
                            match export.kind {
                                ModSymbolKind::Variable => "Variable",
                                ModSymbolKind::TypeAlias => "Type",
                                ModSymbolKind::Interface => "Interface",
                                ModSymbolKind::Pattern => "Pattern",
                                ModSymbolKind::Annotation => "Annotation",
                                _ => "Symbol",
                            },
                            word,
                            import_stmt.from
                        )
                    }
                };

                // Also try to extract doc comment from the module source
                let mut full_content = content;
                if let Ok(module_source) = std::fs::read_to_string(&resolved) {
                    let symbol_kind = match export.kind {
                        ModSymbolKind::Function | ModSymbolKind::Pattern => SymbolKind::Function,
                        ModSymbolKind::Enum | ModSymbolKind::TypeAlias => SymbolKind::Type,
                        _ => SymbolKind::Variable,
                    };
                    if let Some(def_line) = find_definition_line_in_program(
                        &module_info.program,
                        &module_source,
                        word,
                        &symbol_kind,
                    ) {
                        if let Some(doc) = extract_doc_comment(&module_source, def_line) {
                            full_content.push_str(&format!("\n\n---\n\n{}", doc));
                        }
                    }
                }

                return Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: full_content,
                    }),
                    range: None,
                });
            }
        }
    }

    None
}

/// Get hover for a module name (extension module or local `mod`).
/// Get hover for DateTime / io / time namespaces
fn get_namespace_api_hover(word: &str) -> Option<Hover> {
    let doc = match word {
        "DateTime" => {
            "**DateTime API**\n\n\
             Static constructors for creating date/time values.\n\n\
             **Constructors:**\n\
             - `DateTime.now()` — Current local time\n\
             - `DateTime.utc()` — Current UTC time\n\
             - `DateTime.parse(string)` — Parse from ISO 8601, RFC 2822, or common formats\n\
             - `DateTime.from_epoch(ms)` — From milliseconds since Unix epoch\n\n\
             **Instance Methods:**\n\
             - `.year()`, `.month()`, `.day()`, `.hour()`, `.minute()`, `.second()`\n\
             - `.day_of_week()`, `.day_of_year()`, `.week_of_year()`\n\
             - `.is_weekday()`, `.is_weekend()`\n\
             - `.format(pattern)`, `.iso8601()`, `.rfc2822()`, `.unix_timestamp()`\n\
             - `.to_utc()`, `.to_timezone(tz)`, `.to_local()`, `.timezone()`, `.offset()`\n\
             - `.add_days(n)`, `.add_hours(n)`, `.add_minutes(n)`, `.add_seconds(n)`, `.add_months(n)`\n\
             - `.is_before(other)`, `.is_after(other)`, `.is_same_day(other)`"
        }
        "io" => {
            "**io Module**\n\n\
             File system, network, and process operations.\n\n\
             **File Operations:**\n\
             - `io.open(path, mode?)` — Open a file (`\"r\"`, `\"w\"`, `\"a\"`, `\"rw\"`)\n\
             - `io.read(handle, n?)`, `io.read_to_string(handle)`, `io.read_bytes(handle, n?)`\n\
             - `io.write(handle, data)`, `io.flush(handle)`, `io.close(handle)`\n\
             - `io.exists(path)`, `io.stat(path)`, `io.mkdir(path)`, `io.remove(path)`, `io.rename(from, to)`\n\
             - `io.read_dir(path)`\n\n\
             **Path Operations:**\n\
             - `io.join(base, path)`, `io.dirname(path)`, `io.basename(path)`\n\
             - `io.extension(path)`, `io.resolve(path)`\n\n\
             **Network:**\n\
             - `io.tcp_connect(addr)`, `io.tcp_listen(addr)`, `io.tcp_accept(listener)`\n\
             - `io.tcp_read(handle)`, `io.tcp_write(handle, data)`, `io.tcp_close(handle)`\n\
             - `io.udp_bind(addr)`, `io.udp_send(handle, data, addr)`, `io.udp_recv(handle)`\n\n\
             **Process:**\n\
             - `io.spawn(program, args?)`, `io.exec(program, args?)`\n\
             - `io.stdin()`, `io.stdout()`, `io.stderr()`, `io.read_line(handle?)`"
        }
        "time" => {
            "**time Module**\n\n\
             Precision timing utilities.\n\n\
             **Functions:**\n\
             - `time.now()` — Current monotonic instant for measuring elapsed time\n\
             - `time.sleep(ms)` — Sleep for ms milliseconds (async)\n\
             - `time.sleep_sync(ms)` — Sleep for ms milliseconds (blocking)\n\
             - `time.benchmark(fn, iterations?)` — Benchmark a function\n\
             - `time.stopwatch()` — Start a stopwatch (returns Instant)\n\
             - `time.millis()` — Current wall-clock time as epoch milliseconds"
        }
        _ => return None,
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.to_string(),
        }),
        range: None,
    })
}

/// Get hover for DateTime / io / time member access (e.g., DateTime.now, io.open, time.sleep)
fn get_namespace_member_hover(object: &str, member: &str) -> Option<Hover> {
    let doc = match (object, member) {
        // DateTime constructors
        ("DateTime", "now") => {
            "**DateTime.now**(): DateTime\n\nReturn the current local time as a DateTime value.\n\n```shape\nlet now = DateTime.now()\nprint(now.format(\"%Y-%m-%d %H:%M\"))\n```"
        }
        ("DateTime", "utc") => {
            "**DateTime.utc**(): DateTime\n\nReturn the current UTC time.\n\n```shape\nlet utc = DateTime.utc()\n```"
        }
        ("DateTime", "parse") => {
            "**DateTime.parse**(string): DateTime\n\nParse a date/time string. Supports ISO 8601, RFC 2822, and common formats.\n\n```shape\nlet dt = DateTime.parse(\"2024-03-15T10:30:00Z\")\nlet dt2 = DateTime.parse(\"Mar 15, 2024 10:30 AM\")\n```"
        }
        ("DateTime", "from_epoch") => {
            "**DateTime.from_epoch**(ms: number): DateTime\n\nCreate a DateTime from milliseconds since Unix epoch.\n\n```shape\nlet dt = DateTime.from_epoch(1710500000000)\n```"
        }

        // io file operations
        ("io", "open") => {
            "**io.open**(path: string, mode?: string): IoHandle\n\nOpen a file and return a handle.\n\nModes: `\"r\"` (read, default), `\"w\"` (write/create), `\"a\"` (append), `\"rw\"` (read-write).\n\n```shape\nlet f = io.open(\"data.csv\")\nlet f = io.open(\"output.txt\", \"w\")\n```"
        }
        ("io", "read") => {
            "**io.read**(handle: IoHandle, n?: int): string\n\nRead from a file handle. If `n` is given, read up to `n` bytes; otherwise read all."
        }
        ("io", "read_to_string") => {
            "**io.read_to_string**(handle: IoHandle): string\n\nRead the entire file contents as a string."
        }
        ("io", "write") => {
            "**io.write**(handle: IoHandle, data: string): unit\n\nWrite a string to a file handle."
        }
        ("io", "close") => {
            "**io.close**(handle: IoHandle): unit\n\nClose a file handle, releasing the resource."
        }
        ("io", "flush") => "**io.flush**(handle: IoHandle): unit\n\nFlush buffered writes to disk.",
        ("io", "exists") => {
            "**io.exists**(path: string): bool\n\nCheck if a file or directory exists at the given path."
        }
        ("io", "stat") => {
            "**io.stat**(path: string): object\n\nGet file metadata: `{ size, modified, is_dir, is_file }`."
        }
        ("io", "mkdir") => {
            "**io.mkdir**(path: string): unit\n\nCreate a directory (and any missing parent directories)."
        }
        ("io", "remove") => {
            "**io.remove**(path: string): unit\n\nRemove a file or empty directory."
        }
        ("io", "rename") => {
            "**io.rename**(from: string, to: string): unit\n\nRename or move a file or directory."
        }
        ("io", "read_dir") => {
            "**io.read_dir**(path: string): Array<object>\n\nList directory entries as objects with `name`, `path`, `is_dir`, `is_file`."
        }
        ("io", "join") => {
            "**io.join**(base: string, path: string): string\n\nJoin two path components."
        }
        ("io", "dirname") => {
            "**io.dirname**(path: string): string\n\nGet the parent directory of a path."
        }
        ("io", "basename") => {
            "**io.basename**(path: string): string\n\nGet the file name component of a path."
        }
        ("io", "extension") => {
            "**io.extension**(path: string): string\n\nGet the file extension (without the dot)."
        }
        ("io", "resolve") => {
            "**io.resolve**(path: string): string\n\nResolve a path to an absolute path."
        }
        ("io", "tcp_connect") => {
            "**io.tcp_connect**(addr: string): IoHandle\n\nConnect to a TCP server at `addr` (e.g., `\"127.0.0.1:8080\"`)."
        }
        ("io", "tcp_listen") => {
            "**io.tcp_listen**(addr: string): IoHandle\n\nBind a TCP listener on `addr`."
        }
        ("io", "tcp_accept") => {
            "**io.tcp_accept**(listener: IoHandle): IoHandle\n\nAccept a new TCP connection from a listener."
        }
        ("io", "tcp_read") => {
            "**io.tcp_read**(handle: IoHandle): string\n\nRead from a TCP stream."
        }
        ("io", "tcp_write") => {
            "**io.tcp_write**(handle: IoHandle, data: string): unit\n\nWrite to a TCP stream."
        }
        ("io", "tcp_close") => {
            "**io.tcp_close**(handle: IoHandle): unit\n\nClose a TCP connection."
        }
        ("io", "udp_bind") => {
            "**io.udp_bind**(addr: string): IoHandle\n\nBind a UDP socket on `addr`."
        }
        ("io", "udp_send") => {
            "**io.udp_send**(handle: IoHandle, data: string, addr: string): unit\n\nSend a UDP datagram."
        }
        ("io", "udp_recv") => {
            "**io.udp_recv**(handle: IoHandle): object\n\nReceive a UDP datagram, returning `{ data, addr }`."
        }
        ("io", "spawn") => {
            "**io.spawn**(program: string, args?: Array<string>): IoHandle\n\nSpawn a child process. Returns a handle for reading/writing to its stdin/stdout.\n\n```shape\nlet proc = io.spawn(\"ls\", [\"-la\"])\n```"
        }
        ("io", "exec") => {
            "**io.exec**(program: string, args?: Array<string>): object\n\nExecute a command and wait for completion. Returns `{ stdout, stderr, exit_code }`.\n\n```shape\nlet result = io.exec(\"echo\", [\"hello\"])\nprint(result.stdout)\n```"
        }
        ("io", "stdin") => "**io.stdin**(): IoHandle\n\nOpen standard input as a readable handle.",
        ("io", "stdout") => {
            "**io.stdout**(): IoHandle\n\nOpen standard output as a writable handle."
        }
        ("io", "stderr") => {
            "**io.stderr**(): IoHandle\n\nOpen standard error as a writable handle."
        }
        ("io", "read_line") => {
            "**io.read_line**(handle?: IoHandle): string\n\nRead a single line from a handle (or stdin if no handle given)."
        }

        // time module
        ("time", "now") => {
            "**time.now**(): Instant\n\nReturn the current monotonic instant for measuring elapsed time.\n\n```shape\nlet start = time.now()\n// ... work ...\nprint(start.elapsed())\n```"
        }
        ("time", "sleep") => {
            "**time.sleep**(ms: number): unit\n\nSleep for the specified number of milliseconds. **Async** — must be awaited.\n\n```shape\nawait time.sleep(100)\n```"
        }
        ("time", "sleep_sync") => {
            "**time.sleep_sync**(ms: number): unit\n\nSleep for the specified number of milliseconds (blocking, for non-async contexts)."
        }
        ("time", "benchmark") => {
            "**time.benchmark**(fn: function, iterations?: int): object\n\nBenchmark a function over N iterations (default 1000).\n\nReturns `{ elapsed_ms, iterations, avg_ms }`."
        }
        ("time", "stopwatch") => {
            "**time.stopwatch**(): Instant\n\nStart a stopwatch. Call `.elapsed()` on the returned Instant to read elapsed time."
        }
        ("time", "millis") => {
            "**time.millis**(): number\n\nReturn current wall-clock time as milliseconds since Unix epoch."
        }

        _ => return None,
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: doc.to_string(),
        }),
        range: None,
    })
}

fn get_module_hover(text: &str, word: &str) -> Option<Hover> {
    let registry = crate::completion::imports::get_registry();
    if let Some(module) = registry.get(word) {
        let mut content = format!("**Module**: `{}`\n\n{}", module.name, module.description);

        let exports = module.export_names_public_surface(false);
        if !exports.is_empty() {
            content.push_str("\n\n**Exports:**\n");
            for name in &exports {
                if let Some(schema) = module.get_schema(&name) {
                    let params: Vec<String> = schema
                        .params
                        .iter()
                        .map(|p| format!("{}: {}", p.name, p.type_name))
                        .collect();
                    content.push_str(&format!(
                        "- `{}({})`{}\n",
                        name,
                        params.join(", "),
                        schema
                            .return_type
                            .as_ref()
                            .map(|r| format!(" -> {}", r))
                            .unwrap_or_default()
                    ));
                } else {
                    content.push_str(&format!("- `{}`\n", name));
                }
            }
        }

        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        });
    }

    let local_module =
        crate::completion::imports::local_module_schema_from_source(word, Some(text))?;
    let mut content = format!(
        "**Module**: `{}`\n\nLocal module defined in this file.",
        word
    );
    if !local_module.functions.is_empty() {
        content.push_str("\n\n**Exports:**\n");
        for function in &local_module.functions {
            let params = function
                .params
                .iter()
                .map(|param| {
                    if param.required {
                        format!("{}: {}", param.name, param.type_name)
                    } else {
                        format!("{}?: {}", param.name, param.type_name)
                    }
                })
                .collect::<Vec<_>>();
            content.push_str(&format!(
                "- `{}({})`{}\n",
                function.name,
                params.join(", "),
                function
                    .return_type
                    .as_ref()
                    .map(|ret| format!(" -> {}", ret))
                    .unwrap_or_default()
            ));
        }
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

/// Get hover for a module member (e.g., "load" in "csv.load").
fn get_module_member_hover(text: &str, module_name: &str, member_name: &str) -> Option<Hover> {
    let registry = crate::completion::imports::get_registry();
    if let Some(module) = registry.get(module_name)
        && let Some(schema) = module.get_schema(member_name)
    {
        let params: Vec<String> = schema
            .params
            .iter()
            .map(|p| {
                if p.required {
                    format!("{}: {}", p.name, p.type_name)
                } else {
                    format!("{}?: {}", p.name, p.type_name)
                }
            })
            .collect();

        let sig = format!("{}.{}({})", module_name, member_name, params.join(", "));

        let mut content = format!("**Function**: `{}`\n\n{}", sig, schema.description);

        if !schema.params.is_empty() {
            content.push_str("\n\n**Parameters:**\n");
            for p in &schema.params {
                let req = if p.required { "" } else { " (optional)" };
                content.push_str(&format!(
                    "- `{}`: `{}` — {}{}\n",
                    p.name, p.type_name, p.description, req
                ));
            }
        }

        if let Some(ref return_type) = schema.return_type {
            content.push_str(&format!("\n**Returns:** `{}`", return_type));
        }

        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        });
    }

    let local_function = crate::completion::imports::local_module_function_schema_from_source(
        module_name,
        member_name,
        Some(text),
    )?;
    let params = local_function
        .params
        .iter()
        .map(|param| {
            if param.required {
                format!("{}: {}", param.name, param.type_name)
            } else {
                format!("{}?: {}", param.name, param.type_name)
            }
        })
        .collect::<Vec<_>>();
    let sig = format!("{}.{}({})", module_name, member_name, params.join(", "));

    let mut content = format!("**Function**: `{}`\n\nLocal module function.", sig);
    if let Some(return_type) = &local_function.return_type {
        content.push_str(&format!("\n\n**Returns:** `{}`", return_type));
    }

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

/// Get hover for property access expressions (e.g., instr.symbol)
fn get_property_access_hover(text: &str, hovered_word: &str, position: Position) -> Option<Hover> {
    let cursor_offset = position_to_offset(text, position)?;
    let mut program = parse_with_fallback(text)?;
    // Desugar query syntax before analysis
    shape_ast::transform::desugar_program(&mut program);

    struct PropertyAccessFinder<'a> {
        hovered_word: &'a str,
        offset: usize,
        best: Option<(usize, String, String)>, // (span_len, object_name, property)
    }

    impl<'a> Visitor for PropertyAccessFinder<'a> {
        fn visit_expr(&mut self, expr: &Expr) -> bool {
            // Extract (object, property, span) from PropertyAccess or MethodCall
            let (object, property, span) = match expr {
                Expr::PropertyAccess {
                    object,
                    property,
                    span,
                    ..
                } => (object.as_ref(), property.as_str(), *span),
                Expr::MethodCall {
                    receiver,
                    method,
                    span,
                    ..
                } => (receiver.as_ref(), method.as_str(), *span),
                _ => return true,
            };

            if property != self.hovered_word || !span_contains_offset(span, self.offset) {
                return true;
            }

            let Expr::Identifier(object_name, _) = object else {
                return true;
            };

            let len = span.len();
            if self
                .best
                .as_ref()
                .map(|(best_len, _, _)| len < *best_len)
                .unwrap_or(true)
            {
                self.best = Some((len, object_name.clone(), property.to_string()));
            }
            true
        }
    }

    let mut finder = PropertyAccessFinder {
        hovered_word,
        offset: cursor_offset,
        best: None,
    };
    walk_program(&mut finder, &program);
    let (_, object_name, property) = finder.best?;

    // Check if self is a module member access (e.g., csv.load)
    if let Some(hover) = get_module_member_hover(text, &object_name, &property) {
        return Some(hover);
    }

    // Check Content API member access (e.g., Content.text, Color.red)
    if let Some(hover) = get_content_member_hover(&object_name, &property) {
        return Some(hover);
    }

    // Check DateTime / io / time member access
    if let Some(hover) = get_namespace_member_hover(&object_name, &property) {
        return Some(hover);
    }

    // Try engine-inferred type first, fall back to heuristic
    let program_types = infer_program_types(&program);
    let object_type = if object_name == "self" {
        receiver_type_at_offset(&program, cursor_offset)
    } else {
        infer_variable_visible_type_at_offset(&program, &object_name, cursor_offset).or_else(|| {
            choose_best_variable_type(
                program_types.get(&object_name).cloned(),
                infer_variable_type(&program, &object_name),
            )
        })
    }?;

    // Try unified metadata first (Rust-defined types)
    if let Some(properties) = unified_metadata().get_type_properties(&object_type) {
        if let Some(prop_info) = properties.iter().find(|p| p.name == property) {
            let content = format!(
                "**Property**: `{}.{}`\n\n**Type:** `{}`\n\n{}",
                object_name, property, prop_info.property_type, prop_info.description
            );
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            });
        }
    }

    // Try user-defined struct fields from AST (including generic instantiations).
    if let Some(field_type) = resolve_struct_field_type(&program, &object_type, &property) {
        let content = format!(
            "**Property**: `{}.{}`\n\n**Type:** `{}`\n\nField `{}` on `{}`",
            object_name, property, field_type, property, object_type
        );
        return Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        });
    }

    // Fallback: inferred struct literal shapes when no explicit type definition exists.
    let struct_fields = extract_struct_fields(&program);
    if let Some(fields) = struct_fields.get(&object_type) {
        if let Some((_, field_type)) = fields.iter().find(|(name, _)| name == &property) {
            let content = format!(
                "**Property**: `{}.{}`\n\n**Type:** `{}`\n\nField `{}` of inferred type `{}`",
                object_name, property, field_type, property, object_type
            );
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            });
        }
    }

    // Inline/structural object shape (e.g., `{ x: int, y: int }`)
    if let Some(fields) = parse_object_shape_fields(&object_type) {
        if let Some((_, field_type)) = fields.iter().find(|(name, _)| name == &property) {
            let content = format!(
                "**Property**: `{}.{}`\n\n**Type:** `{}`\n\nField `{}` of inferred object type",
                object_name, property, field_type, property
            );
            return Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: content,
                }),
                range: None,
            });
        }
    }

    None
}

/// Get hover for a join strategy keyword showing the resolved return type.
///
/// When hovering over `all`, `race`, `any`, or `settle` in an `await join` expression,
/// self shows the resolved return type based on the join strategy and branch count.
fn get_join_expression_hover(text: &str, word: &str, position: Position) -> Option<Hover> {
    let cursor_offset = position_to_offset(text, position)?;
    let program = parse_with_fallback(text)?;

    struct JoinFinder {
        offset: usize,
        target_kind: shape_ast::ast::JoinKind,
        best: Option<(usize, usize)>, // (span_len, branch_count)
    }

    impl shape_runtime::visitor::Visitor for JoinFinder {
        fn visit_expr(&mut self, expr: &Expr) -> bool {
            if let Expr::Join(join_expr, span) = expr {
                if join_expr.kind == self.target_kind && span_contains_offset(*span, self.offset) {
                    let len = span.len();
                    if self
                        .best
                        .map(|(best_len, _)| len < best_len)
                        .unwrap_or(true)
                    {
                        self.best = Some((len, join_expr.branches.len()));
                    }
                }
            }
            true
        }
    }

    let target_kind = match word {
        "all" => JoinKind::All,
        "race" => JoinKind::Race,
        "any" => JoinKind::Any,
        "settle" => JoinKind::Settle,
        _ => return None,
    };

    let mut finder = JoinFinder {
        offset: cursor_offset,
        target_kind,
        best: None,
    };
    shape_runtime::visitor::walk_program(&mut finder, &program);

    let branch_count = finder.best.map(|(_, count)| count)?;

    let (return_type, description) = match word {
        "all" => (
            format!("(T1, T2, ...T{})", branch_count),
            "Waits for **all** branches to complete. Returns a tuple of all results.",
        ),
        "race" => (
            "T".to_string(),
            "Returns the result of the **first** branch to complete. Cancels remaining branches.",
        ),
        "any" => (
            "T".to_string(),
            "Returns the result of the **first** branch to succeed (non-error). Cancels remaining branches.",
        ),
        "settle" => (
            format!("(Result<T1>, Result<T2>, ...Result<T{}>)", branch_count),
            "Waits for **all** branches. Returns individual Result values preserving success/error status.",
        ),
        _ => return None,
    };

    let content = format!(
        "**Join Strategy**: `{}`\n\n{}\n\n**Branches:** {}\n**Return type:** `{}`",
        word, description, branch_count, return_type
    );

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: content,
        }),
        range: None,
    })
}

#[cfg(test)]
#[path = "hover_tests.rs"]
mod tests;
