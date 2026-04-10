//! Content method dispatch for ContentNode values (v2 native).
//!
//! All methods are MethodFnV2 handlers dispatched via the CONTENT_METHODS PHF map.
//!
//! Supports style methods: `.fg()`, `.bg()`, `.bold()`, `.italic()`, `.underline()`, `.dim()`
//! Table methods: `.border()`, `.max_rows()`
//! Chart methods: `.series()`, `.title()`, `.x_label()`, `.y_label()`

use crate::executor::VirtualMachine;
use shape_value::content::{Color, ContentNode, NamedColor};
use shape_value::{VMError, ValueWord};

// ═══════════════════════════════════════════════════════════════════════════
// V2 (MethodFnV2) Content handlers
// ═══════════════════════════════════════════════════════════════════════════

use std::mem::ManuallyDrop;

/// Borrow a ValueWord from raw u64 bits without taking ownership.
#[inline]
fn borrow_vw(raw: u64) -> ManuallyDrop<ValueWord> {
    ManuallyDrop::new(ValueWord::from_raw_bits(raw))
}

pub fn v2_content_bold(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let node = vw
        .as_content()
        .cloned()
        .unwrap_or_else(|| ContentNode::plain(format!("{}", *vw)));
    Ok(ValueWord::from_content(node.with_bold()).into_raw_bits())
}

pub fn v2_content_italic(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let node = vw
        .as_content()
        .cloned()
        .unwrap_or_else(|| ContentNode::plain(format!("{}", *vw)));
    Ok(ValueWord::from_content(node.with_italic()).into_raw_bits())
}

pub fn v2_content_underline(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let node = vw
        .as_content()
        .cloned()
        .unwrap_or_else(|| ContentNode::plain(format!("{}", *vw)));
    Ok(ValueWord::from_content(node.with_underline()).into_raw_bits())
}

pub fn v2_content_dim(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let node = vw
        .as_content()
        .cloned()
        .unwrap_or_else(|| ContentNode::plain(format!("{}", *vw)));
    Ok(ValueWord::from_content(node.with_dim()).into_raw_bits())
}

pub fn v2_content_fg(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let node = vw
        .as_content()
        .cloned()
        .unwrap_or_else(|| ContentNode::plain(format!("{}", *vw)));
    let color = parse_color_arg_v2(args, 1, "fg")?;
    Ok(ValueWord::from_content(node.with_fg(color)).into_raw_bits())
}

pub fn v2_content_bg(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let node = vw
        .as_content()
        .cloned()
        .unwrap_or_else(|| ContentNode::plain(format!("{}", *vw)));
    let color = parse_color_arg_v2(args, 1, "bg")?;
    Ok(ValueWord::from_content(node.with_bg(color)).into_raw_bits())
}

pub fn v2_content_to_string(
    _vm: &mut VirtualMachine,
    args: &mut [u64],
    _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
) -> Result<u64, VMError> {
    let vw = borrow_vw(args[0]);
    let node = vw
        .as_content()
        .cloned()
        .unwrap_or_else(|| ContentNode::plain(format!("{}", *vw)));
    let text = format!("{}", node);
    Ok(ValueWord::from_string(std::sync::Arc::new(text)).into_raw_bits())
}

/// Generic v2 handler for Content methods that delegate to `shape_runtime::content_methods`.
/// The method name must be passed via the PHF dispatch since v2 handlers don't receive it.
/// We solve this by creating a separate handler per method.
macro_rules! content_runtime_method {
    ($fn_name:ident, $method_name:expr) => {
        pub fn $fn_name(
            _vm: &mut VirtualMachine,
            args: &mut [u64],
            _ctx: Option<&mut shape_runtime::context::ExecutionContext>,
        ) -> Result<u64, VMError> {
            let receiver = (*borrow_vw(args[0])).clone();
            let method_args: Vec<ValueWord> =
                args[1..].iter().map(|&r| (*borrow_vw(r)).clone()).collect();
            match shape_runtime::content_methods::call_content_method(
                $method_name,
                receiver,
                method_args,
            ) {
                Some(Ok(result_nb)) => Ok(result_nb.into_raw_bits()),
                Some(Err(e)) => Err(VMError::RuntimeError(format!("{}", e))),
                None => Err(VMError::RuntimeError(format!(
                    "Unknown method '{}' on Content type",
                    $method_name
                ))),
            }
        }
    };
}

content_runtime_method!(v2_content_border, "border");
content_runtime_method!(v2_content_max_rows, "max_rows");
content_runtime_method!(v2_content_max_rows_camel, "maxRows");
content_runtime_method!(v2_content_series, "series");
content_runtime_method!(v2_content_title, "title");
content_runtime_method!(v2_content_x_label, "x_label");
content_runtime_method!(v2_content_x_label_camel, "xLabel");
content_runtime_method!(v2_content_y_label, "y_label");
content_runtime_method!(v2_content_y_label_camel, "yLabel");

/// Parse a color argument from raw u64 args for v2 handlers
fn parse_color_arg_v2(
    args: &mut [u64],
    start_idx: usize,
    method_name: &str,
) -> Result<Color, VMError> {
    if args.len() <= start_idx {
        return Err(VMError::RuntimeError(format!(
            "Content.{}() requires a color argument",
            method_name
        )));
    }

    let vw = borrow_vw(args[start_idx]);
    // String color name
    if let Some(name) = vw.as_str() {
        return match name.to_lowercase().as_str() {
            "red" => Ok(Color::Named(NamedColor::Red)),
            "green" => Ok(Color::Named(NamedColor::Green)),
            "blue" => Ok(Color::Named(NamedColor::Blue)),
            "yellow" => Ok(Color::Named(NamedColor::Yellow)),
            "magenta" => Ok(Color::Named(NamedColor::Magenta)),
            "cyan" => Ok(Color::Named(NamedColor::Cyan)),
            "white" => Ok(Color::Named(NamedColor::White)),
            "default" => Ok(Color::Named(NamedColor::Default)),
            _ => Err(VMError::RuntimeError(format!(
                "Unknown color name '{}'. Available: red, green, blue, yellow, magenta, cyan, white, default",
                name
            ))),
        };
    }

    // RGB as three numeric args
    if args.len() >= start_idx + 3 {
        let r = borrow_vw(args[start_idx])
            .as_number_coerce()
            .ok_or_else(|| VMError::RuntimeError("RGB red component must be numeric".to_string()))?
            as u8;
        let g = borrow_vw(args[start_idx + 1])
            .as_number_coerce()
            .ok_or_else(|| {
                VMError::RuntimeError("RGB green component must be numeric".to_string())
            })? as u8;
        let b = borrow_vw(args[start_idx + 2])
            .as_number_coerce()
            .ok_or_else(|| {
                VMError::RuntimeError("RGB blue component must be numeric".to_string())
            })? as u8;
        return Ok(Color::Rgb(r, g, b));
    }

    Err(VMError::RuntimeError(format!(
        "Content.{}() requires a color name string or three RGB components",
        method_name
    )))
}

