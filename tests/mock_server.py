#!/usr/bin/env python3
"""Lightweight mock webhook destination for delivery testing.

Controls (env vars):
- MOCK_PORT (default: 8089)
- MOCK_LATENCY_MS (default: 50)
- MOCK_FAIL_RATE (default: 0.0)  # 0.0 - 1.0
- MOCK_TIMEOUT_RATE (default: 0.0)  # 0.0 - 1.0
- MOCK_TIMEOUT_MS (default: 2000)
"""

from __future__ import annotations

import json
import os
import random
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
import sys

PORT = int(os.getenv("MOCK_PORT", "8089"))
LATENCY_MS = int(os.getenv("MOCK_LATENCY_MS", "50"))


def _get_rate(env_name: str, default: float) -> float:
    """Read a probability-like rate from the environment and validate it is in [0.0, 1.0]."""
    raw = os.getenv(env_name)
    if raw is None:
        return default
    try:
        value = float(raw)
    except ValueError:
        raise SystemExit(f"{env_name} must be a float between 0.0 and 1.0, got {raw!r}")
    if not 0.0 <= value <= 1.0:
        raise SystemExit(f"{env_name} must be between 0.0 and 1.0, got {value}")
    return value


FAIL_RATE = _get_rate("MOCK_FAIL_RATE", 0.0)
TIMEOUT_RATE = _get_rate("MOCK_TIMEOUT_RATE", 0.0)
TIMEOUT_MS = int(os.getenv("MOCK_TIMEOUT_MS", "2000"))


class MockHandler(BaseHTTPRequestHandler):
    server_version = "HooRayMock/1.0"

    def _sleep_ms(self, ms: int) -> None:
        if ms > 0:
            time.sleep(ms / 1000.0)

    def do_POST(self) -> None:  # noqa: N802
        request_id = f"req_{int(time.time() * 1000)}_{random.randint(1000, 9999)}"
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length > 0 else b""

        if random.random() < TIMEOUT_RATE:
            self._sleep_ms(TIMEOUT_MS)
            return

        self._sleep_ms(LATENCY_MS)

        if random.random() < FAIL_RATE:
            self.send_response(500)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            payload = {
                "status": "error",
                "request_id": request_id,
                "message": "simulated failure",
            }
            self.wfile.write(json.dumps(payload).encode("utf-8"))
            return

        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        payload = {
            "status": "ok",
            "request_id": request_id,
            "received_bytes": len(body),
        }
        self.wfile.write(json.dumps(payload).encode("utf-8"))

    def log_message(self, format: str, *args) -> None:  # noqa: A003
        return


def run() -> None:
    server = HTTPServer(("0.0.0.0", PORT), MockHandler)
    print(
        "Mock server running on port",
        PORT,
        "| latency",
        f"{LATENCY_MS}ms",
        "| fail_rate",
        FAIL_RATE,
        "| timeout_rate",
        TIMEOUT_RATE,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    run()
