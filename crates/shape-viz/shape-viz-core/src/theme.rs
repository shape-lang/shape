//! Theme and styling system for charts

use serde::{Deserialize, Serialize};

/// RGBA color representation
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self::new(r, g, b, 1.0)
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        )
    }

    /// Create a color from a hex string (e.g., "#ff0066" or "ff0066")
    pub fn from_hex(hex: &str) -> Result<Self, String> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 && hex.len() != 8 {
            return Err("Hex color must be 6 or 8 characters".to_string());
        }

        let r = u8::from_str_radix(&hex[0..2], 16).map_err(|e| e.to_string())?;
        let g = u8::from_str_radix(&hex[2..4], 16).map_err(|e| e.to_string())?;
        let b = u8::from_str_radix(&hex[4..6], 16).map_err(|e| e.to_string())?;
        let a = if hex.len() == 8 {
            u8::from_str_radix(&hex[6..8], 16).map_err(|e| e.to_string())?
        } else {
            255
        };

        Ok(Self::rgba(r, g, b, a))
    }

    /// Create a color from hex literal (for const contexts)
    pub const fn hex(value: u32) -> Self {
        let r = ((value >> 16) & 0xFF) as f32 / 255.0;
        let g = ((value >> 8) & 0xFF) as f32 / 255.0;
        let b = (value & 0xFF) as f32 / 255.0;
        Self::new(r, g, b, 1.0)
    }

    pub fn with_alpha(&self, alpha: f32) -> Self {
        Self::new(self.r, self.g, self.b, alpha)
    }

    pub fn to_array(&self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    /// Lighten the color by a factor (0.0 = no change, 1.0 = white)
    pub fn lighten(&self, factor: f32) -> Self {
        let factor = factor.clamp(0.0, 1.0);
        Self::new(
            self.r + (1.0 - self.r) * factor,
            self.g + (1.0 - self.g) * factor,
            self.b + (1.0 - self.b) * factor,
            self.a,
        )
    }

    /// Darken the color by a factor (0.0 = no change, 1.0 = black)
    pub fn darken(&self, factor: f32) -> Self {
        let factor = factor.clamp(0.0, 1.0);
        Self::new(
            self.r * (1.0 - factor),
            self.g * (1.0 - factor),
            self.b * (1.0 - factor),
            self.a,
        )
    }

    // Common colors
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const RED: Self = Self::rgb(1.0, 0.0, 0.0);
    pub const GREEN: Self = Self::rgb(0.0, 1.0, 0.0);
    pub const BLUE: Self = Self::rgb(0.0, 0.0, 1.0);
    pub const TRANSPARENT: Self = Self::new(0.0, 0.0, 0.0, 0.0);
}

/// Color scheme for different chart themes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorScheme {
    // Background colors
    pub background: Color,
    pub chart_background: Color,
    pub axis_background: Color,

    // Grid colors
    pub grid_major: Color,
    pub grid_minor: Color,

    // Candlestick colors
    pub candle_bullish: Color,
    pub candle_bearish: Color,
    pub candle_doji: Color,
    pub wick_color: Color,
    pub wick_bullish: Color, // Optional separate wick colors
    pub wick_bearish: Color,

    // Text colors
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub text_accent: Color,

    // Axis colors
    pub axis_line: Color,
    pub axis_tick: Color,
    pub axis_label: Color,
    pub axis_separator: Color, // Line between price and time axis

    // Interactive elements
    pub crosshair: Color,
    pub selection: Color,
    pub highlight: Color,
    pub tooltip_background: Color,
    pub tooltip_text: Color,
    pub tooltip_border: Color,

    // Volume colors
    pub volume_bullish: Color,
    pub volume_bearish: Color,
    pub volume_neutral: Color,

    // Indicator colors
    pub indicator_primary: Color,
    pub indicator_secondary: Color,
    pub indicator_tertiary: Color,
    pub indicator_quaternary: Color,
    pub indicator_quinary: Color,

    // Alert/Status colors
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    pub info: Color,

    // Additional UI elements
    pub border: Color,
    pub shadow: Color,
    pub overlay: Color,
}

