#!/usr/bin/env python3
"""
Minimal OpenAI-compatible HTTP server for deterministic phantom tests.

Implements the subset phantom actually calls:
  GET  /v1/models                 -> list available mock models
  POST /v1/chat/completions       -> scripted reply, streaming or not

Usage:
    python3 mock-llm-server.py \
        --port 11999 \
        --responses fixtures/mock-responses.toml

The responses file is TOML:
    [[response]]
    prompt_match = "ping"          # exact substring (case-insensitive)
    text = "pong"
    delay_ms = 0                   # optional fake latency

    [[response]]
    prompt_regex = "(?i)reply.*ok" # alternative: regex; first match wins
    text = "ok"
    delay_ms = 200

    [default]
    text = "MOCK: no scripted response matched"

Matching strategy:
  1. iterate `[[response]]` entries in order
  2. first one whose `prompt_match` (substring, ci) OR `prompt_regex` (re.search)
     matches against the LAST user message wins
  3. if nothing matches and a `[default]` exists, return that
  4. otherwise return "MOCK: no scripted response matched"

The server is single-threaded and blocking — fine for sequential test
scenarios, NOT for benchmarking. Logs every request to stderr so test
harnesses can sanity-check what was hit.

stdlib-only: no pip install needed. Uses tomllib (Python 3.11+) or
falls back to a tiny inline TOML reader for older Python.

Exit cleanly on SIGINT/SIGTERM so wrapping shell scripts can `kill $!`
without leaving zombies.
"""

import argparse
import http.server
import json
import re
import signal
import sys
import time
from pathlib import Path

try:
    import tomllib
    def load_toml(path):
        with open(path, 'rb') as f:
            return tomllib.load(f)
except ImportError:
    # Python < 3.11 fallback: VERY minimal TOML parser handling only
    # the [[response]] / [default] shape this file uses.
    def load_toml(path):
        out = {'response': [], 'default': None}
        cur = None
        with open(path) as f:
            for raw in f:
                line = raw.strip()
                if not line or line.startswith('#'):
                    continue
                if line == '[[response]]':
                    cur = {}
                    out['response'].append(cur)
                    continue
                if line == '[default]':
                    out['default'] = {}
                    cur = out['default']
                    continue
                if '=' in line:
                    k, _, v = line.partition('=')
                    k = k.strip()
                    v = v.strip().strip('"').strip("'")
                    if v.isdigit():
                        v = int(v)
                    if cur is not None:
                        cur[k] = v
        return out


class MockHandler(http.server.BaseHTTPRequestHandler):
    responses = []          # list of dicts (filled at startup)
    default_text = "MOCK: no scripted response matched"
    models = ["mock-instant", "mock-thoughtful"]

    def log_message(self, fmt, *args):
        sys.stderr.write("[mock-llm] %s\n" % (fmt % args))

    def _json(self, obj, code=200):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _sse_chunk(self, delta_content):
        chunk = {
            "choices": [{
                "index": 0,
                "delta": {"content": delta_content},
                "finish_reason": None,
            }],
        }
        line = ("data: " + json.dumps(chunk) + "\n\n").encode()
        self.wfile.write(line)
        self.wfile.flush()

    def _pick_response(self, user_text):
        for r in self.responses:
            sub = r.get("prompt_match")
            if sub and sub.lower() in user_text.lower():
                return r
            rx = r.get("prompt_regex")
            if rx and re.search(rx, user_text):
                return r
        return {"text": self.default_text}

    def do_GET(self):
        if self.path.startswith("/v1/models"):
            self._json({
                "object": "list",
                "data": [{"id": m, "object": "model", "owned_by": "mock"}
                         for m in self.models],
            })
        elif self.path.startswith("/healthz"):
            body = b"ok"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_error(404, "not found")

    def do_POST(self):
        if not self.path.startswith("/v1/chat/completions"):
            self.send_error(404, "not found")
            return
        n = int(self.headers.get("Content-Length", "0"))
        try:
            req = json.loads(self.rfile.read(n).decode("utf-8"))
        except Exception as e:
            self.send_error(400, "bad json: %s" % e)
            return

        # Last user message text.
        user_text = ""
        for m in reversed(req.get("messages", [])):
            if m.get("role") == "user":
                user_text = m.get("content", "")
                if isinstance(user_text, list):
                    # OpenAI multi-modal content array — pull text parts.
                    user_text = " ".join(
                        p.get("text", "") for p in user_text
                        if isinstance(p, dict) and p.get("type") == "text"
                    )
                break

        chosen = self._pick_response(user_text)
        text = chosen.get("text", self.default_text)
        delay_ms = int(chosen.get("delay_ms", 0))
        if delay_ms > 0:
            time.sleep(delay_ms / 1000.0)

        if req.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            # Emit content in two chunks so consumers exercise the chunked path.
            mid = max(1, len(text) // 2)
            self._sse_chunk(text[:mid])
            self._sse_chunk(text[mid:])
            self.wfile.write(b"data: [DONE]\n\n")
            self.wfile.flush()
        else:
            self._json({
                "id": "mock-" + str(int(time.time() * 1000)),
                "object": "chat.completion",
                "model": req.get("model", "mock-instant"),
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": text},
                    "finish_reason": "stop",
                }],
                "usage": {
                    "prompt_tokens": max(1, len(user_text) // 4),
                    "completion_tokens": max(1, len(text) // 4),
                    "total_tokens": max(2, (len(user_text) + len(text)) // 4),
                },
            })


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=11999)
    ap.add_argument("--responses", type=Path, required=True)
    args = ap.parse_args()

    if not args.responses.exists():
        sys.stderr.write("responses file not found: %s\n" % args.responses)
        sys.exit(2)

    cfg = load_toml(args.responses)
    MockHandler.responses = cfg.get("response", [])
    if cfg.get("default"):
        MockHandler.default_text = cfg["default"].get("text", MockHandler.default_text)

    server = http.server.HTTPServer(("127.0.0.1", args.port), MockHandler)

    def shutdown(_signo, _frame):
        sys.stderr.write("[mock-llm] shutting down\n")
        server.shutdown()
    signal.signal(signal.SIGINT, shutdown)
    signal.signal(signal.SIGTERM, shutdown)

    sys.stderr.write("[mock-llm] listening on http://127.0.0.1:%d (responses=%s)\n"
                     % (args.port, args.responses))
    server.serve_forever()


if __name__ == "__main__":
    main()
