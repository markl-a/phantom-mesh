#!/usr/bin/env python3
"""
Clawtex Money Agent - E2E Test Suite (US-012)
Tests all new Stage 3 features via HTTP API and Telegram (pyautogui).

Usage:
    python test_money_agent.py

Prerequisites:
    - clawtex-core daemon running on http://127.0.0.1:7878
    - Telegram Desktop open on right side of screen
    - pip install pyautogui pillow requests
"""

import json
import os
import sys
import time
import traceback
from datetime import datetime
from pathlib import Path

import pyautogui
import requests

# ── Configuration ──────────────────────────────────────────────────────────────

API_BASE = "http://127.0.0.1:7878"
SCREENSHOT_DIR = Path.home() / "Desktop" / "test_money_agent"
SCREENSHOT_DIR.mkdir(exist_ok=True)

# Test results tracking
results = []
pass_count = 0
fail_count = 0


def screenshot(name: str):
    """Take a screenshot and save it."""
    path = SCREENSHOT_DIR / f"{name}_{datetime.now().strftime('%H%M%S')}.png"
    pyautogui.screenshot(str(path))
    print(f"  Screenshot: {path}")
    return path


def test(name: str, func):
    """Run a test and track results."""
    global pass_count, fail_count
    print(f"\n{'='*60}")
    print(f"TEST: {name}")
    print(f"{'='*60}")
    try:
        func()
        print(f"  [OK] PASS: {name}")
        results.append(("PASS", name))
        pass_count += 1
    except Exception as e:
        print(f"  [FAIL] FAIL: {name}")
        print(f"  Error: {e}")
        traceback.print_exc()
        results.append(("FAIL", name, str(e)))
        fail_count += 1
        screenshot(f"FAIL_{name.replace(' ', '_')}")


# ── HTTP API Tests ─────────────────────────────────────────────────────────────

def test_health():
    """Test /health endpoint."""
    resp = requests.get(f"{API_BASE}/health", timeout=5)
    assert resp.status_code == 200, f"Expected 200, got {resp.status_code}"
    data = resp.json()
    assert data["status"] == "ok", f"Expected ok, got {data['status']}"
    assert "version" in data, "Missing version field"
    print(f"  Version: {data['version']}")


def test_tools_list():
    """Test /tools endpoint - verify new tools are registered."""
    resp = requests.get(f"{API_BASE}/tools", timeout=5)
    assert resp.status_code == 200
    data = resp.json()
    tool_names = [t["name"] for t in data["tools"]]
    print(f"  Tools ({len(tool_names)}): {', '.join(sorted(tool_names))}")

    # Check for new tools
    expected_tools = [
        "browser",       # US-001
        "file_edit",     # Sprint 2
        "http_request",  # Sprint 2
        "memory_store",  # Sprint 2
        "memory_recall", # Sprint 2
        "memory_forget", # Sprint 2
        "glob_search",   # Sprint 2
        "content_search",# Sprint 2
    ]
    for tool in expected_tools:
        assert tool in tool_names, f"Missing tool: {tool}"

    assert len(tool_names) >= 15, f"Expected >=15 tools, got {len(tool_names)}"


def test_estop_lifecycle():
    """Test E-Stop activate → status → reset lifecycle."""
    # Activate
    resp = requests.post(f"{API_BASE}/estop", timeout=5)
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "stopped", f"Expected stopped, got {data}"

    # Status check
    resp = requests.get(f"{API_BASE}/estop", timeout=5)
    assert resp.status_code == 200
    data = resp.json()
    assert data["stopped"] == True, f"Expected stopped=True"

    # Reset
    resp = requests.delete(f"{API_BASE}/estop", timeout=5)
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "running", f"Expected running after reset"

    # Verify reset
    resp = requests.get(f"{API_BASE}/estop", timeout=5)
    data = resp.json()
    assert data["stopped"] == False, f"Expected stopped=False after reset"


def test_agent_run():
    """Test /agent/:name/run endpoint."""
    resp = requests.post(
        f"{API_BASE}/agent/master/run",
        json={"prompt": "Say exactly 'hello world' and nothing else."},
        timeout=60,
    )
    assert resp.status_code == 200, f"Agent run failed: {resp.status_code} {resp.text}"
    data = resp.json()
    assert "result" in data, f"Missing result field: {data}"
    result_text = data['result'][:100].encode('ascii', 'replace').decode('ascii')
    print(f"  Agent response: {result_text}...")