impl ColorScheme {
    /// Create a custom theme from primary colors
    pub fn custom(
        background: Color,
        bullish: Color,
        bearish: Color,
        text: Color,
        grid: Color,
    ) -> Self {
        Self {
            // Background colors
            background,
            chart_background: background.lighten(0.05),
            axis_background: background.lighten(0.08),

            // Grid colors
            grid_major: grid.with_alpha(0.5),
            grid_minor: grid.with_alpha(0.3),

            // Candlestick colors
            candle_bullish: bullish,
            candle_bearish: bearish,
            candle_doji: text.with_alpha(0.5),
            wick_color: text.with_alpha(0.5),
            wick_bullish: bullish.darken(0.2),
            wick_bearish: bearish.darken(0.2),

            // Text colors
            text_primary: text,
            text_secondary: text.with_alpha(0.8),
            text_muted: text.with_alpha(0.5),
            text_accent: bullish,

            // Axis colors
            axis_line: grid.lighten(0.2),
            axis_tick: text.with_alpha(0.5),
            axis_label: text.with_alpha(0.8),
            axis_separator: grid.lighten(0.3),

            // Interactive elements
            crosshair: text.with_alpha(0.8),
            selection: bullish.with_alpha(0.3),
            highlight: text.with_alpha(0.1),
            tooltip_background: background.lighten(0.1),
            tooltip_text: text,
            tooltip_border: grid.lighten(0.2),

            // Volume colors
            volume_bullish: bullish.with_alpha(0.7),
            volume_bearish: bearish.with_alpha(0.7),
            volume_neutral: text.with_alpha(0.3),

            // Indicator colors
            indicator_primary: Color::rgba(255, 193, 7, 255),
            indicator_secondary: Color::rgba(156, 39, 176, 255),
            indicator_tertiary: Color::rgba(0, 188, 212, 255),
            indicator_quaternary: Color::rgba(255, 87, 34, 255),
            indicator_quinary: Color::rgba(139, 195, 74, 255),

            // Alert/Status colors
            success: Color::rgba(76, 175, 80, 255),
            warning: Color::rgba(255, 152, 0, 255),
            error: Color::rgba(244, 67, 54, 255),
            info: Color::rgba(33, 150, 243, 255),

            // Additional UI elements
            border: grid.lighten(0.1),
            shadow: Color::rgba(0, 0, 0, 128),
            overlay: background.with_alpha(0.8),
        }
    }

    /// Reference chart theme - matches the provided reference image exactly
    pub fn reference_dark() -> Self {
        Self {
            // Background colors - exact dark blue from reference
            background: Color::hex(0x0d1117), // #0d1117 - Exact dark blue from reference
            chart_background: Color::hex(0x0d1117), // Same as background
            axis_background: Color::hex(0x0d1117), // Same as background

            // Grid colors - very visible to match reference
            grid_major: Color::hex(0x2d3748).with_alpha(1.0), // Full opacity for visibility
            grid_minor: Color::hex(0x2d3748).with_alpha(0.7), // High opacity

            // Candlestick colors - exact match to reference
            candle_bullish: Color::hex(0x00d9ff), // #00d9ff - Bright cyan/blue
            candle_bearish: Color::hex(0xff0080), // #ff0080 - Hot pink/magenta
            candle_doji: Color::hex(0x9ca3af),    // Neutral gray
            wick_color: Color::hex(0x9ca3af).with_alpha(0.8),
            wick_bullish: Color::hex(0x00d9ff),
            wick_bearish: Color::hex(0xff0080),

            // Text colors - brighter for better visibility
            text_primary: Color::hex(0xe0e0e0), // #e0e0e0 - Light gray/white
            text_secondary: Color::hex(0xb0b0b0), // Slightly dimmer
            text_muted: Color::hex(0x808080),   // Muted gray
            text_accent: Color::hex(0x00d9ff),  // Match bullish color

            // Axis colors
            axis_line: Color::hex(0x2a2e3a), // #2a2e3a - Slightly brighter than grid
            axis_tick: Color::hex(0x2a2e3a),
            axis_label: Color::hex(0xb0b0b0), // Light gray for better visibility
            axis_separator: Color::hex(0x2a2e3a),

            // Interactive elements
            crosshair: Color::hex(0x9ca3af).with_alpha(0.8),
            selection: Color::hex(0x00d4aa).with_alpha(0.3),
            highlight: Color::hex(0xffffff).with_alpha(0.05),
            tooltip_background: Color::hex(0x1a1e2a),
            tooltip_text: Color::hex(0x9ca3af),
            tooltip_border: Color::hex(0x2a2e3a),

            // Volume colors - match candles with proper transparency
            volume_bullish: Color::hex(0x00d9ff).with_alpha(0.5), // Same as candle_bullish with 50% alpha
            volume_bearish: Color::hex(0xff0080).with_alpha(0.5), // Same as candle_bearish with 50% alpha
            volume_neutral: Color::hex(0x9ca3af).with_alpha(0.3),

            // Indicator colors
            indicator_primary: Color::hex(0xffd93d), // Yellow
            indicator_secondary: Color::hex(0x6a5acd), // Purple
            indicator_tertiary: Color::hex(0x00bcd4), // Cyan
            indicator_quaternary: Color::hex(0xff5722), // Orange
            indicator_quinary: Color::hex(0x8bc34a), // Green

            // Alert/Status colors
            success: Color::hex(0x4caf50),
            warning: Color::hex(0xff9800),
            error: Color::hex(0xf44336),
            info: Color::hex(0x2196f3),

            // Additional UI elements
            border: Color::hex(0x2a2e3a),
            shadow: Color::rgba(0, 0, 0, 180),
            overlay: Color::hex(0x0a0e1a).with_alpha(0.9),
        }
    }

