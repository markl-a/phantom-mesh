import { beforeEach, describe, expect, it } from "vitest";
import {
  clearBrokerLogin,
  getBearerToken,
  getBrokerLogin,
  hasBrokerToken,
  setBrokerLogin,
} from "../src/lib/auth";

describe("auth", () => {
  beforeEach(() => {
    clearBrokerLogin();
  });

  it("round-trips a fresh login via localStorage", () => {
    setBrokerLogin({
      token: "abc",
      email: "u@example.com",
      expires_at_ms: Date.now() + 5_000,
    });
    expect(hasBrokerToken()).toBe(true);
    expect(getBearerToken()).toBe("abc");
    expect(getBrokerLogin()?.email).toBe("u@example.com");
  });

  it("clears + reports false for an expired login", () => {
    setBrokerLogin({
      token: "old",
      email: "u@example.com",
      expires_at_ms: Date.now() - 1_000,
    });
    expect(hasBrokerToken()).toBe(false);
    // Side-effect: expired entries are GC'd opportunistically.
    expect(localStorage.getItem("phantom.broker_token")).toBeNull();
  });

  it("treats malformed JSON as missing without throwing", () => {
    localStorage.setItem("phantom.broker_token", "not-json");
    expect(hasBrokerToken()).toBe(false);
    expect(getBearerToken()).toBeNull();
  });

  it("rejects partially-shaped payloads", () => {
    localStorage.setItem(
      "phantom.broker_token",
      JSON.stringify({ token: "x" }),
    );
    expect(getBrokerLogin()).toBeNull();
  });
});
