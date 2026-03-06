//! Quick test to verify the reference theme colors

mod test_series;

use shape_viz_core::layers::{GridLayer, PriceAxisLayer, RangeBarLayer, TimeAxisLayer};
use shape_viz_core::theme::ChartTheme;
use shape_viz_core::{Chart, ChartConfig};
use std::error::Error;
use test_series::TestRangeSeries;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Generate 100 test data points with sine wave pattern
    let test_data = TestRangeSeries::sine_wave("TEST", 100, 100.0);

    // Create chart with reference theme
    let mut config = ChartConfig::default();
    config.width = 1200;
    config.height = 600;
    config.theme = ChartTheme::reference_dark();

    println!("Creating chart with reference theme...");
    println!("Background: {:?}", config.theme.colors.background);
    println!("Grid: {:?}", config.theme.colors.grid_major);
    println!("Bullish: {:?}", config.theme.colors.candle_bullish);
    println!("Bearish: {:?}", config.theme.colors.candle_bearish);
    println!("Text: {:?}", config.theme.colors.text_primary);

    let mut chart = Chart::new(config).await?;

    // Add layers
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

    image.save("reference_theme_test.png")?;
    println!("Chart saved to reference_theme_test.png");

    Ok(())
}
