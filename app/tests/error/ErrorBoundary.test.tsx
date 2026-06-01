// Component tests — ErrorBoundary (SPEC-34 Screen 16 crash fallback).
//
// Verifies the boundary renders its children normally and, when a child
// throws during render, swaps in the MobileErrorView (code=RENDER) instead
// of letting the WebView blank out.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import type { ReactElement } from 'react';

import ErrorBoundary from '../../src/components/mobile/ErrorBoundary';

function Boom(): ReactElement {
  throw new Error('boom');
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('<ErrorBoundary />', () => {
  it('renders children when nothing throws', () => {
    render(
      <ErrorBoundary>
        <div>safe child</div>
      </ErrorBoundary>,
    );
    expect(screen.getByText('safe child')).toBeInTheDocument();
  });

  it('renders the Error screen when a child throws', () => {
    // React logs caught boundary errors to console.error — silence the
    // expected noise so the test output stays readable.
    const errSpy = vi.spyOn(console, 'error').mockImplementation(() => {});

    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    );

    expect(screen.getByText('發生未預期錯誤')).toBeInTheDocument();
    expect(screen.getByText(/code: RENDER/)).toBeInTheDocument();
    // The crash was caught (logged), not propagated.
    expect(errSpy).toHaveBeenCalled();
  });
});
