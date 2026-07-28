pub mod alloc_budget;
mod capture_native_kind;
pub mod closure_layout;
pub mod closure_raw;
pub mod concrete_type;
pub mod decimal_obj;
pub mod function_type_registry;
/// The single allocation seam for typed heap carriers (#194, ADR-018 §4).
/// Every raw block a carrier owns is handed out here, which is what makes the
/// `alloc_budget` heap ceiling unbypassable and gives region allocation (#195)
/// one place to land.
pub mod heap_alloc;
pub mod heap_element;
pub mod heap_header;
pub mod refcount;
pub mod string_obj;
pub mod struct_layout;
pub mod typed_array;
pub mod typed_option;
pub mod typed_result;

pub use concrete_type::ConcreteType;
