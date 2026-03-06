#!/usr/bin/env bash
set -euo pipefail

# DLQ operations utility:
# - Poll + decode DLQ messages
# - Summarize root-cause buckets by error class (from latest ATTEMPT#n in events table)
# - Replay selected DLQ message IDs to main queue (dry-run by default)
#
# Defaults:
# - MODE=inspect (inspect|replay)
# - AWS_REGION=us-west-2
# - AWS_PROFILE=hooray-dev
# - STACK_NAME=hooray-dev
# - MAX_MESSAGES=10
# - WAIT_TIME_SECS=2
# - VISIBILITY_TIMEOUT_SECS=30
# - DRY_RUN=true
# - REPLAY_MESSAGE_IDS="" (comma-separated SQS MessageId values)
# - DELETE_AFTER_REPLAY=false
#
# Optional explicit overrides:
# - DLQ_URL
# - MAIN_QUEUE_URL
# - EVENTS_TABLE

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
}

require_cmd aws
require_cmd jq

MODE="${MODE:-inspect}"
AWS_REGION="${AWS_REGION:-us-west-2}"
AWS_PROFILE="${AWS_PROFILE:-hooray-dev}"
STACK_NAME="${STACK_NAME:-hooray-dev}"
MAX_MESSAGES="${MAX_MESSAGES:-10}"
WAIT_TIME_SECS="${WAIT_TIME_SECS:-2}"
VISIBILITY_TIMEOUT_SECS="${VISIBILITY_TIMEOUT_SECS:-30}"
DRY_RUN="${DRY_RUN:-true}"
REPLAY_MESSAGE_IDS="${REPLAY_MESSAGE_IDS:-}"
DELETE_AFTER_REPLAY="${DELETE_AFTER_REPLAY:-false}"

usage() {
  cat <<USAGE
Usage:
  MODE=inspect ./scripts/dlq_ops.sh
  MODE=replay REPLAY_MESSAGE_IDS="<msg-id-1>,<msg-id-2>" ./scripts/dlq_ops.sh

Important env vars:
  MODE=inspect|replay
  DRY_RUN=true|false               (default true)
  REPLAY_MESSAGE_IDS=...           (required for MODE=replay)
  DELETE_AFTER_REPLAY=true|false   (default false)
  DLQ_URL=...                      (optional; auto-resolved from stack)
  MAIN_QUEUE_URL=...               (optional; auto-resolved from stack)
  EVENTS_TABLE=...                 (optional; auto-resolved from stack)
USAGE
}

if [[ "$MODE" != "inspect" && "$MODE" != "replay" ]]; then
  echo "ERROR: MODE must be inspect or replay, got: $MODE" >&2
  usage
  exit 1
fi

resolve_from_stack_if_missing() {
  if [[ -n "${DLQ_URL:-}" && -n "${MAIN_QUEUE_URL:-}" && -n "${EVENTS_TABLE:-}" ]]; then
    return
  fi

  local outputs_json
  outputs_json="$(aws cloudformation describe-stacks \
    --stack-name "$STACK_NAME" \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --query 'Stacks[0].Outputs' \
    --output json)"

  if [[ -z "${DLQ_URL:-}" ]]; then
    DLQ_URL="$(echo "$outputs_json" | jq -r '.[] | select(.OutputKey=="DLQUrl") | .OutputValue')"
  fi
  if [[ -z "${MAIN_QUEUE_URL:-}" ]]; then
    MAIN_QUEUE_URL="$(echo "$outputs_json" | jq -r '.[] | select(.OutputKey=="QueueUrl") | .OutputValue')"
  fi
  if [[ -z "${EVENTS_TABLE:-}" ]]; then
    EVENTS_TABLE="$(echo "$outputs_json" | jq -r '.[] | select(.OutputKey=="EventsTableName") | .OutputValue')"
  fi
}

resolve_from_stack_if_missing

if [[ -z "${DLQ_URL:-}" || "$DLQ_URL" == "null" ]]; then
  echo "ERROR: DLQ_URL is missing (set DLQ_URL or provide a stack with DLQUrl output)" >&2
  exit 1
fi
if [[ -z "${MAIN_QUEUE_URL:-}" || "$MAIN_QUEUE_URL" == "null" ]]; then
  echo "ERROR: MAIN_QUEUE_URL is missing (set MAIN_QUEUE_URL or provide a stack with QueueUrl output)" >&2
  exit 1
fi
if [[ -z "${EVENTS_TABLE:-}" || "$EVENTS_TABLE" == "null" ]]; then
  echo "ERROR: EVENTS_TABLE is missing (set EVENTS_TABLE or provide a stack with EventsTableName output)" >&2
  exit 1
