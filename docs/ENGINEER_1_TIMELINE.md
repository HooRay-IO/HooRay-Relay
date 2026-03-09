# Engineer 1: Ingestion Side - Timeline & Daily Tasks

**Your Mission:** Build the webhook ingestion pipeline that receives events, ensures idempotency, stores them in DynamoDB, and queues them for delivery.

**Timeline:** 10 days  
**Code Reference:** See `code-samples/ENGINEER_1_CODE_SAMPLES.md`  
**Handoff Partner:** Engineer 2 (Delivery Worker)  
**Critical Handoff Day:** Day 5

---

## Week 1: Core Ingestion Pipeline

### Day 1: Infrastructure Setup & DynamoDB Tables

**Goal:** Get AWS infrastructure ready and create all DynamoDB tables

**Morning (9am-12pm): AWS Setup**
- [x] Create AWS SAM project structure
- [x] Create `template.yaml` with DynamoDB tables, SQS queue, Lambda function placeholders
- [x] Deploy infrastructure with `sam deploy --guided`
- [x] Verify all resources created successfully

**Afternoon (1pm-5pm): Rust Project Setup**
- [x] Initialize Rust ingestion project
- [x] Update `Cargo.toml` with all dependencies
- [x] Create project structure (handlers, models, services)
- [x] Define data models in `src/models.rs`
- [x] Initialize git repository with first commit

**Deliverables:**
- AWS infrastructure deployed
- Rust project initialized
- Models defined
- Git repository setup

**Commit:** `feat: initial project setup with AWS infrastructure`

---

### Day 2: Idempotency & Event Storage

**Goal:** Implement idempotency checking and event storage logic

**Morning (9am-12pm): Idempotency Service**
- [x] Create `src/services/idempotency.rs`
- [x] Implement `check_and_record()` with conditional DynamoDB writes
- [x] Implement `get_existing_event_id()` for duplicate detection
- [x] Add helper function `is_conditional_check_failed()`
- [x] Write unit tests for event ID format

**Afternoon (1pm-5pm): Event Storage Service**
- [x] Create `src/services/events.rs`
- [x] Implement `create_event()` with 30-day TTL
- [x] Add JSON payload serialization
- [x] Create `src/services/mod.rs` to export services
- [x] Verify code compiles successfully

**Deliverables:**
- Idempotency service complete
- Event storage service complete
- Unit tests passing

**Commit:** `feat: implement idempotency and event storage services`

---

### Day 3: SQS Integration & Webhook Handler

**Goal:** Complete the ingestion pipeline with SQS queuing

**Morning (9am-12pm): Queue Service**
- [x] Create `src/services/queue.rs`
- [x] Implement `enqueue_event()` with customer_id message attribute
- [x] Add structured logging for queue operations
- [x] Test queue service compilation

**Afternoon (1pm-5pm): Webhook Handler**
- [x] Create `src/handlers/webhook.rs`
- [x] Implement `receive_webhook()` with full pipeline:
  - Request validation
  - Idempotency check
  - Event creation
  - SQS enqueuing
- [x] Add error handling for all failure cases
- [x] Create `src/handlers/mod.rs`
- [x] Test handler logic

**Deliverables:**
- SQS queue service implemented
- Webhook receive handler complete
- Request validation working
- All errors handled properly

**Commit:** `feat: complete webhook ingestion pipeline with SQS`

---

### Day 4: Config Management & Main Entry Point

**Goal:** Add webhook configuration CRUD and Lambda integration

**Morning (9am-12pm): Config Handlers**
- [x] Create `src/handlers/config.rs`
- [x] Implement `create_config()` with auto-generated secrets
- [x] Implement `get_config()` with query parameters
- [x] Add proper error responses (404, 500)
- [x] Test config handlers

**Afternoon (1pm-5pm): Lambda Main Entry Point**
- [x] Create `src/main.rs` with Lambda integration
- [x] Set up Axum router with all endpoints
- [x] Initialize AWS clients and services
- [x] Add structured JSON logging
- [x] Build and test locally with `cargo lambda watch`
- [x] Test endpoints with curl

**Deliverables:**
- Config CRUD handlers complete
- Lambda main entry point working
- Local testing successful
- All endpoints functional

**Commit:** `feat: add config management and Lambda integration`

---

### Day 5: Integration Testing & Handoff Preparation

**Goal:** Test end-to-end flow and prepare for Engineer 2 integration

**Morning (9am-12pm): Integration Tests**
- [ ] Create `tests/integration_test.sh` script
- [ ] Deploy to AWS with `sam build` and `sam deploy --resolve-s3`
- [ ] Run integration tests against deployed API
- [ ] Verify:
  - Config creation works
  - Webhook receives and returns event_id
  - Idempotency prevents duplicates
  - Events appear in DynamoDB
  - Messages appear in SQS queue

**Afternoon (1pm-5pm): Documentation & Handoff**
- [ ] Create handoff document for Engineer 2 with:
  - SQS message format
  - DynamoDB schemas
  - Sample queries
  - What Engineer 2 needs to build
- [ ] Run `cargo clippy` and `cargo fmt`
- [ ] Generate documentation with `cargo doc`
- [ ] Schedule handoff meeting with Engineer 2
- [ ] Prepare demo of working ingestion pipeline

**Deliverables:**
- Integration tests passing
- Deployed to AWS
- Handoff document ready
- Meeting scheduled

**Commit:** `test: add integration tests and handoff documentation`

---

## Week 2: Observability & Polish

### Day 6: CloudWatch Dashboards

**Goal:** Add comprehensive monitoring and logging

**Tasks:**
- [ ] Create CloudWatch dashboard with key metrics:
  - Webhook receive rate (per minute)
  - Idempotency hit rate
  - Event creation errors
  - SQS enqueue failures
  - API latency (p50, p95, p99)
