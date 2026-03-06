# Candle Reference System Design

## Overview

A dual-mode candle reference system that combines absolute datetime references with relative indexing, timezone awareness, and market-specific keywords.

## Core Concepts

### 1. Reference Types

```shape
# Set reference point (returns a CandleReference type)
let ref = candle[@"2024-01-15 09:30:00 EST"];
let ref = candle[@market_open];
let ref = candle[@now];

# Access relative to reference (returns Candle type)
ref[0]     # The candle at the reference time
ref[-1]    # One candle before
ref[1]     # One candle after
ref[-5:5]  # Range from 5 before to 5 after

# Direct access (returns Candle type)
candle[@"2024-01-15 09:30:00 EST"][0]  # Same as ref[0] above
```

### 2. Type System

```rust
// AST types
enum CandleAccess {
    // Creates a reference point
    DateTimeReference {
        datetime: DateTimeExpr,
        timezone: Option<String>,
        timeframe: Option<Timeframe>,
    },
    
    // Accesses candle relative to reference
    RelativeAccess {
        reference: Box<Expr>, // Must evaluate to CandleReference
        index: i32,
    },
    
    // Range access
    RangeAccess {
        reference: Box<Expr>,
        start: i32,
        end: i32,
    },
}

// Runtime types
enum Value {
    // ... existing variants
    CandleReference(CandleReferenceValue),
    // ... Candle variant already exists
}

struct CandleReferenceValue {
    datetime: DateTime<Tz>,
    symbol: String,
    timeframe: Timeframe,
}
```

### 3. DateTime Expressions

```shape
# Literal datetime with timezone
@"2024-01-15 09:30:00 EST"
@"2024-01-15 09:30:00 America/New_York"
@"2024-01-15 09:30:00"  # Local timezone

# Market keywords
@market_open    # Today's market open
@market_close   # Today's market close
@pre_market     # Pre-market open (04:00 ET)
@after_hours    # After-hours open (16:00 ET)

# Relative dates
@today          # Today at market open
@yesterday      # Yesterday at market open
@now            # Current time

# Date arithmetic
@market_open + 30m     # 30 minutes after open
@market_close - 1h     # 1 hour before close
@"2024-01-15" + 2d     # 2 days later

# Market-aware arithmetic
@market_open + 2 bars  # 2 candles after open (timeframe aware)
```

### 4. Timezone Handling

```shape
# Set default timezone for session
use timezone "America/New_York";

# Explicit timezone
let ny_open = candle[@"2024-01-15 09:30:00 America/New_York"];
let tokyo_open = candle[@"2024-01-15 09:00:00 Asia/Tokyo"];

# Convert between timezones
let london_time = ny_open.datetime in "Europe/London";

# Market hours are timezone-aware
@market_open  # Knows NYSE is in ET
@market_open[TSE]  # Tokyo Stock Exchange open
```

### 5. Market Keywords Implementation

```rust
pub struct MarketCalendar {
    exchange: Exchange,
    holidays: Vec<Date>,
    regular_hours: MarketHours,
    extended_hours: Option<ExtendedHours>,
}

pub struct MarketHours {
    open: NaiveTime,
    close: NaiveTime,
    timezone: Tz,
}

impl MarketCalendar {
    pub fn resolve_keyword(&self, keyword: &str, date: Date) -> Result<DateTime<Tz>> {
        match keyword {
            "market_open" => {
                let open_time = date.and_time(self.regular_hours.open);
                Ok(self.regular_hours.timezone.from_local_datetime(&open_time)
                    .single()
                    .ok_or("Invalid market open time")?)
            }
            "market_close" => {
                // Similar for close
            }
            "pre_market" => {
                // 04:00 ET for US markets
            }
            // ... other keywords
        }
    }
}
```

### 6. Usage Examples

```shape
# Strategy that trades relative to market open
let open_ref = candle[@market_open];

# Check first 30 minutes of trading
for i in range(0, 6) {  # 6 x 5-minute bars = 30 minutes
    if open_ref[i].volume > open_ref[0].volume * 2 {
        print("High volume spike at " + (i * 5) + " minutes after open");
    }
}

# Compare London and NY sessions
let london_open = candle[@"09:00:00 Europe/London"];
let ny_open = candle[@"09:30:00 America/New_York"];

# These might be different candles even on same day!
print("London open: " + london_open[0].close);
print("NY open: " + ny_open[0].close);

# Pattern that looks for reversal at specific time
pattern lunch_reversal {
    let noon = candle[@"12:00:00"];
    
    # Check if morning was bullish
    let morning_trend = noon[-1].close > candle[@market_open][0].close;
    
    # Look for reversal after noon
    noon[0].close < noon[0].open and
    noon[1].close < noon[1].open and
    morning_trend
}

# Real-time trading
let current = candle[@now];
if current[0].close > current[-1].high {
    signal("Breakout at " + current[0].datetime);
}
```

### 7. Implementation Phases

#### Phase 1: Basic DateTime References
- Parse `@"datetime"` syntax
- Create CandleReference type
- Implement relative indexing from reference

#### Phase 2: Timezone Support
- Add timezone parsing
- Integrate timezone library (chrono-tz)
- Handle timezone conversions

#### Phase 3: Market Keywords
- Implement market calendar
- Add keyword resolution
- Support exchange-specific keywords

#### Phase 4: Advanced Features
- Date arithmetic
- Bar-based arithmetic
- Multi-exchange support

### 8. Benefits

1. **Intuitive**: Set a reference point and work relative to it
2. **Timezone-Safe**: Explicit timezone handling prevents errors
3. **Market-Aware**: Keywords understand trading hours
4. **Type-Safe**: Different types for references vs candles
5. **Flexible**: Supports both absolute and relative access

### 9. Grammar Updates

```pest
candle_access = {
    "candle" ~ "[" ~ datetime_expr ~ "]" ~ ("[" ~ index ~ "]")?
}

datetime_expr = {
    datetime_literal |
    market_keyword |
    datetime_arithmetic
}

datetime_literal = {
    "@" ~ string ~ timezone?
}

market_keyword = {
    "@" ~ ("market_open" | "market_close" | "pre_market" | "after_hours" | 
           "now" | "today" | "yesterday")
}

timezone = {
    ident  // Like EST, PST, UTC
    | string  // Like "America/New_York"
}

datetime_arithmetic = {
    datetime_expr ~ ("+" | "-") ~ duration
}

duration = {
    number ~ ("s" | "m" | "h" | "d" | "bars")
}
```

### 10. Migration Examples

```shape
# Old way (ambiguous)
candle[0].close  # Which candle?

# New way (explicit)
candle[@now][0].close  # Current candle
candle[@market_open][0].close  # Open candle
candle[@"2024-01-15 09:30:00"][0].close  # Specific time

# Old way (pattern)
pattern hammer {
    candle[0].body < candle[0].range * 0.1
}

# New way (same in pattern context)
pattern hammer {
    # In pattern context, candle[0] still works
    # It's relative to the pattern evaluation position
    candle[0].body < candle[0].range * 0.1
}

# But outside patterns, you need a reference
let last_candle = candle[@now][0];
if last_candle.body < last_candle.range * 0.1 {
    print("Possible hammer at " + last_candle.datetime);
}
```