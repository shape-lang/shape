use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use image::{ColorType, ImageEncoder, RgbaImage, codecs::png::PngEncoder};
use shape_viz_core::ChartTheme;
use std::io::{self, Write};

/// Display image inline using Kitty graphics protocol
pub fn display_image_inline(buffer: Vec<u8>, width: u32, height: u32) -> Result<()> {
    let image =
        RgbaImage::from_vec(width, height, buffer).ok_or_else(|| anyhow!("invalid RGBA buffer"))?;
    let mut png_bytes = Vec::new();
    {
        let encoder = PngEncoder::new(&mut png_bytes);
        encoder.write_image(image.as_raw(), width, height, ColorType::Rgba8.into())?;
    }
    let encoded = BASE64.encode(png_bytes);
    print!("\x1b_Gf=100,s={},v={},q=2;{}\x1b\\", width, height, encoded);
    io::stdout().flush()?;
    Ok(())
}

/// Get chart theme by name
pub fn theme_from_name(name: &str) -> ChartTheme {
    ChartTheme::by_name(name).unwrap_or_else(ChartTheme::reference_dark)
}
