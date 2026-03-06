//! Procedural macros for Shape introspection
//!
//! This crate provides:
//! - `#[shape_builtin]` attribute macro for function metadata extraction
//! - `#[derive(ShapeType)]` derive macro for type/struct property metadata

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{
    Attribute, DeriveInput, Field, ItemFn, Lit, Meta, MetaNameValue, Token, Type,
    parse_macro_input, punctuated::Punctuated,
};

/// Attribute macro for Shape builtin functions.
///
/// Extracts metadata from doc comments and generates a `METADATA_<NAME>` constant.
///
/// # Usage
///
/// ```ignore
/// /// Calculate Simple Moving Average
/// ///
/// /// # Parameters
/// /// * `series: Series` - Input price series
/// /// * `period: Number` - Lookback period
/// ///
/// /// # Returns
/// /// `Series` - Smoothed series
/// ///
/// /// # Example
/// /// ```shape
/// /// sma(series("close"), 20)
/// /// ```
/// #[shape_builtin(category = "Indicator")]
/// pub fn eval_sma(args: Vec<Value>, ctx: &mut ExecutionContext) -> Result<Value> {
///     // implementation
/// }
/// ```
#[proc_macro_attribute]
pub fn shape_builtin(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr with Punctuated::<Meta, Token![,]>::parse_terminated);
    let input = parse_macro_input!(item as ItemFn);

    let expanded = impl_shape_builtin(&args, &input);

    TokenStream::from(expanded)
}

