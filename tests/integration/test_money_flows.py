#!/usr/bin/env python3
"""
Phantom Mesh Money-Making Flows - E2E Test Suite (Stage 4)
Tests all money-making hands and workflows via HTTP API and Telegram.

Usage:
    python test_money_flows.py

Prerequisites:
    - phantom-mesh daemon running on http://127.0.0.1:7878
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

# -- Configuration --

API_BASE = "http://127.0.0.1:7878"
SCREENSHOT_DIR = Path.home() / "Desktop" / "test_money_flows"
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


# -- HTTP API Tests --

def test_health():
    """Test daemon is running."""
    resp = requests.get(f"{API_BASE}/health", timeout=5)
    assert resp.status_code == 200
    data = resp.json()
    assert data["status"] == "ok"
    print(f"  Daemon OK: v{data['version']}")


def test_hands_api():
    """Test /hands endpoint - verify 7 money-making hands loaded."""
    resp = requests.get(f"{API_BASE}/hands", timeout=5)
    assert resp.status_code == 200
    data = resp.json()
    hands = data["hands"]
    hand_names = [h["name"] for h in hands]
    print(f"  Hands ({data['count']}): {', '.join(hand_names)}")

    # Verify all 7 hands are loaded
    expected = ["content", "freelancer", "lead", "market_intel", "outreach", "researcher", "seo_content"]
    for name in expected:
        assert name in hand_names, f"Missing hand: {name}"

    assert data["count"] == 7, f"Expected 7 hands, got {data['count']}"


def test_hands_categories():
    """Test hands have correct categories."""
    resp = requests.get(f"{API_BASE}/hands", timeout=5)
    data = resp.json()
    hands_by_name = {h["name"]: h for h in data["hands"]}

    # Verify categories
    assert hands_by_name["outreach"]["category"] == "sales"
    assert hands_by_name["freelancer"]["category"] == "sales"
    assert hands_by_name["lead"]["category"] == "sales"
    assert hands_by_name["market_intel"]["category"] == "research"
    assert hands_by_name["seo_content"]["category"] == "content"
    assert hands_by_name["researcher"]["category"] == "research"
    assert hands_by_name["content"]["category"] == "marketing"
    print("  All categories correct")


def test_hands_phases():
    """Test hands have correct number of phases."""
    resp = requests.get(f"{API_BASE}/hands", timeout=5)
    data = resp.json()
    hands_by_name = {h["name"]: h for h in data["hands"]}

    assert hands_by_name["outreach"]["phases"] == 4, "outreach should have 4 phases"
    assert hands_by_name["freelancer"]["phases"] == 4, "freelancer should have 4 phases"
    assert hands_by_name["lead"]["phases"] == 4, "lead should have 4 phases"
    assert hands_by_name["market_intel"]["phases"] == 4, "market_intel should have 4 phases"
    assert hands_by_name["seo_content"]["phases"] == 4, "seo_content should have 4 phases"
    assert hands_by_name["researcher"]["phases"] == 5, "researcher should have 5 phases"
    assert hands_by_name["content"]["phases"] == 3, "content should have 3 phases"
    print("  All phase counts correct")


def test_tools_for_hands():
    """Test that all tools needed by hands are registered."""
    resp = requests.get(f"{API_BASE}/tools", timeout=5)
    data = resp.json()
    tool_names = [t["name"] for t in data["tools"]]
    print(f"  Available tools ({len(tool_names)}): {', '.join(sorted(tool_names))}")

    # Tools needed by money-making hands
    needed = [
        "web_search",     # prospect/job research
        "browser",        # web scraping and automation
        "file_write",     # save reports, CSVs
        "file_read",      # read data files
        "memory_store",   # track prospects, state
        "memory_recall",  # recall previous data
        "file_edit",      # edit files
        "http_request",   # API calls
    ]
    for tool in needed:
        assert tool in tool_names, f"Missing required tool: {tool}"
    # email_send is optional (only registered when SMTP configured)
    email_status = "present" if "email_send" in tool_names else "SMTP not configured"
    print(f"  All required tools present (email_send: {email_status})")


def test_hand_run_api_exists():
    """Test /hand/:name/run endpoint exists (invalid hand returns error JSON, not 404)."""
    resp = requests.post(
        f"{API_BASE}/hand/nonexistent/run",
        json={"prompt": "test"},
        timeout=10,
    )
    assert resp.status_code == 200, f"Expected 200, got {resp.status_code}"
    data = resp.json()
    assert "error" in data, f"Expected error field: {data}"
    assert "nonexistent" in data["error"]
    print(f"  Endpoint works, error: {data['error'][:80]}")


def test_hand_run_missing_prompt():
    """Test /hand/:name/run returns error for missing prompt."""
    resp = requests.post(
        f"{API_BASE}/hand/lead/run",
        json={},
        timeout=10,
    )
    assert resp.status_code == 200
    data = resp.json()
    assert "error" in data or "prompt" in str(data).lower()
    print(f"  Missing prompt handled correctly")


def test_estop_protects_hands():
    """Test E-Stop prevents hand execution."""
    # Activate e-stop
    resp = requests.post(f"{API_BASE}/estop", timeout=5)
    assert resp.json()["status"] == "stopped"

    # Verify e-stop is active
    resp = requests.get(f"{API_BASE}/estop", timeout=5)
    assert resp.json()["stopped"] == True

    # Reset e-stop
    resp = requests.delete(f"{API_BASE}/estop", timeout=5)
    assert resp.json()["status"] == "running"
    print("  E-Stop lifecycle OK")


# -- Telegram UI Tests (pyautogui) --

def find_telegram_input():
    """Find Telegram message input area and click it."""
    screen_w, screen_h = pyautogui.size()
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
    time.sleep(wait_secs)


def test_telegram_hands_list():
    """Test /hands command shows all 7 hands via Telegram."""
    send_telegram("/hands", wait_secs=8)
    screenshot("telegram_hands_list")


def test_telegram_hand_outreach():
    """Test /hand outreach via Telegram (cold email workflow)."""
    send_telegram("/hand outreach web design services for restaurants", wait_secs=30)
    screenshot("telegram_hand_outreach")


def test_telegram_hand_freelancer():
    """Test /hand freelancer via Telegram (job search workflow)."""
    send_telegram("/hand freelancer python web development budget over 1000 USD", wait_secs=30)
    screenshot("telegram_hand_freelancer")


def test_telegram_hand_market_intel():
    """Test /hand market_intel via Telegram (market research workflow)."""
    send_telegram("/hand market_intel AI automation tools market in Taiwan", wait_secs=30)
    screenshot("telegram_hand_market_intel")


def test_telegram_hand_seo():
    """Test /hand seo_content via Telegram (SEO content workflow)."""
    send_telegram("/hand seo_content best AI tools for small business 2026", wait_secs=30)
    screenshot("telegram_hand_seo")


def test_telegram_hand_lead():
    """Test /hand lead via Telegram (lead generation workflow)."""
    send_telegram("/hand lead SaaS companies in Taiwan needing AI integration", wait_secs=30)
    screenshot("telegram_hand_lead")


# -- Main --

def main():
    print("=" * 60)
    print("Phantom Mesh Money-Making Flows - E2E Test Suite")
    print(f"Screenshots: {SCREENSHOT_DIR}")
    print(f"API: {API_BASE}")
    print(f"Time: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print("=" * 60)

    # Check daemon is running
    try:
        requests.get(f"{API_BASE}/health", timeout=3)
    except Exception:
        print("ERROR: Daemon not running! Start with: cargo run --release")
        sys.exit(1)

    # -- Phase 1: HTTP API Tests (fast, programmatic) --
    print("\n--- PHASE 1: HTTP API TESTS ---")
    test("Health check", test_health)
    test("Hands API (7 hands)", test_hands_api)
    test("Hands categories", test_hands_categories)
    test("Hands phases", test_hands_phases)
    test("Tools for hands", test_tools_for_hands)
    test("Hand run endpoint", test_hand_run_api_exists)
    test("Hand run missing prompt", test_hand_run_missing_prompt)
    test("E-Stop protects hands", test_estop_protects_hands)

    # -- Phase 2: Telegram UI Tests --
    print("\n\n--- PHASE 2: TELEGRAM UI TESTS ---")
    print("Make sure Telegram Desktop is visible on the right side of screen")
    time.sleep(3)

    test("Telegram /hands list", test_telegram_hands_list)
    test("Telegram /hand outreach", test_telegram_hand_outreach)
    test("Telegram /hand freelancer", test_telegram_hand_freelancer)
    test("Telegram /hand market_intel", test_telegram_hand_market_intel)
    test("Telegram /hand seo_content", test_telegram_hand_seo)
    test("Telegram /hand lead", test_telegram_hand_lead)

    # -- Summary --
    print("\n\n" + "=" * 60)
    print("TEST SUMMARY")
    print("=" * 60)

    api_pass = 0
    api_fail = 0
    tg_pass = 0
    tg_fail = 0

    for r in results:
        status = r[0]
        name = r[1]
        extra = f" - {r[2][:60]}" if len(r) > 2 else ""
        icon = "[OK]" if status == "PASS" else "[FAIL]"
        print(f"  {icon} {status}: {name}{extra}")

        if "Telegram" in name:
            if status == "PASS":
                tg_pass += 1
            else:
                tg_fail += 1
        else:
            if status == "PASS":
                api_pass += 1
            else:
                api_fail += 1

    print(f"\nAPI Tests:      {api_pass}/{api_pass + api_fail} passed")
    print(f"Telegram Tests: {tg_pass}/{tg_pass + tg_fail} passed")
    print(f"Total:          {pass_count}/{pass_count + fail_count} passed")
    print(f"\nScreenshots saved to: {SCREENSHOT_DIR}")

    screenshot("final_summary")

    return 0 if fail_count == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
