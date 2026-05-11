const { chromium } = require('@playwright/test');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1400, height: 900 } });
  const page = await context.newPage();

  const screenshot = async (name) => {
    await page.screenshot({ path: `screenshots/${name}.png`, fullPage: false });
    console.log(`[SCREENSHOT] ${name}.png saved`);
  };

  // ─── Setup: bypass onboarding ───
  console.log('\n=== SETUP: Bypass onboarding ===');
  await page.goto('http://localhost:5173');
  await page.evaluate(() => localStorage.setItem('phantom_mesh_onboarded', 'true'));
  await page.reload();
  await page.waitForTimeout(2000);

  // ─── TEST: Regression - Basic chat ───
  console.log('\n=== TEST: Regression - Basic chat ===');
  try {
    await page.waitForSelector('textarea, input[type="text"]', { timeout: 10000 });
    const input = await page.$('textarea') || await page.$('input[type="text"]');
    if (!input) throw new Error('Cannot find input field');

    await input.fill('\u4f60\u597d');
    await page.waitForTimeout(500);
    await input.press('Enter');

    console.log('Message sent, waiting for response...');
    await page.waitForTimeout(15000);

    const messages = await page.evaluate(() => {
      const cards = document.querySelectorAll('.bg-phantom-card.border');
      return Array.from(cards).map(c => c.textContent || '');
    });
    console.log(`Found ${messages.length} assistant messages`);

    const errorBanner = await page.$('.bg-phantom-danger\\/20');
    if (errorBanner) {
      const errorText = await errorBanner.textContent();
      console.log(`[REGRESSION] FAIL - Error: ${errorText}`);
    } else if (messages.length > 0 && messages[messages.length - 1].length > 5) {
      console.log('[REGRESSION] PASS - Got a response');
      console.log(`  Response (first 200): ${messages[messages.length - 1].substring(0, 200)}`);
    } else {
      console.log('[REGRESSION] FAIL - No meaningful response');
    }

    await screenshot('01_regression_basic_chat');
  } catch (e) {
    console.log(`[REGRESSION] FAIL - Error: ${e.message}`);
    await screenshot('01_regression_basic_chat_error');
  }

  // ─── TEST Bug 1: Tool call shows raw JSON ───
  console.log('\n=== TEST Bug 1: Tool call raw JSON ===');
  try {
    const input1 = await page.$('textarea') || await page.$('input[type="text"]');
    if (!input1) throw new Error('Cannot find input field');

    await input1.fill('\u7528 shell \u5de5\u5177\u57f7\u884c echo hello123');
    await page.waitForTimeout(500);
    await input1.press('Enter');

    console.log('Tool call message sent, waiting for response...');
    await page.waitForTimeout(20000);

    await screenshot('02_bug1_tool_call');

    const pageContent = await page.evaluate(() => {
      const cards = document.querySelectorAll('.bg-phantom-card.border');
      return Array.from(cards).map(c => c.textContent || '');
    });

    console.log('All assistant messages:');
    pageContent.forEach((t, i) => console.log(`  [${i}]: ${t.substring(0, 300)}`));

    // Check the last two messages (regression + this one)
    const lastMsg = pageContent[pageContent.length - 1] || '';
    const hasRawJson = lastMsg.includes('{"agent"') || lastMsg.includes('"output"') || lastMsg.includes('"tool_calls"');
    const hasHello = lastMsg.includes('hello123');

    if (hasRawJson) {
      console.log('[BUG 1] FAIL - Raw JSON visible in the response');
    } else if (hasHello) {
      console.log('[BUG 1] PASS - hello123 visible as readable text, no raw JSON');
    } else {
      console.log(`[BUG 1] UNCLEAR - No raw JSON but no hello123 either`);
      console.log(`  Full last msg: ${lastMsg.substring(0, 500)}`);
    }
  } catch (e) {
    console.log(`[BUG 1] FAIL - Error: ${e.message}`);
    await screenshot('02_bug1_tool_call_error');
  }

  // ─── TEST Bug 2: Security audit ───
  console.log('\n=== TEST Bug 2: Security audit 0 events ===');
  try {
    await page.goto('http://localhost:5173/settings/security');
    await page.waitForTimeout(5000);

    await screenshot('03_bug2_security_audit');

    const noEventsMsg = await page.$('text=\u6c92\u6709\u7b26\u5408\u689d\u4ef6\u7684\u5be9\u8a08\u4e8b\u4ef6');

    const rowCount = await page.evaluate(() => {
      const tbody = document.querySelector('tbody');
      if (!tbody) return 0;
      const rows = tbody.querySelectorAll('tr');
      let dataRows = 0;
      rows.forEach(r => {
        if (!r.textContent.includes('\u6c92\u6709\u7b26\u5408\u689d\u4ef6')) dataRows++;
      });
      return dataRows;
    });

    console.log(`Audit table data rows: ${rowCount}`);

    const statsText = await page.evaluate(() => {
      const statCards = document.querySelectorAll('.grid .text-2xl');
      return Array.from(statCards).map(e => e.textContent).join(', ');
    });
    console.log(`Stats values: ${statsText}`);

    const offlineLabel = await page.$('text=\u96e2\u7dda\u6a21\u5f0f');
    if (offlineLabel) {
      console.log('Note: Page is showing offline mode (using mock data)');
    }

    if (noEventsMsg) {
      console.log('[BUG 2] FAIL - Shows empty audit message');
    } else if (rowCount > 0) {
      console.log(`[BUG 2] PASS - Shows ${rowCount} audit rows`);
    } else {
      console.log('[BUG 2] FAIL - No data rows found');
    }
  } catch (e) {
    console.log(`[BUG 2] FAIL - Error: ${e.message}`);
    await screenshot('03_bug2_security_audit_error');
  }

  // ─── TEST Bug 3: Goals page ───
  console.log('\n=== TEST Bug 3: Goals page ===');
  try {
    // Step 1
    await page.goto('http://localhost:5173/goals');
    await page.waitForTimeout(5000);

    await screenshot('04_bug3_goals_step1');

    const goalCount = await page.evaluate(() => {
      const btns = document.querySelectorAll('button');
      let count = 0;
      btns.forEach(b => {
        if (b.textContent.includes('\u9032\u884c\u4e2d') || b.textContent.includes('\u5df2\u5b8c\u6210')) count++;
      });
      return count;
    });

    console.log(`Existing goals displayed: ${goalCount}`);

    const goalError = await page.$('.bg-phantom-danger\\/20');
    if (goalError) {
      const errorText = await goalError.textContent();
      console.log(`Goal page error: ${errorText}`);
    }

    const noGoals = await page.evaluate(() => document.body.textContent.includes('\u9084\u6c92\u6709\u76ee\u6a19'));

    if (goalCount > 0) {
      console.log('[BUG 3 Step 1] PASS - Existing goals displayed');
    } else if (noGoals) {
      console.log('[BUG 3 Step 1] FAIL - Shows no goals message despite API having goals');
    } else {
      console.log('[BUG 3 Step 1] UNCLEAR - checking page content...');
      const bodyText = await page.evaluate(() => document.body.textContent.substring(0, 500));
      console.log(`  Page content: ${bodyText}`);
    }

    // Step 2: Create new goal
    console.log('\n--- Bug 3 Step 2: Create goal ---');
    // Click + button
    const plusClicked = await page.evaluate(() => {
      const btns = document.querySelectorAll('button');
      for (const btn of btns) {
        if (btn.querySelector('svg') && btn.closest('.flex.items-center.justify-between')) {
          const svg = btn.querySelector('svg');
          if (svg && (svg.classList.contains('lucide-plus') || btn.innerHTML.includes('line x1="12"'))) {
            btn.click();
            return true;
          }
        }
      }
      // Fallback: look for any small button with Plus icon near "my goals"
      for (const btn of btns) {
        if (btn.innerHTML.includes('12" y1="5" x2="12" y2="19"')) {
          btn.click();
          return true;
        }
      }
      return false;
    });

    if (!plusClicked) {
      console.log('Trying alternate + button selector...');
      // Lucide Plus icon has specific path
      const plusBtn = await page.evaluate(() => {
        const btns = document.querySelectorAll('button');
        for (const btn of btns) {
          const svgs = btn.querySelectorAll('svg');
          for (const svg of svgs) {
            const lines = svg.querySelectorAll('line, path');
            if (lines.length <= 3 && svg.closest('button')) {
              // Small icon button near goal header
              const parent = btn.parentElement;
              if (parent && parent.textContent.includes('\u6211\u7684\u76ee\u6a19')) {
                btn.click();
                return true;
              }
            }
          }
        }
        return false;
      });
      console.log(`Alt + button found: ${plusBtn}`);
    }

    await page.waitForTimeout(1000);
    await screenshot('04_bug3_goals_step2_form');

    // Fill form
    const titleInput = await page.$('input[placeholder="\u76ee\u6a19\u540d\u7a31"]');
    if (titleInput) {
      await titleInput.fill('\u6e2c\u8a66\u76ee\u6a19');
      console.log('Filled title');
    } else {
      console.log('Cannot find title input');
    }

    const descTextarea = await page.$('textarea');
    if (descTextarea) {
      await descTextarea.fill('\u81ea\u52d5\u5316\u6e2c\u8a66');
      console.log('Filled description');
    }

    await page.waitForTimeout(500);

    // Click create
    const createClicked = await page.evaluate(() => {
      const btns = document.querySelectorAll('button');
      for (const btn of btns) {
        if (btn.textContent.trim() === '\u5efa\u7acb') {
          btn.click();
          return true;
        }
      }
      return false;
    });
    console.log(`Clicked create: ${createClicked}`);

    await page.waitForTimeout(3000);
    await screenshot('04_bug3_goals_step3_after_create');

    // Step 3: Check new goal
    const newGoalVisible = await page.evaluate(() => {
      return document.body.textContent.includes('\u6e2c\u8a66\u76ee\u6a19');
    });

    if (newGoalVisible) {
      console.log('[BUG 3 Step 2+3] PASS - New goal created and visible');
    } else {
      console.log('[BUG 3 Step 2+3] FAIL - New goal not visible after creation');
    }

  } catch (e) {
    console.log(`[BUG 3] FAIL - Error: ${e.message}`);
    await screenshot('04_bug3_goals_error');
  }

  console.log('\n=== ALL TESTS DONE ===');
  await browser.close();
})().catch(e => {
  console.error('Fatal:', e);
  process.exit(1);
});
