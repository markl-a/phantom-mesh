"""
Stage 2 Feature Testing v2 — corrected Telegram input coordinates.
Tests via Telegram commands + HTTP API.
"""
import pyautogui
import time
import os
import subprocess

SCREENSHOT_DIR = os.path.join(os.path.expanduser('~'), 'Desktop', 'test_screenshots_v2')
os.makedirs(SCREENSHOT_DIR, exist_ok=True)

pyautogui.FAILSAFE = True
pyautogui.PAUSE = 0.2

def screenshot(name):
    path = os.path.join(SCREENSHOT_DIR, f'{name}.png')
    pyautogui.screenshot().save(path)
    print(f'  [screenshot] {name}')
    return path

def find_telegram_input():
    """Find the 'Write a message...' input box in Telegram"""
    # Based on the cropped screenshot analysis:
    # The input box is at the bottom-right area of screen
    # "Write a message..." text was visible at approximately:
    # - x: around 1500 (center of the input field in the chat area)
    # - y: around 1058 (very bottom of the window, above taskbar)
    return (1500, 1058)

def send_telegram(message, wait_secs=8):
    """Click Telegram input, type message, press Enter"""
    print(f'  Sending: {message}')
    x, y = find_telegram_input()

    # Click to focus on Telegram window first
    pyautogui.click(x, y - 100)  # Click in chat area first to ensure Telegram is focused
    time.sleep(0.3)

    # Click on input field
    pyautogui.click(x, y)
    time.sleep(0.3)

    # Type the message character by character for reliability
    if message.isascii():
        pyautogui.typewrite(message, interval=0.02)
    else:
        # Use clipboard for non-ASCII
        subprocess.run(
            ['powershell', '-command', f'Set-Clipboard -Value "{message}"'],
            capture_output=True
        )
        time.sleep(0.1)
        pyautogui.hotkey('ctrl', 'v')

    time.sleep(0.3)
    pyautogui.press('enter')
    print(f'  Waiting {wait_secs}s for response...')
    time.sleep(wait_secs)

def curl(method, url, expected_key=None):
    """HTTP request and print result"""
    cmd = ['curl', '-s']
    if method != 'GET':
        cmd.extend(['-X', method])
    cmd.append(url)
    result = subprocess.run(cmd, capture_output=True, text=True)
    print(f'  {method} {url}: {result.stdout[:200]}')
    return result.stdout

def test(name):
    print(f'\n{"="*60}')
    print(f'TEST: {name}')
    print(f'{"="*60}')

# ═══════════════════════════════════════════════════════════════
print('Stage 2 Feature Test Suite v2')
print(f'Screen: {pyautogui.size()}')
print(f'Screenshots: {SCREENSHOT_DIR}')
# ═══════════════════════════════════════════════════════════════

screenshot('00_initial')

# ── Phase 1: HTTP API Tests (no Telegram needed) ──────────────

test('HTTP /health')
curl('GET', 'http://127.0.0.1:7878/health')

test('HTTP /tools (should list 14+ tools)')
tools_json = curl('GET', 'http://127.0.0.1:7878/tools')
import json
try:
    tools = json.loads(tools_json)
    tool_names = [t['name'] for t in tools.get('tools', [])]
    print(f'  Tools found ({len(tool_names)}): {tool_names}')
    # Verify new tools exist
    new_tools = ['file_edit', 'http_request', 'glob_search', 'content_search']
    for nt in new_tools:
        if nt in tool_names:
            print(f'  [PASS] New tool "{nt}" registered')
        else:
            print(f'  [MISS] New tool "{nt}" NOT found')
except:
    print('  Failed to parse tools JSON')

test('HTTP E-Stop lifecycle')
curl('GET', 'http://127.0.0.1:7878/estop')
curl('POST', 'http://127.0.0.1:7878/estop')
curl('GET', 'http://127.0.0.1:7878/estop')
curl('DELETE', 'http://127.0.0.1:7878/estop')
curl('GET', 'http://127.0.0.1:7878/estop')

test('HTTP Agent run endpoint')
result = subprocess.run(
    ['curl', '-s', '-X', 'POST', '-H', 'Content-Type: application/json',
     '-d', '{"prompt":"What is 1+1?"}',
     'http://127.0.0.1:7878/agent/master/run'],
    capture_output=True, text=True, timeout=60
)
print(f'  POST /agent/master/run: {result.stdout[:300]}')

test('HTTP SSE streaming endpoint')
result = subprocess.run(
    ['curl', '-s', '--max-time', '15',
     'http://127.0.0.1:7878/stream/agent/master?prompt=Say%20hello%20in%20one%20word'],
    capture_output=True, text=True, timeout=20
)
print(f'  SSE: {result.stdout[:300]}')

screenshot('01_api_tests_done')

# ── Phase 2: Telegram Tests ──────────────────────────────────

test('Telegram: /help')
send_telegram('/help', wait_secs=5)
screenshot('02_help')

test('Telegram: /status')
send_telegram('/status', wait_secs=8)
screenshot('03_status')

test('Telegram: /tools')
send_telegram('/tools', wait_secs=5)
screenshot('04_tools')

test('Telegram: /estop')
send_telegram('/estop', wait_secs=3)
screenshot('05_estop')

test('Telegram: message while stopped')
send_telegram('hello', wait_secs=5)
screenshot('06_blocked')

test('Telegram: /resume')
send_telegram('/resume', wait_secs=3)
screenshot('07_resume')

test('Telegram: basic agent response')
send_telegram('What is 2+2?', wait_secs=20)
screenshot('08_agent')

test('Telegram: tool use (shell)')
send_telegram('Run: echo hello-from-clawtex', wait_secs=20)
screenshot('09_shell')

test('Telegram: /history')
send_telegram('/history', wait_secs=3)
screenshot('10_history')

test('Telegram: /clear')
send_telegram('/clear', wait_secs=3)
screenshot('11_clear')

# ── Final ─────────────────────────────────────────────────────
time.sleep(2)
screenshot('99_final')

print('\n' + '='*60)
print('ALL TESTS COMPLETE')
print(f'Screenshots: {SCREENSHOT_DIR}')
print('='*60)
