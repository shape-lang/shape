#!/usr/bin/env python3
"""Check the strict-flip ignored-test source classification baseline.

This is intentionally source-only and cheap: it does not run cargo, nextest, or
Miri. The supervisor-observed lib-test ignored counts are documented in
docs/cluster-audits/w86c-ignored-tests-and-miri-classification.md; this checker
guards the source-level ignore reasons and cause buckets from drifting silently.
"""

from __future__ import annotations

import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path


ROOT = Path(
    subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip()
)

CRATE_ROOTS = {
    "shape-vm": ROOT / "crates/shape-vm/src",
    "shape-jit": ROOT / "crates/shape-jit/src",
}

EXPECTED_COUNTS = {
    "shape-vm": {
        "phase_2c_surface": 93,
        "active_feature_gap": 0,
        "stale_semantic_expectation": 0,
        "deleted_v1_path": 5,
        "diagnostic_only": 1,
    },
    "shape-jit": {
        "deleted_v1_path": 21,
        "active_feature_gap": 0,
        "process_aborting_extern_c_todo": 3,
        "stale_semantic_expectation": 0,
    },
}

EXPECTED_SOURCE_ONLY_STATUS = {
    "shape-vm": {
        "deep-tests": 47,
    },
    "shape-jit": {
        "cfg-any": 1,
    },
}

REPORTED_LIB_IGNORED_BASELINE = {
    "shape-vm": 56,
    "shape-jit": 23,
}

# These are source files whose module declarations are feature-gated behind
# `deep-tests` in the parent module. They still need classified ignore reasons,
# but they do not explain the default lib-test ignored count by themselves.
DEEP_TEST_FILES = {
    ROOT / "crates/shape-vm/src/executor/tests/differential_trusted.rs",
    ROOT / "crates/shape-vm/src/executor/tests/drop_deep_tests.rs",
    ROOT / "crates/shape-vm/src/executor/tests/extend_blocks.rs",
    ROOT / "crates/shape-vm/src/executor/tests/hashmap_ops.rs",
    ROOT / "crates/shape-vm/src/executor/tests/iterator_ops.rs",
    ROOT / "crates/shape-vm/src/executor/tests/module_deep_tests.rs",
    ROOT / "crates/shape-vm/src/executor/tests/operator_overload.rs",
    ROOT / "crates/shape-vm/src/executor/tests/trusted_edge_cases.rs",
    ROOT / "crates/shape-jit/src/compiler/c2_tests.rs",
}

ALLOWED_UNREASONED = {
    "debug_decimal_opcodes",
}

DEEP_TEST_RANGE_CACHE: dict[Path, list[tuple[int, int]]] = {}


def normalize_reason(attr: str) -> str:
    match = re.search(r'ignore\s*=\s*"(.*)"\s*\]', attr)
    if not match:
        return "(no reason)"
    reason = match.group(1).replace(r"\"", '"').replace("\\", " ")
    return re.sub(r"\s+", " ", reason).strip()


def parse_ignored_tests(crate: str, root: Path) -> list[dict[str, object]]:
    entries: list[dict[str, object]] = []
    for path in sorted(root.rglob("*.rs")):
        lines = path.read_text(errors="replace").splitlines()
        i = 0
        while i < len(lines):
            if not re.match(r"\s*#\s*\[\s*ignore\b", lines[i]):
                i += 1
                continue

            start = i
            attr_parts = [lines[i].strip()]
            while not re.search(r"\]\s*(?://.*)?$", attr_parts[-1]) and i + 1 < len(lines):
                i += 1
                attr_parts.append(lines[i].strip())
            attr = " ".join(attr_parts)

            attrs: list[str] = []
            prev = start - 1
            while prev >= 0 and (lines[prev].strip().startswith("#") or not lines[prev].strip()):
                if lines[prev].strip().startswith("#"):
                    attrs.append(lines[prev].strip())
                prev -= 1

            name = "?"
            lookahead = start + 1
            while lookahead < len(lines) and lookahead <= start + 30:
                stripped = lines[lookahead].strip()
                if stripped.startswith("#"):
                    attrs.append(stripped)
                match = re.search(r"\bfn\s+([A-Za-z0-9_]+)", lines[lookahead])
                if match:
                    name = match.group(1)
                    break
                lookahead += 1

            entries.append(
                {
                    "crate": crate,
                    "path": path,
                    "line": start + 1,
                    "name": name,
                    "reason": normalize_reason(attr),
                    "attrs": attrs,
                }
            )
            i += 1
    return entries


