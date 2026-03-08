import http from 'k6/http';
import { check, sleep } from 'k6';
import { Counter, Rate } from 'k6/metrics';
import { uuidv4 } from 'https://jslib.k6.io/k6-utils/1.4.0/index.js';

const API_URL = (__ENV.API_URL || '').replace(/\/+$/, '');
const API_KEY = __ENV.API_KEY || '';
const CUSTOMER_ID = __ENV.CUSTOMER_ID || 'load_test_customer';
const TEST_RUN_ID = __ENV.TEST_RUN_ID || `run_${Date.now()}`;
const ENDPOINT_PROFILE = __ENV.ENDPOINT_PROFILE || 'healthy';

// If TARGET_EVENTS > 0, run deterministic fixed-volume seeding via shared-iterations.
const TARGET_EVENTS = Number(__ENV.TARGET_EVENTS || 0);
const ITERATION_VUS = Number(__ENV.ITERATION_VUS || 50);

// Legacy ramping profile (used when TARGET_EVENTS is not set).
const THINK_TIME_MS = Number(__ENV.THINK_TIME_MS || 250);

const successRate = new Rate('accepted_rate');
const duplicateRate = new Rate('duplicate_rate');
const serverErrorRate = new Rate('server_error_rate');
const statusAccepted = new Counter('status_accepted');
const statusDuplicate = new Counter('status_duplicate');

const acceptedRateThreshold = __ENV.ACCEPTED_RATE_THRESHOLD || 'rate>0.99';
const serverErrorRateThreshold = __ENV.SERVER_ERROR_RATE_THRESHOLD || 'rate<0.005';

function buildScenarios() {
  if (TARGET_EVENTS > 0) {
    return {
      ingestion_seed: {
        executor: 'shared-iterations',
        vus: ITERATION_VUS,
        iterations: TARGET_EVENTS,
        maxDuration: __ENV.MAX_DURATION || '30m',
      },
    };
  }

  return {
    ingestion_load: {
      executor: 'ramping-vus',
      startVUs: Number(__ENV.START_VUS || 0),
      stages: [
        { duration: __ENV.STAGE_1_DURATION || '1m', target: Number(__ENV.STAGE_1_TARGET || 10) },
        { duration: __ENV.STAGE_2_DURATION || '3m', target: Number(__ENV.STAGE_2_TARGET || 50) },
        { duration: __ENV.STAGE_3_DURATION || '5m', target: Number(__ENV.STAGE_3_TARGET || 100) },
        { duration: __ENV.STAGE_4_DURATION || '1m', target: Number(__ENV.STAGE_4_TARGET || 0) },
      ],
      gracefulRampDown: __ENV.GRACEFUL_RAMP_DOWN || '30s',
    },
  };
}

export const options = {
  scenarios: buildScenarios(),
  thresholds: {
    http_req_failed: ['rate<0.01'],
    http_req_duration: ['p(95)<1000'],
    accepted_rate: [acceptedRateThreshold],
    server_error_rate: [serverErrorRateThreshold],
  },
  tags: {
    test_run_id: TEST_RUN_ID,
    endpoint_profile: ENDPOINT_PROFILE,
  },
};

function requiredEnv(name, value) {
  if (!value) {
    throw new Error(`Missing required env var: ${name}`);
  }
}

function headers() {
  const h = { 'Content-Type': 'application/json' };
  if (API_KEY) {
    h['X-API-Key'] = API_KEY;
  }
  return h;
}

export function setup() {
  requiredEnv('API_URL', API_URL);
  return {
    testRunId: TEST_RUN_ID,
    endpointProfile: ENDPOINT_PROFILE,
  };
}

export default function (ctx) {
  const payload = JSON.stringify({
    idempotency_key: `req_${uuidv4()}`,
    customer_id: CUSTOMER_ID,
    data: {
      event_type: 'load_test_event',
      test_run_id: ctx.testRunId,
      endpoint_profile: ctx.endpointProfile,
      order_id: `ord_${uuidv4()}`,
      amount: Math.round(Math.random() * 100000) / 100,
      created_at_ms: Date.now(),
      source: 'k6',
    },
  });

  const res = http.post(`${API_URL}/webhooks/receive`, payload, { headers: headers() });

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
    res.status === 202 && body.status === 'accepted' && typeof body.event_id === 'string';
  const isDuplicate =
    res.status === 200 && body.status === 'duplicate' && typeof body.event_id === 'string';
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
    'status is 202 or 200': (r) => r.status === 202 || r.status === 200,
    'response includes event_id': () => typeof body.event_id === 'string',
    'status is accepted or duplicate': () => body.status === 'accepted' || body.status === 'duplicate',
  });

  if (TARGET_EVENTS <= 0) {
    sleep(THINK_TIME_MS / 1000);
  }
}

function metricValue(data, name, stat = 'value') {
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
      target_events: TARGET_EVENTS,
      iteration_vus: TARGET_EVENTS > 0 ? ITERATION_VUS : null,
    },
    metrics: {
      accepted_rate: metricValue(data, 'accepted_rate', 'rate'),
      duplicate_rate: metricValue(data, 'duplicate_rate', 'rate'),
      server_error_rate: metricValue(data, 'server_error_rate', 'rate'),
      http_req_failed_rate: metricValue(data, 'http_req_failed', 'rate'),
      http_req_duration_p95_ms: metricValue(data, 'http_req_duration', 'p(95)'),
      iterations: metricValue(data, 'iterations', 'count'),
      status_accepted: metricValue(data, 'status_accepted', 'count'),
      status_duplicate: metricValue(data, 'status_duplicate', 'count'),
    },
  };

  const outPath = __ENV.SUMMARY_JSON_PATH || '';
  const rendered = JSON.stringify(summary, null, 2);

  const outputs = {
    stdout:
      `\n=== k6 load test summary ===\n` +
      `test_run_id: ${TEST_RUN_ID}\n` +
      `endpoint_profile: ${ENDPOINT_PROFILE}\n` +
      `target_events: ${TARGET_EVENTS || 'ramping'}\n` +
      `${rendered}\n`,
  };

  if (outPath) {
    outputs[outPath] = rendered;
  }

  return outputs;
}
