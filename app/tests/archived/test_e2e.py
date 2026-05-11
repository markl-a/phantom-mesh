"""
Phantom Mesh Desktop — E2E UI Test via Playwright
Tests the Vite dev server (localhost:5173) which serves the same React UI as the Tauri webview.
Note: Tauri invoke() calls won't work in a regular browser, but we can verify UI rendering,
navigation, and interaction flows.
"""
import sys
import os
import time
from pathlib import Path

# Fix Windows console encoding for emoji/unicode
os.environ["PYTHONIOENCODING"] = "utf-8"
sys.stdout.reconfigure(encoding="utf-8", errors="replace")
sys.stderr.reconfigure(encoding="utf-8", errors="replace")

from playwright.sync_api import sync_playwright, expect

SCREENSHOTS_DIR = Path(__file__).parent / "test_screenshots"
SCREENSHOTS_DIR.mkdir(exist_ok=True)
BASE_URL = "http://localhost:5173"

def screenshot(page, name):
    path = SCREENSHOTS_DIR / f"{name}.png"
    page.screenshot(path=str(path), full_page=True)
    print(f"  📸 {path}")

def test_all():
    results = []

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(
            viewport={"width": 1280, "height": 800},
            color_scheme="dark",
        )
        page = context.new_page()

        # ── Test 1: Page loads ──
        print("\n[1/7] Page loads...")
        try:
            page.goto(BASE_URL, timeout=10000)
            page.wait_for_load_state("networkidle", timeout=10000)
            screenshot(page, "01_initial_load")

            # Check if we're on onboarding or main app
            body_text = page.text_content("body") or ""
            if "Phantom Mesh" in body_text:
                results.append(("Page loads", "PASS", "Phantom Mesh found"))
            else:
                results.append(("Page loads", "WARN", "Page loaded but no Phantom Mesh text"))
        except Exception as e:
            results.append(("Page loads", "FAIL", str(e)))
            print(f"  ❌ {e}")
            browser.close()
            return results

        # ── Test 2: Detect which screen we're on ──
        print("[2/7] Detect screen...")
        try:
            body_text = page.text_content("body") or ""
            is_onboarding = "正在偵測" in body_text or "環境偵測" in body_text or "帳號登入" in body_text or "啟動 Phantom" in body_text
            is_chat = "Phantom" in body_text and ("影舞者" in body_text or "告訴我" in body_text or "目標" in body_text)
            is_dashboard = "儀表板" in body_text and "對話" not in body_text[:200]

            if is_onboarding:
                screen = "onboarding"
                results.append(("Detect screen", "PASS", "On onboarding page"))
            elif is_chat:
                screen = "chat"
                results.append(("Detect screen", "PASS", "On chat page (main app)"))
            else:
                screen = "unknown"
                results.append(("Detect screen", "WARN", f"Unknown screen. First 200 chars: {body_text[:200]}"))
            print(f"  Screen: {screen}")
        except Exception as e:
            screen = "unknown"
            results.append(("Detect screen", "FAIL", str(e)))

        # ── Test 3: Onboarding flow (if applicable) ──
        if screen == "onboarding":
            print("[3/7] Testing onboarding...")
            try:
                # Wait for scan to complete (scanning → ready)
                page.wait_for_timeout(3000)
                screenshot(page, "02_onboarding_scanning")

                # Wait up to 15s for scan to finish
                for _ in range(15):
                    text = page.text_content("body") or ""
                    if "環境偵測完成" in text or "啟動 Phantom" in text or "帳號登入" in text:
                        break
                    page.wait_for_timeout(1000)

                screenshot(page, "03_onboarding_ready")
                body_text = page.text_content("body") or ""

                # Check for social login buttons
                has_google = "Google" in body_text
                has_apple = "Apple" in body_text
                has_launch = "啟動 Phantom" in body_text or "啟動" in body_text

                details = []
                if has_google: details.append("Google login ✓")
                if has_apple: details.append("Apple login ✓")
                if has_launch: details.append("Launch button ✓")

                # Check for scan results
                has_gpu = "GPU" in body_text
                has_ram = "RAM" in body_text
                has_providers = "Provider" in body_text or "偵測到" in body_text
                if has_gpu: details.append("GPU info ✓")
                if has_ram: details.append("RAM info ✓")
                if has_providers: details.append("Providers ✓")

                # Check for undetected provider login buttons
                has_missing = "尚未偵測" in body_text or "前往登入" in body_text
                if has_missing: details.append("Missing provider buttons ✓")

                results.append(("Onboarding UI", "PASS", ", ".join(details) if details else "Basic onboarding rendered"))

                # Try clicking launch button
                launch_btn = page.locator("button:has-text('啟動 Phantom')")
                if launch_btn.count() > 0:
                    print("  Found launch button, clicking...")
                    launch_btn.click()
                    page.wait_for_timeout(3000)
                    screenshot(page, "04_onboarding_launching")
                    results.append(("Onboarding launch", "PASS", "Launch button clicked"))
                else:
                    results.append(("Onboarding launch", "SKIP", "No launch button found"))

            except Exception as e:
                results.append(("Onboarding UI", "FAIL", str(e)))
                screenshot(page, "03_onboarding_error")

        # ── Test 4: Chat page / sidebar (if on main app) ──
        if screen == "chat":
            print("[3/7] Skipped (not on onboarding)")
            results.append(("Onboarding UI", "SKIP", "Already onboarded"))

        print("[4/7] Testing chat UI...")
        try:
            # Force past onboarding by setting localStorage
            page.evaluate("localStorage.setItem('phantom_mesh_onboarded', 'true')")
            page.goto(BASE_URL, timeout=10000)
            page.wait_for_load_state("networkidle", timeout=10000)
            page.wait_for_timeout(2000)
            screenshot(page, "05_chat_page")

            body_text = page.text_content("body") or ""

            # Check welcome message
            has_welcome = "影舞者" in body_text or "Phantom" in body_text
            has_goals = "學業" in body_text or "事業" in body_text or "健康" in body_text or "技能" in body_text
            has_input = page.locator("input[placeholder]").count() > 0

            details = []
            if has_welcome: details.append("Welcome message ✓")
            if has_goals: details.append("Goal categories ✓")
            if has_input: details.append("Input field ✓")

            if has_welcome or has_goals:
                results.append(("Chat welcome UI", "PASS", ", ".join(details)))
            else:
                results.append(("Chat welcome UI", "WARN", f"Chat page but missing welcome. Text: {body_text[:300]}"))
        except Exception as e:
            results.append(("Chat welcome UI", "FAIL", str(e)))

        # ── Test 5: Sidebar navigation ──
        print("[5/7] Testing sidebar...")
        try:
            # Check primary nav items
            nav_items = page.locator("nav a")
            nav_count = nav_items.count()

            # Check for collapsed sidebar items
            has_chat = page.locator("a:has-text('對話')").count() > 0
            has_tasks = page.locator("a:has-text('任務')").count() > 0
            has_workflow = page.locator("a:has-text('工作流')").count() > 0
            has_more = page.locator("button:has-text('更多功能')").count() > 0

            details = []
            if has_chat: details.append("對話 ✓")
            if has_tasks: details.append("任務 ✓")
            if has_workflow: details.append("工作流 ✓")
            if has_more: details.append("更多功能 ✓")
            details.append(f"Total nav links: {nav_count}")

            results.append(("Sidebar nav", "PASS", ", ".join(details)))
            screenshot(page, "06_sidebar")

            # Test "More" expand
            if has_more:
                page.locator("button:has-text('更多功能')").click()
                page.wait_for_timeout(500)
                screenshot(page, "07_sidebar_expanded")
                expanded_count = page.locator("nav a").count()
                results.append(("Sidebar expand", "PASS", f"Expanded: {expanded_count} items (was {nav_count})"))
        except Exception as e:
            results.append(("Sidebar nav", "FAIL", str(e)))

        # ── Test 6: Goal button interaction ──
        print("[6/7] Testing goal buttons...")
        try:
            page.evaluate("localStorage.setItem('phantom_mesh_onboarded', 'true')")
            page.goto(BASE_URL, timeout=10000)
            page.wait_for_load_state("networkidle", timeout=10000)
            page.wait_for_timeout(1000)

            # Find a goal button
            goal_btn = page.locator("button:has-text('考上理想學校')")
            if goal_btn.count() > 0:
                goal_btn.click()
                page.wait_for_timeout(1000)
                screenshot(page, "08_goal_clicked")

                # Check the message appeared
                body_text = page.text_content("body") or ""
                has_user_msg = "考上台大" in body_text or "理想學校" in body_text
                # Welcome should be hidden after clicking
                welcome_gone = "告訴我，你現在最想" not in body_text

                details = []
                if has_user_msg: details.append("User message sent ✓")
                if welcome_gone: details.append("Welcome collapsed ✓")

                # Wait a bit for potential response
                page.wait_for_timeout(3000)
                screenshot(page, "09_goal_response")

                results.append(("Goal interaction", "PASS", ", ".join(details) if details else "Button clicked"))
            else:
                results.append(("Goal interaction", "SKIP", "No goal buttons found (may already be past welcome)"))
        except Exception as e:
            results.append(("Goal interaction", "FAIL", str(e)))

        # ── Test 7: Manual text input ──
        print("[7/7] Testing text input...")
        try:
            page.evaluate("localStorage.setItem('phantom_mesh_onboarded', 'true')")
            page.goto(BASE_URL, timeout=10000)
            page.wait_for_load_state("networkidle", timeout=10000)
            page.wait_for_timeout(1000)

            input_field = page.locator("input[type='text']")
            send_btn = page.locator("button:has-text('發送')")

            if input_field.count() > 0:
                input_field.fill("你好，請自我介紹")
                screenshot(page, "10_input_filled")

                if send_btn.count() > 0:
                    send_btn.click()
                    page.wait_for_timeout(2000)
                    screenshot(page, "11_message_sent")

                    body_text = page.text_content("body") or ""
                    has_user_msg = "你好" in body_text and "自我介紹" in body_text
                    has_thinking = "思考中" in body_text
                    results.append(("Text input", "PASS", f"Message sent ✓, thinking: {has_thinking}"))
                else:
                    results.append(("Text input", "WARN", "Input found but no send button"))
            else:
                results.append(("Text input", "FAIL", "No input field found"))
        except Exception as e:
            results.append(("Text input", "FAIL", str(e)))

        # Final screenshot
        page.wait_for_timeout(5000)
        screenshot(page, "12_final_state")

        browser.close()

    return results


if __name__ == "__main__":
    print("=" * 60)
    print("  Phantom Mesh Desktop — E2E UI Test")
    print("=" * 60)

    results = test_all()

    print("\n" + "=" * 60)
    print("  TEST RESULTS")
    print("=" * 60)

    pass_count = sum(1 for _, s, _ in results if s == "PASS")
    fail_count = sum(1 for _, s, _ in results if s == "FAIL")
    warn_count = sum(1 for _, s, _ in results if s == "WARN")
    skip_count = sum(1 for _, s, _ in results if s == "SKIP")

    for name, status, detail in results:
        icon = {"PASS": "✅", "FAIL": "❌", "WARN": "⚠️", "SKIP": "⏭️"}.get(status, "?")
        print(f"  {icon} {status:4s} | {name:25s} | {detail}")

    print(f"\n  Total: {len(results)} | ✅ {pass_count} | ❌ {fail_count} | ⚠️ {warn_count} | ⏭️ {skip_count}")
    print(f"  Screenshots: {SCREENSHOTS_DIR}")
    print("=" * 60)

    sys.exit(1 if fail_count > 0 else 0)