def collect_multiline_attr(lines: list[str], start: int) -> tuple[str, int]:
    attr_parts = [lines[start].strip()]
    i = start
    while not re.search(r"\]\s*(?://.*)?$", attr_parts[-1]) and i + 1 < len(lines):
        i += 1
        attr_parts.append(lines[i].strip())
    return " ".join(attr_parts), i


def deep_test_cfg_ranges(path: Path) -> list[tuple[int, int]]:
    cached = DEEP_TEST_RANGE_CACHE.get(path)
    if cached is not None:
        return cached

    lines = path.read_text(errors="replace").splitlines()
    ranges: list[tuple[int, int]] = []
    i = 0
    while i < len(lines):
        if not re.match(r"\s*#\s*\[\s*cfg\b", lines[i]):
            i += 1
            continue

        attr, attr_end = collect_multiline_attr(lines, i)
        if "deep-tests" not in attr:
            i = attr_end + 1
            continue

        lookahead = attr_end + 1
        while lookahead < len(lines):
            stripped = lines[lookahead].strip()
            if not stripped or stripped.startswith("#"):
                lookahead += 1
                continue
            break

        opener = lookahead
        while opener < len(lines) and "{" not in lines[opener]:
            opener += 1
        if opener >= len(lines):
            i = attr_end + 1
            continue

        opener_indent = len(lines[opener]) - len(lines[opener].lstrip())
        end_line = len(lines)
        for closing in range(opener + 1, len(lines)):
            stripped = lines[closing].strip()
            indent = len(lines[closing]) - len(lines[closing].lstrip())
            if stripped.startswith("}") and indent == opener_indent:
                end_line = closing + 1
                break

        ranges.append((i + 1, end_line))
        i = attr_end + 1

    DEEP_TEST_RANGE_CACHE[path] = ranges
    return ranges


def has_test_attr(entry: dict[str, object]) -> bool:
    return any(re.match(r"#\s*\[\s*test\b", attr) for attr in entry["attrs"])  # type: ignore[index]


def source_only_status(entry: dict[str, object]) -> str | None:
    path = entry["path"]
    attrs = entry["attrs"]
    if path in DEEP_TEST_FILES:
        return "deep-tests"
    if any("cfg" in attr and "deep-tests" in attr for attr in attrs):  # type: ignore[operator]
        return "deep-tests"
    line = int(entry["line"])
    if any(start <= line <= end for start, end in deep_test_cfg_ranges(path)):  # type: ignore[arg-type]
        return "deep-tests"
    if any("cfg(any())" in attr for attr in attrs):  # type: ignore[operator]
        return "cfg-any"
    return None


