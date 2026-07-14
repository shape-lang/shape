use super::test_utils::{eval, eval_result};
use shape_value::content::ContentNode;
use shape_value::{HeapKind, KindedSlot, NativeKind};

fn assert_fragment_text(slot: &KindedSlot, expected: &[&str]) {
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Content));
    let node = unsafe { &*(slot.raw() as *const ContentNode) };
    let ContentNode::Fragment(parts) = node else {
        panic!("expected Content.fragment result, got {node:?}");
    };
    assert_eq!(parts.len(), expected.len());
    for (part, expected_text) in parts.iter().zip(expected) {
        let ContentNode::Text(text) = part else {
            panic!("expected text fragment part, got {part:?}");
        };
        assert_eq!(text.spans.len(), 1);
        assert_eq!(text.spans[0].text, *expected_text);
    }
}

#[test]
fn content_fragment_accepts_direct_builder_array() {
    let slot = eval(
        r#"
Content.fragment([Content.text("fast"), Content.text("slow")])
"#,
    );
    assert_fragment_text(&slot, &["fast", "slow"]);
}

#[test]
fn content_fragment_accepts_identifier_builder_array() {
    let slot = eval(
        r#"
let fast = Content.text("fast")
let slow = Content.text("slow")
Content.fragment([fast, slow])
"#,
    );
    assert_fragment_text(&slot, &["fast", "slow"]);
}

#[test]
fn content_fragment_accepts_explicit_content_array_annotation() {
    let slot = eval(
        r#"
let parts: Array<content> = [Content.text("summary"), Content.text("chart")]
Content.fragment(parts)
"#,
    );
    assert_fragment_text(&slot, &["summary", "chart"]);
}

#[test]
fn content_fragment_accepts_bound_chart_builders() {
    let slot = eval(
        r#"
let sma_20_data = [[1, 10], [2, 12], [3, 11]]
let sma_50_data = [[1, 9], [2, 10], [3, 12]]

let fast = Content.chart("line")
    .add("SMA 20", sma_20_data)
    .title("Window 20")

let slow = Content.chart("line")
    .add("SMA 50", sma_50_data)
    .title("Window 50")

Content.fragment([fast, slow])
"#,
    );
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Content));
    let node = unsafe { &*(slot.raw() as *const ContentNode) };
    let ContentNode::Fragment(parts) = node else {
        panic!("expected Content.fragment result, got {node:?}");
    };
    assert_eq!(parts.len(), 2);
    assert!(matches!(parts[0], ContentNode::Chart(_)));
    assert!(matches!(parts[1], ContentNode::Chart(_)));
}

#[test]
fn content_fragment_accepts_mixed_content_builder_values() {
    let slot = eval(
        r#"
let summary = Content.text("Report as of 2024-06-15")
    .bold()

let table = Content.table(["name"], [["test"]])
    .border(Border.rounded)

let chart = Content.chart(ChartType.line)
    .add("trend", [[1, 10], [2, 20]])
    .title("Trend")

Content.fragment([summary, table, chart])
"#,
    );
    assert_eq!(slot.kind(), NativeKind::Ptr(HeapKind::Content));
    let node = unsafe { &*(slot.raw() as *const ContentNode) };
    let ContentNode::Fragment(parts) = node else {
        panic!("expected Content.fragment result, got {node:?}");
    };
    assert_eq!(parts.len(), 3);
    assert!(matches!(parts[0], ContentNode::Text(_)));
    assert!(matches!(parts[1], ContentNode::Table(_)));
    assert!(matches!(parts[2], ContentNode::Chart(_)));
}

#[test]
fn content_fragment_rejects_mixed_non_content_array() {
    let err = eval_result(
        r#"
Content.fragment([Content.text("fast"), 1])
"#,
    )
    .expect_err("mixed non-content array must not be coerced into Array<content>");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("content") || msg.contains("Content.fragment"),
        "mixed content array rejection should cite content, got: {msg}"
    );
}
