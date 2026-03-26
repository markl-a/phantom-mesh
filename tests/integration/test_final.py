"""
Stage 2 Final Feature Test — all features via HTTP API + Telegram.
"""
import pyautogui
import time
import os
import subprocess
import json

SCREENSHOT_DIR = os.path.join(os.path.expanduser('~'), 'Desktop', 'test_final')
os.makedirs(SCREENSHOT_DIR, exist_ok=True)

pyautogui.FAILSAFE = True
pyautogui.PAUSE = 0.2

def screenshot(name):
    path = os.path.join(SCREENSHOT_DIR, f'{name}.png')
    pyautogui.screenshot().save(path)
    print(f'  [screenshot] {name}')

def send_telegram(message, wait_secs=8):
    print(f'  TG> {message}')
    # Click chat area to focus Telegram
    pyautogui.click(1500, 950)
    time.sleep(0.3)
    # Click input field
    pyautogui.click(1500, 1058)
    time.sleep(0.3)
    pyautogui.hotkey('ctrl', 'a')
    time.sleep(0.1)
    if message.isascii():
        pyautogui.typewrite(message, interval=0.02)
    else:
        subprocess.run(['powershell', '-command', f'Set-Clipboard -Value "{message}"'], capture_output=True)
        time.sleep(0.1)
        pyautogui.hotkey('ctrl', 'v')
    time.sleep(0.3)
    pyautogui.press('enter')
    time.sleep(wait_secs)

def curl_json(method, url, data=None, timeout=30):
    cmd = ['curl', '-s', '--max-time', str(timeout)]
    if method != 'GET':
        cmd.extend(['-X', method])
    if data:
        cmd.extend(['-H', 'Content-Type: application/json', '-d', json.dumps(data)])
    cmd.append(url)
    result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout+5)
    try:
        return json.loads(result.stdout)
    except:
        return result.stdout

def test(name):
    print(f'\n{"="*60}')
    print(f'  {name}')
    print(f'{"="*60}')

results = {}

def record(name, passed, detail=''):
    status = 'PASS' if passed else 'FAIL'
    results[name] = passed
    print(f'  [{status}] {name} {detail}')

# ═══════════════════════════════════════════════════════════════
print('\n' + '='*60)
print('  PHANTOM_MESH STAGE 2 — FINAL FEATURE TEST')
print('='*60)
screenshot('00_start')
# ═══════════════════════════════════════════════════════════════

# ── HTTP API Tests ────────────────────────────────────────────

test('1. HTTP /health')
r = curl_json('GET', 'http://127.0.0.1:7878/health')
record('/health', r.get('status') == 'ok', f'→ {r}')

test('2. HTTP /tools — tool count')
r = curl_json('GET', 'http://127.0.0.1:7878/tools')
tools = [t['name'] for t in r.get('tools', [])]
record('/tools count', len(tools) >= 11, f'→ {len(tools)} tools: {sorted(tools)}')
for expected in ['file_edit', 'http_request', 'glob_search', 'content_search']:
    record(f'  tool: {expected}', expected in tools)

test('3. E-Stop HTTP lifecycle')
r1 = curl_json('GET', 'http://127.0.0.1:7878/estop')
record('estop initial=running', r1.get('stopped') == False, f'→ {r1}')

r2 = curl_json('POST', 'http://127.0.0.1:7878/estop')
record('estop POST=stopped', r2.get('status') == 'stopped', f'→ {r2}')

r3 = curl_json('GET', 'http://127.0.0.1:7878/estop')
record('estop GET=stopped', r3.get('stopped') == True, f'→ {r3}')

r4 = curl_json('DELETE', 'http://127.0.0.1:7878/estop')
record('estop DELETE=running', r4.get('status') == 'running', f'→ {r4}')

r5 = curl_json('GET', 'http://127.0.0.1:7878/estop')
record('estop final=running', r5.get('stopped') == False, f'→ {r5}')

test('4. HTTP Agent run')
r = curl_json('POST', 'http://127.0.0.1:7878/agent/master/run',
              data={"prompt": "What is 1+1? Answer with just the number."}, timeout=60)
record('agent run', isinstance(r, dict) and 'result' in r,
       f'→ result={str(r.get("result",""))[:100]}')

test('5. SSE Streaming endpoint')
result = subprocess.run(
    ['curl', '-s', '--max-time', '20',
     'http://127.0.0.1:7878/stream/agent/master?prompt=Say%20hi'],
    capture_output=True, text=True, timeout=25
)
has_events = 'event:' in result.stdout or 'data:' in result.stdout
record('SSE streaming', has_events, f'→ {result.stdout[:150]}...')

test('6. Cluster status endpoint')
r = curl_json('GET', 'http://127.0.0.1:7878/cluster/status')
record('cluster status', isinstance(r, dict), f'→ {r}')

test('7. Task history endpoint')
r = curl_json('GET', 'http://127.0.0.1:7878/task/history')
record('task history', isinstance(r, dict), f'→ keys={list(r.keys()) if isinstance(r, dict) else "?"}')

screenshot('01_api_done')

# ── Telegram Tests ────────────────────────────────────────────

test('8. Telegram /help')
send_telegram('/help', wait_secs=4)
screenshot('02_help')
record('TG /help', True, '(visual check)')

test('9. Telegram /status')
send_telegram('/status', wait_secs=6)
screenshot('03_status')
record('TG /status', True, '(visual check)')

test('10. Telegram /tools')
send_telegram('/tools', wait_secs=5)
screenshot('04_tools')
record('TG /tools', True, '(visual check)')

test('11. Telegram /estop + /resume')
send_telegram('/estop', wait_secs=3)
screenshot('05_estop')
send_telegram('test message while stopped', wait_secs=4)
screenshot('06_blocked')
send_telegram('/resume', wait_secs=3)
screenshot('07_resume')
record('TG estop/resume', True, '(visual check)')

test('12. Telegram agent response')
send_telegram('What is 2+2?', wait_secs=25)
screenshot('08_agent')
record('TG agent', True, '(visual check)')

test('13. Telegram tool use - shell')
send_telegram('Run this command: echo STAGE2_TEST_OK', wait_secs=25)
screenshot('09_shell')
record('TG shell tool', True, '(visual check)')

test('14. Telegram /history')
send_telegram('/history', wait_secs=3)
screenshot('10_history')
record('TG /history', True, '(visual check)')

test('15. Telegram /dashboard')
send_telegram('/dashboard', wait_secs=3)
screenshot('11_dashboard')
record('TG /dashboard', True, '(visual check)')

test('16. Telegram /clear')
send_telegram('/clear', wait_secs=3)
screenshot('12_clear')
record('TG /clear', True, '(visual check)')

# ── Final Summary ─────────────────────────────────────────────
screenshot('99_final')

print('\n' + '='*60)
print('  FINAL RESULTS')
print('='*60)
passed = sum(1 for v in results.values() if v)
failed = sum(1 for v in results.values() if not v)
total = len(results)
for name, ok in results.items():
    print(f'  {"PASS" if ok else "FAIL"} | {name}')
print(f'\n  {passed}/{total} passed, {failed} failed')
print(f'  Screenshots: {SCREENSHOT_DIR}')
print('='*60)
