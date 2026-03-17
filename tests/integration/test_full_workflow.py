"""
Clawtex E2E Money-Making Workflow Test
======================================
Tests 4 hand workflows end-to-end via HTTP API + Telegram progress verification.

Scenarios:
1. Content Generation (simplest, 3 phases)
2. Market Intelligence (4 phases, search + analysis)
3. Lead Generation (4 phases, search + structured output)
4. Outreach (4 phases, full sales pipeline)

Requirements:
- clawtex-core daemon running on localhost:7878
- LM Studio running with qwen/qwen3-coder-next loaded
- pip install requests pyautogui pillow
"""

import json
import os
import sys
import time
import datetime
import requests
from pathlib import Path

# ── Configuration ────────────────────────────────────────────────────────────

BASE_URL = "http://localhost:7878"
SCREENSHOT_DIR = Path.home() / "Desktop" / "test_full_workflow"
TIMEOUT_PER_PHASE = 300  # 5 min max per phase
TIMEOUT_TOTAL = 1200     # 20 min max per scenario

# Try to import pyautogui for screenshots, but make it optional
try:
    import pyautogui
    HAS_PYAUTOGUI = True
except ImportError:
    HAS_PYAUTOGUI = False
    print("[WARN] pyautogui not installed — screenshots disabled. pip install pyautogui pillow")

# ── Test Scenarios ───────────────────────────────────────────────────────────

SCENARIOS = [
    {
        "name": "content_generation",
        "hand": "content",
        "prompt": "Write a LinkedIn post about AI productivity tools for small business owners",
        "expected_phases": 3,
        "expected_files": ["content_output.md", "content_queue.json"],
        "timeout": 600,
    },
    {
        "name": "market_intelligence",
        "hand": "market_intel",
        "prompt": "AI chatbot market in Taiwan 2026",
        "expected_phases": 4,
        "expected_files": ["market_intelligence.md", "competitors.csv", "opportunities.json"],
        "timeout": 900,
    },
    {
        "name": "lead_generation",
        "hand": "lead",
        "prompt": "web design companies in Taipei Taiwan",
        "expected_phases": 4,
        "expected_files": ["leads_report.md", "leads_data.csv"],
        "timeout": 900,
    },
    {
        "name": "outreach",
        "hand": "outreach",
        "prompt": "AI automation consulting services for small businesses in Taiwan",
        "expected_phases": 4,
        "expected_files": ["outreach_emails.md", "outreach_report.md", "outreach_tracker.csv"],
        "timeout": 1200,
    },
]


# ── Utility Functions ────────────────────────────────────────────────────────

def timestamp():
    return datetime.datetime.now().strftime("%H:%M:%S")


def log(msg, level="INFO"):
    print(f"[{timestamp()}] [{level}] {msg}")


def take_screenshot(name):
    if not HAS_PYAUTOGUI:
        return None
    SCREENSHOT_DIR.mkdir(parents=True, exist_ok=True)
    path = SCREENSHOT_DIR / f"{name}_{datetime.datetime.now().strftime('%H%M%S')}.png"
    try:
        pyautogui.screenshot(str(path))
        log(f"Screenshot saved: {path}")
        return str(path)
    except Exception as e:
        log(f"Screenshot failed: {e}", "WARN")
        return None


def check_health():
    """Check if clawtex-core daemon is running."""
    try:
        r = requests.get(f"{BASE_URL}/health", timeout=5)
        data = r.json()
        log(f"Daemon health: {data}")
        return data.get("status") == "ok"
    except Exception as e:
        log(f"Daemon not reachable: {e}", "ERROR")
        return False


def check_lmstudio():
    """Check if LM Studio is running and has the model loaded."""
    try:
        r = requests.get("http://localhost:1234/v1/models", timeout=5)
        data = r.json()
        models = [m["id"] for m in data.get("data", [])]
        log(f"LM Studio models: {models}")
        return "qwen/qwen3-coder-next" in models
    except Exception as e:
        log(f"LM Studio not reachable: {e}", "WARN")
        return False


def list_workspace_files():
    """List files in the clawtex workspace via API."""
    try:
        r = requests.get(f"{BASE_URL}/workspace/files", timeout=10)
        data = r.json()
        return data.get("files", []), data.get("workspace", "")
    except Exception as e:
        log(f"Failed to list workspace files: {e}", "ERROR")
        return [], ""


