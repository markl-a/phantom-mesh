import { describe, it, expect, vi, beforeEach } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("Page Commands — Regression", () => {
  beforeEach(() => { mockInvoke.mockReset(); });

  describe("list_pages", () => {
    it("returns empty array when no pages", async () => {
      mockInvoke.mockResolvedValueOnce([]);
      expect(await mockInvoke("list_pages")).toEqual([]);
    });

    it("returns pages with metadata", async () => {
      mockInvoke.mockResolvedValueOnce([
        { name: "expense", description: "記帳", created_at: "2026-04-10" },
      ]);
      const result = await mockInvoke("list_pages");
      expect(result[0].name).toBe("expense");
      expect(result[0]).toHaveProperty("description");
    });
  });

  describe("load_page", () => {
    it("returns HTML with bridge injected", async () => {
      mockInvoke.mockResolvedValueOnce({
        html: "<html><head><script>window.spectyn=...</script></head><body>Hi</body></html>",
        name: "test",
      });
      const result = await mockInvoke("load_page", { name: "test" });
      expect(result.html).toContain("spectyn");
    });
  });

  describe("save_page", () => {
    it("returns page info", async () => {
      mockInvoke.mockResolvedValueOnce({ name: "new", description: "d", created_at: "2026" });
      const result = await mockInvoke("save_page", { args: { name: "new", html: "<h1>Hi</h1>" } });
      expect(result.name).toBe("new");
    });
  });

  describe("delete_page", () => {
    it("returns true", async () => {
      mockInvoke.mockResolvedValueOnce(true);
      expect(await mockInvoke("delete_page", { name: "x" })).toBe(true);
    });
  });

  describe("page_db_get", () => {
    it("returns value when exists", async () => {
      mockInvoke.mockResolvedValueOnce({ value: '{"n":1}' });
      const r = await mockInvoke("page_db_get", { key: "k" });
      expect(r.value).toBeTruthy();
    });

    it("returns null when missing", async () => {
      mockInvoke.mockResolvedValueOnce({ value: null });
      const r = await mockInvoke("page_db_get", { key: "x" });
      expect(r.value).toBeNull();
    });
  });

  describe("page_db_set", () => {
    it("returns true", async () => {
      mockInvoke.mockResolvedValueOnce(true);
      expect(await mockInvoke("page_db_set", { key: "k", value: "v" })).toBe(true);
    });
  });

  describe("page_db_query", () => {
    it("returns columns and rows for SELECT", async () => {
      mockInvoke.mockResolvedValueOnce({ columns: ["key"], rows: [["k1"]] });
      const r = await mockInvoke("page_db_query", { sql: "SELECT * FROM page_kv" });
      expect(r.columns).toContain("key");
    });

    it("rejects non-SELECT", async () => {
      mockInvoke.mockRejectedValueOnce("Only SELECT queries allowed");
      await expect(mockInvoke("page_db_query", { sql: "DROP TABLE x" })).rejects.toMatch(/SELECT/);
    });
  });
});
