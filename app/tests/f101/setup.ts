// F101 · Vitest setup — extend `expect` with @testing-library/jest-dom
// matchers (e.g. `toBeInTheDocument`, `toHaveAttribute`) and clean up
// the JSDOM between tests so component state doesn't leak.

import '@testing-library/jest-dom/vitest';
import { afterEach } from 'vitest';
import { cleanup } from '@testing-library/react';

afterEach(() => {
  cleanup();
});
