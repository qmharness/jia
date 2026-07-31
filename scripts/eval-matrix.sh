#!/usr/bin/env bash
# eval-matrix.sh — run the full coding eval once per provider and print a
# provider × score comparison table.
#
# Usage:
#   scripts/eval-matrix.sh                      # defaults: deepseek LMStudio
#   scripts/eval-matrix.sh deepseek             # explicit provider list
#   scripts/eval-matrix.sh deepseek LMStudio foo
#
# Provider names are resolved from config.toml by the harness via
# JIA_EVAL_PROVIDER (see load_eval_profile in tests/coding_eval.rs); unknown
# names make the harness list the available providers and skip. Remote
# providers need valid credentials in config.toml; LMStudio needs a local
# server on localhost:1234.
set -euo pipefail
cd "$(dirname "$0")/.."

PROVIDERS=("$@")
if [[ ${#PROVIDERS[@]} -eq 0 ]]; then
  PROVIDERS=(deepseek LMStudio)
fi

ROWS=()
for p in "${PROVIDERS[@]}"; do
  echo "=== provider: $p ===" >&2
  LOG=$(mktemp -t jia-eval-matrix)
  START=$(date +%s)
  set +e
  JIA_EVAL=1 JIA_EVAL_PROVIDER="$p" cargo test --test coding_eval -- --nocapture >"$LOG" 2>&1
  EXIT=$?
  set -e
  DURATION=$(( $(date +%s) - START ))
  LINE=$(grep -E '^Eval: [0-9]+/[0-9]+ passed' "$LOG" | tail -1 || true)
  if [[ -n "$LINE" ]]; then
    SCORE=$(echo "$LINE" | sed -E 's/^Eval: ([0-9]+\/[0-9]+) passed/\1/')
  else
    SCORE="n/a"
  fi
  ROWS+=("$p|$SCORE|${DURATION}s|$EXIT")
  rm -f "$LOG"
done

echo
printf '%-15s %-10s %-10s %-6s\n' PROVIDER SCORE DURATION EXIT
for r in "${ROWS[@]}"; do
  IFS='|' read -r p s d x <<< "$r"
  printf '%-15s %-10s %-10s %-6s\n' "$p" "$s" "$d" "$x"
done
