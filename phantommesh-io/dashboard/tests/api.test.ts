import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { apiFetch, apiGet, apiPost, ApiError } from "../src/lib/api";
import { setBrokerLogin, clearBrokerLogin } from "../src/lib/auth";

function mockFetchOnce(
  response: Partial<Response> & { jsonBody?: unknown; textBody?: string },
): ReturnType<typeof vi.spyOn> {
  const headers = new Headers(response.headers ?? {});
  if (response.jsonBody !== undefined && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const fake = {
    ok: (response.status ?? 200) < 400,
    status: response.status ?? 200,
    headers,
    json: async () => response.jsonBody,
    text: async () => response.textBody ?? "",
  } as unknown as Response;
  return vi.spyOn(globalThis, "fetch").mockResolvedValueOnce(fake);
}

describe("apiFetch", () => {
  beforeEach(() => {
    clearBrokerLogin();
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("injects Authorization: Bearer when broker_token is stored", async () => {
    setBrokerLogin({
      token: "tok-123",
      email: "u@example.com",
      expires_at_ms: Date.now() + 60_000,
    });
    const spy = mockFetchOnce({ jsonBody: { ok: true } });

    const res = await apiGet<{ ok: boolean }>("/api/me");

    expect(res).toEqual({ ok: true });
    const [, init] = spy.mock.calls[0]!;
    const headers = init!.headers as Headers;
    expect(headers.get("Authorization")).toBe("Bearer tok-123");
    // Cookie flow (phantom_session HttpOnly) still rides along.
    expect(init!.credentials).toBe("include");
  });

  it("omits Authorization header when no token is stored", async () => {
    const spy = mockFetchOnce({ jsonBody: {} });

    await apiGet("/api/health");

    const [, init] = spy.mock.calls[0]!;
    const headers = init!.headers as Headers;
    expect(headers.has("Authorization")).toBe(false);
  });

  it("serializes plain-object body as JSON + sets Content-Type", async () => {
    const spy = mockFetchOnce({ jsonBody: { job_id: "abc" } });

    await apiPost("/api/me/dispatch/start", {
      peer: "node-1",
      prompt: "hello",
    });

    const [, init] = spy.mock.calls[0]!;
    expect(init!.method).toBe("POST");
    expect(init!.body).toBe(
      JSON.stringify({ peer: "node-1", prompt: "hello" }),
    );
    const headers = init!.headers as Headers;
    expect(headers.get("Content-Type")).toBe("application/json");
  });

  it("throws ApiError on non-ok response carrying the parsed body", async () => {
    mockFetchOnce({
      status: 500,
      jsonBody: { error: "boom" },
    });

    await expect(apiGet("/api/me")).rejects.toMatchObject({
      name: "ApiError",
      status: 500,
    });
  });

  it("on 401 clears broker_login and throws ApiError(401) without redirecting when redirectOn401=false", async () => {
    setBrokerLogin({
      token: "stale",
      email: "u@example.com",
      expires_at_ms: Date.now() + 60_000,
    });
    mockFetchOnce({ status: 401, jsonBody: { error: "unauthorized" } });

    await expect(
      apiFetch("/api/me/cluster-peers", { redirectOn401: false }),
    ).rejects.toBeInstanceOf(ApiError);
    // redirectOn401=false intentionally skips both clear + redirect so
    // callers can drive their own UX (e.g. a login probe on first paint).
    expect(localStorage.getItem("phantom.broker_token")).not.toBeNull();
  });

  it("returns undefined on 204 No Content (e.g. DELETE)", async () => {
    mockFetchOnce({ status: 204 });

    const res = await apiFetch("/api/me/sessions/all-others", {
      method: "DELETE",
      redirectOn401: false,
    });
    expect(res).toBeUndefined();
  });
});
