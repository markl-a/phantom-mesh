// F205 — dispatch recipes CRUD (per-user templates).
// Consumed by F202 dispatch screen "save as recipe" / "RecipeDrawer".

import type { Context } from "hono";
import type { Env } from "../types";
import { authn } from "./api";
import {
  listRecipes, getRecipe, upsertRecipe, deleteRecipe,
} from "../lib/db";

/// GET /api/me/recipes
export async function listRecipesRoute(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const recipes = await listRecipes(c.env, id.userId);
  return c.json({ recipes });
}

/// GET /api/me/recipes/:id
export async function getRecipeRoute(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const rid = c.req.param("id") ?? "";
  if (!rid) return c.json({ error: "missing id" }, 400);
  const r = await getRecipe(c.env, id.userId, rid);
  if (!r) return c.json({ error: "not found" }, 404);
  return c.json({ recipe: r });
}

/// POST /api/me/recipes — create (or upsert if id present).
/// Body: { id?, name, peer?, provider?, model?, prompt?, required_caps?[] }
export async function postRecipeRoute(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  let body: Record<string, unknown>;
  try { body = await c.req.json(); }
  catch { return c.json({ error: "malformed json" }, 400); }
  const name = typeof body.name === "string" ? body.name.trim() : "";
  if (name.length === 0) return c.json({ error: "missing name" }, 400);
  const caps = Array.isArray(body.required_caps)
    ? (body.required_caps as unknown[]).filter((x): x is string => typeof x === "string")
    : [];
  const r = await upsertRecipe(c.env, id.userId, {
    id: typeof body.id === "string" ? body.id : undefined,
    name,
    peer:     typeof body.peer     === "string" ? body.peer     : "",
    provider: typeof body.provider === "string" ? body.provider : "",
    model:    typeof body.model    === "string" ? body.model    : "",
    prompt:   typeof body.prompt   === "string" ? body.prompt   : "",
    required_caps: caps,
  });
  return c.json({ recipe: r });
}

/// PUT /api/me/recipes/:id — same as POST but id from URL.
export async function putRecipeRoute(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const rid = c.req.param("id") ?? "";
  if (!rid) return c.json({ error: "missing id" }, 400);
  let body: Record<string, unknown>;
  try { body = await c.req.json(); }
  catch { return c.json({ error: "malformed json" }, 400); }
  const name = typeof body.name === "string" ? body.name.trim() : "";
  if (name.length === 0) return c.json({ error: "missing name" }, 400);
  const caps = Array.isArray(body.required_caps)
    ? (body.required_caps as unknown[]).filter((x): x is string => typeof x === "string")
    : [];
  const r = await upsertRecipe(c.env, id.userId, {
    id: rid,
    name,
    peer:     typeof body.peer     === "string" ? body.peer     : "",
    provider: typeof body.provider === "string" ? body.provider : "",
    model:    typeof body.model    === "string" ? body.model    : "",
    prompt:   typeof body.prompt   === "string" ? body.prompt   : "",
    required_caps: caps,
  });
  return c.json({ recipe: r });
}

/// DELETE /api/me/recipes/:id
export async function deleteRecipeRoute(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const rid = c.req.param("id") ?? "";
  if (!rid) return c.json({ error: "missing id" }, 400);
  const ok = await deleteRecipe(c.env, id.userId, rid);
  if (!ok) return c.json({ error: "not found" }, 404);
  return c.json({ deleted: rid });
}
