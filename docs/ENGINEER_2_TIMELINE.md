# Engineer 2: Delivery Side - Timeline & Daily Tasks

**Your Mission:** Build the webhook delivery worker that polls SQS, delivers webhooks to customer endpoints, handles retries, and records delivery attempts.

**Timeline:** 10 days  
**Code Reference:** See `code-samples/ENGINEER_2_CODE_SAMPLES.md`  
**Handoff Partner:** Engineer 1 (Ingestion Pipeline)  
**Critical Handoff Day:** Day 5

---

## Week 1: Core Delivery Pipeline

### Day 1: Project Setup & Understanding Ingestion Output

**Goal:** Set up development environment and understand what Engineer 1 is building

**Morning (9am-12pm): Environment Setup**
- [x] Clone repository (if not done)
- [x] Initialize Rust worker project in `worker/` directory
- [x] Update `Cargo.toml` with all dependencies:
  - AWS SDK (DynamoDB, SQS)
  - HTTP client (reqwest)
  - Crypto (hmac, sha2, hex)
  - Utilities and logging
- [x] Create project structure (models, services directories)

**Afternoon (1pm-5pm): Study Integration Points**
- [x] Create `src/models.rs` with data structures:
  - Event (from DynamoDB)
  - EventStatus enum
  - WebhookConfig
  - DeliveryAttempt
  - DeliveryResult
  - WorkerError
- [x] Write unit test for event deserialization
- [x] Document integration expectations from Engineer 1:
  - SQS message format
  - DynamoDB table structures
  - Expected fields and types
- [ ] List questions for Engineer 1

**Deliverables:**
- Rust project initialized
- Data models defined
- Unit tests passing
- Integration expectations documented

**Commit:** `feat: initialize worker project with data models`

---

### Day 2: DynamoDB Service Layer

**Goal:** Build service layer to fetch events and configs from DynamoDB

**Morning (9am-12pm): Event Fetching**
- [x] Create `src/services/dynamodb.rs`
- [x] Implement `DynamoService` struct with table names
- [x] Implement `get_event()` to fetch event by ID
- [x] Implement `get_config()` to fetch webhook config
- [x] Add helper functions:
  - `get_string_attr()`
  - `get_number_attr()`
  - `get_bool_attr()`
  - `parse_status()`
- [x] Test compilation

**Afternoon (1pm-5pm): Attempt Recording & Status Updates**
- [x] Implement `record_attempt()` to store delivery attempts
- [x] Implement `update_event_status()` for status transitions
- [x] Implement `increment_attempt_count()` for retries
- [x] Add comprehensive logging
- [x] Create test script `tests/test_dynamodb.sh`
- [ ] Create test data in DynamoDB
- [ ] Verify can fetch test data

**Deliverables:**
- DynamoDB service layer complete
- Event/config fetching works
- Attempt recording works
- Test data created

**Commit:** `feat: implement DynamoDB service layer`

---

### Day 3: HMAC Signature & HTTP Delivery

**Goal:** Build signature generation and HTTP delivery logic

**Morning (9am-12pm): HMAC Signature Service**
- [x] Create `src/services/signature.rs`
- [x] Implement `SignatureService::generate()`:
  - Create signing string: `<X-Webhook-Timestamp> + "." + <raw request body JSON>`
  - Generate HMAC-SHA256
  - Format as "sha256={hex}"
- [x] Implement `SignatureService::verify()` for testing
- [x] Write unit tests:
  - Signature format validation
  - Signature verification
  - Wrong secret/payload fails

**Afternoon (1pm-5pm): HTTP Delivery Service**
- [x] Create `src/services/delivery.rs`
- [x] Implement `DeliveryService::deliver()`:
  - Generate HMAC signature
  - Set `X-Webhook-Timestamp` using the exact timestamp used for signing
  - Set proper headers
  - Make HTTP POST request
  - Handle timeouts (30s)
  - Return DeliveryAttempt result
- [x] Implement `classify_error()` helper
- [x] Create `src/services/mod.rs`
- [ ] Write integration test with webhook.site
- [ ] Verify successful delivery

**Deliverables:**
- HMAC signature generation working
- HTTP delivery service complete
- Error classification implemented
- Integration test successful

