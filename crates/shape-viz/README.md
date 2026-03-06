# fchart - High-Performance Financial Chart Renderer

A blazing-fast financial chart rendering library built with Rust and WebGPU, supporting multiple platforms including CLI (via Kitty image protocol) and native applications.

## Features

- **GPU-Accelerated Rendering**: Uses WebGPU for maximum performance
- **Multi-Platform Support**:
  - CLI rendering via Kitty/iTerm2/Sixel protocols
  - Native desktop applications
- **High Performance**: Designed for 60+ FPS interactive charts
- **TradingView-Style Charts**: Candlesticks, indicators, overlays
- **Trading Overlays**: Stop-loss, take-profit zones with TradingView-style visualization
- **Multiple Indicators**: Arbitrary number of EMAs with custom colors
- **Rust-Powered**: Memory safe and blazingly fast

## Current Status

- [x] Core architecture and data structures
- [x] GPU renderer with WebGPU
- [x] CLI rendering with Kitty protocol support
- [x] Candlestick chart rendering
- [x] Integration with market-data crate
- [x] Event-driven interactive mode
- [x] Support for futures and stock data
- [x] Advanced theme system with multiple built-in themes
- [x] Technical indicators (EMA with arbitrary periods/colors)
- [x] Trading overlays (stop-loss, take-profit, position regions)
- [ ] Native application wrapper (winit/egui)
- [ ] Real-time data streaming support

## Quick Start

### Prerequisites

- Rust 1.75+ (2024 edition)
- A terminal with Kitty graphics protocol support (Kitty, WezTerm) for CLI rendering

### Building

```bash
cd fchart
cargo build --release
```

### Running the Demo

```bash
# Display a demo chart with all features
cargo run --bin fchart-cli -- demo --all-features

# Save to file
cargo run --bin fchart-cli -- demo --all-features --save chart.png

# With custom size
cargo run --bin fchart-cli -- demo --width 1920 --height 1080 --all-features --save chart.png
```

## CLI Usage

### Demo Command

```bash
fchart-cli demo [OPTIONS]

Options:
  -w, --width <WIDTH>       Chart width in pixels [default: 800]
  -H, --height <HEIGHT>     Chart height in pixels [default: 400]
      --save [<FILE>]       Save chart as PNG file
      --theme <THEME>       Theme name [default: tradingview-dark]
      --layers <LAYERS>     Comma-separated list of layers to enable
      --save-layers         Save each layer as a separate image
  -i, --interactive         Interactive mode
      --dataset <DATASET>   Dataset: synthetic, sample [default: synthetic]
      --stop-loss <PRICE>   Stop-loss price level
      --take-profit <PRICE> Take-profit price level
      --demo-trades         Add demo trade markers
      --all-features        Show all features (multiple EMAs, trading overlay)
```

### Load Command

```bash
fchart-cli load -s <SYMBOL> [OPTIONS]

Options:
  -s, --symbol <SYMBOL>     Symbol to load (e.g., ES1!, AAPL)
  -t, --timeframe <TF>      Timeframe: 1m, 5m, 15m, 30m, 1h, 4h, 1d [default: 1h]
  -d, --days <DAYS>         Last N days of data
      --start <YYYY-MM-DD>  Start date
      --end <YYYY-MM-DD>    End date
  -w, --width <WIDTH>       Chart width in pixels [default: 800]
  -H, --height <HEIGHT>     Chart height in pixels [default: 400]
  -i, --interactive         Enable interactive mode
      --theme <THEME>       Theme name
      --save [<FILE>]       Save chart as PNG file
```

### Import Command

```bash
fchart-cli import -s <SYMBOL> -p <PATH> [OPTIONS]

Options:
  -s, --symbol <SYMBOL>     Symbol to import
  -p, --path <PATH>         Path to CSV file or futures directory
  -t, --timeframe <TF>      Timeframe [default: 1m]
      --futures             Import futures with rollover
      --start <YYYY-MM-DD>  Start date for futures
      --end <YYYY-MM-DD>    End date for futures
```

## Trading Overlay API

fchart provides a complete trading overlay system for visualizing positions, stop-loss, and take-profit levels in TradingView style.

### Types

#### TradeSide

```rust
pub enum TradeSide {
    Long,   // Buy position
    Short,  // Sell position
}
```

#### MarkerType

```rust
pub enum MarkerType {
    Entry,      // Position entry point
    Exit,       // Position exit point
    StopLoss,   // Stop-loss marker
    TakeProfit, // Take-profit marker
    Custom,     // Custom marker (circle)
}
```

#### TradeMarker

Individual entry/exit markers on the chart:

```rust
use fchart_core::layers::{TradeMarker, TradeSide, MarkerType};

// Create an entry marker
let entry = TradeMarker::entry(timestamp, price, TradeSide::Long)
    .with_label("BUY")
    .with_size(1.5);  // 1.5x normal size

// Create an exit marker
let exit = TradeMarker::exit(timestamp, price, TradeSide::Long)
    .with_label("SELL");

// Create custom marker
let marker = TradeMarker::new(timestamp, price, TradeSide::Long, MarkerType::Custom)
    .with_color(Color::hex(0x00ff00));
```

#### PriceLevel

Horizontal price lines for stop-loss, take-profit, or custom levels:

```rust
use fchart_core::layers::{PriceLevel, LineStyle};

// Stop-loss line (red, dashed)
let sl = PriceLevel::stop_loss(95.0);

// Take-profit line (green, dashed)
let tp = PriceLevel::take_profit(110.0);

// Custom price level
let level = PriceLevel::new(100.0, Color::hex(0xffff00))
    .with_label("Support")
    .with_style(LineStyle::Solid)
    .with_width(2.0)
    .with_time_range(Some(start_time), Some(end_time))
    .extend_to_right(false);
```

#### PositionRegion

TradingView-style position visualization with shaded risk/reward zones:

```rust
use fchart_core::layers::{PositionRegion, TradeSide};

// Create a position with entry, exit, and SL/TP levels
let position = PositionRegion::new(
    entry_time,      // DateTime<Utc>
    entry_price,     // f64
    exit_time,       // DateTime<Utc>
    exit_price,      // f64
    TradeSide::Long,
).with_levels(
    Some(stop_loss_price),   // Option<f64>
    Some(take_profit_price), // Option<f64>
);

// Mark as still open (no exit marker)
let open_position = PositionRegion::new(...)
    .with_levels(Some(sl), Some(tp))
    .open();
```

### TradingOverlayLayer

The main layer for rendering trading visualizations:

```rust
use fchart_core::layers::{
    TradingOverlayLayer, TradingOverlayConfig,
    TradeMarker, PriceLevel, PositionRegion, TradeSide,
};

// Create with default config
let mut trading_layer = TradingOverlayLayer::new();

// Or with custom config
let config = TradingOverlayConfig {
    show_entries: true,
    show_exits: true,
    show_regions: true,
    show_levels: true,
    region_opacity: 0.15,
    marker_size: 16.0,
};
let mut trading_layer = TradingOverlayLayer::with_config(config);

// Add markers
trading_layer.add_marker(TradeMarker::entry(ts, 100.0, TradeSide::Long));
trading_layer.add_marker(TradeMarker::exit(ts, 105.0, TradeSide::Long));

// Add price levels
trading_layer.add_level(PriceLevel::stop_loss(95.0));
trading_layer.add_level(PriceLevel::take_profit(110.0));

// Add TradingView-style position with shaded zones
let position = PositionRegion::new(entry_ts, 100.0, exit_ts, 105.0, TradeSide::Long)
    .with_levels(Some(95.0), Some(110.0));
trading_layer.add_position(position);

// Add to chart
chart.add_layer(Box::new(trading_layer));
```

### Complete Example

```rust
use chrono::{DateTime, Utc};
use fchart_core::{Chart, ChartConfig, ChartData};
use fchart_core::layers::{
    EmaConfig, EmaLayer,
    TradingOverlayLayer, PositionRegion, PriceLevel, TradeSide,
};

async fn create_chart_with_trading(data: CandleData) -> Result<()> {
    // Create chart
    let config = ChartConfig {
        width: 1280,
        height: 800,
        ..Default::default()
    };
    let mut chart = Chart::new_financial(config).await?;

    // Add multiple EMAs with custom colors
    chart.add_layer(Box::new(EmaLayer::with_config(EmaConfig {
        period: 10,
        line_width: 1.5,
        color: Some([0.0, 0.8, 0.8, 1.0]), // Cyan
        opacity: 1.0,
    })));

    chart.add_layer(Box::new(EmaLayer::with_config(EmaConfig {
        period: 20,
        line_width: 1.5,
        color: Some([1.0, 0.85, 0.0, 1.0]), // Yellow
        opacity: 1.0,
    })));

    chart.add_layer(Box::new(EmaLayer::with_config(EmaConfig {
        period: 50,
        line_width: 2.0,
        color: Some([0.9, 0.3, 0.9, 1.0]), // Magenta
        opacity: 1.0,
    })));

    // Add trading overlay
    let mut trading = TradingOverlayLayer::new();

    // Add a position with TradingView-style visualization
    let entry_time = DateTime::from_timestamp(data.timestamps[20], 0).unwrap();
    let exit_time = DateTime::from_timestamp(data.timestamps[80], 0).unwrap();
    let entry_price = data.closes[20];
    let stop_loss = entry_price * 0.98;   // 2% stop loss
    let take_profit = entry_price * 1.06; // 6% take profit
    let exit_price = data.closes[80];

    let position = PositionRegion::new(
        entry_time,
        entry_price,
        exit_time,
        exit_price,
        TradeSide::Long,
    ).with_levels(Some(stop_loss), Some(take_profit));

    trading.add_position(position);
    chart.add_layer(Box::new(trading));

    // Set data and render
    chart.set_data(ChartData::new(data))?;
    let rgba = chart.render().await?;

    Ok(())
}
```

