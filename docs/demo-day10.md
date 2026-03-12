# Day 10 Demo Script

## Goal

Show the full webhook delivery flow, retry behavior, and operational visibility in one short walkthrough.

## Demo Story

1. Create or verify a webhook config for a demo customer.
2. Submit a webhook event through the ingestion API.
3. Show the worker consuming the queue message.
4. Show the delivery attempt and final event state in DynamoDB.
5. Show one retry scenario with a controlled failing endpoint.
6. Show CloudWatch or dashboard visibility for queue depth, success/failure, and latency.

## Prep Checklist

- Worker ECS service healthy
- Ingestion API deployed
- Main queue and DLQ URLs available
- Dashboard available
- Test endpoint ready:
  - happy path endpoint
  - failure endpoint that returns `5xx` or times out

## Happy Path Walkthrough

Create config:

```bash
curl -sS -X POST "${API_URL}webhooks/configs" \
  -H 'content-type: application/json' \
  -d '{"customer_id":"cust_demo","url":"https://httpbin.org/post","secret":"whsec_demo","active":true,"max_retries":3}'
```

Submit event:

```bash
curl -sS -X POST "${API_URL}webhooks/receive" \
  -H 'content-type: application/json' \
  -d '{"idempotency_key":"req_demo_1","customer_id":"cust_demo","data":{"demo":"happy-path"}}'
```

Show:
- returned `event_id`
- worker logs for the event
- DynamoDB `ATTEMPT#1`
- DynamoDB `v0` status changed to `delivered`

## Retry Walkthrough

Update config to a failing endpoint, then submit another event.

Show:
- initial attempt fails
- event remains `pending`
- attempt row contains error details
- queue message is not deleted immediately
- subsequent retry occurs after visibility timeout / backoff

If the event ultimately lands in the DLQ:
- run `MODE=inspect ./scripts/dlq_ops.sh`
- explain the failure class
- show dry-run replay

## Monitoring Walkthrough

Show these signals:
- worker service healthy in ECS
- queue depth metrics
- delivery success/failure counts
- delivery latency graph
- DLQ depth

## Demo Close

End on:
- confirmed successful delivery for the happy-path event,
- confirmed retry behavior for the failing event,
- confirmed operator workflow for investigation and replay.
