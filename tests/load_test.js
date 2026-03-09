import http from "k6/http";
import { check, sleep } from "k6";
import { Counter, Rate } from "k6/metrics";
import { uuidv4 } from "https://jslib.k6.io/k6-utils/1.4.0/index.js";

const API_URL = (__ENV.API_URL || __ENV.BASE_URL || "").replace(/\/+$/, "");
const API_KEY = __ENV.API_KEY || "";
const CUSTOMER_ID = __ENV.CUSTOMER_ID || "load_test_customer";
const TARGET_URL = __ENV.TARGET_URL || "https://httpbin.org/post";
const TEST_RUN_ID = __ENV.TEST_RUN_ID || `run_${Date.now()}`;
const ENDPOINT_PROFILE = __ENV.ENDPOINT_PROFILE || "healthy";

const TARGET_EVENTS = Number(__ENV.TARGET_EVENTS || 0);
const ITERATION_VUS = Number(__ENV.ITERATION_VUS || 50);
const MODE = __ENV.MODE || (TARGET_EVENTS > 0 ? "seed" : "steady");

const RATE = Number(__ENV.RATE || 500);
const DURATION = __ENV.DURATION || "2m";
const PREALLOCATED_VUS = Number(__ENV.PRE_ALLOCATED_VUS || 200);
const MAX_VUS = Number(__ENV.MAX_VUS || 400);
const THINK_TIME_MS = Number(__ENV.THINK_TIME_MS || 250);
const SETUP_CREATE_CONFIG = __ENV.SETUP_CREATE_CONFIG === "true";

const successRate = new Rate("accepted_rate");
const duplicateRate = new Rate("duplicate_rate");
const serverErrorRate = new Rate("server_error_rate");
const statusAccepted = new Counter("status_accepted");
const statusDuplicate = new Counter("status_duplicate");

const httpReqFailedThreshold = __ENV.HTTP_REQ_FAILED_THRESHOLD || "rate<0.001";
const httpReqDurationP95Threshold =
  __ENV.HTTP_REQ_DURATION_P95_THRESHOLD || "p(95)<500";
const acceptedRateThreshold = __ENV.ACCEPTED_RATE_THRESHOLD || "rate>0.99";
const serverErrorRateThreshold =
  __ENV.SERVER_ERROR_RATE_THRESHOLD || "rate<0.005";

function buildScenarios() {
  if (MODE === "seed" || TARGET_EVENTS > 0) {
    return {
      ingestion_seed: {
        executor: "shared-iterations",
        vus: ITERATION_VUS,
        iterations: TARGET_EVENTS,
        maxDuration: __ENV.MAX_DURATION || "30m",
      },
    };
  }

  if (MODE === "ramping") {
    return {
      ingestion_load: {
        executor: "ramping-vus",
        startVUs: Number(__ENV.START_VUS || 0),
        stages: [
          {
            duration: __ENV.STAGE_1_DURATION || "1m",
            target: Number(__ENV.STAGE_1_TARGET || 10),
          },
          {
            duration: __ENV.STAGE_2_DURATION || "3m",
            target: Number(__ENV.STAGE_2_TARGET || 50),
          },
          {
            duration: __ENV.STAGE_3_DURATION || "5m",
            target: Number(__ENV.STAGE_3_TARGET || 100),
          },
          {
            duration: __ENV.STAGE_4_DURATION || "1m",
            target: Number(__ENV.STAGE_4_TARGET || 0),
          },
        ],
        gracefulRampDown: __ENV.GRACEFUL_RAMP_DOWN || "30s",
      },
    };
  }

  return {
    steady: {
      executor: "constant-arrival-rate",
      rate: RATE,
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: PREALLOCATED_VUS,
      maxVUs: MAX_VUS,
    },
  };
}

export const options = {
  scenarios: buildScenarios(),
  thresholds: {
    http_req_failed: [httpReqFailedThreshold],
    http_req_duration: [httpReqDurationP95Threshold],
    accepted_rate: [acceptedRateThreshold],
    server_error_rate: [serverErrorRateThreshold],
  },
  tags: {
    test_run_id: TEST_RUN_ID,
    endpoint_profile: ENDPOINT_PROFILE,
  },
};

function buildHeaders() {
  const headers = { "Content-Type": "application/json" };
  if (API_KEY) {
    headers["X-API-Key"] = API_KEY;
  }
  return headers;
}

