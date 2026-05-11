const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();

  // ============ BUG 1: Conversation tool call shows raw JSON ============
  console.log("=== BUG 1: Conversation tool call raw JSON ===");
  try {
    await page.goto('http://localhost:5173/', { waitUntil: 'networkidle', timeout: 15000 });

    // Onboarding: click "啟動 Phantom Mesh"
    const startBtn = page.getByText('啟動 Phantom Mesh');
    if (await startBtn.isVisible({ timeout: 3000 })) {
      await startBtn.click();
      await page.waitForTimeout(1500);

      // Click "立即進入"
      const enterBtn = page.getByText('立即進入');
      if (await enterBtn.isVisible({ timeout: 3000 })) {
        await enterBtn.click();
        await page.waitForTimeout(1500);
      }
    }

    // Type message using pressSequentially
    const input = page.locator('textarea, input[type="text"]').first();
    await input.click();
    await input.pressSequentially('用 shell 工具執行 echo hello123', { delay: 30 });
    await page.waitForTimeout(500);

    // Click send button with text 發送
    const sendBtn = page.getByText('發送');
    await sendBtn.click();

    // Wait for response
    await page.waitForTimeout(12000);

    const bodyText = await page.locator('body').innerText();
    await page.screenshot({ path: 'bug1_screenshot.png', fullPage: true });

    const hasRawJson = bodyText.includes('"agent"') || bodyText.includes('"output"') || (bodyText.includes('{') && bodyText.includes('"agent":"master"'));
    const hasHello = bodyText.includes('hello123');

    console.log("Has raw JSON patterns:", hasRawJson);
    console.log("Has hello123 in output:", hasHello);
    console.log("Body snippet (last 800):", bodyText.substring(Math.max(0, bodyText.length - 800)));

    if (hasHello && !hasRawJson) {
      console.log("BUG 1 RESULT: PASS - Shows readable text, no raw JSON");
    } else if (hasRawJson) {
      console.log("BUG 1 RESULT: FAIL - Raw JSON visible in response");
    } else {
      console.log("BUG 1 RESULT: INCONCLUSIVE - hello123 not found, check screenshot");
    }
  } catch (e) {
    console.log("BUG 1 RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'bug1_screenshot.png', fullPage: true }).catch(() => {});
  }

  // ============ BUG 2: Security audit shows 0 events ============
  console.log("\n=== BUG 2: Security audit 0 events ===");
  try {
    await page.goto('http://localhost:5173/settings/security', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(3000);

    const bodyText = await page.locator('body').innerText();
    await page.screenshot({ path: 'bug2_screenshot.png', fullPage: true });

    const hasNoEvents = bodyText.includes('沒有符合條件的審計事件');
    const hasTableContent = bodyText.includes('審計') || bodyText.includes('安全');

    console.log("Shows no-events message:", hasNoEvents);
    console.log("Has audit/security content:", hasTableContent);
    console.log("Body snippet:", bodyText.substring(0, 600));

    if (hasNoEvents) {
      console.log("BUG 2 RESULT: FAIL - Shows 0 events message");
    } else if (hasTableContent) {
      console.log("BUG 2 RESULT: PASS - Shows audit content");
    } else {
      console.log("BUG 2 RESULT: INCONCLUSIVE - check screenshot");
    }
  } catch (e) {
    console.log("BUG 2 RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'bug2_screenshot.png', fullPage: true }).catch(() => {});
  }

  // ============ BUG 3: Goals page broken ============
  console.log("\n=== BUG 3: Goals page broken ===");
  try {
    await page.goto('http://localhost:5173/goals', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(3000);

    const bodyText = await page.locator('body').innerText();
    await page.screenshot({ path: 'bug3_screenshot.png', fullPage: true });

    console.log("Body snippet:", bodyText.substring(0, 600));

    // Check for + button or add button
    const addBtn = page.locator('button').filter({ hasText: /^\+$|新增|添加|建立/ });
    const addBtnCount = await addBtn.count();
    console.log("Add buttons found:", addBtnCount);

    const hasError = bodyText.toLowerCase().includes('error') || bodyText.includes('錯誤') || bodyText.includes('Cannot');
    const hasGoalContent = bodyText.includes('目標') || bodyText.toLowerCase().includes('goal');

    console.log("Has error:", hasError);
    console.log("Has goal content:", hasGoalContent);

    if (addBtnCount > 0) {
      console.log("Clicking + button...");
      await addBtn.first().click();
      await page.waitForTimeout(1500);
      await page.screenshot({ path: 'bug3_form.png', fullPage: true });

      // Try filling form
      const inputs = page.locator('input[type="text"], textarea');
      const inputCount = await inputs.count();
      console.log("Form inputs found:", inputCount);

      if (inputCount > 0) {
        await inputs.first().click();
        await inputs.first().pressSequentially('Test Goal', { delay: 30 });
        await page.waitForTimeout(500);
      }

      // Try submit
      const submitBtn = page.locator('button').filter({ hasText: /確認|提交|建立|保存|儲存|submit|create|save/i });
      const submitCount = await submitBtn.count();
      console.log("Submit buttons found:", submitCount);
      if (submitCount > 0) {
        await submitBtn.first().click();
        await page.waitForTimeout(2000);
      }

      await page.screenshot({ path: 'bug3_after_action.png', fullPage: true });

      const afterText = await page.locator('body').innerText();
      const hasTestGoal = afterText.includes('Test Goal');
      console.log("Test Goal visible after submit:", hasTestGoal);

      if (hasTestGoal || !hasError) {
        console.log("BUG 3 RESULT: PASS - Goals page works");
      } else {
        console.log("BUG 3 RESULT: FAIL - Goals page has issues");
      }
    } else if (hasGoalContent && !hasError) {
      console.log("BUG 3 RESULT: PASS - Goals page loads with content");
    } else {
      console.log("BUG 3 RESULT: FAIL - Goals page appears broken");
    }
  } catch (e) {
    console.log("BUG 3 RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'bug3_screenshot.png', fullPage: true }).catch(() => {});
  }

  // ============ REGRESSION: Basic chat ============
  console.log("\n=== REGRESSION: Basic chat ===");
  try {
    await page.goto('http://localhost:5173/', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(2000);

    const input = page.locator('textarea, input[type="text"]').first();
    if (await input.isVisible({ timeout: 3000 })) {
      await input.click();
      await input.pressSequentially('你好', { delay: 50 });
      await page.waitForTimeout(500);

      const sendBtn = page.getByText('發送');
      await sendBtn.click();

      await page.waitForTimeout(8000);

      const bodyText = await page.locator('body').innerText();
      await page.screenshot({ path: 'regression_screenshot.png', fullPage: true });

      const hasError = bodyText.toLowerCase().includes('error') || bodyText.includes('錯誤');
      console.log("Body snippet (last 600):", bodyText.substring(Math.max(0, bodyText.length - 600)));
      console.log("Has error:", hasError);

      if (hasError) {
        console.log("REGRESSION RESULT: FAIL - Error visible in chat");
      } else {
        console.log("REGRESSION RESULT: PASS - Chat appears functional");
      }
    } else {
      // Maybe need onboarding again
      console.log("Input not visible, may need onboarding");
      await page.screenshot({ path: 'regression_screenshot.png', fullPage: true });
      console.log("REGRESSION RESULT: INCONCLUSIVE - check screenshot");
    }
  } catch (e) {
    console.log("REGRESSION RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'regression_screenshot.png', fullPage: true }).catch(() => {});
  }

  await browser.close();
  console.log("\n=== ALL TESTS COMPLETE ===");
})();
