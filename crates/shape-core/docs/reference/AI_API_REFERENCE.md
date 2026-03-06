# Shape AI - API Reference

Complete API documentation for all AI features in Shape.

---

## Table of Contents

1. [CLI Commands](#cli-commands)
2. [Shape Functions](#shape-functions)
3. [Intrinsic Functions](#intrinsic-functions)
4. [Rust API](#rust-api)
5. [Configuration](#configuration)
6. [Types & Structures](#types--structures)

---

## CLI Commands

### `ai-eval` - Evaluate Multiple Strategies

Evaluate and rank multiple Shape strategies from a JSON file.

**Syntax:**
```bash
shape ai-eval <STRATEGIES> [OPTIONS]
```

**Arguments:**
- `STRATEGIES` (required): Path to JSON file containing strategies

**Options:**
- `-r, --rank-by <METRIC>`: Metric to rank by (default: `sharpe_ratio`)
- `-f, --format <FORMAT>`: Output format (default: `table`)
  - `table`: Pretty-printed table with colors
  - `json`: JSON array of results
- `-o, --output <FILE>`: Save results to JSON file

**Supported Rank Metrics:**
- `sharpe_ratio`, `sharpe` - Sharpe ratio (risk-adjusted return)
- `sortino_ratio`, `sortino` - Sortino ratio (downside risk)
- `total_return`, `return` - Total percentage return
- `max_drawdown`, `drawdown` - Maximum drawdown (lower is better)
- `win_rate` - Percentage of winning trades
- `profit_factor` - Gross profit / gross loss
- `total_trades`, `trades` - Number of trades

**Examples:**
```bash
# Basic usage
shape ai-eval strategies.json

# Rank by Sortino ratio
shape ai-eval strategies.json --rank-by sortino_ratio

# JSON output
shape ai-eval strategies.json --format json

# Save results
shape ai-eval strategies.json --output results.json
```

**Input JSON Format:**
```json
[
  {
    "name": "Strategy Name",
    "code": "Shape code as string",
    "symbol": "ES",
    "timeframe": "1h",
    "config": {
      "initial_capital": 100000
    }
  }
]
```

**Output Structure:**
```json
[
  {
    "name": "Strategy Name",
    "success": true,
    "error": null,
    "summary": {
      "total_return": 45.3,
      "sharpe_ratio": 2.45,
      "sortino_ratio": 3.12,
      "max_drawdown": 12.45,
      "win_rate": 65.5,
      "profit_factor": 2.8,
      "total_trades": 120,
      "avg_trade_duration": 14400.0
    },
    "metrics": { /* same as summary */ }
  }
]
```

**Exit Codes:**
- `0`: Success
- `1`: Error (file not found, parse error, etc.)

---

### `ai-generate` - Generate Strategy from Natural Language

Generate a Shape trading strategy from natural language description.

**Requires:** `--features ai` build flag

**Syntax:**
```bash
shape ai-generate <PROMPT> [OPTIONS]
```

**Arguments:**
- `PROMPT` (required): Natural language strategy description

**Options:**
- `-o, --output <FILE>`: Save generated code to file
- `-p, --provider <PROVIDER>`: LLM provider to use
  - `openai`: OpenAI (GPT-4, GPT-3.5-turbo)
  - `anthropic`: Anthropic (Claude) - default
  - `deepseek`: DeepSeek (cost-effective)
  - `ollama`: Ollama (local models)
- `-m, --model <MODEL>`: Model name override
- `-c, --config <FILE>`: Configuration file path

**Examples:**
```bash
# Basic usage (uses default: Anthropic Claude Sonnet 4)
shape ai-generate "Create a mean reversion strategy using RSI"

# Specify provider
shape ai-generate --provider openai "Create a MACD strategy"

# Specify model
shape ai-generate --provider openai --model gpt-4-turbo "Complex strategy"

# Save to file
shape ai-generate "Bollinger Bands strategy" --output strategy.shape

# Use custom config
shape ai-generate --config ai_config.toml "Momentum strategy"
```

**Output:**
Prints generated Shape code to stdout (or saves to file if --output specified).

**Environment Variables:**
- `OPENAI_API_KEY`: Required for OpenAI provider
- `ANTHROPIC_API_KEY`: Required for Anthropic provider
- `DEEPSEEK_API_KEY`: Required for DeepSeek provider
- No key needed for Ollama

---

## Shape Functions

These functions are available in Shape programs when you import them from `stdlib/ai/generate`.

### `ai_generate(prompt, config?)`

Generate a trading strategy from natural language description.

**Module:** `stdlib/ai/generate`

**Signature:**
```shape
function ai_generate(prompt: string, config?: object) -> string
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `prompt` | String | Yes | Natural language description of the strategy |
| `config` | Object | No | Configuration options |

**Config Options:**

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `model` | String | Provider default | Model name override |
| `temperature` | Number | 0.7 | Creativity (0.0-2.0) |
| `max_tokens` | Number | 4096 | Maximum tokens to generate |

**Returns:**
- Type: `String`
- Content: Generated Shape strategy code

**Errors:**
- `RuntimeError`: API key not found
- `RuntimeError`: API request failed
- `RuntimeError`: Invalid response from LLM

**Example:**
```shape
import { ai_generate } from "stdlib/ai/generate";

// Simple usage
let strategy = ai_generate("Create an RSI oversold strategy");
print(strategy);

// With configuration
let advanced = ai_generate(
    "Create a Bollinger Bands mean reversion strategy",
    {
        model: "gpt-4-turbo",
        temperature: 0.8,
        max_tokens: 2048
    }
);
```

---

### `ai_evaluate(strategy_code, config?)`

Evaluate a generated strategy (partial implementation).

**Module:** `stdlib/ai/generate`

**Signature:**
```shape
function ai_evaluate(strategy_code: string, config?: object) -> object
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `strategy_code` | String | Yes | Shape strategy code to evaluate |
| `config` | Object | No | Backtest configuration |

**Config Options:**

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `symbol` | String | "ES" | Symbol to backtest |
| `timeframe` | String | "1h" | Timeframe |
| `capital` | Number | 100000 | Initial capital |

**Returns:**
- Type: `Object`
- Fields: Backtest results (implementation pending)

**Status:** ⚠️ Partial implementation - currently returns error

---

### `ai_optimize(parameter, min, max, metric)`

Define parameter optimization directive.

**Module:** `stdlib/ai/generate`

**Signature:**
```shape
function ai_optimize(parameter: string, min: number, max: number, metric: string) -> object
```

**Parameters:**

| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `parameter` | String | Yes | Parameter name to optimize |
| `min` | Number | Yes | Minimum value |
| `max` | Number | Yes | Maximum value |
| `metric` | String | Yes | Metric to optimize for |

**Supported Metrics:**
- `sharpe` - Sharpe ratio
- `sortino` - Sortino ratio
- `return` - Total return
- `drawdown` - Maximum drawdown
- `win_rate` - Win rate percentage
- `profit_factor` - Profit factor

**Returns:**
- Type: `Object`
- Fields:
  - `parameter` (String): Parameter name
  - `min` (Number): Minimum value
  - `max` (Number): Maximum value
  - `metric` (String): Optimization metric

**Example:**
```shape
import { ai_optimize } from "stdlib/ai/generate";

let opt = ai_optimize("rsi_period", 7, 21, "sharpe");
print(opt);
// Output: { parameter: "rsi_period", min: 7, max: 21, metric: "sharpe" }
```

---

## Intrinsic Functions

Low-level functions implemented in Rust. Typically called by stdlib, not directly by users.

### `__intrinsic_ai_generate(prompt, config?)`

**Module:** `runtime/intrinsics/ai.rs`

**Signature:**
```rust
fn intrinsic_ai_generate(args: Vec<Value>, ctx: &mut ExecutionContext) -> Result<Value>
```

**Arguments:**
- `args[0]`: String - Prompt
- `args[1]`: Object (optional) - Configuration

**Returns:** `Value::String` - Generated Shape code

**Implementation:**
1. Loads AI configuration from environment
2. Creates LLM client for configured provider
3. Builds system and user prompts
4. Calls LLM API asynchronously
5. Cleans up response (removes markdown blocks)
6. Returns generated code

**Example (Shape):**
```shape
let code = __intrinsic_ai_generate("Create RSI strategy");
```

---

### `__intrinsic_ai_evaluate(strategy_code, config?)`

**Module:** `runtime/intrinsics/ai.rs`

**Signature:**
```rust
fn intrinsic_ai_evaluate(args: Vec<Value>, ctx: &mut ExecutionContext) -> Result<Value>
```

**Arguments:**
- `args[0]`: String - Shape strategy code
- `args[1]`: Object (optional) - Backtest configuration

**Returns:** `Value::Object` - Backtest results

**Status:** ⚠️ Stub implementation (returns error)

---

### `__intrinsic_ai_optimize(parameter, min, max, metric)`

**Module:** `runtime/intrinsics/ai.rs`

**Signature:**
```rust
fn intrinsic_ai_optimize(args: Vec<Value>, ctx: &mut ExecutionContext) -> Result<Value>
```

**Arguments:**
- `args[0]`: String - Parameter name
- `args[1]`: Number - Min value
- `args[2]`: Number - Max value
- `args[3]`: String - Metric name

**Returns:** `Value::Object` - Optimization configuration

**Example (Shape):**
```shape
let opt = __intrinsic_ai_optimize("rsi_period", 7, 21, "sharpe");
```

---

## Rust API

### `StrategyEvaluator` (Phase 1)

**Module:** `shape::ai_strategy_evaluator`

#### Constructor

```rust
impl StrategyEvaluator {
    pub fn new() -> Result<Self>
}
```

Creates a new strategy evaluator.

**Returns:** `Result<StrategyEvaluator>`

**Errors:**
- Engine initialization failure

#### Methods

**`evaluate_single`**
```rust
pub fn evaluate_single(&self, request: StrategyRequest) -> StrategyEvaluation
```

Evaluate a single strategy.

**Parameters:**
- `request`: `StrategyRequest` - Strategy to evaluate

**Returns:** `StrategyEvaluation` (never fails, errors are in result)

---

**`evaluate_batch`**
```rust
pub fn evaluate_batch(&self, strategies: Vec<StrategyRequest>) -> Vec<StrategyEvaluation>
```

Evaluate multiple strategies sequentially.

**Parameters:**
- `strategies`: Vector of `StrategyRequest`

**Returns:** Vector of `StrategyEvaluation`

---

**`rank_by_metric`**
```rust
pub fn rank_by_metric(
    &self,
    evaluations: Vec<StrategyEvaluation>,
    metric: &str,
) -> Vec<StrategyEvaluation>
```

Rank strategies by specified metric.

**Parameters:**
- `evaluations`: Vector of `StrategyEvaluation`
- `metric`: Metric name (see CLI docs for list)

**Returns:** Sorted vector (best first)

---

**`load_strategies_from_json`**
```rust
pub fn load_strategies_from_json<P: AsRef<Path>>(
    path: P,
) -> Result<Vec<StrategyRequest>>
```

Load strategies from JSON file.

**Parameters:**
- `path`: File path

**Returns:** `Result<Vec<StrategyRequest>>`

**Errors:**
- File not found
- Invalid JSON format

---

**`save_results_to_json`**
```rust
pub fn save_results_to_json<P: AsRef<Path>>(
    path: P,
    results: &[StrategyEvaluation],
) -> Result<()>
```

Save evaluation results to JSON file.

**Parameters:**
- `path`: Output file path
- `results`: Evaluation results

**Returns:** `Result<()>`

---

### `LLMClient` (Phase 2)

**Module:** `shape::ai::LLMClient`

#### Constructor

```rust
impl LLMClient {
    pub fn new(config: LLMConfig) -> Result<Self>
}
```

Create a new LLM client with specified configuration.

**Parameters:**
- `config`: `LLMConfig` - Provider and model configuration

**Returns:** `Result<LLMClient>`

**Errors:**
- API key not found (checks environment variables)
- Invalid provider configuration

**Example:**
```rust
use shape::ai::{LLMClient, LLMConfig, ProviderType};

let config = LLMConfig {
    provider: ProviderType::Anthropic,
    model: "claude-sonnet-4".to_string(),
    api_key: None,  // Will use ANTHROPIC_API_KEY env var
    api_base: None,
    max_tokens: 4096,
    temperature: 0.7,
    top_p: None,
};

let client = LLMClient::new(config)?;
```

#### Methods

**`generate`**
```rust
pub async fn generate(&self, system_prompt: &str, user_prompt: &str) -> Result<String>
```

Generate text using the configured LLM provider.

**Parameters:**
- `system_prompt`: System/instruction prompt
- `user_prompt`: User request/query

**Returns:** `Result<String>` - Generated text

**Errors:**
- API request failed (network, auth, etc.)
- Rate limit exceeded
- Invalid API response
- Timeout

**Example:**
```rust
let runtime = tokio::runtime::Runtime::new()?;
let response = runtime.block_on(async {
    client.generate(
        "You are a trading strategy expert.",
        "Create a simple RSI strategy"
    ).await
})?;
```

**`config`**
```rust
pub fn config(&self) -> &LLMConfig
```

Get the current configuration.

**Returns:** Reference to `LLMConfig`

---

### `AiExecutor` (Phase 3)

**Module:** `shape::runtime::ai_executor::AiExecutor`

#### Constructor

```rust
#[cfg(feature = "ai")]
impl AiExecutor {
    pub fn new(ai_config: AIConfig) -> Self
}
```

Create AI executor with configuration.

**Parameters:**
- `ai_config`: `AIConfig` - AI configuration

**Returns:** `AiExecutor`

#### Methods

**`execute_discover_block`**
```rust
pub async fn execute_discover_block(
    &self,
    block: &AiDiscoverBlock,
    ctx: &mut ExecutionContext,
) -> Result<Value>
```

Execute an AI discover block.

**Parameters:**
- `block`: `&AiDiscoverBlock` - AST node
- `ctx`: `&mut ExecutionContext` - Execution context

**Returns:** `Result<Value>` - Array of generated strategies

**Implementation:**
- Extracts configuration from block
- Creates LLM client
- Generates strategies based on iterations
- Returns array of strategy code strings

---

**`execute_optimize`**
```rust
pub fn execute_optimize(
    &self,
    stmt: &OptimizeStatement,
    ctx: &mut ExecutionContext,
) -> Result<Value>
```

Execute an optimize statement.

**Parameters:**
- `stmt`: `&OptimizeStatement` - AST node
- `ctx`: `&mut ExecutionContext` - Execution context

**Returns:** `Result<Value>` - Optimization configuration object

---

## Configuration

### `LLMConfig` Structure

**Module:** `shape::ai::LLMConfig`

```rust
pub struct LLMConfig {
    pub provider: ProviderType,
    pub model: String,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub max_tokens: usize,
    pub temperature: f64,
    pub top_p: Option<f64>,
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `provider` | `ProviderType` | LLM provider (OpenAI, Anthropic, etc.) |
| `model` | `String` | Model name |
| `api_key` | `Option<String>` | API key (or from environment) |
| `api_base` | `Option<String>` | Custom API endpoint |
| `max_tokens` | `usize` | Maximum tokens to generate |
| `temperature` | `f64` | Sampling temperature (0.0-2.0) |
| `top_p` | `Option<f64>` | Nucleus sampling threshold |

**Default:**
```rust
LLMConfig {
    provider: ProviderType::Anthropic,
    model: "claude-sonnet-4".to_string(),
    api_key: None,
    api_base: None,
    max_tokens: 4096,
    temperature: 0.7,
    top_p: None,
}
```

---

### `AIConfig` Structure

**Module:** `shape::ai::AIConfig`

```rust
pub struct AIConfig {
    pub llm: LLMConfig,
    pub generation: GenerationConfig,
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `llm` | `LLMConfig` | LLM configuration |
| `generation` | `GenerationConfig` | Generation settings |

#### `GenerationConfig`

```rust
pub struct GenerationConfig {
    pub retry_attempts: usize,
    pub timeout_seconds: u64,
    pub validate_code: bool,
}
```

**Fields:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `retry_attempts` | `usize` | 3 | Number of retries on failure |
| `timeout_seconds` | `u64` | 60 | Request timeout in seconds |
| `validate_code` | `bool` | true | Validate generated code |

**Methods:**

```rust
impl AIConfig {
    // Load from TOML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self>

    // Load from environment variables
    pub fn from_env() -> Self

    // Save to TOML file
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()>

    // Create default template
    pub fn create_default_template<P: AsRef<Path>>(path: P) -> Result<()>
}
```

**Example:**
```rust
use shape::ai::AIConfig;

// From environment
let config = AIConfig::from_env();

// From file
let config = AIConfig::from_file("ai_config.toml")?;

// Save to file
config.save_to_file("my_config.toml")?;
```

---

### `ProviderType` Enum

**Module:** `shape::ai::ProviderType`

```rust
pub enum ProviderType {
    OpenAI,
    Anthropic,
    DeepSeek,
    Ollama,
}
```

**Serialization:**
- Serializes to lowercase strings: `"openai"`, `"anthropic"`, `"deepseek"`, `"ollama"`
- Can be used in TOML and JSON configs

**Display:**
```rust
assert_eq!(ProviderType::Anthropic.to_string(), "anthropic");
```

---

## Types & Structures

### `StrategyRequest` (Phase 1)

**Module:** `shape::ai_strategy_evaluator::StrategyRequest`

```rust
pub struct StrategyRequest {
    pub name: String,
    pub code: String,
    pub symbol: String,
    pub timeframe: String,
    pub config: Option<BacktestConfig>,
}
```

**Fields:**

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| `name` | `String` | Yes | - | Strategy identifier |
| `code` | `String` | Yes | - | Shape strategy code |
| `symbol` | `String` | No | `"ES"` | Symbol to backtest |
| `timeframe` | `String` | No | `"1h"` | Timeframe |
| `config` | `Option<BacktestConfig>` | No | Default config | Backtest settings |

**JSON Example:**
```json
{
  "name": "My RSI Strategy",
  "code": "@indicators({ rsi: rsi(series(\"close\"), 14) })\nfunction strategy() { ... }",
  "symbol": "ES",
  "timeframe": "1h"
}
```

---

### `StrategyEvaluation` (Phase 1)

**Module:** `shape::ai_strategy_evaluator::StrategyEvaluation`

```rust
pub struct StrategyEvaluation {
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
    pub summary: Option<BacktestSummary>,
    pub metrics: Option<StrategyMetrics>,
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | Strategy name |
| `success` | `bool` | Whether backtest succeeded |
| `error` | `Option<String>` | Error message if failed |
| `summary` | `Option<BacktestSummary>` | Full backtest summary |
| `metrics` | `Option<StrategyMetrics>` | Key metrics for ranking |

---

### `StrategyMetrics` (Phase 1)

**Module:** `shape::ai_strategy_evaluator::StrategyMetrics`

```rust
pub struct StrategyMetrics {
    pub sharpe_ratio: f64,
    pub sortino_ratio: f64,
    pub max_drawdown: f64,
    pub total_return: f64,
    pub win_rate: f64,
    pub profit_factor: f64,
    pub total_trades: usize,
    pub avg_trade_duration: f64,
}
```

All metrics extracted from `BacktestSummary` for easy ranking.

---

### `AiDiscoverBlock` (Phase 3 AST)

**Module:** `shape::ast::AiDiscoverBlock`

```rust
#[cfg(feature = "ai")]
pub struct AiDiscoverBlock {
    pub config: HashMap<String, Expr>,
    pub body: Vec<Statement>,
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `config` | `HashMap<String, Expr>` | Configuration options from `ai discover(...)` |
| `body` | `Vec<Statement>` | Statements inside the block |

**Shape Syntax:**
```shape
ai discover (
    model: "claude-sonnet-4",
    iterations: 100
) {
    // body statements
}
```

---

### `OptimizeStatement` (Phase 3 AST)

**Module:** `shape::ast::OptimizeStatement`

```rust
pub struct OptimizeStatement {
    pub parameter: String,
    pub range: (Box<Expr>, Box<Expr>),
    pub metric: OptimizationMetric,
}
```

**Fields:**

| Field | Type | Description |
|-------|------|-------------|
| `parameter` | `String` | Parameter name |
| `range` | `(Box<Expr>, Box<Expr>)` | Min and max expressions |
| `metric` | `OptimizationMetric` | Metric to optimize |

**Shape Syntax:**
```shape
optimize rsi_period in [7..21] for sharpe;
```

---

### `OptimizationMetric` (Phase 3)

**Module:** `shape::ast::OptimizationMetric`

```rust
pub enum OptimizationMetric {
    Sharpe,
    Sortino,
    Return,
    Drawdown,
    WinRate,
    ProfitFactor,
    Custom(Box<Expr>),
}
```

Predefined metrics for optimization, or custom expressions.

---

## Error Handling

### Error Types

All AI functions return `Result<T>` with `ShapeError` on failure.

**Common Errors:**

| Error | Cause | Solution |
|-------|-------|----------|
| `RuntimeError: "API key not found"` | Missing environment variable | Set `ANTHROPIC_API_KEY` etc. |
| `RuntimeError: "API request failed"` | Network or API error | Check internet, API status |
| `RuntimeError: "Invalid response"` | Malformed API response | Retry, check API compatibility |
| `ParseError` | Invalid generated code | Lower temperature, try different model |
| `RuntimeError: "AI features not enabled"` | Built without `--features ai` | Rebuild with feature flag |

### Error Recovery

**In Shape:**
```shape
import { ai_generate } from "stdlib/ai/generate";

// Wrap in try-catch (future feature)
let strategy = ai_generate("Create RSI strategy");

// For now, errors propagate to caller
```

**In CLI:**
- CLI shows error message
- Exits with code 1
- No partial results saved

---

## Performance Characteristics

### Time Complexity

| Operation | Complexity | Notes |
|-----------|------------|-------|
| `ai_generate()` | O(1) API call | 2-6 seconds depending on provider |
| `ai_eval` (single) | O(n) backtesting | ~1.6s for 1 year hourly data |
| `ai_eval` (batch) | O(k * n) | k strategies, n candles each |
| Parsing | O(n) | n = code length |

### Space Complexity

| Component | Memory Usage |
|-----------|--------------|
| LLM Client | ~1 MB |
| Generated Strategy | ~2-10 KB per strategy |
| Backtest Results | ~100 KB per strategy |
| Total for 100 strategies | ~10-15 MB |

### Throughput

| Metric | Value | Notes |
|--------|-------|-------|
| Strategy generation | 10-30/minute | Depends on provider |
| Strategy evaluation | 30-40/minute | Using 5,331 c/s engine |
| Combined workflow | 10-15/minute | Generation + evaluation |

---

## Version History

### Phase 1 (v0.1.0) - Complete
- ✅ Strategy evaluation API
- ✅ Batch processing
- ✅ Multi-metric ranking
- ✅ CLI integration

### Phase 2 (v0.1.0) - Complete
- ✅ Multi-provider LLM support
- ✅ Natural language to Shape
- ✅ Configuration system
- ✅ Code validation

### Phase 3 (v0.1.0) - Complete
- ✅ Grammar extensions
- ✅ AST nodes
- ✅ Parser implementation
- ✅ AI intrinsics
- ✅ Shape stdlib wrappers

### Phase 4 (Planned)
- Reinforcement learning
- Strategy optimization
- Hybrid LLM + RL

### Phase 5 (Planned)
- REPL integration
- Web UI
- Advanced analytics

---

## Dependencies

### Rust Crates (with AI feature)

```toml
[dependencies]
reqwest = { version = "0.12", features = ["json"] }  # HTTP client
tokio = { version = "1", features = ["full"] }        # Async runtime
serde = "1.0"                                         # Serialization
serde_json = "1.0"                                    # JSON support
toml = "0.8"                                          # TOML config
```

### External Services

- OpenAI API: https://platform.openai.com/
- Anthropic API: https://console.anthropic.com/
- DeepSeek API: https://platform.deepseek.com/
- Ollama: https://ollama.ai/ (local)

---

## See Also

- [AI_GUIDE.md](../guides/AI_GUIDE.md) - User guide with examples
- [AI_ARCHITECTURE.md](../architecture/AI_ARCHITECTURE.md) - Technical architecture
- [AI_CONFIGURATION.md](../guides/AI_CONFIGURATION.md) - Configuration details
- [performance_optimization_summary.md](../archive/performance_optimization_summary.md) - Backtest performance

---

**Last Updated:** 2026-01-01
**Version:** 0.1.0
**Status:** Production-ready for Phases 1-3
