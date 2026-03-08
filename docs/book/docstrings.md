# Docstrings

Shape docstrings are first-class language metadata.

They are parsed from `///` comments, attached directly to AST nodes, stored in structured form, and consumed by the LSP and stdlib metadata without text scraping or annotation-derived fallbacks.

## Design Rules

- Only `///` creates documentation.
- `/** ... */` is not documentation.
- Applied annotations are never documentation for the item they annotate.
- Doc comments attach to the immediately following AST item or member.
- Documentation is structured data first and markdown second.
- Cross-links must use fully qualified targets.
- Invalid docs are diagnosed instead of guessed.

This keeps docs inspectable by tools, stable under refactors, and suitable for future comptime and AI-facing introspection.

## What Can Be Documented

`///` doc comments can attach to:

- modules
- functions
- foreign functions
- builtin declarations
- annotations
- type aliases
- structs
- struct fields
- enums
- enum variants
- interfaces
- interface members
- traits
- trait members
- associated types
- type parameters

Shape does not use parent-owned child tags. If a field, variant, method, or type parameter needs docs, document that declaration directly.

## Basic Form

```shape
/// Compute the spread between the maximum and minimum value.
///
/// @param series Input series.
/// @returns max(series) - min(series).
pub fn spread(series) {
    __intrinsic_max(series) - __intrinsic_min(series)
}
```

The first prose paragraph becomes the summary. Additional prose remains part of the markdown body.

## Supported Tags

Shape supports these structured tags:

- `@module <fully-qualified-module>`
- `@typeparam <name> <description>`
- `@param <name> <description>`
- `@returns <description>`
- `@throws <description>`
- `@deprecated <description>`
- `@requires <description>`
- `@since <description>`
- `@see <fully-qualified-target>`
- `@link <fully-qualified-target> [label]`
- `@note <description>`
- `@example`

Example:

```shape
/// Compute a Hull moving average.
///
/// @param series Input series.
/// @param period Window size.
/// @returns Hull moving average series.
/// @see std::finance::indicators::moving_averages::wma
/// @example
/// let value = hma(close, 21)
pub fn hma(series, period) {
    // ...
}
```

## Type Parameters, Fields, Variants, and Members

Type parameters have their own doc blocks:

```shape
fn identity<
    /// Element type preserved by the function.
    T
>(value: T) -> T {
    value
}
```

Struct fields have their own doc blocks:

```shape
pub type Candle = {
    /// Opening price.
    open: number;
    /// Highest traded price in the interval.
    high: number;
}
```

Trait and interface members can also be documented directly:

```shape
/// Convert a value into a human-readable string.
trait Display {
    /// Render `self` as text.
    display(): string
}
```

## Fully Qualified Cross-Links

Cross-links are explicit and semantic.

Use `@see` for a simple reference:

```shape
/// @see std::core::utils::rolling::rolling_mean
```

Use `@link` when you want a custom label:

```shape
/// @link std::finance::indicators::volatility::atr ATR helper
```

Rules:

- Targets must be fully qualified.
- Annotation targets use their canonical symbol path, with the final segment
  written as `@name`.
- Relative links are intentionally unsupported.
- Unresolved links are diagnosed.
- LSP link completion inserts canonical fully qualified targets.

In hover rendering, resolved links are shown as linked references when the editor supports file URIs.

## LSP Features

The LSP uses the AST doc model directly.

### Hover

Hover shows:

- the declaration signature
- markdown body text
- structured sections such as parameters, returns, notes, and examples
- resolved `@see` and `@link` references

### Signature Help

Signature help uses:

- `@param` text for parameter documentation
- doc summary/body for function-level help

### Completion Inside Doc Comments

Inside `///` blocks, the LSP offers completion for:

- tag names after `@`
- parameter names inside `@param`
- type parameter names inside `@typeparam`
- fully qualified symbol targets inside `@see` and `@link`

This is semantic completion, not text matching. Parameter and type-parameter suggestions come from the attached AST owner.

### Code Action

`Generate doc comment` inserts a structured stub above supported declarations,
including annotation definitions.

The generated stub includes:

- summary placeholder
- `@typeparam` entries when generics exist
- `@param` entries for callable parameters
- `@returns` when the declaration can return a value

## Validation and Diagnostics

Shape validates doc comments semantically.

Examples of diagnosed problems:

- unknown tag names
- duplicate singleton tags such as `@returns`
- duplicate `@param` or `@typeparam` entries
- `@param` names that do not exist on the callable
- `@typeparam` names that do not exist on the item
- `@returns` on non-callable or `void`-only targets
- non-fully-qualified links
- unresolved `@see` or `@link` targets
- malformed empty tags that require content

The diagnostics are span-precise because tags and link targets are stored with their own spans in the AST doc model.

## Stdlib Rules

Stdlib documentation lives in `.shape` source files.

That means:

- stdlib `.shape` docs are the canonical source of truth
- LSP hover and signature help read stdlib docs from parsed Shape AST
- docs stay adjacent to the declarations they describe
- annotation metadata is not used as a doc source for other declarations

When you document stdlib APIs, prefer concise summaries plus the tags that add real semantic value.

## Writing Good Docstrings

Good Shape docstrings should explain:

- semantic meaning
- invariants or edge cases
- units and coordinate systems when relevant
- availability or capability requirements
- relationships to nearby APIs through `@see` and `@link`

Good docstrings should not duplicate:

- obvious type information already present in the signature
- syntax already visible in the declaration
- annotation names as fake documentation

## First-Class Data

Docstrings are not an editor-only feature.

They exist as structured AST data:

- comment body
- ordered tags
- typed tag kinds
- attachment target metadata
- tag and link spans

This design is intentional. It makes docs suitable for:

- LSP rendering and authoring
- stdlib metadata extraction
- semantic validation
- future site generation
- future comptime inspection
- future AI/tooling workflows that need reliable structured documentation

## Anti-Patterns

Avoid these:

- `/** ... */` doc comments
- relative link targets like `@see ema`
- using annotations as a documentation channel
- documenting child members only in a parent summary
- restating signature types inside prose unless the semantic constraint matters

## Example: Full Trait Doc

```shape
/// Score values for a custom review workflow.
///
/// This is a normal user-defined trait. The docs are attached to the trait and
/// its methods through the AST, exactly the same way they would be for stdlib
/// code.
///
/// @typeparam T Value type evaluated by the scorer.
trait Scorer<T> {
    /// Compute a score for the input value.
    ///
    /// @param value Value to score.
    /// @returns Numeric score used by downstream decisions.
    score(value: T): number,

    /// Return whether the input value passes the requested threshold.
    ///
    /// @param value Value to evaluate.
    /// @param min_score Minimum score required for acceptance.
    /// @returns `true` when the value meets the threshold.
    passes(value: T, min_score: number): bool,
}
```

The semantic connection to `value` and `min_score` comes from the AST
declaration plus the `@param` tags. A backticked name inside prose is only
markdown text, not a structured parameter reference.

## Summary

Shape docstrings are:

- strict
- AST-attached
- structured
- linkable
- validated
- authorable in the editor
- sourced from real Shape code, including the stdlib

That is the intended foundation for long-term inspectability, including comptime and AI-facing tooling.