    /// TradingView-style dark theme (updated)
    pub fn tradingview_dark() -> Self {
        Self {
            // Background colors - deep dark blue/black
            background: Color::hex(0x131722), // Very dark blue-black
            chart_background: Color::hex(0x131722), // Same as background
            axis_background: Color::hex(0x131722), // Same as background for seamless panel

            // Grid colors - subtle gray lines
            grid_major: Color::hex(0x363a45).with_alpha(0.5), // Solid, slightly more prominent
            grid_minor: Color::hex(0x242730).with_alpha(0.3), // Solid, very subtle

            // Candlestick colors - bright cyan/teal and red/pink
            candle_bullish: Color::hex(0x26a69a), // Bright teal/cyan (bullish)
            candle_bearish: Color::hex(0xef5350), // Bright pink/red (bearish)
            candle_doji: Color::rgba(120, 123, 134, 255), // Neutral gray for doji
            wick_color: Color::rgba(120, 123, 134, 255), // Gray wicks
            wick_bullish: Color::hex(0x26a69a),
            wick_bearish: Color::hex(0xef5350),

            // Text colors - various shades of gray/white
            text_primary: Color::rgba(240, 243, 250, 255), // Almost white
            text_secondary: Color::rgba(180, 185, 195, 255), // Light gray
            text_muted: Color::rgba(120, 123, 134, 255),   // Muted gray
            text_accent: Color::rgba(34, 206, 170, 255),

            // Axis colors
            axis_line: Color::rgba(60, 64, 75, 255), // Subtle axis lines
            axis_tick: Color::rgba(120, 123, 134, 255), // Tick marks
            axis_label: Color::rgba(240, 243, 250, 255), // Price/time labels
            axis_separator: Color::rgba(60, 64, 75, 255),

            // Interactive elements
            crosshair: Color::rgba(100, 150, 255, 200), // Blue crosshair
            selection: Color::rgba(100, 150, 255, 100), // Selection highlight
            highlight: Color::rgba(255, 255, 255, 50),  // Hover highlight
            tooltip_background: Color::rgba(30, 34, 45, 240),
            tooltip_text: Color::rgba(240, 243, 250, 255),
            tooltip_border: Color::rgba(60, 64, 75, 255),

            // Volume colors - matching candlestick colors but more subdued
            volume_bullish: Color::hex(0x26a69a).with_alpha(0.4), // Same as candle_bullish with 40% alpha
            volume_bearish: Color::hex(0xef5350).with_alpha(0.4), // Same as candle_bearish with 40% alpha
            volume_neutral: Color::rgba(120, 123, 134, 100),

            // Indicator colors - bright distinguishable colors
            indicator_primary: Color::rgba(255, 193, 7, 255), // Golden yellow
            indicator_secondary: Color::rgba(156, 39, 176, 255), // Purple
            indicator_tertiary: Color::rgba(0, 188, 212, 255), // Cyan
            indicator_quaternary: Color::rgba(255, 87, 34, 255), // Deep orange
            indicator_quinary: Color::rgba(139, 195, 74, 255), // Light green

            // Alert colors
            success: Color::rgba(76, 175, 80, 255), // Green
            warning: Color::rgba(255, 152, 0, 255), // Orange
            error: Color::rgba(244, 67, 54, 255),   // Red
            info: Color::rgba(33, 150, 243, 255),   // Blue

            // Additional UI elements
            border: Color::rgba(60, 64, 75, 255),
            shadow: Color::rgba(0, 0, 0, 180),
            overlay: Color::rgba(16, 21, 30, 230),
        }
    }

