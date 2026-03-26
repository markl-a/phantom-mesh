#!/usr/bin/env python3
"""
Phantom Mesh Lightweight Worker - single-file HTTP server for mobile/light devices.
Zero pip dependencies (stdlib only). Runs on Termux, a-Shell, iSH.

Usage:
    python3 phantom-mesh-worker.py --hub http://100.x.x.x:7878 --name android1 --port 7880

Supported tools: web_search, http_request, email_send
"""

import argparse
import json
import platform
import smtplib
import socket
import subprocess as _subprocess
import sys
import threading
import time
import traceback
from email.mime.text import MIMEText
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.request import Request, urlopen
from urllib.parse import urlencode, quote_plus
from urllib.error import URLError

# -- Configuration ------------------------------------------------------------

HUB_URL = ""
HUB_API_KEY = ""  # Hub auth key (Bearer token)
NODE_NAME = ""
NODE_PORT = 7880
HEARTBEAT_INTERVAL = 15  # seconds
CAPABILITIES = ["web_search", "http_request", "email_send", "shell"]
DEVICE_TYPE = "light"

# Shell security config
SHELL_TIMEOUT = 120  # seconds
SHELL_BLOCKED_PATTERNS = [
    "rm -rf /", "rm -rf /*", "mkfs", ":(){:|:&};:",
    "dd if=/dev/zero", "chmod -R 777 /", "shutdown", "reboot",
    "poweroff", "halt", "init 0", "init 6",
]

# SMTP config (from env vars or leave empty to disable email)
import os
SMTP_HOST = os.environ.get("SMTP_HOST", "")
SMTP_PORT = int(os.environ.get("SMTP_PORT", "587"))
SMTP_USER = os.environ.get("SMTP_USER", "")
SMTP_PASS = os.environ.get("SMTP_PASS", "")
SMTP_FROM = os.environ.get("SMTP_FROM", "")

# Search API key (Serper)
SERPER_API_KEY = os.environ.get("SERPER_API_KEY", "")


# -- CPU load measurement ----------------------------------------------------

_cpu_load_cache = {"value": 0.1, "ts": 0.0}
_CPU_CACHE_TTL = 10  # seconds


def get_cpu_load():
    """Get real CPU load (0.0-1.0), cached for 10 seconds."""
    now = time.time()
    if now - _cpu_load_cache["ts"] < _CPU_CACHE_TTL:
        return _cpu_load_cache["value"]

    load = 0.1  # fallback
    system = platform.system()
    try:
        if system == "Darwin" or system == "Linux":
            # getloadavg returns 1/5/15 min averages; use 1-min
            avg1 = os.getloadavg()[0]
            cpu_count = os.cpu_count() or 1
            load = min(avg1 / cpu_count, 1.0)
        elif system == "Windows":
            # wmic returns overall CPU load percentage
            out = _subprocess.check_output(
                ["wmic", "cpu", "get", "LoadPercentage", "/value"],
                timeout=5, text=True, stderr=_subprocess.DEVNULL,
            )
            for line in out.strip().splitlines():
                if line.startswith("LoadPercentage="):
                    pct = int(line.split("=")[1])
                    load = pct / 100.0
                    break
        else:
            # Fallback: try getloadavg (available on most Unix)
            try:
                avg1 = os.getloadavg()[0]
                cpu_count = os.cpu_count() or 1
                load = min(avg1 / cpu_count, 1.0)
            except (OSError, AttributeError):
                pass
    except Exception:
        pass  # keep previous cached value on error

    _cpu_load_cache["value"] = round(load, 3)
    _cpu_load_cache["ts"] = now
    return _cpu_load_cache["value"]


# -- Tool implementations -----------------------------------------------------

def web_search_tool(args):
    """Search the web using Serper API or DuckDuckGo fallback."""
    query = args.get("query", "")
    if not query:
        return {"success": False, "output": "Missing 'query' parameter"}

    # Try Serper API first
    if SERPER_API_KEY:
        try:
            req = Request(
                "https://google.serper.dev/search",
                data=json.dumps({"q": query, "num": 5}).encode(),
                headers={
                    "X-API-KEY": SERPER_API_KEY,
                    "Content-Type": "application/json",
                },
            )
            with urlopen(req, timeout=15) as resp:
                data = json.loads(resp.read().decode())
                results = []
                for item in data.get("organic", [])[:5]:
                    results.append(f"- {item.get('title', '')}: {item.get('snippet', '')} ({item.get('link', '')})")
                return {"success": True, "output": "\n".join(results) if results else "No results found"}
        except Exception as e:
            pass  # fall through to DuckDuckGo

    # DuckDuckGo instant answer fallback (no API key needed)
    try:
        url = f"https://api.duckduckgo.com/?q={quote_plus(query)}&format=json&no_html=1"
        req = Request(url, headers={"User-Agent": "Phantom MeshWorker/1.0"})
        with urlopen(req, timeout=15) as resp:
            data = json.loads(resp.read().decode())
            abstract_text = data.get("AbstractText", "")
            related = data.get("RelatedTopics", [])
            results = []
            if abstract_text:
                results.append(f"Summary: {abstract_text}")
            for topic in related[:5]:
                if isinstance(topic, dict) and "Text" in topic:
                    results.append(f"- {topic['Text']}")
            if results:
                return {"success": True, "output": "\n".join(results)}
            return {"success": True, "output": f"No instant answers for: {query}"}
    except Exception as e:
        return {"success": False, "output": f"Search failed: {e}"}


