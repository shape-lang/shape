//! IntWidth: shared width-semantics specification for first-class integer width types.
//!
//! This module is the single source of truth for integer width metadata: bit counts,
//! signedness, masks, truncation, and width-joining rules. It lives in shape-ast
//! (bottom of the dependency chain) so every crate can import it.
//!
//! `IntWidth` covers the sub-i64 and u64 widths. Plain `int` (i64) is NOT represented
//! here — it remains the default integer type handled by existing codepaths.

use serde::{Deserialize, Serialize};

/// Integer width types with real width semantics.
///
/// Does NOT include i64 — that stays as the default `int` type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntWidth {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    U64,
}

macro_rules! define_int_width_spec {
    ($($variant:ident => {
        bits: $bits:expr,
        signed: $signed:expr,
        mask: $mask:expr,
        sign_shift: $sign_shift:expr,
        min_i64: $min_i64:expr,
        max_i64: $max_i64:expr,
        max_u64: $max_u64:expr,
        name: $name:expr,
    };)*) => {
        impl IntWidth {
            /// All 7 width variants.
            pub const ALL: [IntWidth; 7] = [
                $(IntWidth::$variant,)*
            ];

            /// Number of bits in this width.
            #[inline]
            pub const fn bits(self) -> u32 {
                match self {
                    $(IntWidth::$variant => $bits,)*
                }
            }

            /// Whether this is a signed integer type.
            #[inline]
            pub const fn is_signed(self) -> bool {
                match self {
                    $(IntWidth::$variant => $signed,)*
                }
            }

            /// Whether this is an unsigned integer type.
            #[inline]
            pub const fn is_unsigned(self) -> bool {
                !self.is_signed()
            }

            /// Bit mask for the value range (e.g., 0xFF for 8-bit).
            #[inline]
            pub const fn mask(self) -> u64 {
                match self {
                    $(IntWidth::$variant => $mask,)*
                }
            }

            /// Bit position of the sign bit (e.g., 7 for i8).
            #[inline]
            pub const fn sign_shift(self) -> u32 {
                match self {
                    $(IntWidth::$variant => $sign_shift,)*
                }
            }

            /// Minimum value representable as i64.
            /// For unsigned types, this is 0.
            #[inline]
            pub const fn min_value(self) -> i64 {
                match self {
                    $(IntWidth::$variant => $min_i64,)*
                }
            }

            /// Maximum value representable as i64.
            /// For U64, this returns i64::MAX (the max *signed* portion).
            #[inline]
            pub const fn max_value(self) -> i64 {
                match self {
                    $(IntWidth::$variant => $max_i64,)*
                }
            }

            /// Maximum value representable as u64 (meaningful for unsigned types).
            #[inline]
            pub const fn max_unsigned(self) -> u64 {
                match self {
                    $(IntWidth::$variant => $max_u64,)*
                }
            }

            /// Human-readable type name (e.g., "i8", "u64").
            #[inline]
            pub const fn type_name(self) -> &'static str {
                match self {
                    $(IntWidth::$variant => $name,)*
                }
            }

            /// Canonical truncation: wraps an i64 value to this width using
            /// two's complement semantics.
            ///
            /// For signed types: mask then sign-extend.
            /// For U64: identity (no truncation needed for i64→u64 bit reinterpret).
            /// For other unsigned: just mask.
            #[inline]
            pub const fn truncate(self, value: i64) -> i64 {
                match self {
                    $(IntWidth::$variant => {
                        if $signed {
                            // Mask to width, then sign-extend
                            let masked = (value as u64) & $mask;
                            // Sign-extend: if sign bit set, fill upper bits
                            if masked & (1u64 << $sign_shift) != 0 {
                                (masked | !$mask) as i64
                            } else {
                                masked as i64
                            }
                        } else if $bits == 64 {
                            // U64: no truncation, value is reinterpreted
                            value
                        } else {
                            // Unsigned sub-64: just mask (always positive in i64)
                            ((value as u64) & $mask) as i64
                        }
                    })*
                }
            }

            /// Unsigned-safe truncation: wraps a u64 value to this width.
            ///
            /// For signed types: mask then sign-extend (returned as u64 bit pattern).
            /// For unsigned types: just mask.
            #[inline]
            pub const fn truncate_u64(self, value: u64) -> u64 {
                match self {
                    $(IntWidth::$variant => {
                        if $bits == 64 {
                            value // U64 or I64-width: identity
                        } else if $signed {
                            let masked = value & $mask;
                            if masked & (1u64 << $sign_shift) != 0 {
                                masked | !$mask
                            } else {
                                masked
                            }
                        } else {
                            value & $mask
                        }
                    })*
                }
            }

            /// Parse a width name (e.g., "i8", "u64") to an IntWidth.
            pub fn from_name(name: &str) -> Option<IntWidth> {
                match name {
                    $($name => Some(IntWidth::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

define_int_width_spec! {
    I8 => {
        bits: 8,
        signed: true,
        mask: 0xFF_u64,
        sign_shift: 7,
        min_i64: -128_i64,
        max_i64: 127_i64,
        max_u64: 127_u64,
        name: "i8",
    };
    U8 => {
        bits: 8,
        signed: false,
        mask: 0xFF_u64,
        sign_shift: 7,
        min_i64: 0_i64,
        max_i64: 255_i64,
        max_u64: 255_u64,
        name: "u8",
    };
    I16 => {
        bits: 16,
        signed: true,
        mask: 0xFFFF_u64,
        sign_shift: 15,
        min_i64: -32768_i64,
        max_i64: 32767_i64,
        max_u64: 32767_u64,
        name: "i16",
    };
    U16 => {
        bits: 16,
        signed: false,
        mask: 0xFFFF_u64,
        sign_shift: 15,
        min_i64: 0_i64,
        max_i64: 65535_i64,
        max_u64: 65535_u64,
        name: "u16",
    };
    I32 => {
        bits: 32,
        signed: true,
        mask: 0xFFFF_FFFF_u64,
        sign_shift: 31,
        min_i64: -2147483648_i64,
        max_i64: 2147483647_i64,
        max_u64: 2147483647_u64,
        name: "i32",
    };
    U32 => {
        bits: 32,
        signed: false,
        mask: 0xFFFF_FFFF_u64,
        sign_shift: 31,
        min_i64: 0_i64,
        max_i64: 4294967295_i64,
        max_u64: 4294967295_u64,
        name: "u32",
    };
    U64 => {
        bits: 64,
        signed: false,
        mask: u64::MAX,
        sign_shift: 63,
        min_i64: 0_i64,
        max_i64: i64::MAX,
        max_u64: u64::MAX,
        name: "u64",
    };
}

impl IntWidth {
    /// Join two widths for mixed-width arithmetic.
    ///
    /// Rules:
    /// - Same width → Ok(same)
    /// - Different widths, same signedness → Ok(wider)
    /// - Mixed sign: u8+i8→I16, u16+i16→I32, u32+i32→I64 (widen to next signed)
    /// - **u64 + any signed → Err(())** (compile error — no safe widening)
    pub fn join(a: IntWidth, b: IntWidth) -> Result<IntWidth, ()> {
        if a == b {
            return Ok(a);
        }

        // Same signedness: pick wider
        if a.is_signed() == b.is_signed() {
            return Ok(if a.bits() >= b.bits() { a } else { b });
        }

        // Mixed sign: identify unsigned and signed
        let (unsigned, signed) = if a.is_unsigned() { (a, b) } else { (b, a) };

        // u64 + any signed → error
        if unsigned == IntWidth::U64 {
            return Err(());
        }

        // Widen to next signed width that fits both
        match (unsigned, signed) {
            // u8 (0..255) + i8 (-128..127) → i16 (-32768..32767)
            (IntWidth::U8, IntWidth::I8) => Ok(IntWidth::I16),
            // u8 + i16/i32 → the signed type is already wide enough
            (IntWidth::U8, s) => Ok(s),

            // u16 (0..65535) + i8/i16 → i32
            (IntWidth::U16, IntWidth::I8 | IntWidth::I16) => Ok(IntWidth::I32),
            // u16 + i32 → i32 is wide enough
            (IntWidth::U16, IntWidth::I32) => Ok(IntWidth::I32),

            // u32 (0..4B) + i8/i16/i32 → need i64 (default int)
            // Return Err to signal "promote to i64" since IntWidth doesn't include i64
            (IntWidth::U32, _) => Err(()),

            _ => Err(()),
        }
    }

    /// Check if a given i64 value is in range for this width.
    #[inline]
    pub const fn in_range_i64(self, value: i64) -> bool {
        value >= self.min_value() && value <= self.max_value()
    }

    /// Check if a given u64 value is in range for this width.
    #[inline]
    pub const fn in_range_u64(self, value: u64) -> bool {
        value <= self.max_unsigned()
    }
}

impl std::fmt::Display for IntWidth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.type_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_i8_boundaries() {
        assert_eq!(IntWidth::I8.truncate(127), 127);
        assert_eq!(IntWidth::I8.truncate(128), -128);
        assert_eq!(IntWidth::I8.truncate(-128), -128);
        assert_eq!(IntWidth::I8.truncate(-129), 127);
        assert_eq!(IntWidth::I8.truncate(255), -1);
        assert_eq!(IntWidth::I8.truncate(256), 0);
    }

    #[test]
    fn truncate_u8_boundaries() {
        assert_eq!(IntWidth::U8.truncate(0), 0);
        assert_eq!(IntWidth::U8.truncate(255), 255);
        assert_eq!(IntWidth::U8.truncate(256), 0);
        assert_eq!(IntWidth::U8.truncate(-1), 255);
    }

    #[test]
    fn truncate_i16_boundaries() {
        assert_eq!(IntWidth::I16.truncate(32767), 32767);
        assert_eq!(IntWidth::I16.truncate(32768), -32768);
        assert_eq!(IntWidth::I16.truncate(-32768), -32768);
        assert_eq!(IntWidth::I16.truncate(-32769), 32767);
    }

    #[test]
    fn truncate_u16_boundaries() {
        assert_eq!(IntWidth::U16.truncate(0), 0);
        assert_eq!(IntWidth::U16.truncate(65535), 65535);
        assert_eq!(IntWidth::U16.truncate(65536), 0);
        assert_eq!(IntWidth::U16.truncate(-1), 65535);
    }

    #[test]
    fn truncate_i32_boundaries() {
        assert_eq!(IntWidth::I32.truncate(2147483647), 2147483647);
        assert_eq!(IntWidth::I32.truncate(2147483648), -2147483648);
        assert_eq!(IntWidth::I32.truncate(-2147483648), -2147483648);
    }

    #[test]
    fn truncate_u32_boundaries() {
        assert_eq!(IntWidth::U32.truncate(0), 0);
        assert_eq!(IntWidth::U32.truncate(4294967295), 4294967295);
        assert_eq!(IntWidth::U32.truncate(4294967296), 0);
        assert_eq!(IntWidth::U32.truncate(-1), 4294967295);
    }

    #[test]
    fn truncate_u64_identity() {
        assert_eq!(IntWidth::U64.truncate(0), 0);
        assert_eq!(IntWidth::U64.truncate(i64::MAX), i64::MAX);
        assert_eq!(IntWidth::U64.truncate(-1), -1); // bit pattern preserved
    }

    #[test]
    fn truncate_u64_unsigned() {
        assert_eq!(IntWidth::U64.truncate_u64(0), 0);
        assert_eq!(IntWidth::U64.truncate_u64(u64::MAX), u64::MAX);
        assert_eq!(IntWidth::U64.truncate_u64(u64::MAX - 1), u64::MAX - 1);
    }

    #[test]
    fn join_same_width() {
        assert_eq!(IntWidth::join(IntWidth::I8, IntWidth::I8), Ok(IntWidth::I8));
        assert_eq!(
            IntWidth::join(IntWidth::U64, IntWidth::U64),
            Ok(IntWidth::U64)
        );
    }

    #[test]
    fn join_same_sign_different_width() {
        assert_eq!(
            IntWidth::join(IntWidth::I8, IntWidth::I16),
            Ok(IntWidth::I16)
        );
        assert_eq!(
            IntWidth::join(IntWidth::I16, IntWidth::I32),
            Ok(IntWidth::I32)
        );
        assert_eq!(
            IntWidth::join(IntWidth::U8, IntWidth::U16),
            Ok(IntWidth::U16)
        );
        assert_eq!(
            IntWidth::join(IntWidth::U16, IntWidth::U32),
            Ok(IntWidth::U32)
        );
    }

    #[test]
    fn join_mixed_sign_widens() {
        assert_eq!(
            IntWidth::join(IntWidth::U8, IntWidth::I8),
            Ok(IntWidth::I16)
        );
        assert_eq!(
            IntWidth::join(IntWidth::I8, IntWidth::U8),
            Ok(IntWidth::I16)
        );
        assert_eq!(
            IntWidth::join(IntWidth::U16, IntWidth::I16),
            Ok(IntWidth::I32)
        );
        assert_eq!(
            IntWidth::join(IntWidth::U8, IntWidth::I16),
            Ok(IntWidth::I16)
        );
        assert_eq!(
            IntWidth::join(IntWidth::U8, IntWidth::I32),
            Ok(IntWidth::I32)
        );
        assert_eq!(
            IntWidth::join(IntWidth::U16, IntWidth::I32),
            Ok(IntWidth::I32)
        );
    }

    #[test]
    fn join_u64_signed_error() {
        assert_eq!(IntWidth::join(IntWidth::U64, IntWidth::I8), Err(()));
        assert_eq!(IntWidth::join(IntWidth::U64, IntWidth::I16), Err(()));
        assert_eq!(IntWidth::join(IntWidth::U64, IntWidth::I32), Err(()));
        assert_eq!(IntWidth::join(IntWidth::I8, IntWidth::U64), Err(()));
    }

    #[test]
    fn join_u32_signed_promotes_to_i64() {
        // u32 + any signed → Err (needs i64, which is outside IntWidth)
        assert_eq!(IntWidth::join(IntWidth::U32, IntWidth::I8), Err(()));
        assert_eq!(IntWidth::join(IntWidth::U32, IntWidth::I32), Err(()));
    }

    #[test]
    fn from_name_roundtrip() {
        for w in IntWidth::ALL {
            assert_eq!(IntWidth::from_name(w.type_name()), Some(w));
        }
        assert_eq!(IntWidth::from_name("i64"), None);
        assert_eq!(IntWidth::from_name("float"), None);
    }

    #[test]
    fn in_range_checks() {
        assert!(IntWidth::I8.in_range_i64(0));
        assert!(IntWidth::I8.in_range_i64(127));
        assert!(IntWidth::I8.in_range_i64(-128));
        assert!(!IntWidth::I8.in_range_i64(128));
        assert!(!IntWidth::I8.in_range_i64(-129));

        assert!(IntWidth::U8.in_range_i64(0));
        assert!(IntWidth::U8.in_range_i64(255));
        assert!(!IntWidth::U8.in_range_i64(-1));
        assert!(!IntWidth::U8.in_range_i64(256));

        assert!(IntWidth::U64.in_range_u64(u64::MAX));
        assert!(IntWidth::U64.in_range_u64(0));
    }

    #[test]
    fn display_impl() {
        assert_eq!(format!("{}", IntWidth::I8), "i8");
        assert_eq!(format!("{}", IntWidth::U64), "u64");
    }
}