**Commit:** `feat: implement HMAC signatures and HTTP delivery`

---

### Day 4: SQS Polling & Main Worker Loop

**Goal:** Build the main worker service that polls SQS and orchestrates delivery

**Morning (9am-12pm): Worker Structure**
- [x] Create `src/main.rs` with Worker struct
- [x] Implement `Worker::new()` constructor
- [x] Implement `run()` infinite loop
- [x] Implement `poll_and_process()`:
  - Long poll SQS (20s wait)
  - Process up to 10 messages
  - Handle empty responses
- [x] Add structured JSON logging

**Afternoon (1pm-5pm): Message Processing & Delivery Logic**
- [x] Implement `process_message()`:
  - Extract event_id and receipt_handle
  - Call deliver_event()
  - Handle Success/Retry/Exhausted results
  - Delete message on success
- [x] Implement `deliver_event()`:
  - Fetch event from DynamoDB
  - Fetch config from DynamoDB
  - Check config is active
  - Attempt delivery
  - Record attempt
  - Update status
  - Determine retry vs exhausted
- [x] Implement `delete_message()` for SQS
- [ ] Test locally with environment variables
- [x] Verify compilation

**Deliverables:**
- Main worker loop complete
- SQS polling working
- Message processing logic complete
- Local testing successful

**Commit:** `feat: implement main worker with SQS polling`

---

### Day 5: Integration Testing & Handoff

**Goal:** Test integration with Engineer 1's components and deploy the worker as a long-running service

**Morning (9am-12pm): Integration Testing**
- [ ] Meet with Engineer 1 for integration sync
- [ ] Verify SQS message format matches expectations
- [ ] Confirm DynamoDB schemas are correct
- [x] Create `scripts/e2e_ingestion_worker.sh`:
  - Create webhook config through ingestion API
  - Submit event through ingestion API
  - Verify worker writes `ATTEMPT#1`
  - Verify final event status is `delivered`
  - Add cleanup for event/config/idempotency test data
- [ ] Test with real events from Engineer 1's API
- [ ] Document any issues found

**Afternoon (1pm-5pm): Run Worker Service and Monitor**
- [ ] Prepare runtime environment:
  - Confirm AWS credentials/profile can access SQS + DynamoDB
  - Confirm `QUEUE_URL`, `EVENTS_TABLE`, `CONFIGS_TABLE`, `AWS_REGION`
- [x] Build and push worker image:
  - Build image from `worker/Dockerfile`
  - Push to ECR
  - Update `WorkerImageUri` tag in `samconfig.local.toml`
- [ ] Deploy worker ECS service:
  - Confirm `samconfig.local.toml` has valid `EcsSubnetIds` and `EcsSecurityGroupIds`
  - Run `./scripts/deploy_dev.sh`
- [ ] Monitor ECS and logs:
  - Confirm ECS service/task reaches steady state
  - Check CloudWatch log group `/ecs/hooray-relay-worker-dev`
  - Monitor CloudWatch queue metrics
- [ ] Verify deliveries to webhook.site
- [ ] Check DynamoDB for delivery attempts
- [ ] Confirm SQS messages being deleted
- [ ] Confirm DLQ depth is not increasing during happy-path tests
- [ ] Write integration test results document

**Deliverables:**
- Integration with Engineer 1 successful
- All tests passing
- Worker service running in target environment (non-Lambda)
- Operational monitoring working
- Documentation updated

**Commit:** `feat: complete worker integration and runtime rollout`

---

## Week 2: Observability & Polish

### Day 6: Monitoring & Alerting

**Goal:** Add comprehensive monitoring for delivery pipeline

**Tasks:**
- [x] Add CloudWatch metrics emission:
  - DeliveryAttempts (count)
  - DeliverySuccess (count)
  - DeliveryFailure (count)
  - DeliveryLatency (milliseconds)
  - HTTPStatusCode (by status)
- [x] Create CloudWatch dashboard for worker:
  - Success/failure rates
  - Latency percentiles
  - Error breakdown
  - Queue depth over time
- [x] Set up alarms:
  - High failure rate (>10%)
  - High latency (p95 >5s)
  - DLQ messages present
