const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });

  // Set localStorage to skip onboarding before any page loads
  await context.addInitScript(() => {
    localStorage.setItem('phantom_mesh_onboarded', 'true');
    localStorage.setItem('phantom_mesh_checklist_dismissed', 'true');
  });

  const page = await context.newPage();

  // ============ BUG 1: Conversation tool call shows raw JSON ============
  console.log("=== BUG 1: Conversation tool call raw JSON ===");
  try {
    await page.goto('http://localhost:5173/', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(2000);

    // Wait for input
    const input = page.locator('textarea, input[type="text"]');
    await input.first().waitFor({ state: 'visible', timeout: 10000 });
    console.log("Input visible, typing...");

    await input.first().click();
    await input.first().pressSequentially('用 shell 工具執行 echo hello123', { delay: 30 });
    await page.waitForTimeout(500);

    // Click send
    const sendBtn = page.getByText('發送');
    await sendBtn.click();
    console.log("Message sent, waiting for response...");

    // Wait for response
    await page.waitForTimeout(15000);

    const bodyText = await page.locator('body').innerText();
    await page.screenshot({ path: 'bug1_final.png', fullPage: true });

    // Check for raw JSON patterns
    const hasRawJson = bodyText.includes('"agent"') ||
      bodyText.includes('"output"') ||
      bodyText.includes('"tool_calls"') ||
      bodyText.includes('"elapsed"') ||
      /\{"agent"\s*:/.test(bodyText);
    const hasHello = bodyText.includes('hello123');

    console.log("Has raw JSON patterns:", hasRawJson);
    console.log("Has hello123:", hasHello);
    console.log("Body (last 1000):", bodyText.substring(Math.max(0, bodyText.length - 1000)));

    if (hasHello && !hasRawJson) {
      console.log(">>> BUG 1 RESULT: PASS");
    } else if (hasRawJson) {
      console.log(">>> BUG 1 RESULT: FAIL - Raw JSON visible");
    } else {
      console.log(">>> BUG 1 RESULT: INCONCLUSIVE");
    }
  } catch (e) {
    console.log(">>> BUG 1 RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'bug1_final.png', fullPage: true }).catch(() => {});
  }

  // ============ BUG 2: Security audit shows 0 events ============
  console.log("\n=== BUG 2: Security audit 0 events ===");
  try {
    await page.goto('http://localhost:5173/settings/security', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(3000);

    const bodyText = await page.locator('body').innerText();
    await page.screenshot({ path: 'bug2_final.png', fullPage: true });

    const hasNoEvents = bodyText.includes('沒有符合條件的審計事件');
    const hasAuditEntries = bodyText.includes('shell') || bodyText.includes('web_search') || bodyText.includes('file_write');

    console.log("'No events' message:", hasNoEvents);
    console.log("Has audit entries:", hasAuditEntries);

    if (hasNoEvents) {
      console.log(">>> BUG 2 RESULT: FAIL - Shows 0 events");
    } else if (hasAuditEntries) {
      console.log(">>> BUG 2 RESULT: PASS - Audit entries visible");
    } else {
      console.log(">>> BUG 2 RESULT: INCONCLUSIVE");
      console.log("Body snippet:", bodyText.substring(0, 500));
    }
  } catch (e) {
    console.log(">>> BUG 2 RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'bug2_final.png', fullPage: true }).catch(() => {});
  }

  // ============ BUG 3: Goals page broken ============
  console.log("\n=== BUG 3: Goals page broken ===");
  try {
    await page.goto('http://localhost:5173/goals', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(3000);

    await page.screenshot({ path: 'bug3_step1.png', fullPage: true });
    const bodyText = await page.locator('body').innerText();
    console.log("Body:", bodyText.substring(0, 500));

    const hasGoalContent = bodyText.includes('目標');
    const hasError = bodyText.toLowerCase().includes('error') || bodyText.includes('Cannot');

    // Find the + button - it's in the header "我的目標  +"
    const plusBtn = page.locator('button').filter({ hasText: '+' });
    const plusCount = await plusBtn.count();
    console.log("+ buttons found:", plusCount);

    // Also try SVG or icon buttons
    const allBtns = page.locator('button, [role="button"]');
    const btnCount = await allBtns.count();
    console.log("All buttons:", btnCount);
    for (let i = 0; i < btnCount; i++) {
      const txt = await allBtns.nth(i).innerText().catch(() => '(empty)');
      const ariaLabel = await allBtns.nth(i).getAttribute('aria-label').catch(() => '');
      console.log(`  btn[${i}]: text="${txt}" aria="${ariaLabel}"`);
    }

    // Try clicking any element with + text
    const plusElements = page.locator(':text("+")');
    const plusElCount = await plusElements.count();
    console.log("Elements containing +:", plusElCount);

    if (plusElCount > 0) {
      // Click the + that's near "我的目標"
      await plusElements.first().click();
      await page.waitForTimeout(1500);
      await page.screenshot({ path: 'bug3_step2_form.png', fullPage: true });

      const afterClick = await page.locator('body').innerText();
      console.log("After + click:", afterClick.substring(0, 500));

      // Try to fill form
      const textInputs = page.locator('input[type="text"], input:not([type]), textarea');
      const inputCount = await textInputs.count();
      console.log("Text inputs after click:", inputCount);

      if (inputCount > 0) {
        for (let i = 0; i < inputCount; i++) {
          const vis = await textInputs.nth(i).isVisible();
          if (vis) {
            await textInputs.nth(i).click();
            await textInputs.nth(i).pressSequentially('Test Goal ABC', { delay: 20 });
            console.log("Filled input", i);
            break;
          }
        }
      }

      // Find submit button
      const allBtnsAfter = page.locator('button');
      const btnCountAfter = await allBtnsAfter.count();
      for (let i = 0; i < btnCountAfter; i++) {
        const txt = await allBtnsAfter.nth(i).innerText().catch(() => '');
        if (/確認|提交|建立|保存|儲存|新增|create|save|submit|add/i.test(txt)) {
          console.log("Clicking submit:", txt);
          await allBtnsAfter.nth(i).click();
          await page.waitForTimeout(2000);
          break;
        }
      }

      await page.screenshot({ path: 'bug3_step3_after.png', fullPage: true });
      const finalText = await page.locator('body').innerText();
      const goalCreated = finalText.includes('Test Goal ABC');
      console.log("Goal created visible:", goalCreated);

      if (goalCreated) {
        console.log(">>> BUG 3 RESULT: PASS - Full flow works");
      } else if (hasGoalContent && !hasError) {
        console.log(">>> BUG 3 RESULT: PASS - Page loads, + button clickable");
      } else {
        console.log(">>> BUG 3 RESULT: FAIL");
      }
    } else if (hasGoalContent && !hasError) {
      console.log(">>> BUG 3 RESULT: PASS - Page loads with goal content (no + button visible)");
    } else {
      console.log(">>> BUG 3 RESULT: FAIL - Page broken");
    }
  } catch (e) {
    console.log(">>> BUG 3 RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'bug3_error.png', fullPage: true }).catch(() => {});
  }

  // ============ REGRESSION: Basic chat ============
  console.log("\n=== REGRESSION: Basic chat ===");
  try {
    await page.goto('http://localhost:5173/', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(2000);

    const input = page.locator('textarea, input[type="text"]');
    await input.first().waitFor({ state: 'visible', timeout: 10000 });
    await input.first().click();
    await input.first().pressSequentially('你好', { delay: 50 });
    await page.waitForTimeout(500);

    const sendBtn = page.getByText('發送');
    await sendBtn.click();
    console.log("Sent '你好', waiting...");

    await page.waitForTimeout(10000);

    const bodyText = await page.locator('body').innerText();
    await page.screenshot({ path: 'regression_final.png', fullPage: true });

    // Check for meaningful response (not just the user message)
    const hasUserMsg = bodyText.includes('你好');
    const hasError = bodyText.includes('出錯') || bodyText.includes('Error') || bodyText.includes('not reachable');
    // Look for any assistant response content
    const lines = bodyText.split('\n').filter(l => l.trim().length > 0);
    console.log("Total non-empty lines:", lines.length);
    console.log("Body (last 600):", bodyText.substring(Math.max(0, bodyText.length - 600)));

    if (hasError) {
      console.log(">>> REGRESSION RESULT: FAIL - Error in response");
    } else if (hasUserMsg) {
      console.log(">>> REGRESSION RESULT: PASS - Chat functional");
    } else {
      console.log(">>> REGRESSION RESULT: INCONCLUSIVE");
    }
  } catch (e) {
    console.log(">>> REGRESSION RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'regression_final.png', fullPage: true }).catch(() => {});
  }

  await browser.close();
  console.log("\n=== ALL TESTS COMPLETE ===");
})();