    /// Classic light theme
    pub fn light() -> Self {
        Self {
            background: Color::rgba(255, 255, 255, 255),
            chart_background: Color::rgba(252, 252, 252, 255),
            axis_background: Color::rgba(248, 248, 248, 255),

            grid_major: Color::rgba(200, 200, 200, 255),
            grid_minor: Color::rgba(230, 230, 230, 255),

            candle_bullish: Color::rgba(76, 175, 80, 255), // Green
            candle_bearish: Color::hex(0xff006e),          // Red
            candle_doji: Color::rgba(158, 158, 158, 255),
            wick_color: Color::rgba(97, 97, 97, 255),
            wick_bullish: Color::rgba(76, 175, 80, 200),
            wick_bearish: Color::rgba(244, 67, 54, 200),

            text_primary: Color::rgba(33, 37, 41, 255),
            text_secondary: Color::rgba(108, 117, 125, 255),
            text_muted: Color::rgba(173, 181, 189, 255),
            text_accent: Color::rgba(76, 175, 80, 255),

            axis_line: Color::rgba(200, 200, 200, 255),
            axis_tick: Color::rgba(150, 150, 150, 255),
            axis_label: Color::rgba(100, 100, 100, 255),
            axis_separator: Color::rgba(200, 200, 200, 255),

            crosshair: Color::rgba(0, 123, 255, 200),
            selection: Color::rgba(0, 123, 255, 100),
            highlight: Color::rgba(0, 0, 0, 30),
            tooltip_background: Color::rgba(255, 255, 255, 240),
            tooltip_text: Color::rgba(33, 37, 41, 255),
            tooltip_border: Color::rgba(200, 200, 200, 255),

            volume_bullish: Color::rgba(76, 175, 80, 180),
            volume_bearish: Color::rgba(244, 67, 54, 180),
            volume_neutral: Color::rgba(158, 158, 158, 100),

            indicator_primary: Color::rgba(255, 193, 7, 255),
            indicator_secondary: Color::rgba(156, 39, 176, 255),
            indicator_tertiary: Color::rgba(0, 188, 212, 255),
            indicator_quaternary: Color::rgba(255, 87, 34, 255),
            indicator_quinary: Color::rgba(139, 195, 74, 255),

            success: Color::rgba(40, 167, 69, 255),
            warning: Color::rgba(255, 193, 7, 255),
            error: Color::rgba(220, 53, 69, 255),
            info: Color::rgba(23, 162, 184, 255),

            border: Color::rgba(200, 200, 200, 255),
            shadow: Color::rgba(0, 0, 0, 50),
            overlay: Color::rgba(255, 255, 255, 230),
        }
    }

    /// Midnight theme - deep blue/purple dark theme
    pub fn midnight() -> Self {
        Self::custom(
            Color::hex(0x0f0f23), // Deep midnight blue background
            Color::hex(0x00ff88), // Bright green bullish
            Color::hex(0xff0055), // Bright red bearish
            Color::hex(0xc9d1d9), // Light gray text
            Color::hex(0x30363d), // Dark gray grid
        )
    }