- [x] Test alarms trigger correctly

**Deliverables:**
- [x] Metrics emitting properly
- [x] Dashboard operational
- [x] Alarms configured and tested

**Commit:** `feat: add CloudWatch monitoring for delivery worker`

---

### Day 7: Retry Optimization

**Goal:** Improve retry logic and add exponential backoff

**Tasks:**
- [x] Research optimal retry strategies
- [x] Implement exponential backoff:
  - Calculate delay: initial_delay * backoff^attempt
  - Add jitter to prevent thundering herd
- [x] Optimize SQS visibility timeout:
  - Dynamic based on attempt number
  - Longer timeout for later retries
- [x] Test retry behavior with failing endpoint (unit-level resilience + worker retry path)
- [x] Document retry schedule:
  - base delay: 5s, multiplier: 2x, max delay: 5m, jitter max: 1s
  - visibility timeout: clamp(retry_delay + 15s overhead, min 30s, max 1h)

**Deliverables:**
- Smarter retry strategy implemented
- Jitter added
- Performance improved

**Note:** Defer async recloser/circuit-breaker hardening to Day 8+ after retry baseline is validated.
- Deferred follow-up: Persist breaker state across worker restarts/instances (DynamoDB or ElastiCache/Redis)
- Deferred follow-up: Add resilience observability metrics (breaker open/half-open/close, retry delay, visibility timeout set)

**Commit:** `feat: optimize retry logic with exponential backoff`

---

### Day 8: Error Handling & DLQ Processing

**Goal:** Harden resilience path (error taxonomy + breaker behavior) and operationalize DLQ handling

**Morning (9am-12pm): Error Taxonomy and Breaker State**
- [x] Finalize error classification matrix in `worker/src/services/delivery.rs`:
  - Retryable network/transient failures (timeout, DNS/connect reset, 5xx, 429)
  - Non-retryable customer errors (most 4xx, invalid URL/signature expectations)
  - Internal/service errors (missing config/event, serialization/parse failures)
- [x] Add deterministic mapping from error class -> worker action:
  - `retry` (requeue with computed backoff)
  - `fail_terminal` (mark failed, no more retries)
  - `drop_invalid` (record attempt + diagnostic reason)
- [x] Implement deferred resilience hardening from Day 7:
  - Persist circuit-breaker state (open/half-open/closed) in DynamoDB
  - Load breaker state at worker boot and refresh during processing

**Afternoon (1pm-5pm): DLQ Operations + Runbook**
- [x] Add DLQ processing utility/script in `scripts/`:
  - Poll and decode DLQ messages
  - Summarize root-cause buckets by error class
  - Support safe replay for selected message IDs (dry-run default)
- [x] Emit resilience metrics:
  - `CircuitBreakerOpen`, `CircuitBreakerHalfOpen`, `CircuitBreakerClose`
  - `RetryDelayMs`, `VisibilityTimeoutSeconds`, `DlqReplayCount`
- [x] Write operator docs:
  - `docs/runbook.md`: DLQ triage and replay steps
  - `docs/troubleshooting.md`: error-class playbook and escalation path
- [x] Run scenario tests:
  - Endpoint outage (5xx/timeouts)
  - Permanent 4xx failure
  - Missing/disabled config
  - Validate expected transition to DLQ and replay behavior

**Deliverables:**
- Error classification matrix implemented and validated
- Circuit-breaker state persistence shipped
- DLQ triage/replay workflow documented and tested
- Resilience metrics visible in CloudWatch

**Commit:** `feat: harden error handling, persist breaker state, and add DLQ ops workflow`

---

### Day 9: Performance Testing

**Goal:** Validate worker performance under load

**Tasks:**
- [ ] Load test setup:
  - Generate 1000 events in SQS
  - Monitor processing rate
  - Track success/failure rates
  - Measure end-to-end latency
- [ ] Identify bottlenecks:
  - DynamoDB read/write latency
  - HTTP delivery latency
  - SQS polling delays
- [ ] Optimize slow paths:
  - Batch DynamoDB operations if possible
  - Concurrent message processing
  - Connection pooling
- [ ] Re-run tests and verify improvements
- [ ] Document performance characteristics:
  - Max throughput
  - Latency percentiles
  - Resource usage

