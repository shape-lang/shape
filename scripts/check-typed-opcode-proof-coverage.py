#!/usr/bin/env python3
"""Guard typed-opcode/static-kind proof coverage in the VM compiler.

This is intentionally source-only and cheap. It does not run cargo, rustc,
nextest, or Miri. The guard scans compiler Rust sources for direct typed
opcode mentions, classifies each production mention into the W91A proof
buckets, and fails if new unclassified production paths appear.

The baseline is count-based rather than line-number based. When a typed
emission site is intentionally added, update this checker and
docs/cluster-audits/w91a-typed-opcode-proof-coverage.md in the same patch.
"""

from __future__ import annotations

import re
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(
    subprocess.check_output(["git", "rev-parse", "--show-toplevel"], text=True).strip()
)
COMPILER_ROOT = ROOT / "crates/shape-vm/src/compiler"

EXPECTED_COUNTS = {
    "covered_by_prove_native_kind": 22,
    "covered_by_equivalent_static_proof_helper": 530,
    "metadata_only_non_executing": 145,
    "unproven_gap": 0,
}

KIND_SUFFIXES = (
    "I64",
    "U64",
    "F64",
    "I32",
    "U32",
    "I16",
    "U16",
    "I8",
    "U8",
    "Bool",
    "Ptr",
)

SCALAR_TYPED_RE = re.compile(
    r"^(Add|Sub|Mul|Div|Mod|Pow|Gt|Lt|Gte|Lte|Eq|Neq|Neg|BitAnd|BitOr|"
    r"BitXor|BitShl|BitShr|BitNot|StringConcat)"
    r"(Int|Number|Decimal|String|Bool|I32)$"
)

EXACT_TYPED_OPCODES = {
    "AddTyped",
    "SubTyped",
    "MulTyped",
    "DivTyped",
    "ModTyped",
    "CmpTyped",
    "GetFieldTyped",
    "SetFieldTyped",
    "NewTypedObject",
    "TypedMergeObject",
    "NewTypedStruct",
    "StoreLocalTyped",
    "StoreModuleBindingTyped",
    "ArrayLenTyped",
    "MapLenTyped",
    "StringLenTyped",
    "StringCharAt",
    "StringConcatTyped",
    "EqTypedObject",
    "IntToNumber",
    "NumberToInt",
    "LoadColF64",
    "LoadColI64",
    "LoadColBool",
    "LoadColStr",
    "GetElemI64",
    "GetElemF64",
    "SetElemI64",
    "SetElemF64",
    "ArrayPushI64",
    "ArrayPushF64",
}


@dataclass(frozen=True)
class Hit:
    path: Path
    line_no: int
    opcode: str
    text: str
    in_test: bool

    @property
    def relpath(self) -> str:
        return self.path.relative_to(ROOT).as_posix()


def strip_comments(line: str, in_block_comment: bool) -> tuple[str, bool]:
    """Remove Rust comments well enough for this source inventory."""
    out = []
    i = 0
    while i < len(line):
        if in_block_comment:
            end = line.find("*/", i)
            if end == -1:
                return "".join(out), True
            i = end + 2
            in_block_comment = False
            continue

        block = line.find("/*", i)
        slash = line.find("//", i)
        if slash != -1 and (block == -1 or slash < block):
            out.append(line[i:slash])
            break
        if block == -1:
            out.append(line[i:])
            break
        out.append(line[i:block])
        i = block + 2
        in_block_comment = True

    return "".join(out), in_block_comment


def is_typed_opcode(opcode: str) -> bool:
    if opcode in EXACT_TYPED_OPCODES or SCALAR_TYPED_RE.match(opcode):
        return True
    if opcode.startswith("NewTypedArray") or opcode.startswith("TypedArray"):
        return True
    if opcode.startswith("FieldLoad") or opcode.startswith("FieldStore"):
        return True
    for prefix in (
        "LoadLocal",
        "StoreLocal",
        "LoadModuleBinding",
        "StoreModuleBinding",
        "ReturnValue",
        "LoadSharedCapture",
        "StoreSharedCapture",
        "LoadOwnedMutableCapture",
        "StoreOwnedMutableCapture",
    ):
        if opcode.startswith(prefix) and opcode.removeprefix(prefix) in KIND_SUFFIXES:
            return True
    return False


