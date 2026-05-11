#!/usr/bin/env python3
"""
Mac → iPhone secure one-shot identity transfer.

Spins up an HTTP server bound to Mac's Tailscale IP (NOT 0.0.0.0 — only
authenticated tailnet peers can reach it), serves ONE landing page at a
random-nonce URL, then self-shuts. Page links to phantom://oauth/callback?
p=<base64> with the Mac's saved broker_token + email so the iPhone app's
broker_login_finish + auto-sync runs and pulls 9 vault keys + cluster
peers into the iPhone sandbox.

Threat model:
- Tailscale tailnet is the auth boundary (your devices only).
- Random URL path nonce stops scanners on the tailnet from harvesting.
- Self-shutdown after first successful transfer = no replay window.
- 5-minute TTL kills the server even if no one connects.

Usage:
    python3 scripts/transfer-to-mobile.py
    → prints URL to type into iPhone Safari address bar
    → tap "點我匯入" → Safari "Open in Phantom Mesh?" → Open
    → done

Auth source: ~/.phantom-mesh/auth.json (must have broker_token).
"""

import base64
import http.server
import json
import secrets
import subprocess
import sys
import threading
import time
from pathlib import Path

# ── Config ──────────────────────────────────────────────────────────────
PORT = 8889
TTL_SECS = 300
AUTH_PATH = Path.home() / ".phantom-mesh" / "auth.json"


def tailscale_ip() -> str:
    """Get this Mac's Tailscale IPv4. Errors out if Tailscale is down."""
    candidates = [
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ]
    for cmd in candidates:
        if Path(cmd).exists():
            try:
                out = subprocess.check_output([cmd, "ip", "-4"], timeout=3).decode().strip()
                ip = out.splitlines()[0].strip()
                if ip:
                    return ip
            except Exception:
                continue
    raise SystemExit("[transfer] could not resolve Tailscale IP — is Tailscale running?")


def load_payload() -> str:
    """Read Mac's broker_token from auth.json + UTF-8 base64url-encode the
    full CliPayload shape the iOS app's broker_login_finish expects."""
    if not AUTH_PATH.exists():
        raise SystemExit(
            f"[transfer] {AUTH_PATH} not found — log in on Mac first via "
            "`phantom login`."
        )
    state = json.loads(AUTH_PATH.read_text())
    if not state.get("broker_token"):
        raise SystemExit("[transfer] auth.json has no broker_token")
    cli_payload = {
        "provider": state.get("provider", "google"),
        "email": state["email"],
        "sub": state.get("sub"),
        "name": state.get("display_name"),
        "picture": state.get("avatar_url"),
        "id_token": state.get("id_token", ""),
        "access_token": state.get("access_token", ""),
        "broker_token": state["broker_token"],
        "broker_token_expires_at_ms": state.get("broker_token_expires_at_ms", 0),
    }
    raw = json.dumps(cli_payload).encode("utf-8")
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


# ── Per-launch state ────────────────────────────────────────────────────
NONCE = secrets.token_urlsafe(16)
PATH_PREFIX = f"/t/{NONCE}"
SHUTDOWN_FLAG = threading.Event()


def html_page(deep_url: str) -> str:
    return f"""<!doctype html><html lang="zh-Hant"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Phantom Mesh 一鍵匯入</title>
<style>
*{{box-sizing:border-box}}
body{{font-family:-apple-system,system-ui,sans-serif;background:#0c0c10;color:#e8e2d4;
     margin:0;padding:24px;text-align:center;min-height:100vh}}
h2{{color:#d6b270;margin:32px 0 8px}}
p{{color:#8a8578;font-size:14px;line-height:1.5}}
.btn{{display:inline-block;background:#d6b270;color:#0c0c10;padding:18px 32px;
     border-radius:10px;text-decoration:none;font-weight:600;font-size:18px;
     margin-top:24px}}
.foot{{color:#4d4a42;font-size:11px;margin-top:48px;line-height:1.5}}
</style></head><body>
<h2>◆ Phantom Mesh</h2>
<p>把 Mac 的 broker_token + LLM keys + cluster peers 一次寫進這台 iPhone 的沙盒</p>
<a class="btn" href="{deep_url}">點我匯入 →</a>
<p class="foot">
Safari 會跳「在 Phantom Mesh 中開啟？」對話框 — 按 Open。<br>
這個 URL 一次性，匯入後 server 自動關閉。
</p></body></html>"""


class Handler(http.server.BaseHTTPRequestHandler):
    deep_url: str = ""

    def do_GET(self):  # noqa: N802
        if self.path != PATH_PREFIX:
            self.send_response(404)
            self.end_headers()
            self.wfile.write(b"not found\n")
            return
        body = html_page(Handler.deep_url).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)
        # Schedule shutdown — give the response time to flush + the
        # phantom:// click to fire before tearing down.
        threading.Timer(2.0, lambda: SHUTDOWN_FLAG.set()).start()

    def log_message(self, format, *args):  # noqa: A002 — match base class signature
        sys.stderr.write(f"[transfer] {self.address_string()} - {format % args}\n")


def main():
    bind_ip = tailscale_ip()
    payload = load_payload()
    deep_url = f"phantom://oauth/callback?p={payload}"
    transfer_url = f"http://{bind_ip}:{PORT}{PATH_PREFIX}"
    Handler.deep_url = deep_url

    server = http.server.HTTPServer((bind_ip, PORT), Handler)

    bar = "═" * 60
    print()
    print(bar)
    print("  Phantom Mesh — Mac → iPhone identity transfer")
    print(bar)
    print()
    print(f"  打開 iPhone Safari 並輸入：")
    print()
    print(f"    {transfer_url}")
    print()
    print(f"  伺服器只接受路徑 {PATH_PREFIX}（隨機 nonce）")
    print(f"  bound to Tailscale {bind_ip} 而非 0.0.0.0")
    print(f"  TTL {TTL_SECS}s，傳完即 shutdown")
    print()
    print(bar)

    # TTL killer
    def kill_after_ttl():
        time.sleep(TTL_SECS)
        if not SHUTDOWN_FLAG.is_set():
            print(f"\n[transfer] TTL {TTL_SECS}s 到，server 關閉")
            SHUTDOWN_FLAG.set()
            server.shutdown()

    threading.Thread(target=kill_after_ttl, daemon=True).start()

    # Watcher: when SHUTDOWN_FLAG fires, stop the server
    def watch_shutdown():
        SHUTDOWN_FLAG.wait()
        time.sleep(0.5)  # let final response flush
        server.shutdown()

    threading.Thread(target=watch_shutdown, daemon=True).start()

    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\n[transfer] cancelled")
    finally:
        server.server_close()
        if SHUTDOWN_FLAG.is_set():
            print("[transfer] server closed (transfer complete or TTL hit)")


if __name__ == "__main__":
    main()