export function setup() {
  if (!API_URL) {
    throw new Error(
      "API_URL or BASE_URL env var is required (e.g. https://<api-id>.execute-api.us-west-2.amazonaws.com/Prod)",
    );
  }

  if (SETUP_CREATE_CONFIG) {
    const configBody = JSON.stringify({
      customer_id: CUSTOMER_ID,
      url: TARGET_URL,
    });
    const res = http.post(`${API_URL}/webhooks/configs`, configBody, {
      headers: buildHeaders(),
      tags: { endpoint: "configs" },
    });
    check(res, {
      "config create success": (r) => r.status === 200 || r.status === 201,
    });
  }

  return { customerId: CUSTOMER_ID };
}

export default function (data) {
  const payload = JSON.stringify({
    idempotency_key: `req_${uuidv4()}`,
    customer_id: data.customerId,
    data: {
      event_type: "load_test_event",
      test_run_id: TEST_RUN_ID,
      endpoint_profile: ENDPOINT_PROFILE,
      source: "k6",
      seq: `${__VU}-${__ITER}`,
      ts: Date.now(),
      order_id: `ord_${uuidv4()}`,
      amount: Math.round(Math.random() * 100000) / 100,
      created_at_ms: Date.now(),
    },
  });

  const res = http.post(`${API_URL}/webhooks/receive`, payload, {
    headers: buildHeaders(),
    tags: { endpoint: "receive" },
  });

  const body = res.body
    ? (() => {
        try {
          return JSON.parse(res.body);
        } catch (_) {
          return {};
        }
      })()
    : {};

  const isAccepted =
    res.status === 202 &&
    body.status === "accepted" &&
    typeof body.event_id === "string";
  const isDuplicate =
    res.status === 200 &&
    body.status === "duplicate" &&
    typeof body.event_id === "string";
  const isServerError = res.status >= 500;

  if (isAccepted) {
    statusAccepted.add(1);
  }
  if (isDuplicate) {
    statusDuplicate.add(1);
  }

  successRate.add(isAccepted);
  duplicateRate.add(isDuplicate);
  serverErrorRate.add(isServerError);

  check(res, {
    "status is 202 or 200": (r) => r.status === 202 || r.status === 200,
    "response includes event_id": () => typeof body.event_id === "string",
    "status is accepted or duplicate":
      () => body.status === "accepted" || body.status === "duplicate",
  });

  if (MODE === "ramping") {
    sleep(THINK_TIME_MS / 1000);
  }
}

function metricValue(data, name, stat = "value") {
  const metric = data.metrics?.[name];
  if (!metric) return null;

  if (metric.values && Object.prototype.hasOwnProperty.call(metric.values, stat)) {
    return metric.values[stat];
  }

  return null;
}

export function handleSummary(data) {
  const summary = {
    meta: {
      generated_at: new Date().toISOString(),
      api_url: API_URL,
      customer_id: CUSTOMER_ID,
      test_run_id: TEST_RUN_ID,
      endpoint_profile: ENDPOINT_PROFILE,
      mode: MODE,
      target_events: TARGET_EVENTS,
      iteration_vus: TARGET_EVENTS > 0 ? ITERATION_VUS : null,
      rate: MODE === "steady" ? RATE : null,
      duration: MODE === "steady" ? DURATION : null,
    },
    metrics: {
      accepted_rate: metricValue(data, "accepted_rate", "rate"),
      duplicate_rate: metricValue(data, "duplicate_rate", "rate"),
      server_error_rate: metricValue(data, "server_error_rate", "rate"),
      http_req_failed_rate: metricValue(data, "http_req_failed", "rate"),
      http_req_duration_p95_ms: metricValue(data, "http_req_duration", "p(95)"),
      iterations: metricValue(data, "iterations", "count"),
      status_accepted: metricValue(data, "status_accepted", "count"),
      status_duplicate: metricValue(data, "status_duplicate", "count"),
    },
  };

  const outPath = __ENV.SUMMARY_JSON_PATH || "";
  const rendered = JSON.stringify(summary, null, 2);

  const outputs = {
    stdout:
      `\n=== k6 load test summary ===\n` +
      `test_run_id: ${TEST_RUN_ID}\n` +
      `mode: ${MODE}\n` +
      `target_events: ${TARGET_EVENTS || "ramping/steady"}\n` +
      `${rendered}\n`,
  };

  if (outPath) {
    outputs[outPath] = rendered;
  }

  return outputs;
}