def scan() -> list[Hit]:
    hits: list[Hit] = []
    for path in sorted(COMPILER_ROOT.rglob("*.rs")):
        lines = path.read_text(errors="replace").splitlines()
        in_block_comment = False
        pending_cfg_test = False
        brace_depth = 0
        test_depths: list[int] = []
        for line_no, raw in enumerate(lines, start=1):
            code, in_block_comment = strip_comments(raw, in_block_comment)
            stripped = code.strip()
            if "#[cfg(test)]" in stripped:
                pending_cfg_test = True

            starts_cfg_test_item = False
            if pending_cfg_test and re.search(r"\b(fn|mod|impl)\b", stripped):
                starts_cfg_test_item = True
                pending_cfg_test = False

            line_in_test = bool(test_depths) or starts_cfg_test_item
            for match in re.finditer(r"\bOpCode::([A-Za-z0-9_]+)", code):
                opcode = match.group(1)
                if not is_typed_opcode(opcode):
                    continue
                hits.append(
                    Hit(
                        path=path,
                        line_no=line_no,
                        opcode=opcode,
                        text=raw.strip(),
                        in_test=line_in_test,
                    )
                )

            opens = code.count("{")
            closes = code.count("}")
            if starts_cfg_test_item and opens:
                test_depths.append(brace_depth + 1)
            brace_depth += opens - closes
            while test_depths and brace_depth < test_depths[-1]:
                test_depths.pop()
    return hits


def classify(hit: Hit) -> tuple[str, str]:
    rel = hit.relpath
    op = hit.opcode
    text = hit.text

    if hit.in_test:
        return "metadata_only_non_executing", "test/assertion/doc-only code"

    if op.startswith("ReturnValue") and op != "ReturnValue":
        if rel == "crates/shape-vm/src/compiler/helpers.rs":
            return (
                "covered_by_prove_native_kind",
                "typed_return_value_opcode reached from exact_scalar_return_kind_for_expr/prove_native_kind",
            )
        return "unclassified", "typed ReturnValue outside return proof helper"

    if op.startswith("LoadCol"):
        if rel == "crates/shape-vm/src/compiler/helpers.rs" and "default" in text:
            return "unproven_gap", "row_view_field_opcode LoadColF64 fallback without schema proof"
        if rel == "crates/shape-vm/src/compiler/helpers.rs":
            return "covered_by_equivalent_static_proof_helper", "RowView schema FieldType maps to LoadCol kind"
        return "unclassified", "LoadCol opcode outside RowView helper"

    if op.startswith("NewTypedArray") or op.startswith("TypedArray"):
        return "covered_by_equivalent_static_proof_helper", "TypedArrayKind/ConcreteType gate"

    if op in {"GetElemI64", "GetElemF64", "SetElemI64", "SetElemF64", "ArrayPushI64", "ArrayPushF64", "ArrayLenTyped", "MapLenTyped"}:
        return "covered_by_equivalent_static_proof_helper", "tracked v2 typed-array receiver kind"

    if op in {"StringLenTyped", "StringCharAt"}:
        return "covered_by_equivalent_static_proof_helper", "tracked string receiver type"

    if op in {"GetFieldTyped", "SetFieldTyped", "NewTypedObject", "TypedMergeObject", "NewTypedStruct"}:
        return "covered_by_equivalent_static_proof_helper", "schema/TypedField operand proof"

    if op.startswith("FieldLoad") or op.startswith("FieldStore"):
        return "covered_by_equivalent_static_proof_helper", "StructLayout FieldKind proof"

    if op.startswith(("LoadSharedCapture", "StoreSharedCapture", "LoadOwnedMutableCapture", "StoreOwnedMutableCapture")):
        return "covered_by_equivalent_static_proof_helper", "captured cell FieldKind proof"

    if op.startswith(("LoadLocal", "StoreLocal", "LoadModuleBinding", "StoreModuleBinding")):
        return "covered_by_equivalent_static_proof_helper", "StorageHint/FieldKind or width operand proof"

    if op in {"IntToNumber", "NumberToInt"} or op == "StringConcatTyped" or SCALAR_TYPED_RE.match(op) or op in {"AddTyped", "SubTyped", "MulTyped", "DivTyped", "ModTyped", "CmpTyped", "EqTypedObject"}:
        return "covered_by_equivalent_static_proof_helper", "NumericType/EqOperandType/strict default proof"

    return "unclassified", "no W91A classification rule"


def main() -> int:
    hits = scan()
    classified = [(hit, *classify(hit)) for hit in hits]
    counts = Counter(category for _, category, _ in classified)
    unclassified = [(hit, reason) for hit, category, reason in classified if category == "unclassified"]

    print("typed-opcode proof coverage source inventory:")
    for category in sorted(EXPECTED_COUNTS):
        print(f"  {category}: {counts.get(category, 0)}")

    if unclassified:
        print("\nFAILED: unclassified typed-opcode/static-kind compiler paths found.", file=sys.stderr)
        for hit, reason in unclassified:
            print(f"{hit.relpath}:{hit.line_no}: {hit.opcode}: {reason}: {hit.text}", file=sys.stderr)
        return 1

    expected = Counter(EXPECTED_COUNTS)
    observed = Counter({key: counts.get(key, 0) for key in EXPECTED_COUNTS})
    if observed != expected:
        print("\nFAILED: typed-opcode proof coverage counts changed.", file=sys.stderr)
        print("expected:", dict(expected), file=sys.stderr)
        print("observed:", dict(observed), file=sys.stderr)
        print("Update the audit doc and this checker when the new path is classified.", file=sys.stderr)
        return 1

    print("typed-opcode proof coverage guard clean: audited baseline unchanged.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