def list_hands():
    """List available hands via API."""
    try:
        r = requests.get(f"{BASE_URL}/hands", timeout=10)
        data = r.json()
        return data.get("hands", [])
    except Exception as e:
        log(f"Failed to list hands: {e}", "ERROR")
        return []


def run_hand(hand_name, prompt, timeout=TIMEOUT_TOTAL):
    """Run a hand workflow via HTTP API and return the result."""
    log(f"Starting hand '{hand_name}' with prompt: {prompt[:80]}...")
    try:
        r = requests.post(
            f"{BASE_URL}/hand/{hand_name}/run",
            json={"prompt": prompt},
            timeout=timeout,
        )
        if r.status_code == 200:
            return r.json()
        else:
            log(f"HTTP {r.status_code}: {r.text}", "ERROR")
            return {"error": f"HTTP {r.status_code}"}
    except requests.exceptions.Timeout:
        log(f"Hand '{hand_name}' timed out after {timeout}s", "ERROR")
        return {"error": "timeout"}
    except Exception as e:
        log(f"Hand '{hand_name}' request failed: {e}", "ERROR")
        return {"error": str(e)}


# ── Test Runner ──────────────────────────────────────────────────────────────

class TestResult:
    def __init__(self, scenario_name):
        self.name = scenario_name
        self.status = "PENDING"
        self.phases_completed = 0
        self.total_phases = 0
        self.elapsed_secs = 0.0
        self.workspace_files_before = []
        self.workspace_files_after = []
        self.new_files = []
        self.expected_files_found = []
        self.expected_files_missing = []
        self.output_preview = ""
        self.error = None
        self.screenshots = []

    def to_dict(self):
        return {
            "name": self.name,
            "status": self.status,
            "phases_completed": self.phases_completed,
            "total_phases": self.total_phases,
            "elapsed_secs": round(self.elapsed_secs, 1),
            "new_files": self.new_files,
            "expected_found": self.expected_files_found,
            "expected_missing": self.expected_files_missing,
            "output_preview": self.output_preview[:500],
            "error": self.error,
        }


def run_scenario(scenario):
    """Run a single test scenario and return TestResult."""
    result = TestResult(scenario["name"])
    result.total_phases = scenario["expected_phases"]

    log(f"\n{'='*60}")
    log(f"SCENARIO: {scenario['name']}")
    log(f"Hand: {scenario['hand']}, Expected phases: {scenario['expected_phases']}")
    log(f"Prompt: {scenario['prompt']}")
    log(f"{'='*60}")

    # Capture workspace state before
    files_before, ws_dir = list_workspace_files()
    result.workspace_files_before = [f["name"] for f in files_before]
    log(f"Workspace before: {len(files_before)} files in {ws_dir}")

    take_screenshot(f"{scenario['name']}_before")

    # Run the hand
    start = time.time()
    response = run_hand(scenario["hand"], scenario["prompt"], timeout=scenario.get("timeout", TIMEOUT_TOTAL))
    result.elapsed_secs = time.time() - start

    if "error" in response and not response.get("phases_completed"):
        result.status = "FAIL"
        result.error = response["error"]
        log(f"FAILED: {response['error']}", "ERROR")
        take_screenshot(f"{scenario['name']}_error")
        return result

    # Parse result
    result.phases_completed = response.get("phases_completed", 0)
    result.output_preview = response.get("final_output", "")

    log(f"Completed: {result.phases_completed}/{result.total_phases} phases in {result.elapsed_secs:.1f}s")

    # Check workspace for new files
    files_after, _ = list_workspace_files()
    result.workspace_files_after = [f["name"] for f in files_after]
    names_before = set(result.workspace_files_before)
    result.new_files = [f["name"] for f in files_after if f["name"] not in names_before]

    log(f"Workspace after: {len(files_after)} files, {len(result.new_files)} new")
    if result.new_files:
        log(f"New files: {result.new_files}")

    # Check expected files
    all_files = set(result.workspace_files_after)
    for expected in scenario.get("expected_files", []):
        if expected in all_files:
            result.expected_files_found.append(expected)
        else:
            result.expected_files_missing.append(expected)

    # Determine pass/fail
    has_output = len(result.output_preview) > 50
    has_phases = result.phases_completed >= 1
    has_files = len(result.new_files) > 0

    if has_output and has_phases:
        if has_files:
            result.status = "PASS"
        else:
            result.status = "PARTIAL"  # Got output but no workspace files
    else:
        result.status = "FAIL"

    take_screenshot(f"{scenario['name']}_result")

    status_emoji = {"PASS": "PASS", "PARTIAL": "PARTIAL", "FAIL": "FAIL"}[result.status]
    log(f"Result: {status_emoji} — {result.phases_completed}/{result.total_phases} phases, "
        f"{len(result.new_files)} new files, {len(result.output_preview)} chars output")

    return result


