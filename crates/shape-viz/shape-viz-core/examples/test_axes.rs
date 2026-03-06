//! Test example to verify axis rendering

mod test_series;

use shape_viz_core::layers::{GridLayer, PriceAxisLayer, RangeBarLayer, TimeAxisLayer};
use shape_viz_core::{Chart, ChartConfig, ChartTheme};
use std::error::Error;
use test_series::TestRangeSeries;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Generate 100 candles with sine wave pattern
    let test_data = TestRangeSeries::sine_wave("TEST", 100, 100.0);

    // Create chart with axes
    let mut config = ChartConfig::default();
    config.width = 1200;
    config.height = 600;
    config.theme = ChartTheme::tradingview_dark();

    let mut chart = Chart::new(config).await?;

    // Add layers in order
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

    image.save("test_axes_output.png")?;
    println!("Chart with axes saved to test_axes_output.png");

    Ok(())
}
