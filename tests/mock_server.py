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

PORT = int(os.getenv("MOCK_PORT", "8089"))
LATENCY_MS = int(os.getenv("MOCK_LATENCY_MS", "50"))
FAIL_RATE = float(os.getenv("MOCK_FAIL_RATE", "0.0"))
TIMEOUT_RATE = float(os.getenv("MOCK_TIMEOUT_RATE", "0.0"))
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
