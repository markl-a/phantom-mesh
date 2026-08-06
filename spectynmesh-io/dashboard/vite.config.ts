import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";

// Vite config for phantommesh.io dashboard (F200).
//
// Build target: `dist/app/` (NOT `dist/`) — the Cloudflare Worker
// mounts the SPA on the `/app/*` route, and serving the bundle from
// `dist/app/<asset>` keeps file paths in lockstep with the URL prefix.
//
// CSP constraint (E003 §Acceptance, F200 §Scope): no `unsafe-eval`,
// no third-party CDN. Vite's default production build emits only
// hashed `.js` + `.css` chunks from the local module graph — no
// runtime `eval`, no external `<script>` tags. Test asserts this in
// `tests/csp.test.ts` (grep on built output for the forbidden tokens).
export default defineConfig({
  plugins: [react()],
  base: "/app/",
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "src"),
    },
  },
  build: {
    outDir: "dist/app",
    emptyOutDir: true,
    // Disable inline assets so the worker can serve every file with
    // its own hashed Cache-Control header. F206 will wire this into
    // the Lighthouse CI gate.
    assetsInlineLimit: 0,
    sourcemap: false,
    rollupOptions: {
      output: {
        // Predictable asset paths so the worker's MIME map in
        // `src/routes/app.ts` (F205 sibling adds this) can be a
        // simple extension lookup table.
        entryFileNames: "assets/[name]-[hash].js",
        chunkFileNames: "assets/[name]-[hash].js",
        assetFileNames: "assets/[name]-[hash][extname]",
      },
    },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./tests/setup.ts"],
    css: false,
  },
});