# ── Telegram UI Tests (pyautogui) ─────────────────────────────────────────────

def find_telegram_input():
    """Find Telegram message input area and click it."""
    # Try to find Telegram window - should be on right side
    # Click on the message input area (bottom of Telegram)
    screen_w, screen_h = pyautogui.size()
    # Telegram on right side: try right 40% of screen, bottom 10%
    input_x = int(screen_w * 0.75)
    input_y = int(screen_h * 0.92)
    pyautogui.click(input_x, input_y)
    time.sleep(0.3)
    return input_x, input_y


def send_telegram(message: str, wait_secs: int = 15):
    """Send a message via Telegram Desktop UI."""
    find_telegram_input()
    time.sleep(0.3)
    pyautogui.typewrite(message, interval=0.02)
    time.sleep(0.2)
    pyautogui.press("enter")
    print(f"  Sent: {message}")
    time.sleep(wait_secs)  # Wait for response


def test_telegram_help():
    """Test /help command via Telegram."""
    send_telegram("/help", wait_secs=5)
    screenshot("telegram_help")


def test_telegram_status():
    """Test /status command via Telegram."""
    send_telegram("/status", wait_secs=5)
    screenshot("telegram_status")


def test_telegram_tools():
    """Test /tools command via Telegram."""
    send_telegram("/tools", wait_secs=5)
    screenshot("telegram_tools")


def test_telegram_hands():
    """Test /hands command via Telegram."""
    send_telegram("/hands", wait_secs=5)
    screenshot("telegram_hands")


def test_telegram_estop():
    """Test /estop and /resume via Telegram."""
    send_telegram("/estop", wait_secs=3)
    screenshot("telegram_estop")
    # Try sending a message while stopped
    send_telegram("This should be blocked", wait_secs=3)
    screenshot("telegram_estop_blocked")
    # Resume
    send_telegram("/resume", wait_secs=3)
    screenshot("telegram_resume")


def test_telegram_clear():
    """Test /clear command via Telegram."""
    send_telegram("/clear", wait_secs=3)
    screenshot("telegram_clear")


# ── Main ───────────────────────────────────────────────────────────────────────

def main():
    print("=" * 60)
    print("Clawtex Money Agent - E2E Test Suite")
    print(f"Screenshots: {SCREENSHOT_DIR}")
    print(f"API: {API_BASE}")
    print("=" * 60)

    # Check daemon is running
    try:
        requests.get(f"{API_BASE}/health", timeout=3)
    except Exception:
        print("ERROR: Daemon not running! Start with: cargo run --release")
        sys.exit(1)

    # ── HTTP API Tests ──
    test("Health endpoint", test_health)
    test("Tools list (15+ tools)", test_tools_list)
    test("E-Stop lifecycle", test_estop_lifecycle)
    test("Agent run", test_agent_run)

    # ── Telegram UI Tests ──
    print("\n\n--- TELEGRAM UI TESTS ---")
    print("Make sure Telegram Desktop is visible on the right side of screen")
    time.sleep(2)

    test("Telegram /help", test_telegram_help)
    test("Telegram /status", test_telegram_status)
    test("Telegram /tools", test_telegram_tools)
    test("Telegram /hands", test_telegram_hands)
    test("Telegram /estop + /resume", test_telegram_estop)
    test("Telegram /clear", test_telegram_clear)

    # ── Summary ──
    print("\n\n" + "=" * 60)
    print("TEST SUMMARY")
    print("=" * 60)
    for r in results:
        status = r[0]
        name = r[1]
        extra = f" - {r[2]}" if len(r) > 2 else ""
        icon = "[OK]" if status == "PASS" else "[FAIL]"
        print(f"  {icon} {status}: {name}{extra}")

    print(f"\nTotal: {pass_count + fail_count} tests, {pass_count} passed, {fail_count} failed")
    print(f"Screenshots saved to: {SCREENSHOT_DIR}")

    # Take final screenshot
    screenshot("final_summary")

    return 0 if fail_count == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
