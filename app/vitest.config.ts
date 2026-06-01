import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import path from 'path';

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  test: {
    // F101 adds component/hook tests under tests/f101/*.test.tsx. The
    // legacy daemon-spawning suite stays under tests/*.test.ts.
    include: ['tests/**/*.test.ts', 'tests/**/*.test.tsx'],
    environment: 'jsdom',
    environmentMatchGlobs: [
      // Force the legacy e2e daemon suite to run in node, where it can
      // spawn child processes and hit localhost. The tsx tests below
      // get jsdom by virtue of the default `environment` setting.
      ['tests/e2e-flow.test.ts', 'node'],
      ['tests/archived/**', 'node'],
      ['tests/regression/**', 'node'],
    ],
    setupFiles: ['./tests/f101/setup.ts'],
    testTimeout: 120_000, // Some legacy tests spawn daemon processes.
  },
});
