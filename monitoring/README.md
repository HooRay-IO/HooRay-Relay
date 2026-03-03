# Day 6 Monitoring Artifacts

These files support Day 6 monitoring rollout for both ingestion and worker services.

## Worker: Apply Dashboard

```bash
aws cloudwatch put-dashboard \
  --dashboard-name hooray-relay-worker-dev \
  --dashboard-body file://monitoring/worker-dashboard.json \
  --region us-west-2 \
  --profile hooray-dev
```

## Worker: Apply Alarms

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

## Worker: Validate Day 6 End-to-End

```bash
FORCE_ALARM_STATE_TEST=true ./scripts/e2e_day6_worker_observability.sh
```

This validates metric visibility, delivery-attempt log fields, alarm existence, and forced alarm transitions.

## Ingestion: Validate Day 6

```bash
APPLY_DASHBOARD=true ./scripts/e2e_day6_ingestion_observability.sh
```

This validates ingestion dashboard presence, required ingestion metric names, and required ingestion alarms.
