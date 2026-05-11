const { chromium } = require('playwright');

(async () => {
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();

  console.log("=== BUG 1: Conversation tool call raw JSON ===");
  try {
    // Go directly to conversation page (onboarding already done from previous run)
    await page.goto('http://localhost:5173/', { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(2000);

    // Wait for the input to appear
    const input = page.locator('textarea, input[type="text"]');
    await input.first().waitFor({ state: 'visible', timeout: 10000 });
    console.log("Input found, typing message...");

    await input.first().click();
    await input.first().pressSequentially('用 shell 工具執行 echo hello123', { delay: 30 });
    await page.waitForTimeout(500);

    // Click send button
    const sendBtn = page.getByText('發送');
    await sendBtn.click();
    console.log("Message sent, waiting for response...");

    // Wait longer for tool call to complete
    await page.waitForTimeout(15000);

    const bodyText = await page.locator('body').innerText();
    await page.screenshot({ path: 'bug1_screenshot2.png', fullPage: true });

    console.log("Full body text:");
    console.log(bodyText);

    const hasRawJson = bodyText.includes('"agent"') || bodyText.includes('"output"') ||
      bodyText.includes('"agent":"master"') || bodyText.includes('"agent": "master"') ||
      (bodyText.includes('{"') && bodyText.includes('"}'));
    const hasHello = bodyText.includes('hello123');

    console.log("\nHas raw JSON patterns:", hasRawJson);
    console.log("Has hello123 in output:", hasHello);

    if (hasHello && !hasRawJson) {
      console.log("BUG 1 RESULT: PASS - Shows readable text, no raw JSON");
    } else if (hasRawJson) {
      console.log("BUG 1 RESULT: FAIL - Raw JSON visible in response");
    } else {
      console.log("BUG 1 RESULT: INCONCLUSIVE - check screenshot");
    }
  } catch (e) {
    console.log("BUG 1 RESULT: ERROR -", e.message);
    await page.screenshot({ path: 'bug1_screenshot2.png', fullPage: true }).catch(() => {});
  }

  await browser.close();
})();