def http_request_tool(args):
    """Make an HTTP request."""
    url = args.get("url", "")
    method = args.get("method", "GET").upper()
    body = args.get("body", None)
    headers_dict = args.get("headers", {})

    if not url:
        return {"success": False, "output": "Missing 'url' parameter"}

    try:
        data = None
        if body:
            if isinstance(body, dict):
                data = json.dumps(body).encode()
                headers_dict.setdefault("Content-Type", "application/json")
            else:
                data = str(body).encode()

        req = Request(url, data=data, method=method)
        req.add_header("User-Agent", "Phantom MeshWorker/1.0")
        for k, v in headers_dict.items():
            req.add_header(k, v)

        with urlopen(req, timeout=30) as resp:
            content = resp.read().decode("utf-8", errors="replace")
            # Truncate very large responses
            if len(content) > 10000:
                content = content[:10000] + "\n... (truncated)"
            return {
                "success": True,
                "output": json.dumps({
                    "status": resp.status,
                    "headers": dict(resp.headers),
                    "body": content,
                }),
            }
    except URLError as e:
        return {"success": False, "output": f"HTTP error: {e}"}
    except Exception as e:
        return {"success": False, "output": f"Request failed: {e}"}


def email_send_tool(args):
    """Send an email via SMTP."""
    to = args.get("to", "")
    subject = args.get("subject", "")
    body = args.get("body", "")

    if not to or not subject:
        return {"success": False, "output": "Missing 'to' or 'subject' parameter"}
    if not SMTP_HOST:
        return {"success": False, "output": "SMTP not configured (set SMTP_HOST, SMTP_USER, SMTP_PASS env vars)"}

    try:
        msg = MIMEText(body, "plain", "utf-8")
        msg["Subject"] = subject
        msg["From"] = SMTP_FROM or SMTP_USER
        msg["To"] = to

        with smtplib.SMTP(SMTP_HOST, SMTP_PORT) as server:
            server.starttls()
            if SMTP_USER and SMTP_PASS:
                server.login(SMTP_USER, SMTP_PASS)
            server.send_message(msg)

        return {"success": True, "output": f"Email sent to {to}"}
    except Exception as e:
        return {"success": False, "output": f"Email failed: {e}"}


def shell_tool(args):
    """Execute a shell command with security restrictions."""
    import subprocess

    command = args.get("command", "")
    if not command:
        return {"success": False, "output": "Missing 'command' parameter"}

    # Security: block dangerous patterns
    cmd_lower = command.lower().strip()
    for pattern in SHELL_BLOCKED_PATTERNS:
        if pattern in cmd_lower:
            return {"success": False, "output": f"Blocked: command matches dangerous pattern '{pattern}'"}

    timeout = min(args.get("timeout", SHELL_TIMEOUT), SHELL_TIMEOUT)
    cwd = args.get("cwd", None)

    try:
        print(f"[{time.strftime('%H:%M:%S')}] Shell: {command[:100]}")
        result = subprocess.run(
            command,
            shell=True,
            capture_output=True,
            text=True,
            timeout=timeout,
            cwd=cwd,
        )
        output = result.stdout or ""
        if result.stderr:
            output += f"\n[stderr]\n{result.stderr}"

        # Truncate long output
        if len(output) > 50000:
            output = output[:50000] + f"\n... (truncated, {len(output)} total chars)"

        return {
            "success": result.returncode == 0,
            "output": output or f"(exit code {result.returncode})",
        }
    except subprocess.TimeoutExpired:
        return {"success": False, "output": f"Command timed out after {timeout}s"}
    except Exception as e:
        return {"success": False, "output": f"Shell error: {e}"}


# Tool dispatch table
TOOLS = {
    "web_search": web_search_tool,
    "http_request": http_request_tool,
    "email_send": email_send_tool,
    "shell": shell_tool,
}


# -- HTTP Server --------------------------------------------------------------

