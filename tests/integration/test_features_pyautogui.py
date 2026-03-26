"""
Stage 2 Feature Testing via pyautogui + Telegram
Tests all Sprint 1-4 features by sending Telegram commands and verifying responses.
"""
import pyautogui
import time
import os

# Telegram input box location (right half of 1920x1080 screen)
TG_INPUT_X = 1400
TG_INPUT_Y = 1050
SCREENSHOT_DIR = os.path.join(os.path.expanduser('~'), 'Desktop', 'test_screenshots')
os.makedirs(SCREENSHOT_DIR, exist_ok=True)

# Safety settings
pyautogui.FAILSAFE = True
pyautogui.PAUSE = 0.3

def screenshot(name):
    """Take and save a screenshot"""
    path = os.path.join(SCREENSHOT_DIR, f'{name}.png')
    pyautogui.screenshot().save(path)
    print(f'  [screenshot] {name}.png')
    return path

def send_telegram(message, wait_secs=8):
    """Click Telegram input, type message, press Enter, wait for response"""
    print(f'  Sending: {message}')
    # Click on Telegram input area
    pyautogui.click(TG_INPUT_X, TG_INPUT_Y)
    time.sleep(0.3)
    # Clear any existing text
    pyautogui.hotkey('ctrl', 'a')
    time.sleep(0.1)
    # Type message
    pyautogui.typewrite(message, interval=0.02) if message.isascii() else type_unicode(message)
    time.sleep(0.2)
    # Send
    pyautogui.press('enter')
    print(f'  Waiting {wait_secs}s for response...')
    time.sleep(wait_secs)

def type_unicode(text):
    """Type unicode text using clipboard"""
    import subprocess
    # Copy to clipboard via powershell
    subprocess.run(['powershell', '-command', f'Set-Clipboard -Value "{text}"'], capture_output=True)
    time.sleep(0.1)
    pyautogui.hotkey('ctrl', 'v')

def test_header(name):
    print(f'\n{"="*60}')
    print(f'TEST: {name}')
    print(f'{"="*60}')

# ═══════════════════════════════════════════════════════════════
# Test Sequence
# ═══════════════════════════════════════════════════════════════

print('Stage 2 Feature Test Suite')
print(f'Screen size: {pyautogui.size()}')
print(f'Screenshots: {SCREENSHOT_DIR}')

# Initial screenshot
screenshot('00_initial')

# ── Test 1: /help ─────────────────────────────────────────────
test_header('/help command')
send_telegram('/help', wait_secs=5)
screenshot('01_help')

# ── Test 2: /status (shows uptime, LLM, tools, agents) ───────
test_header('/status command')
send_telegram('/status', wait_secs=8)
screenshot('02_status')

# ── Test 3: /tools (should show all 14+ tools) ───────────────
test_header('/tools command (should show new tools)')
send_telegram('/tools', wait_secs=5)
screenshot('03_tools')

# ── Test 4: E-Stop test ──────────────────────────────────────
test_header('/estop (Emergency Stop)')
send_telegram('/estop', wait_secs=3)
screenshot('04_estop_activated')

# ── Test 5: Send message while stopped ────────────────────────
test_header('Message while E-Stop active')
send_telegram('hello', wait_secs=5)
screenshot('05_estop_blocked')

# ── Test 6: Resume ────────────────────────────────────────────
test_header('/resume (Deactivate E-Stop)')
send_telegram('/resume', wait_secs=3)
screenshot('06_resume')

# ── Test 7: Basic agent response ─────────────────────────────
test_header('Basic agent response')
send_telegram('What is 2+2? Reply with just the number.', wait_secs=15)
screenshot('07_basic_response')

# ── Test 8: Tool usage - file_read ────────────────────────────
test_header('Tool usage: file_read')
send_telegram('Read the file ~/.phantom-mesh/agents.toml and show the first 5 lines', wait_secs=20)
screenshot('08_file_read')

# ── Test 9: Tool usage - shell ────────────────────────────────
test_header('Tool usage: shell command')
send_telegram('Run: echo "phantom-mesh stage 2 test OK"', wait_secs=15)
screenshot('09_shell')

# ── Test 10: /history ─────────────────────────────────────────
test_header('/history command')
send_telegram('/history', wait_secs=3)
screenshot('10_history')

# ── Test 11: /dashboard ──────────────────────────────────────
test_header('/dashboard command')
send_telegram('/dashboard', wait_secs=3)
screenshot('11_dashboard')

# ── Test 12: E-Stop via HTTP API ─────────────────────────────
test_header('E-Stop HTTP API test')
import subprocess
# Check E-Stop status
result = subprocess.run(['curl', '-s', 'http://127.0.0.1:7878/estop'], capture_output=True, text=True)
print(f'  GET /estop: {result.stdout}')

# Activate E-Stop via API
result = subprocess.run(['curl', '-s', '-X', 'POST', 'http://127.0.0.1:7878/estop'], capture_output=True, text=True)
print(f'  POST /estop: {result.stdout}')

# Check status
result = subprocess.run(['curl', '-s', 'http://127.0.0.1:7878/estop'], capture_output=True, text=True)
print(f'  GET /estop (after): {result.stdout}')

# Reset
result = subprocess.run(['curl', '-s', '-X', 'DELETE', 'http://127.0.0.1:7878/estop'], capture_output=True, text=True)
print(f'  DELETE /estop: {result.stdout}')
screenshot('12_estop_api')

# ── Test 13: Health endpoint ──────────────────────────────────
test_header('HTTP Health endpoint')
result = subprocess.run(['curl', '-s', 'http://127.0.0.1:7878/health'], capture_output=True, text=True)
print(f'  GET /health: {result.stdout}')

# ── Test 14: Tools endpoint ───────────────────────────────────
test_header('HTTP Tools endpoint')
result = subprocess.run(['curl', '-s', 'http://127.0.0.1:7878/tools'], capture_output=True, text=True)
print(f'  GET /tools: {result.stdout[:500]}...')

# ── Test 15: SSE streaming endpoint ──────────────────────────
test_header('SSE streaming endpoint')
result = subprocess.run(
    ['curl', '-s', '--max-time', '3', 'http://127.0.0.1:7878/stream/agent/master?prompt=say%20hello'],
    capture_output=True, text=True
)
print(f'  SSE response: {result.stdout[:300]}...')

# ── Test 16: Conversation memory ─────────────────────────────
test_header('Conversation memory test')
send_telegram('My favorite color is blue. Remember this.', wait_secs=15)
screenshot('16a_memory_set')
time.sleep(2)
send_telegram('What is my favorite color?', wait_secs=15)
screenshot('16b_memory_recall')

# ── Test 17: /clear ──────────────────────────────────────────
test_header('/clear command')
send_telegram('/clear', wait_secs=3)
screenshot('17_clear')

# Final screenshot
time.sleep(2)
screenshot('99_final')

print('\n' + '='*60)
print('ALL TESTS COMPLETE')
print(f'Screenshots saved to: {SCREENSHOT_DIR}')
print('='*60)
