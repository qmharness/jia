#!/usr/bin/env bash
# eval-nightly.sh — run the full coding eval and append a JSONL record to
# eval-history.jsonl (gitignored) at the repo root.
#
# Usage:
#   scripts/eval-nightly.sh          # run full eval, append one JSONL record
#   scripts/eval-nightly.sh --save   # same, plus print the last 10 history lines
#
# Env passthrough: JIA_EVAL_PROVIDER, JIA_EVAL_ONLY, JIA_EVAL_API_BASE/MODEL/KEY
# are honored by the harness (tests/coding_eval.rs).
#
# Scheduling (NOT registered by this script — examples only):
#   cron (run at 03:00 daily):
#     0 3 * * *  cd /path/to/jia && scripts/eval-nightly.sh >> /tmp/jia-eval-nightly.log 2>&1
#   launchd: create ~/Library/LaunchAgents/com.jia.eval-nightly.plist with
#     ProgramArguments = [/path/to/jia/scripts/eval-nightly.sh] and a
#     StartCalendarInterval dict (Hour=3, Minute=0), then:
#     launchctl load ~/Library/LaunchAgents/com.jia.eval-nightly.plist
set -euo pipefail
cd "$(dirname "$0")/.."

HISTORY="eval-history.jsonl"
START=$(date +%s)
LOG=$(mktemp -t jia-eval-nightly)
trap 'rm -f "$LOG"' EXIT

set +e
JIA_EVAL=1 cargo test --test coding_eval -- --nocapture >"$LOG" 2>&1
TEST_EXIT=$?
set -e

DURATION=$(( $(date +%s) - START ))

# Summary line looks like: "Eval: 21/22 passed"
TOTAL_LINE=$(grep -E '^Eval: [0-9]+/[0-9]+ passed' "$LOG" | tail -1 || true)
PASSED=0
TOTAL=0
if [[ -n "$TOTAL_LINE" ]]; then
  PASSED=$(echo "$TOTAL_LINE" | sed -E 's/^Eval: ([0-9]+)\/([0-9]+) passed/\1/')
  TOTAL=$(echo "$TOTAL_LINE" | sed -E 's/^Eval: ([0-9]+)\/([0-9]+) passed/\2/')
fi

# Category rows look like: "  baseline   5/5" (after the "By category:" header)
CATEGORIES=$(awk '
  /^By category:/ { f=1; next }
  f && NF >= 2 {
    split($2, a, "/")
    printf "%s\"%s\":{\"passed\":%s,\"total\":%s}", (c++ ? "," : ""), $1, a[1], a[2]
  }
' "$LOG")

GIT_SHA=$(git rev-parse --short HEAD 2>/dev/null || echo "unknown")
if [[ -n $(git status --porcelain 2>/dev/null) ]]; then
  GIT_DIRTY=true
else
  GIT_DIRTY=false
fi
PROVIDER="${JIA_EVAL_PROVIDER:-default}"

printf '{"date":"%s","git_sha":"%s","git_dirty":%s,"provider":"%s","passed":%s,"total":%s,"exit_code":%s,"duration_secs":%s,"categories":{%s}}\n' \
  "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$GIT_SHA" "$GIT_DIRTY" "$PROVIDER" \
  "$PASSED" "$TOTAL" "$TEST_EXIT" "$DURATION" "$CATEGORIES" >> "$HISTORY"

echo "Eval: ${PASSED}/${TOTAL} passed in ${DURATION}s (test exit ${TEST_EXIT}) — appended to ${HISTORY}"

if [[ "${1:-}" == "--save" ]]; then
  echo "--- trend (last 10 records) ---"
  tail -10 "$HISTORY"
fi

exit "$TEST_EXIT"
