# Worker Monitoring Artifacts

These files support Day 6 monitoring rollout for the delivery worker.

## Apply Dashboard

```bash
aws cloudwatch put-dashboard \
  --dashboard-name hooray-relay-worker-dev \
  --dashboard-body file://monitoring/worker-dashboard.json \
  --region us-west-2 \
  --profile hooray-dev
```

## Apply Alarms

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
