import type { Context } from "hono";
import type { Env } from "../types";

export async function health(c: Context<{ Bindings: Env }>) {
  return c.json({
    status: "ok",
    version: c.env.BROKER_VERSION,
    providers: ["google", "email"],
    spec_url: "https://github.com/markl-a/phantom-mesh/blob/main/docs/PHANTOMMESH-IO-DESIGN.md",
  });
}
