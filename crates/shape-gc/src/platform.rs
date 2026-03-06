//! Hardware pointer masking platform detection and operations.
//!
//! Supports:
//! - **ARM64 TBI** (Top Byte Ignore): Always available on ARM64. Hardware ignores
//!   the top byte of pointers, giving us 8 bits for GC metadata.
//! - **x86-64 LAM** (Linear Address Masking): Available on recent Intel/AMD.
//!   Masks bits 62:57 (LAM57) or 62:48 (LAM48).
//! - **Software fallback**: Manually AND-mask before dereferencing.
//!
//! ## Pointer format
//!
//! ```text
//! ARM TBI:     [COLOR:2][GEN:1][unused:5][ADDRESS:56]
//! x86-64 LAM:  [0][COLOR:2][GEN:1][unused:3][ADDRESS:57]
//! Fallback:    Same format, software AND mask before deref
//! ```
//!
//! ## Integration with NaN-boxing
//!
//! NaN-boxing uses a 48-bit PAYLOAD_MASK which already strips upper bits.
//! GC metadata lives in bits 48-55 which are zeroed by PAYLOAD_MASK. So
//! existing ValueWord pointer extraction automatically strips GC tags.
//! Tags are only relevant inside the GC itself.

use crate::header::{GcColor, Generation};
use std::sync::atomic::{AtomicU8, Ordering};

/// Detected pointer masking mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaskingMode {
    /// ARM64 Top Byte Ignore — always available on ARM64.
    ArmTbi,
    /// x86-64 Linear Address Masking (57-bit).
    X86Lam57,
    /// Software fallback — manual AND before dereferencing.
    Software,
}

/// Cached masking mode for fast repeated queries.
///
/// 0 = not initialized, 1 = Software, 2 = ArmTbi, 3 = X86Lam57
static CACHED_MASKING_MODE: AtomicU8 = AtomicU8::new(0);

/// Get the cached masking mode, detecting on first call.
pub fn cached_masking_mode() -> MaskingMode {
    let cached = CACHED_MASKING_MODE.load(Ordering::Relaxed);
    match cached {
        1 => MaskingMode::Software,
        2 => MaskingMode::ArmTbi,
        3 => MaskingMode::X86Lam57,
        _ => {
            let mode = detect_masking_mode();
            let val = match mode {
                MaskingMode::Software => 1,
                MaskingMode::ArmTbi => 2,
                MaskingMode::X86Lam57 => 3,
            };
            CACHED_MASKING_MODE.store(val, Ordering::Relaxed);
            mode
        }
    }
}

/// Check if x86-64 LAM is available (runtime-detected).
pub fn has_x86_lam() -> bool {
    cached_masking_mode() == MaskingMode::X86Lam57
}

/// Address mask: zero out metadata bits to get the raw address.
const ADDRESS_MASK_48: u64 = 0x0000_FFFF_FFFF_FFFF; // 48-bit (NaN-box compatible)
const ADDRESS_MASK_56: u64 = 0x00FF_FFFF_FFFF_FFFF; // 56-bit (ARM TBI)
const ADDRESS_MASK_57: u64 = 0x01FF_FFFF_FFFF_FFFF; // 57-bit (x86 LAM57)

/// Bit positions for metadata in the upper pointer bits.
const COLOR_SHIFT: u32 = 62;
const GEN_SHIFT: u32 = 61;

/// Detect the best available pointer masking mode for this platform.
pub fn detect_masking_mode() -> MaskingMode {
    #[cfg(target_arch = "aarch64")]
    {
        // ARM64 TBI is always available — the hardware ignores the top byte
        return MaskingMode::ArmTbi;
    }

    #[cfg(target_arch = "x86_64")]
    {
        // Try to enable LAM57 via prctl
        if try_enable_lam57() {
            return MaskingMode::X86Lam57;
        }
    }

    MaskingMode::Software
}

/// Try to enable x86-64 LAM57 via prctl.
#[cfg(target_arch = "x86_64")]
fn try_enable_lam57() -> bool {
    // PR_SET_TAGGED_ADDR_CTRL = 54, PR_TAGGED_ADDR_ENABLE = 1
    // LAM57: PR_MTE_TAG_MASK with appropriate flags
    // For now, conservatively return false until kernel support is confirmed
    false
}

/// Strip metadata bits from a tagged pointer, returning the raw address.
///
/// Safe to call on any pointer — if no metadata is present, this is a no-op
/// (the upper bits will already be zero for valid user-space pointers).
#[inline(always)]
pub fn mask_ptr(tagged: *mut u8, mode: MaskingMode) -> *mut u8 {
    let addr = tagged as u64;
    let masked = match mode {
        MaskingMode::ArmTbi => addr & ADDRESS_MASK_56,
        MaskingMode::X86Lam57 => addr & ADDRESS_MASK_57,
        MaskingMode::Software => addr & ADDRESS_MASK_48,
    };
    masked as *mut u8
}

/// Tag a raw pointer with GC metadata (color and generation).
#[inline(always)]
pub fn tag_ptr(ptr: *mut u8, color: GcColor, generation: Generation) -> *mut u8 {
    let addr = ptr as u64;
    let color_bits = (color as u64) << COLOR_SHIFT;
    let gen_bit = (generation as u64) << GEN_SHIFT;
    (addr | color_bits | gen_bit) as *mut u8
}

/// Read the color from a tagged pointer.
#[inline(always)]
pub fn read_color(tagged: *mut u8) -> GcColor {
    let bits = tagged as u64;
    match (bits >> COLOR_SHIFT) & 0b11 {
        0 => GcColor::White,
        1 => GcColor::Gray,
        2 => GcColor::Black,
        _ => GcColor::White,
    }
}

/// Read the generation from a tagged pointer.
#[inline(always)]
pub fn read_generation(tagged: *mut u8) -> Generation {
    let bits = tagged as u64;
    if (bits >> GEN_SHIFT) & 1 == 0 {
        Generation::Young
    } else {
        Generation::Old
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_software_mask_strips_upper_bits() {
        let raw = 0xDEAD_0000_1234_5678_u64 as *mut u8;
        let masked = mask_ptr(raw, MaskingMode::Software);
        assert_eq!(masked as u64, 0x0000_0000_1234_5678);
    }

    #[test]
    fn test_tag_and_read_color() {
        let raw = 0x0000_0000_1234_5678_u64 as *mut u8;

        let tagged = tag_ptr(raw, GcColor::Gray, Generation::Young);
        assert_eq!(read_color(tagged), GcColor::Gray);
        assert_eq!(read_generation(tagged), Generation::Young);

        // Masking should recover the original address
        let unmasked = mask_ptr(tagged, MaskingMode::Software);
        // The 48-bit mask may strip more than needed, but the lower 48 bits are preserved
        assert_eq!(
            unmasked as u64 & 0x0000_FFFF_FFFF_FFFF,
            0x0000_0000_1234_5678
        );
    }

    #[test]
    fn test_tag_and_read_generation() {
        let raw = 0x0000_0000_ABCD_EF00_u64 as *mut u8;
        let tagged = tag_ptr(raw, GcColor::Black, Generation::Old);
        assert_eq!(read_color(tagged), GcColor::Black);
        assert_eq!(read_generation(tagged), Generation::Old);
    }
}