fn impl_shape_builtin(args: &Punctuated<Meta, Token![,]>, input: &ItemFn) -> TokenStream2 {
    // Extract category from attribute args
    let category = extract_category(args).unwrap_or_else(|| "Utility".to_string());

    // Extract function name (remove eval_ prefix if present)
    let fn_name = input.sig.ident.to_string();
    let builtin_name = fn_name
        .strip_prefix("eval_")
        .or_else(|| fn_name.strip_prefix("intrinsic_"))
        .unwrap_or(&fn_name)
        .to_string();

    // Parse doc comments
    let doc_info = parse_doc_comments(&input.attrs);

    // Generate metadata constant name
    let metadata_ident = format_ident!("METADATA_{}", builtin_name.to_uppercase());

    // Generate parameter array
    let params = generate_params(&doc_info.parameters);

    // Build signature string
    let signature = build_signature(&builtin_name, &doc_info.parameters, &doc_info.return_type);

    // Extract string values from doc_info
    let description = &doc_info.description;
    let return_type = &doc_info.return_type;

    // Generate example option
    let example_tokens = match &doc_info.example {
        Some(ex) => quote! { Some(#ex) },
        None => quote! { None },
    };

    // Generate the metadata constant and preserve the original function
    let vis = &input.vis;
    let sig = &input.sig;
    let block = &input.block;
    let attrs = &input.attrs;

    quote! {
        /// Metadata for the builtin function (auto-generated)
        pub const #metadata_ident: crate::builtin_metadata::BuiltinMetadata = crate::builtin_metadata::BuiltinMetadata {
            name: #builtin_name,
            signature: #signature,
            description: #description,
            category: #category,
            parameters: &[#params],
            return_type: #return_type,
            example: #example_tokens,
        };

        #(#attrs)*
        #vis #sig #block
    }
}

fn extract_category(args: &Punctuated<Meta, Token![,]>) -> Option<String> {
    for meta in args {
        if let Meta::NameValue(MetaNameValue {
            path,
            value: syn::Expr::Lit(expr_lit),
            ..
        }) = meta
        {
            if path.is_ident("category") {
                if let Lit::Str(lit_str) = &expr_lit.lit {
                    return Some(lit_str.value());
                }
            }
        }
    }
    None
}

#[derive(Default)]
struct DocInfo {
    description: String,
    parameters: Vec<ParamInfo>,
    return_type: String,
    example: Option<String>,
}

struct ParamInfo {
    name: String,
    param_type: String,
    optional: bool,
    description: String,
}

fn parse_doc_comments(attrs: &[Attribute]) -> DocInfo {
    let mut info = DocInfo::default();
    let mut current_section = Section::Description;
    let mut example_lines: Vec<String> = Vec::new();
    let mut in_code_block = false;

    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta.value {
                    if let Lit::Str(lit_str) = &expr_lit.lit {
                        let line = lit_str.value();
                        let trimmed = line.trim();

                        // Check for section headers
                        if trimmed == "# Parameters" {
                            current_section = Section::Parameters;
                            continue;
                        } else if trimmed == "# Returns" {
                            current_section = Section::Returns;
                            continue;
                        } else if trimmed == "# Example" || trimmed == "# Examples" {
                            current_section = Section::Example;
                            continue;
                        }

                        // Check for code block markers
                        if trimmed.starts_with("```") {
                            in_code_block = !in_code_block;
                            if current_section == Section::Example && !in_code_block {
                                // End of example code block
                                info.example = Some(example_lines.join("\n"));
                                example_lines.clear();
                            }
                            continue;
                        }

                        match current_section {
                            Section::Description => {
                                if !trimmed.is_empty() {
                                    if !info.description.is_empty() {
                                        info.description.push(' ');
                                    }
                                    info.description.push_str(trimmed);
                                }
                            }
                            Section::Parameters => {
                                if let Some(param) = parse_param_line(trimmed) {
                                    info.parameters.push(param);
                                }
                            }
                            Section::Returns => {
                                if let Some(ret) = parse_returns_line(trimmed) {
                                    info.return_type = ret;
                                }
                            }
                            Section::Example => {
                                if in_code_block {
                                    example_lines.push(line.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Default return type if not specified
    if info.return_type.is_empty() {
        info.return_type = "Any".to_string();
    }

    info
}

#[derive(PartialEq)]
enum Section {
    Description,
    Parameters,
    Returns,
    Example,
}

fn parse_param_line(line: &str) -> Option<ParamInfo> {
    // Parse: * `name: Type` - Description
    // or:   * `name?: Type` - Description (optional)
    let line = line.trim_start_matches('*').trim();
    if !line.starts_with('`') {
        return None;
    }

    let line = line.trim_start_matches('`');
    let end_tick = line.find('`')?;
    let param_spec = &line[..end_tick];
    let description = line[end_tick + 1..]
        .trim_start_matches(" - ")
        .trim()
        .to_string();

    // Parse name: Type or name?: Type
    let (name, param_type, optional) = if let Some(colon_pos) = param_spec.find(':') {
        let name_part = &param_spec[..colon_pos];
        let type_part = param_spec[colon_pos + 1..].trim();

        let (name, optional) = if name_part.ends_with('?') {
            (name_part.trim_end_matches('?').to_string(), true)
        } else {
            (name_part.to_string(), false)
        };

        (name, type_part.to_string(), optional)
    } else {
        (param_spec.to_string(), "Any".to_string(), false)
    };

    Some(ParamInfo {
        name,
        param_type,
        optional,
        description,
    })
}

fn parse_returns_line(line: &str) -> Option<String> {
    // Parse: `Type` - Description
    let line = line.trim();
    if !line.starts_with('`') {
        return None;
    }

    let line = line.trim_start_matches('`');
    let end_tick = line.find('`')?;
    Some(line[..end_tick].to_string())
}

fn generate_params(params: &[ParamInfo]) -> TokenStream2 {
    let param_tokens: Vec<TokenStream2> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let param_type = &p.param_type;
            let optional = p.optional;
            let description = &p.description;

            quote! {
                crate::builtin_metadata::BuiltinParam {
                    name: #name,
                    param_type: #param_type,
                    optional: #optional,
                    description: #description,
                }
            }
        })
        .collect();

    quote! { #(#param_tokens),* }
}

fn build_signature(name: &str, params: &[ParamInfo], return_type: &str) -> String {
    let param_strs: Vec<String> = params
        .iter()
        .map(|p| {
            if p.optional {
                format!("{}?: {}", p.name, p.param_type)
            } else {
                format!("{}: {}", p.name, p.param_type)
            }
        })
        .collect();

    format!("{}({}) -> {}", name, param_strs.join(", "), return_type)
}

/// Derive macro for Shape type metadata.
///
/// Extracts property metadata from struct fields and generates a `TYPE_METADATA_<NAME>` constant.
///
/// # Usage
///
/// ```ignore
/// /// A single price bar (OHLCV data)
/// #[derive(ShapeType)]
/// #[shape(name = "Candle")]
/// pub struct CandleValue {
///     /// Opening price
///     pub open: f64,
///     /// Highest price
///     pub high: f64,
///     /// Closing price
///     pub close: f64,
/// }
/// ```
#[proc_macro_derive(ShapeType, attributes(shape))]
pub fn derive_shape_type(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let expanded = impl_shape_type(&input);
    TokenStream::from(expanded)
}

fn impl_shape_type(input: &DeriveInput) -> TokenStream2 {
    // Extract type name from attribute or use struct name
    let type_name = extract_type_name(&input.attrs).unwrap_or_else(|| input.ident.to_string());

    // Extract description from doc comments
    let description = extract_struct_description(&input.attrs);

    // Generate metadata constant name
    let metadata_ident = format_ident!("TYPE_METADATA_{}", type_name.to_uppercase());

    // Extract fields and generate property metadata
    let properties = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => generate_property_metadata(&fields.named),
            _ => quote! {},
        },
        _ => {
            return syn::Error::new_spanned(input, "ShapeType can only be derived for structs")
                .to_compile_error();
        }
    };

    quote! {
        /// Type metadata for LSP introspection (auto-generated)
        pub const #metadata_ident: crate::builtin_metadata::TypeMetadata = crate::builtin_metadata::TypeMetadata {
            name: #type_name,
            description: #description,
            properties: &[#properties],
        };
    }
}

fn extract_type_name(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if attr.path().is_ident("shape") {
            if let Ok(nested) =
                attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            {
                for meta in nested {
                    if let Meta::NameValue(MetaNameValue {
                        path,
                        value: syn::Expr::Lit(expr_lit),
                        ..
                    }) = meta
                    {
                        if path.is_ident("name") {
                            if let Lit::Str(lit_str) = &expr_lit.lit {
                                return Some(lit_str.value());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

fn extract_struct_description(attrs: &[Attribute]) -> String {
    let mut description = String::new();

    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta.value {
                    if let Lit::Str(lit_str) = &expr_lit.lit {
                        let line = lit_str.value();
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            if !description.is_empty() {
                                description.push(' ');
                            }
                            description.push_str(trimmed);
                        }
                    }
                }
            }
        }
    }

    description
}

fn generate_property_metadata(fields: &Punctuated<Field, Token![,]>) -> TokenStream2 {
    let props: Vec<TokenStream2> = fields
        .iter()
        .filter_map(|field| {
            // Skip fields with #[shape(skip)]
            if has_shape_skip(&field.attrs) {
                return None;
            }

            let name = field.ident.as_ref()?.to_string();
            let prop_type = extract_field_type(&field.attrs, &field.ty);
            let description = extract_field_description(&field.attrs);

            Some(quote! {
                crate::builtin_metadata::PropertyMetadata {
                    name: #name,
                    prop_type: #prop_type,
                    description: #description,
                }
            })
        })
        .collect();

    quote! { #(#props),* }
}

fn has_shape_skip(attrs: &[Attribute]) -> bool {
    for attr in attrs {
        if attr.path().is_ident("shape") {
            if let Ok(nested) =
                attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            {
                for meta in nested {
                    if let Meta::Path(path) = meta {
                        if path.is_ident("skip") {
                            return true;
                        }
                    }
                }
            }
        }
    }
    false
}

fn extract_field_type(attrs: &[Attribute], ty: &Type) -> String {
    // First check for explicit #[shape(type = "...")] override
    for attr in attrs {
        if attr.path().is_ident("shape") {
            if let Ok(nested) =
                attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            {
                for meta in nested {
                    if let Meta::NameValue(MetaNameValue {
                        path,
                        value: syn::Expr::Lit(expr_lit),
                        ..
                    }) = meta
                    {
                        if path.is_ident("type") {
                            if let Lit::Str(lit_str) = &expr_lit.lit {
                                return lit_str.value();
                            }
                        }
                    }
                }
            }
        }
    }

    // Otherwise, infer from Rust type
    rust_type_to_shape(ty)
}

fn rust_type_to_shape(ty: &Type) -> String {
    match ty {
        Type::Path(type_path) => {
            let segments: Vec<_> = type_path
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            let type_str = segments.last().map(|s| s.as_str()).unwrap_or("Any");

            match type_str {
                "f64" | "f32" => "Number".to_string(),
                "i64" | "i32" | "i16" | "i8" | "u64" | "u32" | "u16" | "u8" | "usize" | "isize" => {
                    "Number".to_string()
                }
                "String" => "String".to_string(),
                "bool" => "Boolean".to_string(),
                "DateTime" => "DateTime".to_string(),
                "Series" => "Series".to_string(),
                "Vec" => {
                    // Try to extract inner type
                    if let Some(seg) = type_path.path.segments.last() {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                                let inner_type = rust_type_to_shape(inner);
                                return format!("Array<{}>", inner_type);
                            }
                        }
                    }
                    "Array".to_string()
                }
                "Option" => {
                    if let Some(seg) = type_path.path.segments.last() {
                        if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
                            if let Some(syn::GenericArgument::Type(inner)) = args.args.first() {
                                let inner_type = rust_type_to_shape(inner);
                                return format!("{}?", inner_type);
                            }
                        }
                    }
                    "Any?".to_string()
                }
                "HashMap" | "BTreeMap" => "Object".to_string(),
                _ => type_str.to_string(),
            }
        }
        _ => "Any".to_string(),
    }
}

fn extract_field_description(attrs: &[Attribute]) -> String {
    let mut description = String::new();

    for attr in attrs {
        if attr.path().is_ident("doc") {
            if let Meta::NameValue(meta) = &attr.meta {
                if let syn::Expr::Lit(expr_lit) = &meta.value {
                    if let Lit::Str(lit_str) = &expr_lit.lit {
                        let line = lit_str.value();
                        let trimmed = line.trim();
                        if !trimmed.is_empty() {
                            if !description.is_empty() {
                                description.push(' ');
                            }
                            description.push_str(trimmed);
                        }
                    }
                }
            }
        }
    }

    description
}

/// Attribute macro for Shape data providers.
///
/// Extracts metadata from doc comments and generates a `PROVIDER_METADATA_<NAME>` constant.
///
/// # Usage
///
/// ```ignore
/// /// Market data provider with DuckDB backend
/// ///
/// /// # Parameters
/// /// * `symbol: String` - Stock symbol (required)
/// /// * `timeframe: String` - Time period (required)
/// ///
/// /// # Example
/// /// ```shape
/// /// data('market_data', {symbol: 'ES', timeframe: '1h'})
/// /// ```
/// #[shape_provider(category = "Market Data")]
/// pub fn market_data_provider(args: Vec<Value>, ctx: &mut ExecutionContext) -> Result<Value> {
///     // implementation
/// }
/// ```
#[proc_macro_attribute]
pub fn shape_provider(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(attr with Punctuated::<Meta, Token![,]>::parse_terminated);
    let input = parse_macro_input!(item as ItemFn);

    let expanded = impl_shape_provider(&args, &input);

    TokenStream::from(expanded)
}

fn impl_shape_provider(args: &Punctuated<Meta, Token![,]>, input: &ItemFn) -> TokenStream2 {
    // Extract category from attribute args
    let category = extract_category(args).unwrap_or_else(|| "Data Provider".to_string());

    // Extract provider name (remove _provider suffix if present, or eval_ prefix)
    let fn_name = input.sig.ident.to_string();
    let provider_name = fn_name
        .strip_suffix("_provider")
        .or_else(|| fn_name.strip_prefix("eval_"))
        .unwrap_or(&fn_name)
        .to_string();

    // Parse doc comments
    let doc_info = parse_doc_comments(&input.attrs);

    // Generate metadata constant name
    let metadata_ident = format_ident!("PROVIDER_METADATA_{}", provider_name.to_uppercase());

    // Generate parameter array
    let params = generate_provider_params(&doc_info.parameters);

    // Extract string values from doc_info
    let description = &doc_info.description;

    // Generate example option
    let example_tokens = match &doc_info.example {
        Some(ex) => quote! { Some(#ex) },
        None => quote! { None },
    };

    // Generate the metadata constant and preserve the original function
    let vis = &input.vis;
    let sig = &input.sig;
    let block = &input.block;
    let attrs = &input.attrs;

    quote! {
        /// Provider metadata for LSP introspection (auto-generated)
        pub const #metadata_ident: crate::data::provider_metadata::ProviderMetadata = crate::data::provider_metadata::ProviderMetadata {
            name: #provider_name,
            description: #description,
            category: #category,
            parameters: &[#params],
            example: #example_tokens,
        };

        #(#attrs)*
        #vis #sig #block
    }
}

fn generate_provider_params(params: &[ParamInfo]) -> TokenStream2 {
    let param_tokens: Vec<TokenStream2> = params
        .iter()
        .map(|p| {
            let name = &p.name;
            let param_type = &p.param_type;
            let required = !p.optional; // Invert: optional=false means required=true
            let description = &p.description;

            quote! {
                crate::data::provider_metadata::ProviderParam {
                    name: #name,
                    param_type: #param_type,
                    required: #required,
                    description: #description,
                    default: None,
                }
            }
        })
        .collect();

    quote! { #(#param_tokens),* }
}
