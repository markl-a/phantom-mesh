"""
Phantom Mesh Desktop — Goals Page E2E Test via Playwright
Tests the Goals page UI rendering against the Vite dev server (localhost:5173).
Tauri invoke() won't work in browser, so we verify UI structure and navigation.
"""
import sys
import os
from pathlib import Path

os.environ["PYTHONIOENCODING"] = "utf-8"
sys.stdout.reconfigure(encoding="utf-8", errors="replace")
sys.stderr.reconfigure(encoding="utf-8", errors="replace")

from playwright.sync_api import sync_playwright

SCREENSHOTS_DIR = Path(__file__).parent / "test_screenshots"
SCREENSHOTS_DIR.mkdir(exist_ok=True)
BASE_URL = "http://localhost:5173"

def screenshot(page, name):
    path = SCREENSHOTS_DIR / f"{name}.png"
    page.screenshot(path=str(path), full_page=True)
    print(f"  screenshot: {path}")

def test_goals():
    results = []

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(
            viewport={"width": 1280, "height": 800},
            color_scheme="dark",
        )
        page = context.new_page()

        # Skip onboarding
        page.goto(BASE_URL, timeout=10000)
        page.evaluate("localStorage.setItem('phantom_mesh_onboarded', 'true')")
        page.goto(BASE_URL, timeout=10000)
        page.wait_for_load_state("networkidle", timeout=10000)
        page.wait_for_timeout(1000)

        # ── Test 1: Goals nav item exists ──
        print("[1/6] Goals nav item...")
        try:
            goals_nav = page.locator("a:has-text('目標')")
            if goals_nav.count() > 0:
                results.append(("Goals nav item", "PASS", "Found in sidebar"))
            else:
                results.append(("Goals nav item", "FAIL", "Not found in sidebar"))
                browser.close()
                return results
        except Exception as e:
            results.append(("Goals nav item", "FAIL", str(e)))
            browser.close()
            return results

        # ── Test 2: Navigate to Goals page ──
        print("[2/6] Navigate to Goals page...")
        try:
            goals_nav.click()
            page.wait_for_timeout(2000)
            screenshot(page, "goals_01_page")

            body = page.text_content("body") or ""
            has_my_goals = "我的目標" in body
            has_empty_msg = "還沒有目標" in body or "選擇一個目標" in body

            details = []
            if has_my_goals:
                details.append("'我的目標' heading found")
            if has_empty_msg:
                details.append("Empty state message found")

            if has_my_goals:
                results.append(("Goals page renders", "PASS", ", ".join(details)))
            else:
                results.append(("Goals page renders", "WARN", f"Page loaded but missing key elements. Text: {body[:300]}"))
        except Exception as e:
            results.append(("Goals page renders", "FAIL", str(e)))
            screenshot(page, "goals_01_error")

        # ── Test 3: Goals page URL is correct ──
        print("[3/6] URL check...")
        try:
            current_url = page.url
            if "/goals" in current_url:
                results.append(("Goals URL", "PASS", current_url))
            else:
                results.append(("Goals URL", "FAIL", f"Expected /goals, got {current_url}"))
        except Exception as e:
            results.append(("Goals URL", "FAIL", str(e)))

        # ── Test 4: New goal form ──
        print("[4/6] New goal form...")
        try:
            plus_btn = page.locator("button").filter(has=page.locator("svg"))
            # Find the + button near "我的目標"
            add_btns = page.locator("button")
            found_plus = False
            for i in range(add_btns.count()):
                btn = add_btns.nth(i)
                # Check if this is near the goals section
                try:
                    inner = btn.inner_html()
                    if "plus" in inner.lower() or "Plus" in inner or "lucide-plus" in inner or '<line' in inner:
                        btn.click()
                        found_plus = True
                        break
                except:
                    pass

            if not found_plus:
                # Try clicking any button that might toggle the form
                # Look for the target icon + plus combo
                page.locator("button").filter(has=page.locator('[class*="lucide"]')).last.click()

            page.wait_for_timeout(500)
            screenshot(page, "goals_02_new_form")

            body = page.text_content("body") or ""
            has_form = "目標名稱" in body or "分類" in body

            if has_form:
                # Try filling the form
                title_input = page.locator("input[placeholder*='目標名稱']")
                if title_input.count() > 0:
                    title_input.fill("學好日文 N2")
                    category_input = page.locator("input[placeholder*='分類']")
                    if category_input.count() > 0:
                        category_input.fill("技能")
                    screenshot(page, "goals_03_form_filled")
                    results.append(("New goal form", "PASS", "Form visible, fields fillable"))
                else:
                    results.append(("New goal form", "WARN", "Form text found but input not located"))
            else:
                results.append(("New goal form", "WARN", f"Form not detected after click. Body: {body[:200]}"))
        except Exception as e:
            results.append(("New goal form", "FAIL", str(e)))
            screenshot(page, "goals_02_error")

        # ── Test 5: Detail panel placeholder ──
        print("[5/6] Detail panel...")
        try:
            body = page.text_content("body") or ""
            has_placeholder = "選擇一個目標查看詳情" in body or "在對話中告訴" in body
            if has_placeholder:
                results.append(("Detail placeholder", "PASS", "Empty state shown correctly"))
            else:
                results.append(("Detail placeholder", "WARN", "Placeholder not found (may have goals loaded)"))
        except Exception as e:
            results.append(("Detail placeholder", "FAIL", str(e)))

        # ── Test 6: Sidebar highlight ──
        print("[6/6] Sidebar active state...")
        try:
            active_link = page.locator("a.bg-phantom-primary\\/15")
            if active_link.count() > 0:
                active_text = active_link.first.text_content() or ""
                if "目標" in active_text:
                    results.append(("Sidebar active", "PASS", "Goals link highlighted"))
                else:
                    results.append(("Sidebar active", "WARN", f"Active link is: {active_text}"))
            else:
                # Try with different selector
                nav_links = page.locator("nav a")
                found_active = False
                for i in range(nav_links.count()):
                    cls = nav_links.nth(i).get_attribute("class") or ""
                    txt = nav_links.nth(i).text_content() or ""
                    if "primary" in cls and "目標" in txt:
                        found_active = True
                        break
                if found_active:
                    results.append(("Sidebar active", "PASS", "Goals link highlighted"))
                else:
                    results.append(("Sidebar active", "WARN", "Could not verify active state"))
        except Exception as e:
            results.append(("Sidebar active", "FAIL", str(e)))

        screenshot(page, "goals_04_final")
        browser.close()

    return results


if __name__ == "__main__":
    print("=" * 60)
    print("  Phantom Mesh — Goals Page E2E Test")
    print("=" * 60)

    results = test_goals()

    print("\n" + "=" * 60)
    print("  RESULTS")
    print("=" * 60)

    pass_count = sum(1 for _, s, _ in results if s == "PASS")
    fail_count = sum(1 for _, s, _ in results if s == "FAIL")
    warn_count = sum(1 for _, s, _ in results if s == "WARN")

    for name, status, detail in results:
        icon = {"PASS": "OK", "FAIL": "FAIL", "WARN": "WARN"}.get(status, "?")
        print(f"  [{icon:4s}] {name:25s} | {detail}")

    print(f"\n  Total: {len(results)} | PASS {pass_count} | FAIL {fail_count} | WARN {warn_count}")
    print(f"  Screenshots: {SCREENSHOTS_DIR}")
    print("=" * 60)

    sys.exit(1 if fail_count > 0 else 0)
