#!/usr/bin/env bash
# Guard against HeapKind wildcard dispatch surfaces growing silently.
#
# This is intentionally non-cargo: it is safe to run in merge-verifier fast
# lanes and in parallel-agent worktrees. The current residual list is an
# audited baseline; any new hit must either be made exhaustive or consciously
# added here with a Wave-2 note.

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

tmp_found="$(mktemp)"
tmp_known="$(mktemp)"
tmp_new="$(mktemp)"
trap 'rm -f "$tmp_found" "$tmp_known" "$tmp_new"' EXIT

target_roots=(
  crates/shape-value/src
  crates/shape-vm/src
  crates/shape-jit/src
  crates/shape-runtime/src
  crates/shape-wire/src
)

scan_ptr_wildcard_arms() {
  while IFS=: read -r file line rest; do
    [[ -n "${file:-}" && -n "${line:-}" ]] || continue
    # Preserve any further ':' in the source line.
    local text
    text="${rest}"
    text="${text#"${text%%[![:space:]]*}"}"
    text="${text%"${text##*[![:space:]]}"}"
    printf 'ptr-wildcard-arm\t%s:%s\t%s\n' "$file" "$line" "$text"
  done < <(
    rg --no-heading -n -P '(^|[|[:space:]])NativeKind::Ptr\(_\)[[:space:]]*=>' \
      "${target_roots[@]}" 2>/dev/null || true
  )
}

scan_jit_legacy_heap_kind_catchalls() {
  awk '
    function delta(s,    a,b,t) {
      t=s; a=gsub(/\{/, "{", t)
      t=s; b=gsub(/\}/, "}", t)
      return a-b
    }
    function trim(s) {
      sub(/^[[:space:]]+/, "", s)
      sub(/[[:space:]]+$/, "", s)
      return s
    }
    function indent(s) {
      match(s, /^[ \t]*/)
      return RLENGTH
    }
    function reset() {
      in_match=0; depth=0; has_catch=0
      start_line=0; start_text=""; catch_text=""; arm_indent=0
    }
    BEGIN { reset() }
    {
      line=$0
      if (!in_match && line ~ /match[[:space:]]+heap_kind\([^)]*\)[[:space:]]*\{/) {
        in_match=1
        depth=0
        start_line=FNR
        start_text=trim(line)
        arm_indent=indent(line)+4
      }
      if (in_match) {
        if (indent(line) <= arm_indent && line ~ /^[[:space:]]*(_|[a-z][A-Za-z0-9_]*)[[:space:]]*=>/) {
          has_catch=1
          catch_text=trim(line)
        }
        depth += delta(line)
        if (depth <= 0) {
          if (has_catch) {
            printf "jit-legacy-heap-kind-catchall\t%s:%d\t%s | %s\n", FILENAME, start_line, start_text, catch_text
          }
          reset()
        }
      }
    }
  ' $(rg --files crates/shape-jit/src -g '*.rs')
}

scan_heapkind_match_catchalls() {
  awk '
    function delta(s,    a,b,t) {
      t=s; a=gsub(/\{/, "{", t)
      t=s; b=gsub(/\}/, "}", t)
      return a-b
    }
    function trim(s) {
      sub(/^[[:space:]]+/, "", s)
      sub(/[[:space:]]+$/, "", s)
      return s
    }
    function indent(s) {
      match(s, /^[ \t]*/)
      return RLENGTH
    }
    function reset() {
      in_match=0; depth=0; has_heap_arm=0; has_catch=0
      start_line=0; start_text=""; catch_text=""; arm_indent=0
    }
    BEGIN { reset() }
    {
      line=$0
      if (!in_match && line ~ /match[[:space:]]+(hk|heap_kind|expected_kind)[[:space:]]*\{/) {
        in_match=1
        depth=0
        start_line=FNR
        start_text=trim(line)
        arm_indent=indent(line)+4
      }
      if (in_match) {
        if (indent(line) <= arm_indent && line ~ /^[[:space:]]*HeapKind::[A-Za-z0-9_]+/) {
          has_heap_arm=1
        }
        if (indent(line) <= arm_indent && line ~ /^[[:space:]]*(_|[a-z][A-Za-z0-9_]*)[[:space:]]*=>/) {
          has_catch=1
          catch_text=trim(line)
        }
        depth += delta(line)
        if (depth <= 0) {
          if (has_heap_arm && has_catch) {
            printf "heapkind-match-catchall\t%s:%d\t%s | %s\n", FILENAME, start_line, start_text, catch_text
          }
          reset()
        }
      }
    }
  ' $(rg --files "${target_roots[@]}" -g '*.rs')
}

{
  scan_ptr_wildcard_arms
  scan_jit_legacy_heap_kind_catchalls
  scan_heapkind_match_catchalls
} | LC_ALL=C sort > "$tmp_found"

cat > "$tmp_known" <<'EOF'
heapkind-match-catchall	crates/shape-runtime/src/snapshot.rs:1198	match expected_kind { | other => Err(format!(
heapkind-match-catchall	crates/shape-runtime/src/wire_conversion.rs:1035	let node = match hk { | _ => None,
heapkind-match-catchall	crates/shape-vm/src/compiler/comptime.rs:1379	NativeKind::Ptr(hk) => match hk { | _ => {
heapkind-match-catchall	crates/shape-vm/src/executor/control_flow/foreign_marshal.rs:194	match heap_kind { | other => Err(VMError::NotImplemented(format!(
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/conversion.rs:1097	let num = match heap_kind(value_bits) { | _ => f64::NAN,
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/conversion.rs:187	match heap_kind(value_bits) { | _ => false,
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/conversion.rs:234	match heap_kind(value_bits) { | _ => "[object]".to_string(),
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/conversion.rs:36	match heap_kind(value_bits) { | _ => "unknown",
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/conversion.rs:68	match heap_kind(value_bits) { | _ => "[unknown]".to_string(),
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/iterator.rs:101	match heap_kind(iter_bits) { | _ => TAG_NULL,
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/iterator.rs:38	let done = match heap_kind(iter_bits) { | _ => true, // Unknown type = done
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/object/format.rs:108	match heap_kind(bits) { | _ => "[unknown]".to_string(),
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/object/object_ops.rs:67	match heap_kind(obj_bits) { | _ => obj_bits,
jit-legacy-heap-kind-catchall	crates/shape-jit/src/ffi/object/property_access.rs:304	let len = match heap_kind(value_bits) { | _ => {
ptr-wildcard-arm	crates/shape-jit/src/ffi/call_method/mod.rs:701	NativeKind::Ptr(_) => false,
ptr-wildcard-arm	crates/shape-jit/src/ffi/call_method/mod.rs:977	NativeKind::Ptr(_) => TAG_NULL,
ptr-wildcard-arm	crates/shape-jit/src/mir_compiler/v2_array.rs:82	NativeKind::Ptr(_) => (types::I64, 8),
ptr-wildcard-arm	crates/shape-jit/src/mir_compiler/v2_call_abi.rs:185	NativeKind::String | NativeKind::Ptr(_) => types::I64,
ptr-wildcard-arm	crates/shape-jit/src/mir_compiler/v2_field.rs:100	NativeKind::String | NativeKind::Ptr(_) => types::I64,
ptr-wildcard-arm	crates/shape-jit/src/mir_compiler/v2_field.rs:140	NativeKind::String | NativeKind::Ptr(_) => 8,
ptr-wildcard-arm	crates/shape-runtime/src/type_schema/mod.rs:266	NativeKind::String | NativeKind::Ptr(_) => true,
ptr-wildcard-arm	crates/shape-vm/src/executor/builtins/array_ops.rs:153	NativeKind::Ptr(_) => {
ptr-wildcard-arm	crates/shape-vm/src/executor/comparison/mod.rs:425	NativeKind::Ptr(_) => a_bits == b_bits,
ptr-wildcard-arm	crates/shape-vm/src/executor/comparison/mod.rs:604	NativeKind::String | NativeKind::Ptr(_) => bits == 0,
ptr-wildcard-arm	crates/shape-vm/src/executor/control_flow/mod.rs:67	NativeKind::String | NativeKind::Ptr(_) => bits != 0,
ptr-wildcard-arm	crates/shape-vm/src/executor/exceptions/mod.rs:1188	NativeKind::String | NativeKind::Ptr(_) => bits == 0,
ptr-wildcard-arm	crates/shape-vm/src/executor/logical/mod.rs:209	NativeKind::String | NativeKind::Ptr(_) => bits == 0,
ptr-wildcard-arm	crates/shape-vm/src/executor/logical/mod.rs:79	NativeKind::String | NativeKind::Ptr(_) => bits != 0,
ptr-wildcard-arm	crates/shape-vm/src/executor/loops/mod.rs:345	NativeKind::Ptr(_) => Err(VMError::NotImplemented(format!(
ptr-wildcard-arm	crates/shape-vm/src/executor/objects/array_aggregation.rs:106	NativeKind::String | NativeKind::Ptr(_) => bits != 0,
ptr-wildcard-arm	crates/shape-vm/src/executor/objects/array_query.rs:129	NativeKind::String | NativeKind::Ptr(_) => bits != 0,
ptr-wildcard-arm	crates/shape-vm/src/executor/objects/array_transform.rs:845	| NativeKind::Ptr(_) => Err(VMError::RuntimeError(format!(
ptr-wildcard-arm	crates/shape-vm/src/executor/objects/deque_methods.rs:121	NativeKind::Ptr(_) => {
ptr-wildcard-arm	crates/shape-vm/src/executor/objects/hashmap_methods.rs:2131	NativeKind::Ptr(_) => {
ptr-wildcard-arm	crates/shape-vm/src/executor/objects/mod.rs:750	NativeKind::Ptr(_) => None,
ptr-wildcard-arm	crates/shape-vm/src/executor/objects/property_access.rs:281	NativeKind::Ptr(_) => Err(VMError::NotImplemented(format!(
ptr-wildcard-arm	crates/shape-vm/src/executor/objects/property_access.rs:795	NativeKind::Ptr(_) => Err(VMError::TypeError {
ptr-wildcard-arm	crates/shape-vm/src/executor/result_option_carrier.rs:224	NativeKind::Ptr(_) => true,
ptr-wildcard-arm	crates/shape-vm/src/executor/vm_impl/modules.rs:86	NativeKind::Ptr(_) => true,
EOF

LC_ALL=C sort -o "$tmp_known" "$tmp_known"
LC_ALL=C comm -23 "$tmp_found" "$tmp_known" > "$tmp_new"

if [[ -s "$tmp_new" ]]; then
  echo "FAILED: new HeapKind wildcard dispatch patterns found."
  echo
  cat "$tmp_new"
  echo
  echo "Make the match exhaustive, or update the audited residual catalog"
  echo "in docs/cluster-audits/w83b-heapkind-wildcards.md and this checker."
  exit 1
fi

echo "HeapKind wildcard guard clean: audited baseline unchanged ($(wc -l < "$tmp_found") residual patterns)."
