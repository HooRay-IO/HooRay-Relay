# Monitoring Artifacts

These files support Day 6 monitoring rollout for both the ingestion Lambda and the delivery worker.

## Dashboards

### Ingestion (Lambda) — `ingestion-dashboard.json`

8 widgets: receive rate, latency p50/p95/p99, idempotency hit rate, SQS enqueue failures,
config API counts, Lambda error rate, Lambda duration, and API Gateway 4xx/5xx errors.
Namespace: `HoorayRelay/Ingestion`.

```bash
aws cloudwatch put-dashboard \
  --dashboard-name hooray-relay-ingestion-dev \
  --dashboard-body file://monitoring/ingestion-dashboard.json \
  --region us-west-2 \
  --profile hooray-dev
```

### Worker (ECS) — `worker-dashboard.json`

Namespace: `HoorayRelay/Worker`.

```bash
aws cloudwatch put-dashboard \
  --dashboard-name hooray-relay-worker-dev \
  --dashboard-body file://monitoring/worker-dashboard.json \
  --region us-west-2 \
  --profile hooray-dev
```

## Alarms

### Worker alarms

```bash
aws cloudwatch put-metric-alarm \
  --cli-input-json file://monitoring/alarms/worker-failure-rate.json \
  --region us-west-2 \
  --profile hooray-dev

aws cloudwatch put-metric-alarm \
  --cli-input-json file://monitoring/alarms/worker-latency-p95.json \
  --region us-west-2 \
  --profile hooray-dev
```

`DLQDepthAlarm` is provisioned via `template.yaml` and should remain enabled.

### Ingestion alarms

Ingestion alarms (error rate threshold) are provisioned directly in `template.yaml`
via the `AWS::CloudWatch::Alarm` resource — no separate CLI step needed.
