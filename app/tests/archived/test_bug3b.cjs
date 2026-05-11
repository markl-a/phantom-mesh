const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();

  console.log("=== BUG 3: Goals page - test + button ===");
  try {
    await page.goto('http://localhost:5173/goals', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(3000);

    // The + button is visible in screenshot. Let's find it more precisely
    const plusBtn = page.locator('button:has-text("+")');
    const allButtons = page.locator('button');
    const btnCount = await allButtons.count();
    console.log("Total buttons on page:", btnCount);
    for (let i = 0; i < btnCount; i++) {
      const txt = await allButtons.nth(i).innerText().catch(() => '');
      console.log("  Button", i, ":", JSON.stringify(txt));
    }

    // Try clicking the + in the header area
    const headerPlus = page.locator('text="+"');
    const plusCount = await headerPlus.count();
    console.log("Elements with '+' text:", plusCount);

    if (plusCount > 0) {
      await headerPlus.first().click();
      await page.waitForTimeout(1500);
      await page.screenshot({ path: 'bug3_form.png', fullPage: true });

      const bodyAfterClick = await page.locator('body').innerText();
      console.log("After clicking +:", bodyAfterClick.substring(0, 600));

      // Look for form inputs
      const inputs = page.locator('input, textarea');
      const inputCount = await inputs.count();
      console.log("Inputs found:", inputCount);

      for (let i = 0; i < inputCount; i++) {
        const placeholder = await inputs.nth(i).getAttribute('placeholder').catch(() => '');
        const type = await inputs.nth(i).getAttribute('type').catch(() => '');
        const visible = await inputs.nth(i).isVisible().catch(() => false);
        console.log("  Input", i, "type:", type, "placeholder:", placeholder, "visible:", visible);
      }

      // Fill first visible text input
      for (let i = 0; i < inputCount; i++) {
        const visible = await inputs.nth(i).isVisible().catch(() => false);
        const type = await inputs.nth(i).getAttribute('type').catch(() => '');
        if (visible && (type === 'text' || type === '' || type === null)) {
          await inputs.nth(i).click();
          await inputs.nth(i).pressSequentially('Test Goal 123', { delay: 30 });
          console.log("Filled input", i);
          break;
        }
      }

      await page.waitForTimeout(500);

      // Look for submit/save/confirm button
      const submitBtns = page.locator('button');
      const submitCount = await submitBtns.count();
      for (let i = 0; i < submitCount; i++) {
        const txt = await submitBtns.nth(i).innerText().catch(() => '');
        if (txt.match(/確認|提交|建立|保存|儲存|新增|submit|create|save|add/i)) {
          console.log("Clicking submit button:", txt);
          await submitBtns.nth(i).click();
          await page.waitForTimeout(2000);
          break;
        }
      }

      await page.screenshot({ path: 'bug3_after_submit.png', fullPage: true });
      const finalText = await page.locator('body').innerText();
      console.log("After submit:", finalText.substring(0, 600));

      if (finalText.includes('Test Goal 123')) {
        console.log("BUG 3 RESULT: PASS - Goal created successfully");
      } else {
        console.log("BUG 3 RESULT: PARTIAL - + button works but goal creation unclear");
      }
    } else {
      console.log("BUG 3 RESULT: FAIL - No + button found");
    }
  } catch (e) {
    console.log("BUG 3 RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'bug3_error.png', fullPage: true }).catch(() => {});
  }

  await browser.close();
})();