def classify(entry: dict[str, object]) -> str:
    reason = str(entry["reason"]).lower()
    name = str(entry["name"])

    if (
        'extern "c"' in reason
        and (
            "todo" in reason
            or "abort" in reason
            or "sigabrt" in reason
            or "can't unwind" in reason
        )
    ):
        return "process_aborting_extern_c_todo"

    if "diagnostic-only" in reason or name == "debug_decimal_opcodes":
        return "diagnostic_only"

    if any(
        pattern in reason
        for pattern in (
            "copy-on-write aliasing",
            "v1 semantics",
            "stale numeric trait",
            "out-param sugar is stale",
            "diagnostics are reordered",
            "error message that no longer matches",
        )
    ):
        return "stale_semantic_expectation"

    if any(
        pattern in reason
        for pattern in (
            "deleted bytecodetoir",
            "deleted jitarray",
            "jitarray/jit_array_info",
            "deleted nan-box",
            "deleted __native_ptr",
            "deleted `typedarraydata` enum",
            "deleted v1 vmarray",
            "tier 1 whole-function jit (compile_single_function) deprecated",
            "deleted host-tier iterator carrier",
        )
    ):
        return "deleted_v1_path"

    if any(
        pattern in reason
        for pattern in (
            "pre-existing jit bug",
            "build_kernel_ir is stubbed",
            "build_correlated_kernel_ir is stubbed",
            "turbofish",
            "flatten monomorphization",
            "const-specialization",
            "generic vec extension",
            "multi-extend resolver",
            "module return-kind",
            "module recursion",
            "module match lowering",
            "module method resolution",
            "module trait method resolution",
            "module method chaining",
            "module-qualified type annotations",
            "temporal arithmetic retarget",
            "matrix runtime carrier retarget",
            "matrix/vector arithmetic retarget",
            "mir reference-escape",
            "destructuring",
            "extern-c out-param caller-visible arity",
            "internal intrinsic diagnostic ordering",
        )
    ):
        return "active_feature_gap"

    if any(
        pattern in reason
        for pattern in (
            "phase-2c",
            "surface",
            "t1 class-shift",
            "typed-arc heapvalue layout",
            "state-snapshot rebuild",
            "host-tier eval/marshal",
            "host argument conversion",
        )
    ):
        return "phase_2c_surface"

    raise ValueError("unknown ignore classification")


def format_location(entry: dict[str, object]) -> str:
    path = Path(entry["path"]).relative_to(ROOT)
    return f"{path}:{entry['line']} {entry['name']}"


def main() -> int:
    entries: list[dict[str, object]] = []
    for crate, root in CRATE_ROOTS.items():
        entries.extend(parse_ignored_tests(crate, root))

    errors: list[str] = []
    counts: dict[str, Counter[str]] = defaultdict(Counter)
    status_counts: dict[str, Counter[str]] = defaultdict(Counter)

    for entry in entries:
        if entry["name"] == "?":
            errors.append(f"ignore without following test function: {format_location(entry)}")
            continue
        if not has_test_attr(entry):
            errors.append(f"ignore not paired with #[test]: {format_location(entry)}")
        if entry["reason"] == "(no reason)" and entry["name"] not in ALLOWED_UNREASONED:
            errors.append(f"ignored test lacks reason: {format_location(entry)}")
        try:
            category = classify(entry)
        except ValueError:
            errors.append(
                "unknown ignore reason category: "
                f"{format_location(entry)} reason={entry['reason']!r}"
            )
            continue
        crate = str(entry["crate"])
        counts[crate][category] += 1
        status = source_only_status(entry)
        if status:
            status_counts[crate][status] += 1

    print("Ignored test source classification:")
    for crate in sorted(CRATE_ROOTS):
        total = sum(counts[crate].values())
        print(f"  {crate}: {total}")
        print(
            "    reported --lib ignored baseline "
            f"(not source-derived): {REPORTED_LIB_IGNORED_BASELINE[crate]}"
        )
        for category, count in sorted(counts[crate].items()):
            print(f"    {category}: {count}")
        if status_counts[crate]:
            print("    source-only gates:")
            for status, count in sorted(status_counts[crate].items()):
                print(f"      {status}: {count}")

    for crate, expected in EXPECTED_COUNTS.items():
        if counts[crate] != Counter(expected):
            errors.append(
                f"{crate} classification drift: got {dict(counts[crate])}, "
                f"expected {expected}"
            )
    for crate, expected in EXPECTED_SOURCE_ONLY_STATUS.items():
        if status_counts[crate] != Counter(expected):
            errors.append(
                f"{crate} source-only status drift: got {dict(status_counts[crate])}, "
                f"expected {expected}"
            )

    if errors:
        print("\nIgnored test classification check FAILED:", file=sys.stderr)
        for error in errors:
            print(f"  - {error}", file=sys.stderr)
        return 1

    print("Ignored test classification check passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