## Indicator API

### EmaLayer

Add exponential moving averages with custom configuration:

```rust
use fchart_core::layers::{EmaLayer, EmaConfig};

// Simple EMA with period
let ema = EmaLayer::new(50);

// EMA with full configuration
let ema = EmaLayer::with_config(EmaConfig {
    period: 20,
    line_width: 2.0,
    color: Some([1.0, 0.5, 0.0, 1.0]), // Orange RGBA
    opacity: 0.8,
});

chart.add_layer(Box::new(ema));
```

You can add any number of indicators - each is an independent layer:

```rust
// Add multiple EMAs
for period in [10, 20, 50, 100, 200] {
    chart.add_layer(Box::new(EmaLayer::new(period)));
}
```

## Theme System

### Built-in Themes

1. **tradingview-dark** - TradingView-inspired dark theme (default)
2. **reference-dark** - Deep dark blue theme
3. **midnight** - Deep blue/purple dark theme
4. **monokai** - Popular code editor theme
5. **light** - Clean light theme
6. **high-contrast-dark** - Accessibility-focused

### Using Themes

```bash
fchart-cli demo --theme midnight --save chart.png
```

```rust
use fchart_core::theme::ChartTheme;

let mut config = ChartConfig::default();
config.theme = ChartTheme::by_name("midnight")
    .unwrap_or_else(ChartTheme::tradingview_dark);
```

### Creating Custom Themes

```rust
use fchart_core::theme::{ChartTheme, ColorScheme, Color};

let colors = ColorScheme::custom(
    Color::hex(0x0a0e1a),  // Background
    Color::hex(0x00d4aa),  // Bullish candles
    Color::hex(0xff0066),  // Bearish candles
    Color::hex(0x9ca3af),  // Text
    Color::hex(0x1a1e2a),  // Grid
);

let theme = ChartTheme::new("My Theme", colors);
```

## Layer System

fchart uses a composable layer system. Each visual element is a separate layer:

| Layer | Stage | Description |
|-------|-------|-------------|
| BackgroundLayer | ScreenBackground | Chart background color |
| SessionShadingLayer | ChartBackground | Trading session highlights |
| GridLayer | ChartUnderlay | Price/time grid lines |
| CandlestickLayer | ChartMain | OHLC candlesticks |
| EmaLayer | ChartIndicator | EMA overlay |
| VolumeLayer | VolumePane | Volume bars |
| TradingOverlayLayer | ChartOverlay | Positions, SL/TP zones |
| CurrentPriceLayer | ChartOverlay | Current price line |
| PriceAxisLayer | PriceAxis | Y-axis labels |
| TimeAxisLayer | TimeAxis | X-axis labels |
| CrosshairLayer | Hud | Cursor crosshair |
| WatermarkLayer | ChartBackground | Symbol watermark |

### Managing Layers

```rust
// Add a layer
chart.add_layer(Box::new(MyLayer::new()));

// Remove a layer by name
chart.remove_layer("EMA(50)");

// Enable/disable a layer
chart.set_layer_enabled("Volume", false);

// Get layer reference
if let Some(layer) = chart.get_layer_mut("Candlestick") {
    layer.set_enabled(true);
}
```

## Architecture

```
fchart/
├── fchart-core/        # Core rendering engine (WebGPU)
│   ├── src/
│   │   ├── chart.rs        # Main Chart struct
│   │   ├── renderer.rs     # GPU rendering context
│   │   ├── viewport.rs     # View transformations
│   │   ├── theme.rs        # Color themes
│   │   ├── layers/         # Layer implementations
│   │   │   ├── candlestick.rs
│   │   │   ├── trading.rs      # Trading overlay
│   │   │   ├── indicators/     # Technical indicators
│   │   │   └── ...
│   │   └── shaders/        # WGSL shaders
├── fchart-cli/         # CLI application
└── fchart-native/      # Native desktop app (planned)
```

## Interactive Controls

In interactive mode (`-i` flag):

| Key | Action |
|-----|--------|
| Arrow Keys | Pan chart |
| +/- | Zoom in/out |
| Mouse Wheel | Zoom at cursor |
| q / ESC | Quit |

## Performance

- **GPU Rendering**: All rendering on GPU via WebGPU
- **Event-Driven**: Only renders when data/viewport changes
- **Buffer Reuse**: Persistent GPU buffers across frames
- **Layer Caching**: Layers cache computed geometry

## License

This project is dual-licensed under MIT OR Apache-2.0.