- [ ] Add CloudWatch metric emission to handlers
- [ ] Set up alarms for error rates
- [ ] Add X-Ray tracing (optional)
- [ ] Test dashboard displays metrics

**Deliverables:**
- CloudWatch dashboard operational
- Metrics emitting properly
- Alarms configured

**Commit:** `feat: add CloudWatch monitoring and dashboards`

---

### Day 7: API Documentation

**Goal:** Create comprehensive API docs

**Tasks:**
- [x] Write OpenAPI/Swagger spec in `docs/api-spec.yaml`
- [x] Document all endpoints with:
  - Request/response examples
  - Error codes
  - Authentication requirements
- [x] Create `docs/customer-guide.md` for API users
- [x] Create Postman collection for testing
- [x] Add example code snippets (curl, Python, Node.js)

**Deliverables:**
- OpenAPI spec complete
- Customer onboarding guide ready
- Postman collection available

**Commit:** `docs: add API documentation and customer guide`

---

### Day 8: Deployment Pipeline

**Goal:** Automate deployment with CI/CD

**Tasks:**
- [ ] Create `.github/workflows/deploy.yml`
- [ ] Add automated tests to CI pipeline
- [ ] Configure AWS credentials for GitHub Actions
- [ ] Set up staging environment
- [ ] Create deployment runbook in `docs/deployment.md`
- [ ] Test pipeline with test commit
- [ ] Verify auto-deployment works

**Deliverables:**
- GitHub Actions workflow operational
- Automated tests running
- Deployment runbook complete

**Commit:** `ci: add GitHub Actions deployment pipeline`

---

### Day 9: Load Testing & Optimization

**Goal:** Validate performance under load

**Tasks:**
- [x] Create shared k6 load test script in `tests/load_test.js` (merged with Engineer 2)
- [ ] Run load test targeting 500 req/sec (`MODE=steady`) or ramping VUs (`MODE=ramping`)
- [ ] Capture structured summary output (`SUMMARY_JSON_PATH=...`) and share results
- [ ] Monitor CloudWatch metrics during test (ingestion latency + accept/duplicate rates)
- [ ] Identify bottlenecks
- [ ] Optimize slow code paths
- [ ] Add caching if beneficial
- [ ] Re-run tests and verify improvements
- [ ] Document performance characteristics

**Target Metrics:**
- 500 requests/second sustained
- < 100ms p95 latency
- < 0.1% error rate
- Zero duplicate deliveries

**Deliverables:**
- Load tests passing
- Performance optimized
- Metrics documented

**Commit:** `perf: optimize ingestion pipeline for 500 req/sec`

---

### Day 10: Final Polish & Demo Prep

**Goal:** Wrap up MVP and prepare for demo

**Tasks:**
- [ ] Final code review
- [ ] Update main README.md with:
  - Architecture overview
  - Quick start guide
  - API endpoint summary
  - Monitoring guide
- [ ] Create demo script with:
  - Show API endpoint receiving webhooks
  - Demonstrate idempotency
  - Show CloudWatch dashboard
  - Walkthrough DynamoDB tables
  - Integration with Engineer 2's worker
- [ ] Record performance metrics for demo
- [ ] Prepare handover documentation
- [ ] Clean up any technical debt
- [ ] Tag release v1.0.0

**Deliverables:**
- Production-ready code
- Complete documentation
- Demo prepared
- Release tagged

**Commit:** `chore: prepare v1.0.0 release`

---

## Daily Standup Template

**What I did yesterday:**
- [List completed tasks]

**What I'm doing today:**
- [List planned tasks from daily checklist]

**Blockers:**
- [Any blockers or dependencies on Engineer 2]

**Questions for Engineer 2:**
- [Integration questions or clarifications needed]

---

## Success Criteria

By end of Week 2, verify:
- [ ] Fully functional ingestion API
- [ ] 100% idempotency accuracy (no duplicates)
- [ ] < 100ms p95 ingestion latency
- [ ] Comprehensive monitoring dashboards
- [ ] Complete API documentation
- [ ] Automated CI/CD pipeline
- [ ] Load tested to 500 req/sec
- [ ] Zero production incidents
- [ ] All integration tests passing
- [ ] Handoff to Engineer 2 complete

---

## Key Files Checklist

By end of sprint, you should have:
- [ ] `template.yaml` - Infrastructure as code
- [ ] `src/main.rs` - Lambda entry point
- [ ] `src/models.rs` - Data structures
- [ ] `src/services/idempotency.rs` - Duplicate prevention
- [ ] `src/services/events.rs` - Event storage
- [ ] `src/services/queue.rs` - SQS integration
- [ ] `src/handlers/webhook.rs` - Webhook receive handler
- [ ] `src/handlers/config.rs` - Config CRUD handlers
- [ ] `tests/integration_test.sh` - Integration tests
- [ ] `tests/load_test.js` - Load tests
- [ ] `.github/workflows/deploy.yml` - CI/CD pipeline
- [ ] `docs/api-spec.yaml` - API documentation
- [ ] `docs/customer-guide.md` - User guide
- [ ] `README.md` - Project overview

---

## Resources

**Code Samples:** All code referenced in this timeline is in `code-samples/ENGINEER_1_CODE_SAMPLES.md`

**AWS Documentation:**
- [DynamoDB Developer Guide](https://docs.aws.amazon.com/dynamodb/)
- [SQS Developer Guide](https://docs.aws.amazon.com/sqs/)
- [Lambda Developer Guide](https://docs.aws.amazon.com/lambda/)

**Project Dictionary:** See `PROJECT_DICTIONARY.md` for complete schemas, architecture, and patterns

---

**Good luck! You've got this! 🚀**
