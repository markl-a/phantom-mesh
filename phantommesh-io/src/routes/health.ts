import type { Context } from "hono";
import type { Env } from "../types";
import { appleConfigured } from "../lib/oauth";

export async function health(c: Context<{ Bindings: Env }>) {
  const providers = ["google", "email"];
  // Advertise apple only when the broker actually has the credentials
  // wired — keeps clients from offering a button that 404s.
  if (appleConfigured({
    clientId:   c.env.APPLE_CLIENT_ID,
    teamId:     c.env.APPLE_TEAM_ID,
    keyId:      c.env.APPLE_KEY_ID,
    privateKey: c.env.APPLE_PRIVATE_KEY,
  })) {
    providers.push("apple");
  }
  return c.json({
    status: "ok",
    version: c.env.BROKER_VERSION,
    providers,
    spec_url: "https://github.com/markl-a/phantom-mesh/blob/main/docs/PHANTOMMESH-IO-DESIGN.md",
  });
}
