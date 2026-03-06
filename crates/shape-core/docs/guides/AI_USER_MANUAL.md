# Shape AI - User Manual

## What is Shape AI?

Shape AI helps you create and test trading strategies using natural language. Instead of writing code manually, describe what you want and let AI generate it for you.

**What you can do:**
- Generate trading strategies from plain English descriptions
- Test multiple strategies at once and rank them by performance
- Use AI directly in your Shape programs
- Switch between different AI providers (OpenAI, Anthropic, DeepSeek, local models)

---

## Getting Started

### 1. Build Shape with AI Support

```bash
cd shape
cargo build --features ai
```

### 2. Get an API Key

Pick one provider and get an API key:

- **Anthropic (Claude)** - Recommended, best quality
  - Sign up: https://console.anthropic.com/
  - Get API key from dashboard
  - Cost: ~$0.003 per strategy

- **DeepSeek** - Cheapest, good quality
  - Sign up: https://platform.deepseek.com/
  - Get API key
  - Cost: ~$0.0001 per strategy (50x cheaper!)

- **OpenAI (GPT)** - Popular, expensive
  - Sign up: https://platform.openai.com/
  - Get API key
  - Cost: ~$0.01 per strategy

- **Ollama** - Free, runs on your computer
  - Install: https://ollama.ai/
  - No API key needed
  - Cost: $0 (free!)

### 3. Set Your API Key

```bash
# For Anthropic
export ANTHROPIC_API_KEY=sk-ant-your-key-here

# For OpenAI
export OPENAI_API_KEY=sk-your-key-here

# For DeepSeek
export DEEPSEEK_API_KEY=your-key-here

# For Ollama (no key needed, just run)
ollama serve
```

### 4. Generate Your First Strategy

```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a mean reversion strategy using RSI"
```

You'll see generated Shape code printed to your screen!

---

## Main Features

### Feature 1: Generate Strategies from Natural Language

**What it does:** Converts your description into working Shape code.

**How to use:**
```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a momentum strategy using MACD crossovers"
```

**Save to file:**
```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a Bollinger Bands strategy" \
  --output my_strategy.shape
```

**Use different AI:**
```bash
# Use OpenAI instead
cargo run --features ai -p shape --bin shape -- ai-generate \
  --provider openai \
  "Create a trend following strategy"

# Use DeepSeek (cheapest)
cargo run --features ai -p shape --bin shape -- ai-generate \
  --provider deepseek \
  "Create a breakout strategy"

# Use local Ollama (free)
cargo run --features ai -p shape --bin shape -- ai-generate \
  --provider ollama --model llama3 \
  "Create a simple SMA strategy"
```

### Feature 2: Test Multiple Strategies at Once

**What it does:** Backtests multiple strategies and shows which performs best.

**Step 1:** Create a JSON file with your strategies

**File:** `my_strategies.json`
```json
[
  {
    "name": "RSI Strategy",
    "code": "@indicators({ rsi: rsi(series(\"close\"), 14) })\nfunction strategy() {\n  if (rsi[-1] < 30) return { action: \"buy\" };\n  return \"none\";\n}",
    "symbol": "ES",
    "timeframe": "1h"
  },
  {
    "name": "SMA Crossover",
    "code": "@indicators({ sma_fast: sma(series(\"close\"), 10), sma_slow: sma(series(\"close\"), 30) })\nfunction strategy() {\n  if (sma_fast[-1] > sma_slow[-1]) return { action: \"buy\" };\n  return \"none\";\n}",
    "symbol": "ES",
    "timeframe": "1h"
  }
]
```

**Step 2:** Run evaluation

```bash
cargo run -p shape --bin shape -- ai-eval my_strategies.json
```

**You'll see a ranked table:**
```
Rank   Strategy              Sharpe    Return%    MaxDD%    Win%    Trades
#1     SMA Crossover          2.45      45.30     12.45    65.50      120
#2     RSI Strategy           2.12      38.20     15.20    62.30      110
```

**Rank by different metrics:**
```bash
# By win rate
cargo run -p shape --bin shape -- ai-eval my_strategies.json --rank-by win_rate

# By max drawdown (lower is better)
cargo run -p shape --bin shape -- ai-eval my_strategies.json --rank-by max_drawdown
```