class WorkerHandler(BaseHTTPRequestHandler):
    """HTTP request handler for the lightweight worker."""

    def log_message(self, format, *args):
        # Suppress default logging, use our own
        print(f"[{time.strftime('%H:%M:%S')}] {format % args}")

    def do_GET(self):
        if self.path == "/health":
            self._respond(200, {
                "status": "ok",
                "name": NODE_NAME,
                "capabilities": CAPABILITIES,
                "device_type": DEVICE_TYPE,
                "version": "1.0.0-py",
            })
        elif self.path == "/worker/status":
            self._respond(200, {
                "name": NODE_NAME,
                "hub": HUB_URL,
                "capabilities": CAPABILITIES,
                "device_type": DEVICE_TYPE,
                "tools_available": list(TOOLS.keys()),
                "version": "1.0.0-py",
            })
        else:
            self._respond(404, {"error": "not found"})

    def do_POST(self):
        if self.path == "/worker/execute":
            try:
                length = int(self.headers.get("Content-Length", 0))
                body = json.loads(self.rfile.read(length).decode())
                tool_name = body.get("tool", "")
                tool_input = body.get("input", {})

                if tool_name not in TOOLS:
                    self._respond(200, {
                        "success": False,
                        "output": f"Unknown tool: {tool_name}. Available: {list(TOOLS.keys())}",
                    })
                    return

                print(f"[{time.strftime('%H:%M:%S')}] Executing tool: {tool_name}")
                result = TOOLS[tool_name](tool_input)
                self._respond(200, result)
            except Exception as e:
                traceback.print_exc()
                self._respond(200, {
                    "success": False,
                    "output": f"Execution error: {e}",
                })
        else:
            self._respond(404, {"error": "not found"})

    def _respond(self, status, data):
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(json.dumps(data).encode())


# -- Hub Registration & Heartbeat ---------------------------------------------

def register_with_hub():
    """Register this worker with the hub."""
    if not HUB_URL:
        return

    url = f"{HUB_URL}/cluster/register"
    payload = json.dumps({
        "name": NODE_NAME,
        "host": get_local_ip(),
        "port": NODE_PORT,
        "capabilities": CAPABILITIES,
        "device_type": DEVICE_TYPE,
    }).encode()

    headers = {"Content-Type": "application/json"}
    if HUB_API_KEY:
        headers["Authorization"] = f"Bearer {HUB_API_KEY}"

    try:
        req = Request(url, data=payload, headers=headers)
        with urlopen(req, timeout=10) as resp:
            result = json.loads(resp.read().decode())
            print(f"Registered with hub: {result}")
    except Exception as e:
        print(f"Registration failed (will retry via heartbeat): {e}")


def heartbeat_loop():
    """Send heartbeat to hub every HEARTBEAT_INTERVAL seconds."""
    if not HUB_URL:
        return

    while True:
        time.sleep(HEARTBEAT_INTERVAL)
        url = f"{HUB_URL}/cluster/heartbeat"
        payload = json.dumps({
            "name": NODE_NAME,
            "cpu_load": get_cpu_load(),
        }).encode()

        headers = {"Content-Type": "application/json"}
        if HUB_API_KEY:
            headers["Authorization"] = f"Bearer {HUB_API_KEY}"

        try:
            req = Request(url, data=payload, headers=headers)
            with urlopen(req, timeout=5) as resp:
                pass  # heartbeat ok
        except Exception:
            pass  # silently retry next interval


def get_local_ip():
    """Get the local IP address."""
    try:
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.connect(("8.8.8.8", 80))
        ip = s.getsockname()[0]
        s.close()
        return ip
    except Exception:
        return "127.0.0.1"


# -- Main --------------------------------------------------------------------

def main():
    global HUB_URL, HUB_API_KEY, NODE_NAME, NODE_PORT

    parser = argparse.ArgumentParser(description="Phantom Mesh Lightweight Worker")
    parser.add_argument("--hub", required=True, help="Hub URL (e.g., http://10.0.1.1:7878)")
    parser.add_argument("--hub-key", default="your-hub-token-here", help="Hub API key (default: your-hub-token-here)")
    parser.add_argument("--name", default=None, help="Worker name (default: auto-detect)")
    parser.add_argument("--port", type=int, default=7880, help="Port to listen on (default: 7880)")
    args = parser.parse_args()

    HUB_URL = args.hub.rstrip("/")
    HUB_API_KEY = args.hub_key
    NODE_NAME = args.name or socket.gethostname() or f"light-{args.port}"
    NODE_PORT = args.port

    print(f"=== Phantom Mesh Light Worker '{NODE_NAME}' ===")
    print(f"Hub:          {HUB_URL}")
    print(f"Port:         {NODE_PORT}")
    print(f"Capabilities: {CAPABILITIES}")
    print(f"Tools:        {list(TOOLS.keys())}")
    print()

    # Register with hub
    register_with_hub()

    # Start heartbeat in background
    heartbeat_thread = threading.Thread(target=heartbeat_loop, daemon=True)
    heartbeat_thread.start()

    # Start HTTP server
    server = HTTPServer(("0.0.0.0", NODE_PORT), WorkerHandler)
    print(f"Listening on http://0.0.0.0:{NODE_PORT}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nShutting down...")
        server.shutdown()


if __name__ == "__main__":
    main()
