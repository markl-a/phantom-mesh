/// <reference types="vite/client" />

// Injected at build time by the Vite `define` in vite.config.ts. Declared here
// so TypeScript knows the bare `__APP_VERSION__` identifier (used in
// UpdatePanel.tsx) is a string constant rather than an undefined global.
declare const __APP_VERSION__: string;