    /// Monokai theme - inspired by the popular code editor theme
    pub fn monokai() -> Self {
        Self::custom(
            Color::hex(0x272822), // Monokai background
            Color::hex(0xa6e22e), // Monokai green
            Color::hex(0xf92672), // Monokai red
            Color::hex(0xf8f8f2), // Monokai foreground
            Color::hex(0x3e3d32), // Monokai comments
        )
    }

    /// High contrast dark theme for accessibility
    pub fn high_contrast_dark() -> Self {
        Self::custom(
            Color::hex(0x000000), // Pure black background
            Color::hex(0x00ff00), // Pure green bullish
            Color::hex(0xff0000), // Pure red bearish
            Color::hex(0xffffff), // Pure white text
            Color::hex(0x404040), // Dark gray grid
        )
    }
}

/// Typography settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Typography {
    pub primary_font_size: f32,
    pub secondary_font_size: f32,
    pub small_font_size: f32,
    pub font_family: String,
    pub line_height: f32,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            primary_font_size: 12.0,          // Larger to match reference
            secondary_font_size: 11.0,        // Larger axis labels
            small_font_size: 10.0,            // Larger small text
            font_family: "Inter".to_string(), // Modern, readable font
            line_height: 1.2,
        }
    }
}

/// Spacing and sizing constants
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spacing {
    pub axis_margin: f32,
    pub tick_length: f32,
    pub label_padding: f32,
    pub candle_min_width: f32,
    pub candle_max_width: f32,
    pub grid_spacing_min: f32,
    pub crosshair_width: f32,
}

impl Default for Spacing {
    fn default() -> Self {
        Self {
            axis_margin: 60.0,      // Space for price/time axes
            tick_length: 5.0,       // Length of axis tick marks
            label_padding: 8.0,     // Padding around text labels
            candle_min_width: 1.0,  // Minimum candle width
            candle_max_width: 20.0, // Maximum candle width
            grid_spacing_min: 40.0, // Minimum pixels between grid lines
            crosshair_width: 0.5,   // Crosshair line width
        }
    }
}

/// Complete chart theme
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartTheme {
    pub name: String,
    pub colors: ColorScheme,
    pub typography: Typography,
    pub spacing: Spacing,
}

impl ChartTheme {
    /// Create a custom theme with a name and color scheme
    pub fn new(name: impl Into<String>, colors: ColorScheme) -> Self {
        Self {
            name: name.into(),
            colors,
            typography: Typography::default(),
            spacing: Spacing::default(),
        }
    }

    /// Create a custom theme with full control
    pub fn custom(
        name: impl Into<String>,
        colors: ColorScheme,
        typography: Typography,
        spacing: Spacing,
    ) -> Self {
        Self {
            name: name.into(),
            colors,
            typography,
            spacing,
        }
    }

    /// Create reference dark theme - matches the provided reference image
    pub fn reference_dark() -> Self {
        Self {
            name: "Reference Dark".to_string(),
            colors: ColorScheme::reference_dark(),
            typography: Typography::default(),
            spacing: Spacing::default(),
        }
    }

    /// Create TradingView-style dark theme
    pub fn tradingview_dark() -> Self {
        Self {
            name: "TradingView Dark".to_string(),
            colors: ColorScheme::tradingview_dark(),
            typography: Typography::default(),
            spacing: Spacing::default(),
        }
    }

    /// Create light theme
    pub fn light() -> Self {
        Self {
            name: "Light".to_string(),
            colors: ColorScheme::light(),
            typography: Typography::default(),
            spacing: Spacing::default(),
        }
    }

    /// Create midnight theme
    pub fn midnight() -> Self {
        Self {
            name: "Midnight".to_string(),
            colors: ColorScheme::midnight(),
            typography: Typography::default(),
            spacing: Spacing::default(),
        }
    }

    /// Create monokai theme
    pub fn monokai() -> Self {
        Self {
            name: "Monokai".to_string(),
            colors: ColorScheme::monokai(),
            typography: Typography::default(),
            spacing: Spacing::default(),
        }
    }

