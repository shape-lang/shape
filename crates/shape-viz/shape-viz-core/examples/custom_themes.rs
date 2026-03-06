//! Example demonstrating the custom theme system in fchart
//!
//! This shows how to:
//! - Use the built-in themes including the reference theme
//! - Create custom themes from scratch
//! - Modify existing themes

mod test_series;

use shape_viz_core::layers::{GridLayer, PriceAxisLayer, RangeBarLayer, TimeAxisLayer};
use shape_viz_core::theme::{ChartTheme, Color, ColorScheme};
use shape_viz_core::{Chart, ChartConfig};
use std::error::Error;
use test_series::TestRangeSeries;

async fn create_chart_with_theme(
    theme: ChartTheme,
    test_data: TestRangeSeries,
    filename: &str,
) -> Result<(), Box<dyn Error>> {
    // Create chart configuration
    let mut config = ChartConfig::default();
    config.width = 1200;
    config.height = 600;
    config.theme = theme;

    // Create chart
    let mut chart = Chart::new(config).await?;

    // Add layers in order (background to foreground)
    chart.add_layer(Box::new(GridLayer::new()));
    chart.add_layer(Box::new(RangeBarLayer::new()));
    chart.add_layer(Box::new(PriceAxisLayer::new()));
    chart.add_layer(Box::new(TimeAxisLayer::new()));

    // Set data
    let chart_data = test_data.into_chart_data();
    chart.set_data(chart_data)?;

    // Render
    let image_data = chart.render().await?;

    // Save to file
    let image = image::RgbaImage::from_raw(chart.dimensions().0, chart.dimensions().1, image_data)
        .ok_or("Failed to create image")?;

    image.save(filename)?;
    println!("✓ Saved {}", filename);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Example 1: Using the reference theme (matches the provided image exactly)
    println!("Creating chart with reference theme matching the provided image...");
    create_chart_with_theme(
        ChartTheme::reference_dark(),
        TestRangeSeries::sine_wave("DEMO", 100, 100.0),
        "reference_theme_chart.png",
    )
    .await?;

    // Example 2: Using other built-in themes
    let themes = vec![
        ("tradingview", ChartTheme::tradingview_dark()),
        ("midnight", ChartTheme::midnight()),
        ("monokai", ChartTheme::monokai()),
        ("light", ChartTheme::light()),
        ("high_contrast", ChartTheme::high_contrast_dark()),
    ];

    for (name, theme) in themes {
        create_chart_with_theme(
            theme,
            TestRangeSeries::sine_wave("DEMO", 100, 100.0),
            &format!("{}_theme_chart.png", name),
        )
        .await?;
    }

    // Example 3: Creating custom themes
    println!("\nCreating custom themes...");

    // Custom theme 1: Cyberpunk style
    let cyberpunk_colors = ColorScheme::custom(
        Color::hex(0x0a0014), // Deep purple-black background
        Color::hex(0x00ffff), // Cyan bullish
        Color::hex(0xff00ff), // Magenta bearish
        Color::hex(0xe0e0ff), // Light purple text
        Color::hex(0x2a0033), // Purple grid
    );

    let cyberpunk_theme = ChartTheme::new("Cyberpunk", cyberpunk_colors);

    create_chart_with_theme(
        cyberpunk_theme,
        TestRangeSeries::sine_wave("DEMO", 100, 100.0),
        "cyberpunk_theme_chart.png",
    )
    .await?;

    // Custom theme 2: Ocean theme
    let ocean_colors = ColorScheme::custom(
        Color::hex(0x001f3f), // Deep ocean blue background
        Color::hex(0x39cccc), // Teal bullish
        Color::hex(0xff851b), // Orange bearish
        Color::hex(0xaaaaaa), // Gray text
        Color::hex(0x003366), // Dark blue grid
    );

    let ocean_theme = ChartTheme::new("Ocean", ocean_colors);

    create_chart_with_theme(
        ocean_theme,
        TestRangeSeries::sine_wave("DEMO", 100, 100.0),
        "ocean_theme_chart.png",
    )
    .await?;

    // Example 4: Creating a theme from hex color strings
    println!("\nCreating theme from hex strings...");

    let hex_theme_colors = ColorScheme {
        // Parse colors from hex strings
        background: Color::from_hex("#1a1a2e")?,
        chart_background: Color::from_hex("#1a1a2e")?,
        axis_background: Color::from_hex("#1a1a2e")?,

        grid_major: Color::from_hex("#16213e")?,
        grid_minor: Color::from_hex("#16213e")?.with_alpha(0.5),

        candle_bullish: Color::from_hex("#00f5ff")?,
        candle_bearish: Color::from_hex("#ff006e")?,
        candle_doji: Color::from_hex("#8b8b8b")?,
        wick_color: Color::from_hex("#8b8b8b")?,
        wick_bullish: Color::from_hex("#00f5ff")?.with_alpha(0.8),
        wick_bearish: Color::from_hex("#ff006e")?.with_alpha(0.8),

        text_primary: Color::from_hex("#eaeaea")?,
        text_secondary: Color::from_hex("#b8b8b8")?,
        text_muted: Color::from_hex("#6b6b6b")?,
        text_accent: Color::from_hex("#00f5ff")?,

        axis_line: Color::from_hex("#3a3a5c")?,
        axis_tick: Color::from_hex("#3a3a5c")?,
        axis_label: Color::from_hex("#b8b8b8")?,
        axis_separator: Color::from_hex("#3a3a5c")?,

        crosshair: Color::from_hex("#ffffff")?.with_alpha(0.8),
        selection: Color::from_hex("#00f5ff")?.with_alpha(0.3),
        highlight: Color::from_hex("#ffffff")?.with_alpha(0.1),
        tooltip_background: Color::from_hex("#16213e")?,
        tooltip_text: Color::from_hex("#eaeaea")?,
        tooltip_border: Color::from_hex("#3a3a5c")?,

        volume_bullish: Color::from_hex("#00f5ff")?.with_alpha(0.7),
        volume_bearish: Color::from_hex("#ff006e")?.with_alpha(0.7),
        volume_neutral: Color::from_hex("#8b8b8b")?.with_alpha(0.3),

        indicator_primary: Color::from_hex("#ffd93d")?,
        indicator_secondary: Color::from_hex("#b967ff")?,
        indicator_tertiary: Color::from_hex("#05ffa1")?,
        indicator_quaternary: Color::from_hex("#ff6b6b")?,
        indicator_quinary: Color::from_hex("#4ecdc4")?,

        success: Color::from_hex("#00d084")?,
        warning: Color::from_hex("#ffb900")?,
        error: Color::from_hex("#ff3838")?,
        info: Color::from_hex("#0099ff")?,

        border: Color::from_hex("#3a3a5c")?,
        shadow: Color::rgba(0, 0, 0, 180),
        overlay: Color::from_hex("#1a1a2e")?.with_alpha(0.9),
    };

    let hex_theme = ChartTheme::new("Neon Night", hex_theme_colors);

    create_chart_with_theme(
        hex_theme,
        TestRangeSeries::sine_wave("DEMO", 100, 100.0),
        "neon_night_theme_chart.png",
    )
    .await?;

    // Example 5: Demonstrating theme lookup by name
    println!("\nDemonstrating theme lookup by name...");
    if let Some(theme) = ChartTheme::by_name("reference dark") {
        println!("Found theme: {}", theme.name);
        println!("Background color: {:?}", theme.colors.background);
        println!("Bullish color: {:?}", theme.colors.candle_bullish);
        println!("Bearish color: {:?}", theme.colors.candle_bearish);
    }

    println!("\nAll available built-in themes:");
    for theme in ChartTheme::all_themes() {
        println!("  - {}", theme.name);
    }

    println!("\nAll theme charts have been generated successfully!");

    Ok(())
}