**Target Metrics:**
- Handle 500+ events/minute
- < 5s p95 delivery latency
- 99.9% delivery success rate

**Deliverables:**
- Load tests complete
- Bottlenecks identified and fixed
- Performance documented

**Commit:** `perf: optimize worker for high-volume delivery`

---

### Day 10: Documentation & Handoff

**Goal:** Complete documentation and prepare for production

**Tasks:**
- [ ] Write operational runbook:
  - How to check worker health
  - How to investigate failures
  - How to manually retry events
  - How to update configs
- [ ] Create troubleshooting guide:
  - Worker not processing messages
  - High failure rates
  - SQS queue backing up
  - DLQ messages appearing
- [ ] Document retry behavior:
  - Retry schedule
  - Max attempts
  - Backoff strategy
- [ ] Prepare demo:
  - Show end-to-end flow
  - Demonstrate retry logic
  - Show monitoring dashboards
- [ ] Final code review
- [ ] Tag release v1.0.0

**Deliverables:**
- Operational runbook complete
- Troubleshooting guide ready
- Demo prepared
- Production-ready code

**Commit:** `docs: add operational guides and prepare v1.0.0`

---

## Daily Standup Template

**What I did yesterday:**
- [List completed tasks]

**What I'm doing today:**
- [List planned tasks from daily checklist]

**Blockers:**
- [Any blockers or dependencies on Engineer 1]

**Questions for Engineer 1:**
- [Integration questions or clarifications needed]

---

## Success Criteria

By end of Week 2, verify:
- [ ] 99.9% delivery success rate (excluding customer errors)
- [ ] < 5s p95 delivery latency
- [ ] Handle 500+ events/minute
- [ ] Zero message loss
- [ ] Comprehensive monitoring dashboards
- [ ] Complete operational documentation
- [ ] All integration tests passing
- [ ] Retry logic working as expected
- [ ] DLQ handling in place
- [ ] Production deployment stable

---

## Key Files Checklist

By end of sprint, you should have:
- [x] `src/main.rs` - Worker main loop
- [ ] `src/models.rs` - Data structures
- [x] `src/services/dynamodb.rs` - DynamoDB operations
- [x] `src/services/delivery.rs` - HTTP delivery
- [x] `src/services/signature.rs` - HMAC generation
- [x] `src/services/mod.rs` - Service exports
- [x] `worker/tests/end_to_end_test.sh` - Contract integration helper
- [x] `scripts/e2e_ingestion_worker.sh` - Full ingestion->worker e2e
- [x] `tests/test_dynamodb.sh` - DynamoDB tests
- [x] `docs/runbook.md` - Operational guide
- [x] `docs/troubleshooting.md` - Common issues
- [ ] `README.md` - Worker overview

---

## Emergency Procedures

### Worker Stopped Processing
1. Check worker service logs for errors (`journalctl`, container logs, or process logs)
2. Verify SQS queue has messages: `aws sqs get-queue-attributes --queue-url $QUEUE_URL`
3. Check IAM permissions for the worker runtime role/user
4. Restart worker process or service (systemd/ECS task rollout)

### High Failure Rate
1. Check customer endpoint health: `curl -X POST $CUSTOMER_URL`
2. Review delivery attempt logs in DynamoDB
3. Verify HMAC signatures are correct
4. Check network connectivity from worker runtime to customer endpoints
5. Temporarily disable failing endpoint configs

### Queue Backing Up
1. Check worker replica/process count: scale out if needed
2. Monitor worker errors: Fix any code issues
3. Check DynamoDB throttling: May need provisioned capacity
4. Consider increasing worker CPU/memory limits in runtime platform

---

## Resources

**Code Samples:** All code referenced in this timeline is in `code-samples/ENGINEER_2_CODE_SAMPLES.md`

**AWS Documentation:**
- [DynamoDB Developer Guide](https://docs.aws.amazon.com/dynamodb/)
- [SQS Developer Guide](https://docs.aws.amazon.com/sqs/)

**Project Dictionary:** See `PROJECT_DICTIONARY.md` for complete schemas, architecture, and patterns

---

**You've got this! Build something great! 🚀**