### Feature 3: Use AI in Shape Code

**What it does:** Call AI functions directly in your Shape programs.

**File:** `my_program.shape`
```shape
import { ai_generate } from "stdlib/ai/generate";

// Generate a strategy
let strategy = ai_generate("Create a volatility breakout strategy using ATR");

// Print the code
print(strategy);

// You can save it, test it, or use it however you want
```

**Run:**
```bash
cargo run --features ai -p shape --bin shape -- run my_program.shape
```

---

## Common Use Cases

### Use Case 1: Quick Strategy Ideas

"I want to test a trading idea quickly without writing code."

```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a strategy that buys when price crosses above 200-day moving average with volume confirmation" \
  --output quick_idea.shape
```

### Use Case 2: Learning Shape

"I want to learn Shape syntax by seeing examples."

```bash
# Generate different types to learn patterns
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a simple RSI strategy" > example1.shape

cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a MACD strategy" > example2.shape

# Study the generated code to learn
cat example1.shape
cat example2.shape
```

### Use Case 3: Strategy Variations

"I have a strategy but want to try variations."

```bash
# Original idea
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create RSI oversold strategy" > rsi_v1.shape

# Variation 1: Add confirmation
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create RSI oversold strategy with SMA trend confirmation" > rsi_v2.shape

# Variation 2: Add risk management
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create RSI oversold strategy with ATR-based stops" > rsi_v3.shape

# Test all three (create JSON batch file)
cargo run -p shape --bin shape -- ai-eval rsi_versions.json
```

### Use Case 4: Experimenting with Indicators

"I want to try different indicator combinations."

```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a strategy combining RSI, MACD, and Bollinger Bands"
```

---

## Configuration

### Simple Setup (Environment Variables)

```bash
# Set your API key
export ANTHROPIC_API_KEY=your-key

# Optional: Choose provider (default is Anthropic)
export SHAPE_AI_PROVIDER=anthropic

# Optional: Choose model (default is claude-sonnet-4)
export SHAPE_AI_MODEL=claude-sonnet-4
```

### Advanced Setup (Config File)

Create `ai_config.toml`:

```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4"
max_tokens = 4096
temperature = 0.7

[generation]
validate_code = true
retry_attempts = 3
timeout_seconds = 60
```

Use it:
```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  --config ai_config.toml \
  "Your prompt"
```

---

## Available AI Providers

### Anthropic (Claude) - Recommended

**Best for:** High-quality code generation
**Cost:** ~$0.003 per strategy
**Speed:** 3-5 seconds

**Setup:**
```bash
export ANTHROPIC_API_KEY=sk-ant-...
cargo run --features ai -p shape --bin shape -- ai-generate "prompt"
```

**Models:**
- `claude-sonnet-4` - Best value (recommended)
- `claude-opus-4` - Highest quality

### DeepSeek - Most Affordable

**Best for:** Experimentation, learning, high volume
**Cost:** ~$0.0001 per strategy (50x cheaper!)
**Speed:** 2-3 seconds

**Setup:**
```bash
export DEEPSEEK_API_KEY=your-key
cargo run --features ai -p shape --bin shape -- ai-generate \
  --provider deepseek \
  "prompt"
```

### OpenAI (GPT) - Most Popular

**Best for:** If you already have OpenAI credits
**Cost:** ~$0.01 per strategy
**Speed:** 4-6 seconds

**Setup:**
```bash
export OPENAI_API_KEY=sk-...
cargo run --features ai -p shape --bin shape -- ai-generate \
  --provider openai \
  "prompt"
```

**Models:**
- `gpt-4-turbo` - Recommended
- `gpt-4` - Highest quality, slower
- `gpt-3.5-turbo` - Fastest, cheapest

### Ollama - Free & Private

**Best for:** Privacy, unlimited generation, no internet
**Cost:** $0 (completely free!)
**Speed:** 10-20 seconds (CPU), 2-3 seconds (GPU)

