## Load Test Report (Short)

### Summary
- Test window: 2026-03-22
- Mock server latency: 50ms baseline
- Worker runtime: ECS (per CI defaults)

### Results

#### Baseline steady (20 RPS, 2m)
- Accepted rate: **98.79%**
- Server error rate: **1.21%** (threshold crossed)
- p95 latency: **109 ms**
- Iterations: **2400**

#### Normal load (60 RPS, 2m)
- Accepted rate: **100%**
- Server error rate: **0%**
- p95 latency: **105 ms**
- Iterations: **7201**

#### Peak load (100 RPS, 2m)
- Accepted rate: **99.34%**
- Server error rate: **0.66%** (threshold crossed)
- p95 latency: **105 ms**
- Iterations: **12001**

#### Burst + recovery (200 RPS burst, 2m total)
- Accepted rate: **100%**
- Server error rate: **0%**
- p95 latency: **98 ms**
- Iterations: **2400**

#### Failure injection (60 RPS, 2m)
- Final captured run used 60 RPS for 30s (failure injection enabled).
- Accepted rate: **100%**
- Server error rate: **0%**
- p95 latency: **104 ms**
- Iterations: **1800**

> Note: Several 2m runs were interrupted; this 30s run provides the clean captured summary.

#### Duplicate storm (50 RPS, 2m)
- Accepted rate: **100%**
- Server error rate: **0%**
- p95 latency: **103 ms**
- Iterations: **6001**

### Notes
- Error rates at 20 RPS and 100 RPS triggered k6 thresholds. This suggests intermittent ingestion failures that should be investigated (CloudWatch logs + API Gateway metrics).
- Sequential worker loop means throughput scales primarily with ECS task count; consider documenting worker count during each test run for reproducibility.
- Delivery metrics were initially missing due to inactive/missing config. After creating an active config for `cust_load_test`, `webhook.delivery.*` EMF metrics appeared in dev logs and CloudWatch.

### CloudWatch snapshot (last 30m)
- **Namespace:** `HoorayRelay/Ingestion`
- **Metric:** `webhook.receive.count` (status_code=202)
	- Observed per-minute sums ranged from **~127** to **~5964** during the test window.
- **Namespace:** `HoorayRelay/Worker`
- **Metric:** `webhook.queue.depth` (queue_name=webhook_delivery_dev)
	- Average depth ranged from **0** to **~2459** in the same window.
- Delivery success/failure counts were **not present** in CloudWatch for the window (check log group parsing or metric filters).
- After creating the active config, CloudWatch reported `webhook.delivery.success` counts (e.g., sums of **4** and **296** in the last 10 minutes window).

### Next steps
- Re-run failure injection and capture final metrics.
- Pull CloudWatch metrics for queue depth and retry behavior to correlate with k6 results.
