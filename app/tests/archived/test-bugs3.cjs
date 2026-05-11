const { chromium } = require('@playwright/test');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1400, height: 900 } });
  const page = await context.newPage();

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
  await page.goto('http://localhost:5173');
  await page.waitForTimeout(2000);

  // ═══ REGRESSION: Basic chat ═══
  console.log('\n=== REGRESSION: Basic chat ===');
  try {
    const inputSel = 'input[type="text"]';
    await page.waitForSelector(inputSel, { timeout: 10000 });
    await page.fill(inputSel, 'say hi in Chinese briefly');
    await page.waitForTimeout(300);
    await page.press(inputSel, 'Enter');
    console.log('Sent, waiting 20s for LLM response...');
    await page.waitForTimeout(20000);

    const assistantTexts = await page.evaluate(() => {
      // Assistant messages have bg-phantom-card + border classes and contain <p>
      const cards = document.querySelectorAll('.bg-phantom-card.border.border-phantom-border');
      const result = [];
      cards.forEach(c => {
        const p = c.querySelector('p.text-sm');
        if (p) result.push(p.textContent || '');
      });
      return result;
    });

    console.log(`Assistant responses: ${assistantTexts.length}`);
    assistantTexts.forEach((t, i) => console.log(`  [${i}] "${t.substring(0, 200)}"`));

    const lastResp = assistantTexts[assistantTexts.length - 1] || '';
    const errorBanner = await page.$('.bg-phantom-danger\\/20');

    if (errorBanner) {
      const et = await errorBanner.textContent();
      console.log(`[REGRESSION] FAIL - Error: ${et}`);
    } else if (lastResp.length > 3) {
      console.log('[REGRESSION] PASS - Got meaningful response');
    } else {
      console.log('[REGRESSION] FAIL - Empty or no response');
    }
    await screenshot('20_regression');
  } catch (e) {
    console.log(`[REGRESSION] ERROR - ${e.message}`);
    await screenshot('20_regression_err');
  }

  // ═══ BUG 1: Tool call raw JSON ═══
  console.log('\n=== BUG 1: Tool call raw JSON ===');
  try {
    const inputSel = 'input[type="text"]';
    await page.fill(inputSel, 'use shell tool to run: echo hello123');
    await page.waitForTimeout(300);
    await page.press(inputSel, 'Enter');
    console.log('Sent tool command, waiting 25s...');
    await page.waitForTimeout(25000);

    const allMsgs = await page.evaluate(() => {
      const cards = document.querySelectorAll('.bg-phantom-card.border.border-phantom-border');
      return Array.from(cards).map(c => {
        const p = c.querySelector('p.text-sm');
        return {
          pText: p?.textContent || '',
          fullText: c.textContent || '',
        };
      });
    });

    console.log(`Total assistant bubbles: ${allMsgs.length}`);
    const last = allMsgs[allMsgs.length - 1];
    console.log(`Last bubble pText: "${last?.pText?.substring(0, 300)}"`);
    console.log(`Last bubble full: "${last?.fullText?.substring(0, 300)}"`);

    const text = last?.fullText || '';
    const hasRawJson = text.includes('{"agent"') || (text.includes('"output"') && text.includes('"tool_calls"'));
    const hasHello = text.includes('hello123');

    if (hasRawJson) {
      console.log('[BUG 1] FAIL - Raw JSON in response');
    } else if (hasHello || (last?.pText?.length || 0) > 10) {
      console.log('[BUG 1] PASS - Readable text, no raw JSON');
    } else {
      console.log('[BUG 1] UNCLEAR - check screenshot');
    }
    await screenshot('21_bug1');
  } catch (e) {
    console.log(`[BUG 1] ERROR - ${e.message}`);
    await screenshot('21_bug1_err');
  }

  // ═══ BUG 2: Security audit ═══
  console.log('\n=== BUG 2: Security audit ===');
  try {
    await page.goto('http://localhost:5173/settings/security');
    await page.waitForTimeout(6000);

    const result = await page.evaluate(() => {
      const tbody = document.querySelector('tbody');
      if (!tbody) return { rows: 0, noEventsMsg: false, offline: false, stats: [] };
      const rows = tbody.querySelectorAll('tr');
      let dataRows = 0, noEventsMsg = false;
      rows.forEach(r => {
        if (r.textContent.includes('\u6c92\u6709\u7b26\u5408\u689d\u4ef6')) noEventsMsg = true;
        else dataRows++;
      });
      const offline = document.body.textContent.includes('\u96e2\u7dda\u6a21\u5f0f');
      const stats = Array.from(document.querySelectorAll('.text-2xl.font-bold')).map(e => e.textContent);
      return { rows: dataRows, noEventsMsg, offline, stats };
    });

    console.log(`Rows: ${result.rows}, NoEventsMsg: ${result.noEventsMsg}, Offline: ${result.offline}, Stats: ${JSON.stringify(result.stats)}`);

    if (result.noEventsMsg) {
      console.log('[BUG 2] FAIL - Empty audit message');
    } else if (result.rows >= 20) {
      console.log(`[BUG 2] PASS - ${result.rows} real audit entries from API`);
    } else {
      console.log(`[BUG 2] FAIL - Only ${result.rows} rows`);
    }
    await screenshot('22_bug2');
  } catch (e) {
    console.log(`[BUG 2] ERROR - ${e.message}`);
    await screenshot('22_bug2_err');
  }

  // ═══ BUG 3: Goals page ═══
  console.log('\n=== BUG 3: Goals page ===');
  try {
    await page.goto('http://localhost:5173/goals');
    await page.waitForTimeout(5000);

    // Step 1: existing goals
    const step1 = await page.evaluate(() => {
      const body = document.body.textContent || '';
      const noGoals = body.includes('\u9084\u6c92\u6709\u76ee\u6a19');
      // Check for goal items by looking for status badges
      const badges = document.querySelectorAll('span');
      let goalCount = 0;
      badges.forEach(b => {
        if (b.textContent === '\u9032\u884c\u4e2d' || b.textContent === '\u5df2\u5b8c\u6210') goalCount++;
      });
      return { noGoals, goalCount };
    });

    console.log(`Step 1: noGoals=${step1.noGoals}, goalCount=${step1.goalCount}`);
    await screenshot('23_bug3_step1');

    if (step1.goalCount > 0) {
      console.log(`[BUG 3 Step 1] PASS - ${step1.goalCount} goals visible`);
    } else {
      console.log('[BUG 3 Step 1] FAIL - No goals displayed (API has 3+)');
    }

    // Step 2: Create goal via + button
    console.log('\n--- Step 2: Create goal ---');
    // The + button is inside a flex container with "my goals" header
    await page.evaluate(() => {
      const h3s = document.querySelectorAll('h3');
      for (const h of h3s) {
        if (h.textContent.includes('\u6211\u7684\u76ee\u6a19')) {
          const parent = h.closest('.flex.items-center.justify-between');
          if (parent) {
            const btn = parent.querySelector('button');
            if (btn) btn.click();
          }
        }
      }
    });
    await page.waitForTimeout(1000);

    const formExists = await page.$('input[placeholder="\u76ee\u6a19\u540d\u7a31"]');
    if (formExists) {
      console.log('Form appeared');
      await page.fill('input[placeholder="\u76ee\u6a19\u540d\u7a31"]', '\u6e2c\u8a66\u76ee\u6a19');

      const descInput = await page.$('textarea');
      if (descInput) await descInput.fill('\u81ea\u52d5\u5316\u6e2c\u8a66');

      await screenshot('23_bug3_step2_filled');

      // Click create button
      await page.evaluate(() => {
        const btns = document.querySelectorAll('button');
        for (const b of btns) {
          if (b.textContent.trim() === '\u5efa\u7acb') { b.click(); break; }
        }
      });
      await page.waitForTimeout(4000);

      await screenshot('23_bug3_step3');

      // Check result
      const step3 = await page.evaluate(() => {
        const visible = document.body.textContent.includes('\u6e2c\u8a66\u76ee\u6a19');
        const noGoals = document.body.textContent.includes('\u9084\u6c92\u6709\u76ee\u6a19');
        return { visible, noGoals };
      });

      // Also check API directly
      const apiCheck = await page.evaluate(async () => {
        const r = await fetch('http://localhost:7878/goals', {
          headers: { 'Authorization': 'Bearer e9723eea85484da6b39d5abdcdcef6bf' }
        });
        const d = await r.json();
        return d.goals?.map(g => g.title) || [];
      });
      console.log(`API goals: ${JSON.stringify(apiCheck)}`);

      if (step3.visible) {
        console.log('[BUG 3 Step 2+3] PASS - Goal visible in UI');
      } else if (apiCheck.includes('\u6e2c\u8a66\u76ee\u6a19')) {
        console.log('[BUG 3 Step 2+3] PARTIAL FAIL - Goal created in API but NOT shown in UI');
      } else if (step3.noGoals) {
        console.log('[BUG 3 Step 2+3] FAIL - Still shows no goals');
      } else {
        console.log('[BUG 3 Step 2+3] FAIL - Goal not found anywhere');
      }
    } else {
      console.log('[BUG 3 Step 2] FAIL - Form did not appear after clicking +');
      await screenshot('23_bug3_no_form');
    }
  } catch (e) {
    console.log(`[BUG 3] ERROR - ${e.message}`);
    await screenshot('23_bug3_err');
  }

  if (consoleErrors.length > 0) {
    console.log('\n=== Console Errors ===');
    consoleErrors.forEach(e => console.log(`  ${e}`));
  }

  console.log('\n=== COMPLETE ===');
  await browser.close();
})().catch(e => { console.error('Fatal:', e); process.exit(1); });
