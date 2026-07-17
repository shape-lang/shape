//! Single-source generator for [`HeapKind`](crate::HeapKind) and its catalog.
//!
//! The enum and `HeapKind::ALL` must be emitted from the same variant tokens:
//! a separately maintained list can silently omit a newly appended kind while
//! still satisfying a last-discriminant length check.

/// Define a `#[repr(u8)]` heap-kind enum and its complete ordinal catalog from
/// one variant list.
///
/// The generated const assertion preserves the historical gap-free ordinal
/// contract. Adding, removing, or reordering a variant changes both the enum
/// and catalog together; consumers cannot update one without the other.
#[doc(hidden)]
#[macro_export]
macro_rules! define_heap_kind_and_catalog {
    (
        $(#[$enum_meta:meta])*
        $visibility:vis enum $name:ident {
            $($variant:ident),+ $(,)?
        }
    ) => {
        $(#[$enum_meta])*
        $visibility enum $name {
            $($variant),+
        }

        impl $name {
            /// Every heap kind in canonical `#[repr(u8)]` ordinal order.
            pub const ALL: [Self; $crate::define_heap_kind_and_catalog!(@count $($variant),+)] = [
                $(Self::$variant),+
            ];

            /// Whether this kind has a valid nonzero 8-byte `KindedSlot`
            /// representation.
            ///
            /// `NativeScalar` is a width-preserving enum of up to 16 bytes.
            /// Its old heap variant has no chosen 8-byte slot carrier, so
            /// treating arbitrary nonzero bits as one would fabricate
            /// ownership. All other variants have an explicit typed carrier
            /// or an intentional inline-scalar representation in the
            /// canonical `KindedSlot` clone/drop dispatch.
            #[inline]
            pub const fn has_kinded_slot_carrier(self) -> bool {
                !matches!(self, Self::NativeScalar)
            }
        }

        const _: () = {
            let mut ordinal = 0;
            while ordinal < $name::ALL.len() {
                assert!($name::ALL[ordinal] as usize == ordinal);
                ordinal += 1;
            }
        };
    };

    (@count $($variant:ident),+) => {
        <[()]>::len(&[
            $($crate::define_heap_kind_and_catalog!(@unit $variant)),+
        ])
    };

    (@unit $variant:ident) => { () };
}
