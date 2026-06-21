#!/usr/bin/env bash
# Harness: run a shape file under both vm and jit, print ec + output.
BIN=/home/dev/dev/shape-lang/shape-strict-flip-collection-dispatch/target/release/shape
FILE="$1"
for MODE in vm jit; do
  echo "===== MODE=$MODE FILE=$FILE ====="
  out=$(timeout 30 $BIN run --mode $MODE "$FILE" 2>&1); ec=$?
  echo "EC=$ec"
  echo "$out"
  echo "----- end $MODE -----"
done
