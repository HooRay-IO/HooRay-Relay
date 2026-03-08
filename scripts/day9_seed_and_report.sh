#!/usr/bin/env bash
set -euo pipefail

# Day 9 helper: seed events via k6 and emit quick evidence artifacts.
#
# Usage:
#   ./scripts/day9_seed_and_report.sh
#   TARGET_EVENTS=5000 ENDPOINT_PROFILE=failure ./scripts/day9_seed_and_report.sh
#
# Required from direnv/.envrc: API_URL, EVENTS_TABLE, AWS_REGION, AWS_PROFILE

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v direnv >/dev/null 2>&1; then
  echo "ERROR: direnv not found" >&2
  exit 1
fi

if ! command -v k6 >/dev/null 2>&1; then
  echo "ERROR: k6 not found" >&2
  exit 1
fi

TARGET_EVENTS="${TARGET_EVENTS:-1000}"
ITERATION_VUS="${ITERATION_VUS:-50}"
ENDPOINT_PROFILE="${ENDPOINT_PROFILE:-healthy}"
TEST_RUN_ID="${TEST_RUN_ID:-day9_$(date +%Y%m%d_%H%M%S)}"
CUSTOMER_ID="${CUSTOMER_ID:-cust_load_test_${TEST_RUN_ID}}"
OUT_DIR="${OUT_DIR:-artifacts/day9/${TEST_RUN_ID}}"
SUMMARY_JSON_PATH="${SUMMARY_JSON_PATH:-${OUT_DIR}/k6-summary.json}"
K6_LOG_PATH="${K6_LOG_PATH:-${OUT_DIR}/k6.log}"
COUNTS_JSON_PATH="${COUNTS_JSON_PATH:-${OUT_DIR}/dynamodb-counts.json}"
META_PATH="${META_PATH:-${OUT_DIR}/meta.env}"

mkdir -p "$OUT_DIR"

# Persist a redacted environment snapshot for reproducibility (avoid secrets/tokens).
cat > "${OUT_DIR}/env.snapshot" <<ENV_SNAPSHOT
TEST_RUN_ID=$TEST_RUN_ID
CUSTOMER_ID=$CUSTOMER_ID
TARGET_EVENTS=$TARGET_EVENTS
ITERATION_VUS=$ITERATION_VUS
ENDPOINT_PROFILE=$ENDPOINT_PROFILE
OUT_DIR=$OUT_DIR
SUMMARY_JSON_PATH=$SUMMARY_JSON_PATH
AWS_PROFILE=
AWS_REGION=
STACK_NAME=
API_URL=
ENV_SNAPSHOT

# Fill non-sensitive runtime values from direnv context.
direnv exec . bash -lc 'printf "AWS_PROFILE=%s\nAWS_REGION=%s\nSTACK_NAME=%s\nAPI_URL=%s\n" "${AWS_PROFILE:-}" "${AWS_REGION:-}" "${STACK_NAME:-}" "${API_URL:-}"' >> "${OUT_DIR}/env.snapshot" || true

cat > "$META_PATH" <<META
TEST_RUN_ID=$TEST_RUN_ID
CUSTOMER_ID=$CUSTOMER_ID
TARGET_EVENTS=$TARGET_EVENTS
ITERATION_VUS=$ITERATION_VUS
ENDPOINT_PROFILE=$ENDPOINT_PROFILE
OUT_DIR=$OUT_DIR
SUMMARY_JSON_PATH=$SUMMARY_JSON_PATH
META

echo "[1/3] Running k6 seed"
direnv exec . bash -lc '
set -euo pipefail
k6 run \
  --env API_URL="$API_URL" \
  --env API_KEY="${API_KEY:-}" \
  --env CUSTOMER_ID="'"$CUSTOMER_ID"'" \
  --env TEST_RUN_ID="'"$TEST_RUN_ID"'" \
  --env ENDPOINT_PROFILE="'"$ENDPOINT_PROFILE"'" \
  --env TARGET_EVENTS="'"$TARGET_EVENTS"'" \
  --env ITERATION_VUS="'"$ITERATION_VUS"'" \
  --env SUMMARY_JSON_PATH="'"$SUMMARY_JSON_PATH"'" \
  tests/load_test.js
' | tee "$K6_LOG_PATH"

echo "[2/3] Counting seeded events in DynamoDB"
direnv exec . bash -lc '
set -euo pipefail
aws dynamodb scan \
  --table-name "$EVENTS_TABLE" \
  --select COUNT \
  --filter-expression "sk = :v0 AND customer_id = :c" \
  --expression-attribute-values "{\":v0\":{\"S\":\"v0\"},\":c\":{\"S\":\"'"$CUSTOMER_ID"'\"}}" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --output json
' > "$COUNTS_JSON_PATH"

echo "[3/3] Done"
echo "Artifacts:"
echo "  - $SUMMARY_JSON_PATH"
echo "  - $K6_LOG_PATH"
echo "  - $COUNTS_JSON_PATH"
echo "  - $META_PATH"

direnv exec . bash -lc '
count=$(aws dynamodb scan \
  --table-name "$EVENTS_TABLE" \
  --select COUNT \
  --filter-expression "sk = :v0 AND customer_id = :c" \
  --expression-attribute-values "{\":v0\":{\"S\":\"v0\"},\":c\":{\"S\":\"'"$CUSTOMER_ID"'\"}}" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --query Count --output text)
printf "Seeded events for %s: %s\n" "'"$CUSTOMER_ID"'" "$count"
'
