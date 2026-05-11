const { chromium } = require('@playwright/test');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1400, height: 900 } });
  const page = await context.newPage();

  // Capture console errors
  const consoleErrors = [];
  page.on('console', msg => {
    if (msg.type() === 'error' || msg.type() === 'warn') {
      consoleErrors.push(`[${msg.type()}] ${msg.text()}`);
    }
  });

  const screenshot = async (name) => {
    await page.screenshot({ path: `screenshots/${name}.png`, fullPage: false });
    console.log(`[SCREENSHOT] ${name}.png`);
  };

  // Setup: bypass onboarding
  console.log('\n=== SETUP ===');
  await page.goto('http://localhost:5173');
  await page.evaluate(() => localStorage.setItem('phantom_mesh_onboarded', 'true'));
  await page.reload();
  await page.waitForTimeout(2000);

  // ═══ REGRESSION: Basic chat ═══
  console.log('\n=== REGRESSION: Basic chat ===');
  try {
    await page.waitForSelector('textarea', { timeout: 5000 });

    // Use English to avoid encoding issues with the daemon API
    await page.fill('textarea', 'say hello in Chinese, one sentence only');
    await page.waitForTimeout(300);
    await page.press('textarea', 'Enter');

    console.log('Sent message, waiting 15s...');
    await page.waitForTimeout(15000);

    // Check response content
    const assistantMsgs = await page.evaluate(() => {
      const cards = document.querySelectorAll('.bg-phantom-card.border.border-phantom-border');
      return Array.from(cards).map(c => ({
        text: c.querySelector('p')?.textContent || '',
        full: c.textContent || ''
      }));
    });

    console.log(`Assistant messages: ${assistantMsgs.length}`);
    assistantMsgs.forEach((m, i) => console.log(`  [${i}] text: "${m.text.substring(0, 200)}"`));

    const lastMsg = assistantMsgs[assistantMsgs.length - 1];
    const errorBanner = await page.$('.bg-phantom-danger\\/20');

    if (errorBanner) {
      const et = await errorBanner.textContent();
      console.log(`[REGRESSION] FAIL - Error banner: ${et}`);
    } else if (lastMsg && lastMsg.text.length > 3) {
      console.log('[REGRESSION] PASS - Got meaningful response');
    } else {
      console.log('[REGRESSION] FAIL - Empty or missing response');
    }
    await screenshot('10_regression_chat');
  } catch (e) {
    console.log(`[REGRESSION] ERROR - ${e.message}`);
    await screenshot('10_regression_error');
  }

  // ═══ BUG 1: Tool call raw JSON ═══
  console.log('\n=== BUG 1: Tool call raw JSON ===');
  try {
    await page.fill('textarea', 'use shell tool to execute: echo hello123');
    await page.waitForTimeout(300);
    await page.press('textarea', 'Enter');

    console.log('Sent tool call message, waiting 25s...');
    await page.waitForTimeout(25000);

    // Get all assistant messages
    const allMsgs = await page.evaluate(() => {
      const cards = document.querySelectorAll('.bg-phantom-card.border.border-phantom-border');
      return Array.from(cards).map(c => ({
        text: c.querySelector('p')?.textContent || '',
        hasToolCall: c.querySelector('.font-mono.text-phantom-primary') !== null,
        toolCallText: c.querySelector('.font-mono.text-phantom-primary')?.textContent || '',
        full: c.textContent || ''
      }));
    });

    console.log(`Total assistant messages: ${allMsgs.length}`);
    const lastMsg = allMsgs[allMsgs.length - 1];
    console.log(`Last msg text: "${lastMsg?.text?.substring(0, 300)}"`);
    console.log(`Last msg full: "${lastMsg?.full?.substring(0, 300)}"`);

    // Check for raw JSON pattern
    const fullText = lastMsg?.full || '';
    const hasRawJsonAgent = fullText.includes('{"agent"');
    const hasRawJsonOutput = fullText.includes('"output"');
    const hasRawJsonToolCalls = fullText.includes('"tool_calls"');
    const hasHello = fullText.includes('hello123');

    if (hasRawJsonAgent || (hasRawJsonOutput && hasRawJsonToolCalls)) {
      console.log('[BUG 1] FAIL - Raw JSON object visible in response');
    } else if (hasHello || lastMsg?.text?.length > 10) {
      console.log('[BUG 1] PASS - Readable text shown, no raw JSON');
    } else {
      console.log('[BUG 1] UNCLEAR - Response may be empty or unexpected');
    }
    await screenshot('11_bug1_tool_call');
  } catch (e) {
    console.log(`[BUG 1] ERROR - ${e.message}`);
    await screenshot('11_bug1_error');
  }

  // ═══ BUG 2: Security audit ═══
  console.log('\n=== BUG 2: Security audit ===');
  try {
    await page.goto('http://localhost:5173/settings/security');
    await page.waitForTimeout(5000);

    // Check for loading state
    const isLoading = await page.$('text=\u8f09\u5165\u5be9\u8a08\u65e5\u8a8c\u4e2d');
    if (isLoading) {
      console.log('Still loading, waiting more...');
      await page.waitForTimeout(5000);
    }

    const result = await page.evaluate(() => {
      const tbody = document.querySelector('tbody');
      if (!tbody) return { rows: 0, noEventsMsg: false, offline: false };

      const rows = tbody.querySelectorAll('tr');
      let dataRows = 0;
      let noEventsMsg = false;
      rows.forEach(r => {
        if (r.textContent.includes('\u6c92\u6709\u7b26\u5408\u689d\u4ef6')) noEventsMsg = true;
        else dataRows++;
      });

      const offline = document.body.textContent.includes('\u96e2\u7dda\u6a21\u5f0f');
      const stats = Array.from(document.querySelectorAll('.text-2xl.font-bold')).map(e => e.textContent);

      return { rows: dataRows, noEventsMsg, offline, stats };
    });

    console.log(`Rows: ${result.rows}, NoEventsMsg: ${result.noEventsMsg}, Offline: ${result.offline}`);
    console.log(`Stats: ${JSON.stringify(result.stats)}`);

    if (result.noEventsMsg) {
      console.log('[BUG 2] FAIL - Shows empty audit message');
    } else if (result.rows >= 20) {
      console.log(`[BUG 2] PASS - Shows ${result.rows} real audit entries from API`);
      if (result.offline) console.log('  Note: showing offline mode despite having data');
    } else if (result.rows > 0) {
      console.log(`[BUG 2] PARTIAL PASS - Shows ${result.rows} rows (expected 24)`);
    } else {
      console.log('[BUG 2] FAIL - No data rows');
    }
    await screenshot('12_bug2_security');
  } catch (e) {
    console.log(`[BUG 2] ERROR - ${e.message}`);
    await screenshot('12_bug2_error');
  }

  // ═══ BUG 3: Goals page ═══
  console.log('\n=== BUG 3: Goals page ===');
  try {
    // Step 1: Check existing goals
    await page.goto('http://localhost:5173/goals');
    await page.waitForTimeout(5000);

    const goalsResult = await page.evaluate(() => {
      const body = document.body.textContent || '';
      const hasNoGoals = body.includes('\u9084\u6c92\u6709\u76ee\u6a19');
      const hasLoading = body.includes('\u8f09\u5165\u76ee\u6a19\u8cc7\u6599');
      const hasError = document.querySelector('.bg-phantom-danger\\/20');

      // Look for goal buttons with status labels
      const btns = document.querySelectorAll('button');
      let goalBtns = 0;
      btns.forEach(b => {
        if (b.textContent.includes('\u9032\u884c\u4e2d') && b.closest('.space-y-1\\.5')) goalBtns++;
      });

      return { hasNoGoals, hasLoading, goalBtns, hasError: !!hasError };
    });

    console.log(`Goals result: ${JSON.stringify(goalsResult)}`);
    await screenshot('13_bug3_goals_step1');

    if (goalsResult.goalBtns > 0) {
      console.log(`[BUG 3 Step 1] PASS - ${goalsResult.goalBtns} goals displayed`);
    } else if (goalsResult.hasNoGoals) {
      console.log('[BUG 3 Step 1] FAIL - Shows "no goals" despite API having 3 goals');
      // Root cause: tauri-compat goals_list returns the array directly,
      // but Goals.tsx expects { goals: [...] }
    } else {
      console.log('[BUG 3 Step 1] FAIL - Goals not visible');
    }

    // Step 2: Create a new goal
    console.log('\n--- Bug 3 Step 2: Create goal ---');

    // Click + button
    await page.click('button:has(svg)');  // The Plus button
    await page.waitForTimeout(500);

    // Try to find the + specifically near "my goals"
    const plusClicked = await page.evaluate(() => {
      // Find the header that says our goals
      const headers = document.querySelectorAll('h3');
      for (const h of headers) {
        if (h.textContent.includes('\u6211\u7684\u76ee\u6a19')) {
          const container = h.closest('.flex');
          if (container) {
            const btn = container.querySelector('button');
            if (btn) { btn.click(); return true; }
          }
        }
      }
      return false;
    });
    console.log(`Plus button clicked: ${plusClicked}`);
    await page.waitForTimeout(1000);

    // Check if form appeared
    const formVisible = await page.$('input[placeholder="\u76ee\u6a19\u540d\u7a31"]');
    if (!formVisible) {
      console.log('Form not visible, trying again...');
      await screenshot('13_bug3_no_form');
    } else {
      await page.fill('input[placeholder="\u76ee\u6a19\u540d\u7a31"]', '\u6e2c\u8a66\u76ee\u6a19');
      // Fill description
      const textarea = await page.$('textarea[placeholder*="\u63cf\u8ff0"]');
      if (textarea) await textarea.fill('\u81ea\u52d5\u5316\u6e2c\u8a66');

      await screenshot('13_bug3_goals_step2_filled');

      // Click create
      await page.click('button:has-text("\u5efa\u7acb")');
      await page.waitForTimeout(3000);

      await screenshot('13_bug3_goals_step3_created');

      // Verify via API
      const apiGoals = await page.evaluate(async () => {
        const resp = await fetch('http://localhost:7878/goals', {
          headers: { 'Authorization': 'Bearer e9723eea85484da6b39d5abdcdcef6bf' }
        });
        const data = await resp.json();
        return data.goals?.map(g => ({ title: g.title, desc: g.description })) || [];
      });
      console.log(`API goals after create: ${JSON.stringify(apiGoals)}`);

      const newGoalInApi = apiGoals.some(g => g.title === '\u6e2c\u8a66\u76ee\u6a19');
      const newGoalInUI = await page.evaluate(() => document.body.textContent.includes('\u6e2c\u8a66\u76ee\u6a19'));

      if (newGoalInApi && newGoalInUI) {
        console.log('[BUG 3 Step 2+3] PASS - Goal created and visible in both API and UI');
      } else if (newGoalInApi && !newGoalInUI) {
        console.log('[BUG 3 Step 2+3] PARTIAL - Goal in API but NOT visible in UI (display bug)');
      } else {
        console.log(`[BUG 3 Step 2+3] FAIL - API: ${newGoalInApi}, UI: ${newGoalInUI}`);
      }
    }
  } catch (e) {
    console.log(`[BUG 3] ERROR - ${e.message}`);
    await screenshot('13_bug3_error');
  }

  // Print console errors
  if (consoleErrors.length > 0) {
    console.log('\n=== Browser Console Errors/Warnings ===');
    consoleErrors.slice(0, 20).forEach(e => console.log(`  ${e}`));
  }

  console.log('\n=== ALL TESTS COMPLETE ===');
  await browser.close();
})().catch(e => {
  console.error('Fatal:', e);
  process.exit(1);
});