    /// Create high contrast dark theme
    pub fn high_contrast_dark() -> Self {
        Self {
            name: "High Contrast Dark".to_string(),
            colors: ColorScheme::high_contrast_dark(),
            typography: Typography::default(),
            spacing: Spacing::default(),
        }
    }

    /// Get a list of all available built-in themes
    pub fn all_themes() -> Vec<Self> {
        vec![
            Self::reference_dark(),
            Self::tradingview_dark(),
            Self::light(),
            Self::midnight(),
            Self::monokai(),
            Self::high_contrast_dark(),
        ]
    }

    /// Get a theme by name
    pub fn by_name(name: &str) -> Option<Self> {
        Self::all_themes()
            .into_iter()
            .find(|theme| theme.name.eq_ignore_ascii_case(name))
    }
}

impl Default for ChartTheme {
    fn default() -> Self {
        Self::reference_dark() // Use reference dark as default to match the provided image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_creation() {
        let color = Color::rgba(255, 128, 64, 200);
        assert_eq!(color.r, 1.0);
        assert_eq!(color.g, 128.0 / 255.0);
        assert_eq!(color.b, 64.0 / 255.0);
        assert_eq!(color.a, 200.0 / 255.0);
    }

    #[test]
    fn test_color_from_hex() {
        let color = Color::from_hex("#ff0066").unwrap();
        assert_eq!(color.r, 1.0);
        assert_eq!(color.g, 0.0);
        assert!((color.b - 0.4).abs() < 0.01);
        assert_eq!(color.a, 1.0);

        let color2 = Color::from_hex("00d4aa").unwrap();
        assert_eq!(color2.r, 0.0);
        assert!((color2.g - 0.831).abs() < 0.01);
        assert!((color2.b - 0.667).abs() < 0.01);
    }

    #[test]
    fn test_color_hex_const() {
        let color = Color::hex(0xff0066);
        assert_eq!(color.r, 1.0);
        assert_eq!(color.g, 0.0);
        assert!((color.b - 0.4).abs() < 0.01);
    }

    #[test]
    fn test_color_with_alpha() {
        let color = Color::RED.with_alpha(0.5);
        assert_eq!(color.r, 1.0);
        assert_eq!(color.g, 0.0);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 0.5);
    }

    #[test]
    fn test_theme_creation() {
        let theme = ChartTheme::tradingview_dark();
        assert_eq!(theme.name, "TradingView Dark");

        // Verify some key colors match TradingView style
        let colors = &theme.colors;
        // Bullish color should be teal/cyan-ish
        assert!(colors.candle_bullish.g > 0.6); // High green component
        assert!(colors.candle_bullish.b > 0.5); // Some blue component

        // Bearish color should be red/pink-ish
        assert!(colors.candle_bearish.r > 0.8); // High red component
    }

    #[test]
    fn test_reference_theme() {
        let theme = ChartTheme::reference_dark();
        assert_eq!(theme.name, "Reference Dark");

        let colors = &theme.colors;
        // Test exact color matches – these should mirror the definition in reference_dark()
        assert_eq!(colors.background, Color::hex(0x0d1117));
        assert_eq!(colors.candle_bullish, Color::hex(0x00d9ff));
        assert_eq!(colors.candle_bearish, Color::hex(0xff0080));
        assert_eq!(colors.text_primary, Color::hex(0xe0e0e0));
    }

    #[test]
    fn test_theme_by_name() {
        assert!(ChartTheme::by_name("reference dark").is_some());
        assert!(ChartTheme::by_name("TRADINGVIEW DARK").is_some());
        assert!(ChartTheme::by_name("light").is_some());
        assert!(ChartTheme::by_name("nonexistent").is_none());
    }

    #[test]
    fn test_color_modifications() {
        let color = Color::hex(0xff0066);
        let lighter = color.lighten(0.2);
        assert!(lighter.r >= color.r);
        assert!(lighter.g >= color.g);
        assert!(lighter.b >= color.b);

        let darker = color.darken(0.2);
        assert!(darker.r <= color.r);
        assert!(darker.g <= color.g);
        assert!(darker.b <= color.b);
    }
}
