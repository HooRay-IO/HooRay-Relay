import http from "k6/http";
import { check } from "k6";
import { randomUUID } from "https://jslib.k6.io/k6-utils/1.4.0/index.js";

const BASE_URL = __ENV.BASE_URL;
const API_KEY = __ENV.API_KEY;
const CUSTOMER_ID = __ENV.CUSTOMER_ID || "cust_loadtest";
const TARGET_URL = __ENV.TARGET_URL || "https://httpbin.org/post";

const RATE = Number(__ENV.RATE || 500);
const DURATION = __ENV.DURATION || "2m";
const PREALLOCATED_VUS = Number(__ENV.PRE_ALLOCATED_VUS || 200);
const MAX_VUS = Number(__ENV.MAX_VUS || 400);
const SETUP_CREATE_CONFIG = __ENV.SETUP_CREATE_CONFIG === "true";

export const options = {
  scenarios: {
    steady: {
      executor: "constant-arrival-rate",
      rate: RATE,
      timeUnit: "1s",
      duration: DURATION,
      preAllocatedVUs: PREALLOCATED_VUS,
      maxVUs: MAX_VUS,
    },
  },
  thresholds: {
    http_req_failed: ["rate<0.001"],
    http_req_duration: ["p(95)<100"],
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
  if (!BASE_URL) {
    throw new Error("BASE_URL env var is required (e.g. https://<api-id>.execute-api.us-west-2.amazonaws.com/Prod)");
  }

  if (SETUP_CREATE_CONFIG) {
    const configBody = JSON.stringify({
      customer_id: CUSTOMER_ID,
      url: TARGET_URL,
    });
    const res = http.post(`${BASE_URL}/webhooks/configs`, configBody, {
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
  const payload = {
    idempotency_key: `load_${randomUUID()}`,
    customer_id: data.customerId,
    data: {
      source: "k6",
      seq: `${__VU}-${__ITER}`,
      ts: Date.now(),
    },
  };

  const res = http.post(`${BASE_URL}/webhooks/receive`, JSON.stringify(payload), {
    headers: buildHeaders(),
    tags: { endpoint: "receive" },
  });

  check(res, {
    "receive accepted/duplicate": (r) => r.status === 200 || r.status === 202,
  });
}