# ── Main ─────────────────────────────────────────────────────────────────────

def main():
    log("Clawtex E2E Workflow Test Starting")
    log(f"Screenshot dir: {SCREENSHOT_DIR}")

    # Pre-flight checks
    log("\n--- Pre-flight Checks ---")

    if not check_health():
        log("ABORT: clawtex-core daemon not running. Start with: cargo run --release", "ERROR")
        sys.exit(1)

    lmstudio_ok = check_lmstudio()
    if not lmstudio_ok:
        log("WARNING: LM Studio not detected or qwen3-coder-next not loaded", "WARN")
        log("The daemon may fall back to Ollama (llama3.2:1b) which is much weaker", "WARN")

    hands = list_hands()
    hand_names = [h["name"] for h in hands]
    log(f"Available hands: {hand_names}")

    # Filter scenarios to only run hands that exist
    runnable = [s for s in SCENARIOS if s["hand"] in hand_names]
    skipped = [s for s in SCENARIOS if s["hand"] not in hand_names]
    if skipped:
        log(f"Skipping scenarios (hand not found): {[s['name'] for s in skipped]}", "WARN")

    if not runnable:
        log("ABORT: No runnable scenarios — no matching hands found", "ERROR")
        sys.exit(1)

    # Select which scenarios to run
    if len(sys.argv) > 1:
        selected = sys.argv[1]
        if selected == "all":
            pass  # run all
        else:
            runnable = [s for s in runnable if s["name"] == selected or s["hand"] == selected]
            if not runnable:
                log(f"No scenario matching '{selected}'. Available: {[s['name'] for s in SCENARIOS]}", "ERROR")
                sys.exit(1)

    log(f"\nRunning {len(runnable)} scenarios: {[s['name'] for s in runnable]}")

    # Run scenarios
    results = []
    for scenario in runnable:
        try:
            result = run_scenario(scenario)
            results.append(result)
        except KeyboardInterrupt:
            log("Interrupted by user", "WARN")
            break
        except Exception as e:
            log(f"Scenario '{scenario['name']}' crashed: {e}", "ERROR")
            r = TestResult(scenario["name"])
            r.status = "CRASH"
            r.error = str(e)
            results.append(r)

    # Summary
    log(f"\n{'='*60}")
    log("TEST SUMMARY")
    log(f"{'='*60}")

    for r in results:
        icon = {"PASS": "[PASS]", "PARTIAL": "[PART]", "FAIL": "[FAIL]", "CRASH": "[CRASH]", "PENDING": "[????]"}
        status = icon.get(r.status, r.status)
        log(f"  {status} {r.name}: {r.phases_completed}/{r.total_phases} phases, "
            f"{r.elapsed_secs:.1f}s, {len(r.new_files)} files")

    passed = sum(1 for r in results if r.status == "PASS")
    partial = sum(1 for r in results if r.status == "PARTIAL")
    failed = sum(1 for r in results if r.status in ("FAIL", "CRASH"))

    log(f"\nTotal: {passed} PASS, {partial} PARTIAL, {failed} FAIL out of {len(results)}")

    # Save detailed results
    SCREENSHOT_DIR.mkdir(parents=True, exist_ok=True)
    report_path = SCREENSHOT_DIR / "test_results.json"
    with open(report_path, "w", encoding="utf-8") as f:
        json.dump([r.to_dict() for r in results], f, indent=2, ensure_ascii=False)
    log(f"Detailed results saved to: {report_path}")

    # Exit code
    if failed > 0:
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
