//! Configuration types for multi-series alignment

/// Gap fill method for missing data
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GapFillMethod {
    Forward,
    None,
}

/// Alignment mode for multiple series
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlignmentMode {
    /// Only include timestamps present in all series
    Intersection,
    /// Include all timestamps from any series
    Union,
    /// Use timestamps from a specific reference series
    Reference(usize),
    /// Use a fixed interval regardless of data
    FixedInterval,
}

/// Multi-series alignment configuration
#[derive(Debug, Clone)]
pub struct AlignmentConfig {
    /// Alignment mode
    pub mode: AlignmentMode,
    /// Gap filling method
    pub gap_fill: GapFillMethod,
}

impl Default for AlignmentConfig {
    fn default() -> Self {
        Self {
            mode: AlignmentMode::Intersection,
            gap_fill: GapFillMethod::Forward,
        }
    }
}
