# Day 6 Monitoring Artifacts

These files support Day 6 monitoring rollout for both ingestion and worker services.

## Worker: Apply Dashboard

```bash
./scripts/apply_worker_monitoring.sh dev
```

## Worker: Apply Alarms

```bash
./scripts/apply_worker_monitoring.sh staging
./scripts/apply_worker_monitoring.sh prod
```

`DLQDepthAlarm` is provisioned via `template.yaml` and should remain enabled.

Worker dashboard and worker alarm payloads are rendered from environment-aware
templates before upload. Source templates:

- `monitoring/worker-dashboard.template.json`
- `monitoring/alarms/worker-failure-rate.template.json`
- `monitoring/alarms/worker-latency-p95.template.json`

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
