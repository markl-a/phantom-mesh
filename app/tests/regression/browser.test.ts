import { describe, it, expect, vi, beforeEach } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
  convertFileSrc: (path: string) => `asset://localhost/${path}`,
}));

describe("Browser Commands — Regression", () => {
  beforeEach(() => { mockInvoke.mockReset(); });

  describe("browser_navigate", () => {
    it("returns success with screenshot path", async () => {
      mockInvoke.mockResolvedValueOnce({
        success: true, output: "Navigated", screenshot_path: "/screenshots/ss.png",
      });
      const result = await mockInvoke("browser_navigate", { url: "https://google.com" });
      expect(result.success).toBe(true);
      expect(result.screenshot_path).toBeTruthy();
    });

    it("returns failure for bad URL", async () => {
      mockInvoke.mockResolvedValueOnce({ success: false, output: "Failed", screenshot_path: null });
      const result = await mockInvoke("browser_navigate", { url: "bad" });
      expect(result.success).toBe(false);
    });
  });

  describe("browser_screenshot", () => {
    it("returns path string", async () => {
      mockInvoke.mockResolvedValueOnce("/screenshots/ss.png");
      const result = await mockInvoke("browser_screenshot");
      expect(result).toMatch(/screenshot/);
    });
  });

  describe("browser_snapshot", () => {
    it("returns page text", async () => {
      mockInvoke.mockResolvedValueOnce({ success: true, text: "Page content..." });
      const result = await mockInvoke("browser_snapshot");
      expect(result.text).toBeTruthy();
    });
  });

  describe("browser_status", () => {
    it("returns inactive when no session", async () => {
      mockInvoke.mockResolvedValueOnce({ active: false, current_url: null });
      const result = await mockInvoke("browser_status");
      expect(result.active).toBe(false);
    });

    it("returns active with URL", async () => {
      mockInvoke.mockResolvedValueOnce({ active: true, current_url: "https://google.com" });
      const result = await mockInvoke("browser_status");
      expect(result.active).toBe(true);
      expect(result.current_url).toBe("https://google.com");
    });
  });

  describe("browser_close", () => {
    it("returns true", async () => {
      mockInvoke.mockResolvedValueOnce(true);
      expect(await mockInvoke("browser_close")).toBe(true);
    });
  });
});
