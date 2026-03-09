#!/usr/bin/env bash
set -euo pipefail

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
}

require_cmd k6
require_cmd jq

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"
source "$SCRIPT_DIR/lib/artifact_utils.sh"

TEST_RUN_ID="${TEST_RUN_ID:-day9_$(date +%Y%m%d_%H%M%S)}"
OUT_DIR="${OUT_DIR:-artifacts/day9/${TEST_RUN_ID}}"
SUMMARY_JSON_PATH="${SUMMARY_JSON_PATH:-${OUT_DIR}/k6-summary.json}"
CUSTOMER_ID="${CUSTOMER_ID:-cust_load_test_${TEST_RUN_ID}}"
ENDPOINT_PROFILE="${ENDPOINT_PROFILE:-healthy}"
TARGET_EVENTS="${TARGET_EVENTS:-1000}"
ITERATION_VUS="${ITERATION_VUS:-50}"
MODE="${MODE:-seed}"

mkdir -p "$OUT_DIR"

META_ENV_PATH="${OUT_DIR}/meta.env"
ENV_SNAPSHOT_PATH="${OUT_DIR}/env.snapshot"

cat > "$META_ENV_PATH" <<EOF
TEST_RUN_ID=${TEST_RUN_ID}
CUSTOMER_ID=${CUSTOMER_ID}
TARGET_EVENTS=${TARGET_EVENTS}
ITERATION_VUS=${ITERATION_VUS}
ENDPOINT_PROFILE=${ENDPOINT_PROFILE}
OUT_DIR=${OUT_DIR}
SUMMARY_JSON_PATH=${SUMMARY_JSON_PATH}
EOF

write_sanitized_env_snapshot "$ENV_SNAPSHOT_PATH"

echo "[day9] Writing artifacts to ${OUT_DIR}"
echo "[day9] Sanitized env snapshot: ${ENV_SNAPSHOT_PATH}"
echo "[day9] Summary JSON: ${SUMMARY_JSON_PATH}"

(
  cd "$REPO_ROOT"
  TEST_RUN_ID="$TEST_RUN_ID" \
  CUSTOMER_ID="$CUSTOMER_ID" \
  ENDPOINT_PROFILE="$ENDPOINT_PROFILE" \
  TARGET_EVENTS="$TARGET_EVENTS" \
  ITERATION_VUS="$ITERATION_VUS" \
  MODE="$MODE" \
  SUMMARY_JSON_PATH="$SUMMARY_JSON_PATH" \
  k6 run tests/load_test.js
)

if [[ -f "$SUMMARY_JSON_PATH" ]]; then
  echo "[day9] Summary excerpt"
  jq '{meta, metrics}' "$SUMMARY_JSON_PATH"
else
  echo "ERROR: expected summary file was not created: $SUMMARY_JSON_PATH" >&2
  exit 1
fi
