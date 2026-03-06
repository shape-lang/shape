# Shape AI - Configuration Guide

Complete guide to configuring Shape's AI features.

---

## Table of Contents

1. [Configuration Methods](#configuration-methods)
2. [TOML Configuration](#toml-configuration)
3. [Environment Variables](#environment-variables)
4. [CLI Arguments](#cli-arguments)
5. [Provider-Specific Configuration](#provider-specific-configuration)
6. [Best Practices](#best-practices)
7. [Examples](#examples)

---

## Configuration Methods

Shape AI supports **3 configuration methods** with priority order:

### Priority Order (Highest to Lowest)

1. **CLI Arguments** - Flags like `--provider`, `--model`
2. **TOML Config File** - Specified with `--config path.toml`
3. **Environment Variables** - `SHAPE_AI_*` and API keys
4. **Default Values** - Built-in sensible defaults

### When to Use Each Method

| Method | Use Case |
|--------|----------|
| **CLI Arguments** | Quick overrides, experimentation |
| **TOML File** | Project-specific settings, team config |
| **Environment Variables** | Personal settings, CI/CD |
| **Defaults** | Getting started quickly |

---

## TOML Configuration

### File Location

**Default search paths:**
1. `./ai_config.toml` (current directory)
2. `~/.config/shape/ai_config.toml` (user config)
3. Specify with `--config` flag

### Complete Configuration File

**File:** `ai_config.toml`

```toml
# ============================================
# Shape AI Configuration
# ============================================

[llm]
# ----------------------------------------
# LLM Provider Settings
# ----------------------------------------

# Provider selection
# Options: "openai", "anthropic", "deepseek", "ollama"
provider = "anthropic"

# Model name (provider-specific)
# See "Provider-Specific Configuration" section below
model = "claude-sonnet-4"

# API key (optional - uses environment variable if not set)
# WARNING: Don't commit API keys to version control!
# api_key = "your-key-here"

# Custom API base URL (optional)
# Useful for proxies, custom endpoints, or Ollama
# api_base = "https://custom-endpoint.com/v1"

# Maximum tokens to generate
# Higher = longer responses, higher cost
# Range: 1-32000 (model-dependent)
max_tokens = 4096

# Temperature (creativity vs consistency)
# 0.0 = deterministic, focused
# 1.0 = balanced (recommended)
# 2.0 = very creative, random
temperature = 0.7

# Top-p nucleus sampling (optional)
# 0.9 = use top 90% probability mass
# Lower = more focused, higher = more diverse
# top_p = 0.9


[generation]
# ----------------------------------------
# Strategy Generation Settings
# ----------------------------------------

# Number of retry attempts on failure
retry_attempts = 3

# Timeout for each generation attempt (seconds)
# Increase for slower providers or complex prompts
timeout_seconds = 60

# Validate generated code before returning
# Recommended: true (catches syntax errors)
validate_code = true
```

### Loading Configuration

**From file:**
```rust
use shape::ai::AIConfig;

let config = AIConfig::from_file("ai_config.toml")?;
```

**From environment:**
```rust
let config = AIConfig::from_env();
```

**Save to file:**
```rust
let config = AIConfig::default();
config.save_to_file("my_config.toml")?;
```

**Create template:**
```rust
AIConfig::create_default_template("ai_config.toml")?;
```

---

## Environment Variables

### API Keys (Required)

Set the API key for your chosen provider:

```bash
# Anthropic (Claude)
export ANTHROPIC_API_KEY=sk-ant-api03-...

# OpenAI (GPT)
export OPENAI_API_KEY=sk-...

# DeepSeek
export DEEPSEEK_API_KEY=...

# Ollama (no key needed)
# Just run: ollama serve
```

**Make permanent** (add to `~/.bashrc` or `~/.zshrc`):
```bash
echo 'export ANTHROPIC_API_KEY=sk-ant-...' >> ~/.bashrc
source ~/.bashrc
```

---

### Configuration Variables (Optional)

Override configuration without TOML file:

```bash
# Provider selection
export SHAPE_AI_PROVIDER=anthropic   # openai, anthropic, deepseek, ollama

# Model selection
export SHAPE_AI_MODEL=claude-sonnet-4

# Custom API endpoint
export SHAPE_AI_API_BASE=https://custom.api.com

# Generation parameters
export SHAPE_AI_MAX_TOKENS=8000
export SHAPE_AI_TEMPERATURE=0.8
export SHAPE_AI_TOP_P=0.95
```

**Example:**
```bash
# Configure for OpenAI GPT-4 with custom settings
export SHAPE_AI_PROVIDER=openai
export SHAPE_AI_MODEL=gpt-4-turbo
export SHAPE_AI_TEMPERATURE=0.5  # More deterministic
export SHAPE_AI_MAX_TOKENS=6000
export OPENAI_API_KEY=sk-...

# Now all ai-generate commands use these settings
cargo run --features ai -p shape --bin shape -- ai-generate "Your prompt"
```

---

## CLI Arguments

### Override Any Configuration

CLI arguments have **highest priority** and override everything else.

```bash
# Override provider
cargo run --features ai -p shape --bin shape -- ai-generate \
  --provider openai \
  "Your prompt"

# Override model
cargo run --features ai -p shape --bin shape -- ai-generate \
  --model gpt-4-turbo \
  "Your prompt"

# Use custom config file
cargo run --features ai -p shape --bin shape -- ai-generate \
  --config custom_config.toml \
  "Your prompt"

# Combine overrides
cargo run --features ai -p shape --bin shape -- ai-generate \
  --config base_config.toml \
  --provider openai \
  --model gpt-4 \
  "Your prompt"
```

---

## Provider-Specific Configuration

### OpenAI Configuration

**Recommended Models:**
- `gpt-4` - Most capable (expensive)
- `gpt-4-turbo` - Fast GPT-4 (recommended)
- `gpt-3.5-turbo` - Cheapest (good for testing)

**TOML:**
```toml
[llm]
provider = "openai"
model = "gpt-4-turbo"
max_tokens = 4096
temperature = 0.7
```

**Environment:**
```bash
export SHAPE_AI_PROVIDER=openai
export SHAPE_AI_MODEL=gpt-4-turbo
export OPENAI_API_KEY=sk-...
```

**Cost per 1M tokens (input/output):**
- GPT-4: $30/$60
- GPT-4-turbo: $10/$30
- GPT-3.5-turbo: $0.50/$1.50

---

### Anthropic Configuration

**Recommended Models:**
- `claude-sonnet-4` - Best balance (recommended)
- `claude-opus-4` - Most capable
- `claude-3-5-sonnet-20241022` - Previous version

**TOML:**
```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4"
max_tokens = 4096
temperature = 0.7
```

**Environment:**
```bash
export SHAPE_AI_PROVIDER=anthropic
export SHAPE_AI_MODEL=claude-sonnet-4
export ANTHROPIC_API_KEY=sk-ant-...
```

**Cost per 1M tokens (input/output):**
- Claude Opus 4: $15/$75
- Claude Sonnet 4: $3/$15
- Claude 3.5 Sonnet: $3/$15

---

### DeepSeek Configuration

**Recommended Models:**
- `deepseek-chat` - General purpose
- `deepseek-coder` - Code-optimized

**TOML:**
```toml
[llm]
provider = "deepseek"
model = "deepseek-chat"
max_tokens = 4096
temperature = 0.7
```

**Environment:**
```bash
export SHAPE_AI_PROVIDER=deepseek
export SHAPE_AI_MODEL=deepseek-chat
export DEEPSEEK_API_KEY=...
```

**Cost per 1M tokens (input/output):**
- DeepSeek Chat: $0.10/$0.20 (50x cheaper than GPT-4!)
- DeepSeek Coder: $0.10/$0.20

---

### Ollama Configuration (Local)

**Available Models:**
- `llama3` - Meta's Llama 3 (8B or 70B)
- `mistral` - Mistral 7B
- `codellama` - Code-specialized
- `qwen` - Alibaba's model
- Any other Ollama model

**TOML:**
```toml
[llm]
provider = "ollama"
model = "llama3"
api_base = "http://localhost:11434"  # Default
max_tokens = 4096
temperature = 0.7
```

**Environment:**
```bash
export SHAPE_AI_PROVIDER=ollama
export SHAPE_AI_MODEL=llama3
# No API key needed!
```

**Setup:**
```bash
# Install Ollama
curl https://ollama.ai/install.sh | sh

# Start server
ollama serve

# Pull a model (one-time)
ollama pull llama3

# Now you can generate unlimited strategies for free!
```

**Cost:** $0 (free, runs locally)

**Hardware Requirements:**
- **CPU**: Any modern CPU (slow but works)
- **GPU**: NVIDIA GPU recommended (10x faster)
- **RAM**: 8GB minimum (for 7-8B models)
- **Disk**: 4-8GB per model

---

## Best Practices

### Configuration Organization

**For Individual Developers:**
```bash
# Use environment variables
~/.bashrc:
  export ANTHROPIC_API_KEY=sk-ant-...
  export SHAPE_AI_PROVIDER=anthropic
```

**For Teams:**
```bash
# Check in base config (no API keys!)
project/ai_config.toml:
  [llm]
  provider = "anthropic"
  model = "claude-sonnet-4"
  temperature = 0.7

# Each dev sets their own API key
export ANTHROPIC_API_KEY=...
```

**For CI/CD:**
```yaml
# GitHub Actions example
env:
  ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}
  SHAPE_AI_PROVIDER: anthropic
  SHAPE_AI_MODEL: claude-sonnet-4
```

---

### Cost Optimization

**Strategy 1: Use DeepSeek for experimentation**
```toml
[llm]
provider = "deepseek"  # 50x cheaper than GPT-4
model = "deepseek-chat"
```

**Strategy 2: Lower max_tokens**
```toml
[llm]
max_tokens = 2048  # Shorter strategies = lower cost
```

**Strategy 3: Reduce temperature**
```toml
[llm]
temperature = 0.3  # More focused = fewer tokens used
```

**Strategy 4: Use local Ollama**
```toml
[llm]
provider = "ollama"  # Free, unlimited
model = "llama3"
```

---

### Quality Optimization

**For better code quality:**

```toml
[llm]
provider = "anthropic"
model = "claude-opus-4"    # Most capable
temperature = 0.5          # More consistent
max_tokens = 6000          # Allow detailed code

[generation]
validate_code = true       # Always validate
retry_attempts = 5         # More retries
```

**For faster iteration:**

```toml
[llm]
provider = "deepseek"
model = "deepseek-chat"
temperature = 0.7
max_tokens = 2048

[generation]
retry_attempts = 1
timeout_seconds = 30
```

---

### Security Best Practices

**✅ DO:**
- Store API keys in environment variables
- Use `.gitignore` for config files with keys
- Rotate API keys regularly
- Use read-only API keys when available
- Monitor API usage/costs

**❌ DON'T:**
- Commit API keys to version control
- Share API keys in team config files
- Use production keys for testing
- Expose keys in logs or error messages

**Example `.gitignore`:**
```gitignore
# Never commit these
ai_config.toml
.env
*.key

# Can commit these (templates)
ai_config.toml.example
```

---

## Examples

### Example 1: Development Setup

**File:** `~/.bashrc`
```bash
# Personal AI configuration
export ANTHROPIC_API_KEY=sk-ant-your-personal-key
export SHAPE_AI_PROVIDER=anthropic
export SHAPE_AI_MODEL=claude-sonnet-4
export SHAPE_AI_TEMPERATURE=0.7
```

**Usage:**
```bash
# Just works, uses your defaults
cargo run --features ai -p shape --bin shape -- ai-generate "RSI strategy"
```

---

### Example 2: Project Configuration

**File:** `project/ai_config.toml`
```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4"
max_tokens = 4096
temperature = 0.7

[generation]
retry_attempts = 3
timeout_seconds = 60
validate_code = true
```

**File:** `project/.env`
```bash
ANTHROPIC_API_KEY=sk-ant-project-specific-key
```

**Usage:**
```bash
# Load .env
source .env

# Use project config
cargo run --features ai -p shape --bin shape -- ai-generate \
  --config ai_config.toml \
  "Project strategy"
```

---

### Example 3: Multi-Provider Setup

**File:** `configs/anthropic.toml`
```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4"
temperature = 0.7
```

**File:** `configs/openai.toml`
```toml
[llm]
provider = "openai"
model = "gpt-4-turbo"
temperature = 0.7
```

**File:** `configs/deepseek.toml`
```toml
[llm]
provider = "deepseek"
model = "deepseek-chat"
temperature = 0.7
```

**Environment:**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
export OPENAI_API_KEY=sk-...
export DEEPSEEK_API_KEY=...
```

**Usage:**
```bash
# Try same prompt with different providers
PROMPT="Create a momentum strategy"

cargo run --features ai -p shape --bin shape -- ai-generate \
  --config configs/anthropic.toml "$PROMPT" > claude_version.shape

cargo run --features ai -p shape --bin shape -- ai-generate \
  --config configs/openai.toml "$PROMPT" > gpt_version.shape

cargo run --features ai -p shape --bin shape -- ai-generate \
  --config configs/deepseek.toml "$PROMPT" > deepseek_version.shape

# Compare results
diff claude_version.shape gpt_version.shape
```

---

### Example 4: Local Ollama Setup

**File:** `configs/local.toml`
```toml
[llm]
provider = "ollama"
model = "llama3"
api_base = "http://localhost:11434"
max_tokens = 4096
temperature = 0.8

[generation]
retry_attempts = 1       # Faster locally
timeout_seconds = 120    # Local inference can be slow
validate_code = true
```

**Setup:**
```bash
# Install Ollama
curl https://ollama.ai/install.sh | sh

# Start server
ollama serve &

# Pull model (one-time, ~4GB download)
ollama pull llama3

# Test
ollama run llama3 "Hello"
```

**Usage:**
```bash
# Generate unlimited strategies for free!
cargo run --features ai -p shape --bin shape -- ai-generate \
  --config configs/local.toml \
  "Create RSI strategy"
```

---

## Provider-Specific Configuration

### OpenAI Models

| Model | Context | Input $/1M | Output $/1M | Speed | Quality |
|-------|---------|------------|-------------|-------|---------|
| `gpt-4` | 8K | $30 | $60 | Slow | Excellent |
| `gpt-4-turbo` | 128K | $10 | $30 | Fast | Excellent |
| `gpt-4-turbo-preview` | 128K | $10 | $30 | Fast | Excellent |
| `gpt-3.5-turbo` | 16K | $0.50 | $1.50 | Very Fast | Good |
| `gpt-3.5-turbo-16k` | 16K | $3 | $4 | Very Fast | Good |

**Recommended:** `gpt-4-turbo` (best balance)

**Configuration:**
```toml
[llm]
provider = "openai"
model = "gpt-4-turbo"
max_tokens = 4096       # Adjust based on strategy complexity
temperature = 0.7       # 0.5-0.8 recommended for code
```

---

### Anthropic Models

| Model | Context | Input $/1M | Output $/1M | Speed | Quality |
|-------|---------|------------|-------------|-------|---------|
| `claude-opus-4` | 200K | $15 | $75 | Medium | Excellent |
| `claude-sonnet-4` | 200K | $3 | $15 | Fast | Excellent |
| `claude-3-5-sonnet-20241022` | 200K | $3 | $15 | Fast | Excellent |
| `claude-haiku-3-5` | 200K | $0.80 | $4 | Very Fast | Good |

**Recommended:** `claude-sonnet-4` (best value)

**Configuration:**
```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4"
max_tokens = 4096
temperature = 0.7
```

**Notes:**
- Claude is generally better at code generation
- Lower cost than OpenAI for same quality
- Faster response times
- Better instruction following

---

### DeepSeek Models

| Model | Context | Input $/1M | Output $/1M | Speed | Quality |
|-------|---------|------------|-------------|-------|---------|
| `deepseek-chat` | 32K | $0.10 | $0.20 | Fast | Good |
| `deepseek-coder` | 32K | $0.10 | $0.20 | Fast | Very Good |

**Recommended:** `deepseek-coder` (optimized for code)

**Configuration:**
```toml
[llm]
provider = "deepseek"
model = "deepseek-coder"
max_tokens = 4096
temperature = 0.7
```

**Notes:**
- 50-100x cheaper than OpenAI
- Surprisingly good code quality
- Fast response times
- Great for experimentation

---

### Ollama Models (Local)

| Model | Size | RAM | Speed (CPU) | Speed (GPU) | Quality |
|-------|------|-----|-------------|-------------|---------|
| `llama3:8b` | 4.7GB | 8GB | Slow | Fast | Good |
| `llama3:70b` | 40GB | 64GB | Very Slow | Medium | Excellent |
| `mistral` | 4.1GB | 8GB | Slow | Fast | Good |
| `codellama:7b` | 3.8GB | 8GB | Slow | Fast | Very Good |
| `codellama:34b` | 19GB | 32GB | Very Slow | Medium | Excellent |

**Recommended:** `codellama:7b` or `llama3:8b`

**Configuration:**
```toml
[llm]
provider = "ollama"
model = "codellama:7b"
api_base = "http://localhost:11434"
max_tokens = 4096
temperature = 0.8  # Can be higher for local, no cost

[generation]
timeout_seconds = 180  # Local can be slow on CPU
```

**Pull models:**
```bash
ollama pull llama3
ollama pull codellama:7b
ollama pull mistral
```

**List installed:**
```bash
ollama list
```

---

## Advanced Configuration

### Custom API Endpoints

**Use case:** Proxy, load balancer, or custom deployment

```toml
[llm]
provider = "openai"
model = "gpt-4"
api_base = "https://my-proxy.com/v1"
```

**Grok (xAI) via OpenAI-compatible API:**
```toml
[llm]
provider = "openai"
model = "grok-beta"
api_base = "https://api.x.ai/v1"
```

Then:
```bash
export OPENAI_API_KEY=your-grok-api-key
```

---

### Temperature Tuning

**Impact of temperature:**

| Value | Behavior | Use Case |
|-------|----------|----------|
| 0.0 | Deterministic, repetitive | Testing, consistency |
| 0.3-0.5 | Focused, conservative | Production strategies |
| 0.7-0.8 | Balanced, varied | Normal use (default) |
| 1.0-1.5 | Creative, diverse | Exploration, novel strategies |
| 1.5-2.0 | Very random, unusual | Experimentation only |

**Recommendation:**
- Start with 0.7
- Lower to 0.5 if getting invalid code
- Raise to 0.9 if strategies too similar

---

### Max Tokens Tuning

**Impact:**

| Value | Result Size | Cost | Use Case |
|-------|------------|------|----------|
| 1024 | Short, simple | Low | Basic strategies |
| 2048 | Medium | Medium | Most strategies |
| 4096 | Detailed | Higher | Complex strategies |
| 8192 | Very detailed | High | Multi-indicator systems |

**Recommendation:**
- Start with 4096
- Reduce to 2048 for cost savings
- Increase to 8192 for complex prompts

**Formula:**
- Simple strategy: ~500-1000 tokens
- Medium strategy: ~1000-2000 tokens
- Complex strategy: ~2000-4000 tokens

---

### Timeout Configuration

**Recommended values:**

| Provider | Recommended Timeout |
|----------|-------------------|
| OpenAI | 60 seconds |
| Anthropic | 60 seconds |
| DeepSeek | 45 seconds (faster) |
| Ollama (CPU) | 180 seconds (slower) |
| Ollama (GPU) | 60 seconds |

**Configuration:**
```toml
[generation]
timeout_seconds = 60  # Adjust based on provider
```

---

## Validation

### Code Validation Settings

```toml
[generation]
validate_code = true  # Recommended
```

**When enabled:**
- Parses generated code with Shape parser
- Reports syntax errors
- Shows warnings for suspicious patterns
- Still returns code even if invalid (user decides)

**When disabled:**
- Faster (skips parsing step)
- Returns raw LLM output
- May contain syntax errors
- Use only if you'll validate separately

---

## Configuration Profiles

### Profile: Conservative (Production)

**File:** `profiles/conservative.toml`
```toml
[llm]
provider = "anthropic"
model = "claude-opus-4"    # Most capable
max_tokens = 6000
temperature = 0.5          # Focused, consistent

[generation]
retry_attempts = 5         # More retries
timeout_seconds = 90
validate_code = true
```

**Use for:** Production strategies, real money

---

### Profile: Experimental (Research)

**File:** `profiles/experimental.toml`
```toml
[llm]
provider = "deepseek"      # Cheap
model = "deepseek-chat"
max_tokens = 3000
temperature = 1.2          # More creative

[generation]
retry_attempts = 1
timeout_seconds = 30
validate_code = false      # Skip for speed
```

**Use for:** Exploration, testing ideas, learning

---

### Profile: Budget (Cost-Effective)

**File:** `profiles/budget.toml`
```toml
[llm]
provider = "deepseek"
model = "deepseek-chat"
max_tokens = 2048
temperature = 0.7

[generation]
retry_attempts = 2
timeout_seconds = 45
validate_code = true
```

**Use for:** High-volume generation, tight budgets

---

### Profile: Local (Private)

**File:** `profiles/local.toml`
```toml
[llm]
provider = "ollama"
model = "codellama:7b"
api_base = "http://localhost:11434"
max_tokens = 4096
temperature = 0.8

[generation]
retry_attempts = 1
timeout_seconds = 180
validate_code = true
```

**Use for:** Privacy-sensitive, unlimited generation

---

## Troubleshooting Configuration

### Check Current Configuration

```bash
# See what config will be used (shows effective config)
cargo run --features ai -p shape --bin shape -- ai-generate \
  "test" --config my_config.toml

# It will print: "Using: anthropic / claude-sonnet-4"
```

### Verify API Key

```bash
# Check if key is set
echo $ANTHROPIC_API_KEY

# Should print: sk-ant-...
# If empty, key is not set
```

### Test Configuration

```bash
# Minimal test
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a simple RSI strategy that buys when RSI < 30" \
  --config test_config.toml
```

### Debug Configuration Loading

```rust
// Add to your code temporarily
let config = AIConfig::from_file("ai_config.toml")?;
eprintln!("Loaded config: {:?}", config);
```

---

## Migration Guide

### From Environment Variables to TOML

```bash
# Current: Environment variables
export ANTHROPIC_API_KEY=sk-ant-...
export SHAPE_AI_MODEL=claude-sonnet-4

# Create TOML config
cat > ai_config.toml << EOF
[llm]
provider = "anthropic"
model = "claude-sonnet-4"
max_tokens = 4096
temperature = 0.7
EOF

# API key still in environment (more secure)
# Now use config file for other settings
cargo run --features ai -p shape --bin shape -- ai-generate \
  --config ai_config.toml "prompt"
```

---

### From One Provider to Another

**From OpenAI to Anthropic:**

**Before:**
```toml
[llm]
provider = "openai"
model = "gpt-4"
```

**After:**
```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4"  # Equivalent capability
```

**API Key:**
```bash
# Was:
export OPENAI_API_KEY=sk-...

# Now:
export ANTHROPIC_API_KEY=sk-ant-...
```

**Cost Impact:**
- GPT-4: $30-60/1M tokens
- Claude Sonnet: $3-15/1M tokens
- **Savings: 80-90%**

---

## Reference

### All Configuration Options

| Category | Option | Type | Default | Description |
|----------|--------|------|---------|-------------|
| **llm** | provider | String | "anthropic" | Provider name |
| | model | String | "claude-sonnet-4" | Model name |
| | api_key | String? | None | API key (use env var) |
| | api_base | String? | None | Custom endpoint |
| | max_tokens | Number | 4096 | Max generation tokens |
| | temperature | Number | 0.7 | Sampling temperature |
| | top_p | Number? | None | Nucleus sampling |
| **generation** | retry_attempts | Number | 3 | Retry count |
| | timeout_seconds | Number | 60 | Request timeout |
| | validate_code | Boolean | true | Validate syntax |

### All Environment Variables

| Variable | Type | Example | Description |
|----------|------|---------|-------------|
| `ANTHROPIC_API_KEY` | String | sk-ant-... | Anthropic API key |
| `OPENAI_API_KEY` | String | sk-... | OpenAI API key |
| `DEEPSEEK_API_KEY` | String | ... | DeepSeek API key |
| `SHAPE_AI_PROVIDER` | String | anthropic | Provider override |
| `SHAPE_AI_MODEL` | String | claude-sonnet-4 | Model override |
| `SHAPE_AI_MAX_TOKENS` | Number | 4096 | Token limit |
| `SHAPE_AI_TEMPERATURE` | Number | 0.7 | Temperature |
| `SHAPE_AI_TOP_P` | Number | 0.9 | Top-p sampling |
| `SHAPE_AI_API_BASE` | String | https://... | Custom endpoint |

---

## See Also

- [AI_GUIDE.md](./AI_GUIDE.md) - User guide
- [AI_API_REFERENCE.md](../reference/AI_API_REFERENCE.md) - API documentation
- [AI_ARCHITECTURE.md](../architecture/AI_ARCHITECTURE.md) - Technical architecture

---

**Last Updated:** 2026-01-01
**Version:** 1.0
**Status:** Complete for Phases 1-3
