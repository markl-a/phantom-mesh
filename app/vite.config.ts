import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";
import { createRequire } from "module";

// ESM config ("type": "module") has no `require`; createRequire gives us one so
// we can read the version from package.json as the build-time fallback when
// npm_package_version isn't set (e.g. invoked via `pnpm exec vite` directly).
const require = createRequire(import.meta.url);
const pkgVersion: string =
  process.env.npm_package_version ?? require("./package.json").version;

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  // UpdatePanel.tsx renders `__APP_VERSION__`; without this define it shows the
  // literal token instead of the real version. JSON.stringify so the value is
  // injected as a string literal, not a bare identifier.
  define: {
    __APP_VERSION__: JSON.stringify(pkgVersion),
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 5174 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
});
