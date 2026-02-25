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

**Goal:** Build the main worker that polls SQS and orchestrates delivery

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

**Goal:** Test integration with Engineer 1's components and coordinate handoff

**Morning (9am-12pm): Integration Testing**
- [ ] Meet with Engineer 1 for integration sync
- [ ] Verify SQS message format matches expectations
- [ ] Confirm DynamoDB schemas are correct
- [ ] Create `tests/end_to_end_test.sh`:
  - Create test event in DynamoDB
  - Create webhook config
  - Send message to SQS
  - Verify worker processes it
- [ ] Test with real events from Engineer 1's API
- [ ] Document any issues found

**Afternoon (1pm-5pm): Deploy and Monitor**
- [ ] Add WorkerFunction to `template.yaml`:
  - SQS event source
  - DynamoDB permissions
  - Environment variables
- [ ] Build Lambda: `cargo lambda build --release --arm64`
- [ ] Deploy: `sam build && sam deploy`
- [ ] Monitor logs: `sam logs --name WorkerFunction --tail`
- [ ] Verify deliveries to webhook.site
- [ ] Check DynamoDB for delivery attempts
- [ ] Confirm SQS messages being deleted
- [ ] Write integration test results document

**Deliverables:**
- Integration with Engineer 1 successful
- All tests passing
- Worker deployed to AWS
- Production monitoring working
- Documentation updated

**Commit:** `feat: complete worker integration and deployment`

---

## Week 2: Observability & Polish

### Day 6: Monitoring & Alerting

**Goal:** Add comprehensive monitoring for delivery pipeline

**Tasks:**
- [ ] Add CloudWatch metrics emission:
  - DeliveryAttempts (count)
  - DeliverySuccess (count)
  - DeliveryFailure (count)
  - DeliveryLatency (milliseconds)
  - HTTPStatusCode (by status)
- [ ] Create CloudWatch dashboard for worker:
  - Success/failure rates
  - Latency percentiles
  - Error breakdown
  - Queue depth over time
- [ ] Set up alarms:
  - High failure rate (>10%)
  - High latency (p95 >5s)
  - DLQ messages present
- [ ] Test alarms trigger correctly

**Deliverables:**
- Metrics emitting properly
- Dashboard operational
- Alarms configured and tested

**Commit:** `feat: add CloudWatch monitoring for delivery worker`

---

### Day 7: Retry Optimization

**Goal:** Improve retry logic and add exponential backoff

**Tasks:**
- [ ] Research optimal retry strategies
- [ ] Implement exponential backoff:
  - Calculate delay: initial_delay * backoff^attempt
  - Add jitter to prevent thundering herd
- [ ] Optimize SQS visibility timeout:
  - Dynamic based on attempt number
  - Longer timeout for later retries
- [ ] Add circuit breaker pattern (optional):
  - Track failures per endpoint
  - Temporarily stop delivery if threshold exceeded
- [ ] Test retry behavior with failing endpoint
- [ ] Document retry schedule

**Deliverables:**
- Smarter retry strategy implemented
- Jitter added
- Performance improved

**Commit:** `feat: optimize retry logic with exponential backoff`

---

### Day 8: Error Handling & DLQ Processing

**Goal:** Robust error handling and dead letter queue management

**Tasks:**
- [ ] Improve error classification:
  - Network errors (timeout, connection refused)
  - HTTP errors (4xx vs 5xx)
  - Service errors (config not found)
- [ ] Create DLQ processor Lambda:
  - Alert on messages in DLQ
  - Log DLQ message details
  - Optional: Manual retry mechanism
- [ ] Add manual retry API endpoint (optional)
- [ ] Create runbook for common errors:
  - Customer endpoint down
  - Invalid config
  - Network issues
- [ ] Test error scenarios

**Deliverables:**
- Error handling improved
- DLQ processor created
- Runbook complete

**Commit:** `feat: enhance error handling and add DLQ processor`

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
- [ ] `src/main.rs` - Worker main loop
- [ ] `src/models.rs` - Data structures
- [ ] `src/services/dynamodb.rs` - DynamoDB operations
- [ ] `src/services/delivery.rs` - HTTP delivery
- [ ] `src/services/signature.rs` - HMAC generation
- [ ] `src/services/mod.rs` - Service exports
- [ ] `tests/end_to_end_test.sh` - Integration tests
- [ ] `tests/test_dynamodb.sh` - DynamoDB tests
- [ ] `docs/runbook.md` - Operational guide
- [ ] `docs/troubleshooting.md` - Common issues
- [ ] `README.md` - Worker overview

---

## Emergency Procedures

### Worker Stopped Processing
1. Check CloudWatch logs for errors: `sam logs --name WorkerFunction --tail`
2. Verify SQS queue has messages: `aws sqs get-queue-attributes --queue-url $QUEUE_URL`
3. Check IAM permissions on Lambda role
4. Restart Lambda: Redeploy with `sam deploy`

### High Failure Rate
1. Check customer endpoint health: `curl -X POST $CUSTOMER_URL`
2. Review delivery attempt logs in DynamoDB
3. Verify HMAC signatures are correct
4. Check network connectivity from Lambda to customer endpoints
5. Temporarily disable failing endpoint configs

### Queue Backing Up
1. Check worker Lambda concurrency: May need to increase
2. Monitor Lambda errors: Fix any code issues
3. Check DynamoDB throttling: May need provisioned capacity
4. Consider temporarily scaling up Lambda memory/timeout

---

## Resources

**Code Samples:** All code referenced in this timeline is in `code-samples/ENGINEER_2_CODE_SAMPLES.md`

**AWS Documentation:**
- [DynamoDB Developer Guide](https://docs.aws.amazon.com/dynamodb/)
- [SQS Developer Guide](https://docs.aws.amazon.com/sqs/)
- [Lambda Developer Guide](https://docs.aws.amazon.com/lambda/)

**Project Dictionary:** See `PROJECT_DICTIONARY.md` for complete schemas, architecture, and patterns

---

**You've got this! Build something great! 🚀**