**Setup:**
```bash
# One-time setup
curl https://ollama.ai/install.sh | sh
ollama pull llama3

# Run server
ollama serve

# Generate
cargo run --features ai -p shape --bin shape -- ai-generate \
  --provider ollama --model llama3 \
  "prompt"
```

---

## Tips for Writing Good Prompts

### ✅ Good Prompts

**Be specific:**
```
"Create a mean reversion strategy that buys when RSI goes below 30 and sells when it goes above 70"
```

**Name indicators:**
```
"Create a strategy using SMA(20), SMA(50), and RSI(14) for trend following"
```

**Include risk management:**
```
"Create a momentum strategy with 2% stop loss and 3:1 risk-reward ratio"
```

**Specify conditions:**
```
"Create a strategy that only trades during uptrends (price above 200 SMA) using RSI oversold"
```

### ❌ Prompts to Avoid

Too vague:
```
"Create a good trading strategy"
```

Unrealistic:
```
"Create a strategy that never loses"
```

Too complex:
```
"Create a multi-timeframe strategy with machine learning and quantum indicators..."
```

---

## Troubleshooting

### "AI features not enabled"

You need to build with the `ai` feature:
```bash
cargo build --features ai -p shape
```

### "API key not found"

Set the environment variable:
```bash
export ANTHROPIC_API_KEY=your-key
```

Check if it's set:
```bash
echo $ANTHROPIC_API_KEY
```

### "Generated code has errors"

Try lowering the temperature:
```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  --model claude-opus-4 \  # Use more capable model
  "your prompt"
```

Or try a different provider:
```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  --provider openai --model gpt-4 \
  "your prompt"
```

### "Connection to Ollama failed"

Make sure Ollama is running:
```bash
ollama serve
```

Check if it's running:
```bash
curl http://localhost:11434/api/tags
```

---

## Examples

### Example 1: Generate and Save

```bash
cargo run --features ai -p shape --bin shape -- ai-generate \
  "Create a Bollinger Bands mean reversion strategy" \
  --output bb_strategy.shape

# Now you have bb_strategy.shape ready to use!
```

### Example 2: Compare Strategies

```bash
# Generate 3 variations
cargo run --features ai -p shape --bin shape -- ai-generate \
  "RSI strategy" --output v1.shape

cargo run --features ai -p shape --bin shape -- ai-generate \
  "RSI strategy with SMA filter" --output v2.shape

cargo run --features ai -p shape --bin shape -- ai-generate \
  "RSI strategy with Bollinger Bands" --output v3.shape

# Create JSON batch file with all 3
# Then evaluate:
cargo run -p shape --bin shape -- ai-eval versions.json
```

### Example 3: Use in Shape

```shape
import { ai_generate } from "stdlib/ai/generate";

let strategy = ai_generate("Create momentum strategy");
print(strategy);
```

---

## Getting Help

### Check Command Help

```bash
cargo run --features ai -p shape --bin shape -- ai-generate --help
cargo run -p shape --bin shape -- ai-eval --help
```

### More Documentation

- **AI_API_REFERENCE.md** - Complete API documentation
- **AI_CONFIGURATION.md** - All configuration options
- **AI_ARCHITECTURE.md** - How it works internally

---

## Frequently Asked Questions

**Q: Does AI guarantee profitable strategies?**
No. AI generates syntactically correct code based on common trading patterns, but profitability depends on market conditions, risk management, and many other factors. Always backtest thoroughly.

**Q: Which provider should I use?**
- For best quality: Anthropic Claude
- For lowest cost: DeepSeek
- For privacy: Ollama (local)

**Q: Can I use this without internet?**
Yes, with Ollama. All other providers require internet.

**Q: How much does it cost?**
- DeepSeek: ~$0.10 for 1000 strategies
- Anthropic: ~$3 for 1000 strategies
- OpenAI: ~$10 for 1000 strategies
- Ollama: $0 (free)

**Q: Is my strategy idea shared with the AI company?**
- OpenAI/Anthropic/DeepSeek: Yes, your prompts are sent to their servers
- Ollama: No, everything stays on your computer

**Q: Can AI copy my strategies?**
AI providers have policies against training on your data. But for maximum privacy, use Ollama.

---

**Version:** 1.0
**Last Updated:** 2026-01-01