fi

if [[ "$MODE" == "replay" && -z "$REPLAY_MESSAGE_IDS" ]]; then
  echo "ERROR: REPLAY_MESSAGE_IDS is required when MODE=replay" >&2
  exit 1
fi

TMP_RAW="$(mktemp "${TMPDIR:-/tmp}/dlq_ops.raw.XXXXXX.json")"
TMP_ENRICHED="$(mktemp "${TMPDIR:-/tmp}/dlq_ops.enriched.XXXXXX.ndjson")"
cleanup() {
  rm -f "$TMP_RAW" "$TMP_ENRICHED"
}
trap cleanup EXIT INT TERM

aws sqs receive-message \
  --queue-url "$DLQ_URL" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --max-number-of-messages "$MAX_MESSAGES" \
  --wait-time-seconds "$WAIT_TIME_SECS" \
  --visibility-timeout "$VISIBILITY_TIMEOUT_SECS" \
  --attribute-names All \
  --message-attribute-names All \
  --output json > "$TMP_RAW"

message_count="$(jq '.Messages | length // 0' "$TMP_RAW")"
if [[ "$message_count" -eq 0 ]]; then
  echo "No messages received from DLQ: $DLQ_URL"
  exit 0
fi

echo "Received $message_count DLQ messages"

jq -c '.Messages[]' "$TMP_RAW" | while IFS= read -r msg; do
  message_id="$(echo "$msg" | jq -r '.MessageId')"
  receipt_handle="$(echo "$msg" | jq -r '.ReceiptHandle')"
  body_raw="$(echo "$msg" | jq -r '.Body // ""')"

  event_id="$(echo "$msg" | jq -r '
    .Body as $b
    | ($b | fromjson? // $b) as $decoded
    | if ($decoded | type) == "object" then
        ($decoded.event_id // empty)
      elif ($decoded | type) == "string" then
        (($decoded | fromjson? // {}) | .event_id // empty)
      else
        empty
      end
  ' 2>/dev/null || true)"
  if [[ -z "$event_id" ]]; then
    event_id="unknown"
  fi

  receive_count="$(echo "$msg" | jq -r '.Attributes.ApproximateReceiveCount // "unknown"')"
  first_received_at_ms="$(echo "$msg" | jq -r '.Attributes.ApproximateFirstReceiveTimestamp // ""')"

  error_class="unknown"
  last_error_message=""

  if [[ "$event_id" != "unknown" ]]; then
    attempt_items="$(aws dynamodb query \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --table-name "$EVENTS_TABLE" \
      --key-condition-expression 'pk = :pk AND begins_with(sk, :prefix)' \
      --expression-attribute-values "{\":pk\":{\"S\":\"EVENT#${event_id}\"},\":prefix\":{\"S\":\"ATTEMPT#\"}}" \
      --projection-expression 'attempt_number,error_message,sk' \
      --output json 2>/dev/null || true)"

    if [[ -n "$attempt_items" ]]; then
      latest_error="$(echo "$attempt_items" | jq -r '
        .Items
        | map({
            attempt_number: ((.attempt_number.N // "0") | tonumber),
            error_message: (.error_message.S // "")
          })
        | sort_by(.attempt_number)
        | last
        | .error_message // ""
      ' 2>/dev/null || true)"

      if [[ -n "$latest_error" ]]; then
        last_error_message="$latest_error"
        parsed_class="$(echo "$latest_error" | sed -n 's/^\[\([^]]\+\)\].*/\1/p')"
        if [[ -n "$parsed_class" ]]; then
          error_class="$parsed_class"
        else
          lowered_error="$(echo "$latest_error" | tr '[:upper:]' '[:lower:]')"
          if [[ "$lowered_error" == *"http 429"* || "$lowered_error" == *"rate limit"* ]]; then
            error_class="http_rate_limited"
          elif [[ "$lowered_error" == *"http 5"* ]]; then
            error_class="http_server_error"
          elif [[ "$lowered_error" == *"http 4"* ]]; then
            error_class="http_client_error"
          elif [[ "$lowered_error" == *"timeout"* ]]; then
            error_class="network_timeout"
          elif [[ "$lowered_error" == *"connect"* || "$lowered_error" == *"connection reset"* || "$lowered_error" == *"connection refused"* ]]; then
            error_class="network_connect"
          elif [[ "$lowered_error" == *"dns"* || "$lowered_error" == *"resolve"* || "$lowered_error" == *"name or service not known"* ]]; then
            error_class="network_request"
          else
            error_class="unclassified"
          fi
        fi
      else
        error_class="no_attempt_error"
      fi
    fi
  fi

  jq -cn \
    --arg message_id "$message_id" \
    --arg receipt_handle "$receipt_handle" \
    --arg event_id "$event_id" \
    --arg error_class "$error_class" \
    --arg error_message "$last_error_message" \
    --arg receive_count "$receive_count" \
    --arg first_received_at_ms "$first_received_at_ms" \
    --arg body "$body_raw" \
    '{
      message_id: $message_id,
      event_id: $event_id,
      error_class: $error_class,
      last_error_message: $error_message,
      receive_count: $receive_count,
      first_received_at_ms: $first_received_at_ms,
      body: $body,
      receipt_handle: $receipt_handle
    }' >> "$TMP_ENRICHED"

done

echo
printf '%-36s %-28s %-24s %-8s\n' "MESSAGE_ID" "EVENT_ID" "ERROR_CLASS" "RECV"
printf '%-36s %-28s %-24s %-8s\n' "----------" "--------" "-----------" "----"
while IFS= read -r row; do
  message_id="$(echo "$row" | jq -r '.message_id')"
  event_id="$(echo "$row" | jq -r '.event_id')"
  error_class="$(echo "$row" | jq -r '.error_class')"
  receive_count="$(echo "$row" | jq -r '.receive_count')"
  printf '%-36s %-28s %-24s %-8s\n' "$message_id" "$event_id" "$error_class" "$receive_count"
done < "$TMP_ENRICHED"

echo
echo "Root-cause bucket summary"
jq -s '
  group_by(.error_class)
  | map({error_class: .[0].error_class, count: length})
  | sort_by(-.count)
' "$TMP_ENRICHED"

if [[ "$MODE" == "inspect" ]]; then
  echo
  echo "Inspect mode only. No replay performed."
  echo "Tip: MODE=replay REPLAY_MESSAGE_IDS=\"id1,id2\" DRY_RUN=true ./scripts/dlq_ops.sh"
  exit 0
fi

selected_json="$(jq -n --arg ids "$REPLAY_MESSAGE_IDS" '$ids | split(",") | map(gsub("^\\s+|\\s+$"; "")) | map(select(length > 0))')"
selected_count="$(echo "$selected_json" | jq 'length')"
if [[ "$selected_count" -eq 0 ]]; then
  echo "ERROR: no valid IDs parsed from REPLAY_MESSAGE_IDS" >&2
  exit 1
fi

echo
echo "Replay candidates"
replay_rows="$(jq -s --argjson ids "$selected_json" '
  map(select(.message_id as $id | $ids | index($id)))
  | sort_by(.message_id)
  | group_by(.message_id)
  | map(.[0])
' "$TMP_ENRICHED")"

found_count="$(echo "$replay_rows" | jq 'length')"
if [[ "$found_count" -eq 0 ]]; then
  echo "ERROR: none of REPLAY_MESSAGE_IDS are present in the current receive batch" >&2
  echo "Hint: increase MAX_MESSAGES or rerun to fetch more DLQ messages" >&2
  exit 1
fi

echo "$replay_rows" | jq

if [[ "$DRY_RUN" == "true" ]]; then
  echo
  echo "DRY_RUN=true: no messages sent and nothing deleted."
  exit 0
fi

echo
for idx in $(seq 0 $((found_count - 1))); do
  row="$(echo "$replay_rows" | jq ".[$idx]")"
  message_id="$(echo "$row" | jq -r '.message_id')"
  event_id="$(echo "$row" | jq -r '.event_id')"
  receipt_handle="$(echo "$row" | jq -r '.receipt_handle')"
  body="$(echo "$row" | jq -r '.body')"

  # Mark replay path for worker metrics/diagnostics.
  replay_attributes='{"dlq_replay":{"DataType":"String","StringValue":"true"}}'

  aws sqs send-message \
    --queue-url "$MAIN_QUEUE_URL" \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --message-body "$body" \
    --message-attributes "$replay_attributes" \
    --output json >/dev/null

  echo "Replayed message_id=$message_id event_id=$event_id to main queue"

  if [[ "$DELETE_AFTER_REPLAY" == "true" ]]; then
    aws sqs delete-message \
      --queue-url "$DLQ_URL" \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --receipt-handle "$receipt_handle" >/dev/null
    echo "Deleted message_id=$message_id from DLQ"
  fi
done

echo
if [[ "$DELETE_AFTER_REPLAY" == "true" ]]; then
  echo "Replay complete. Selected messages were sent to main queue and deleted from DLQ."
else
  echo "Replay complete. Selected messages were sent to main queue. DLQ originals were kept (DELETE_AFTER_REPLAY=false)."
fi
