// Mobile route error boundary.
//
// WHY: two shipped crashes (the onboarding BigInt-serialize throw + the cluster
// peerBadge `undefined.text` throw) each unmounted the ENTIRE React tree to a
// blank screen — the bottom nav disappeared too, leaving the user stuck with no
// way to navigate out. There was no error boundary, so any single screen's
// render error took down the whole app. This boundary contains a route-level
// crash: it shows a recoverable fallback (with a "回首頁" reset) while the nav
// shell stays mounted, instead of a dead blank page.

import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  /** Bump this (e.g. the current pathname) to auto-reset on navigation. */
  resetKey?: string;
}
interface State {
  error: Error | null;
}

export default class MobileErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidUpdate(prev: Props) {
    // Auto-clear the error when the route changes so navigating away recovers.
    if (this.state.error && prev.resetKey !== this.props.resetKey) {
      this.setState({ error: null });
    }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // eslint-disable-next-line no-console
    console.error("[MobileErrorBoundary] route render crashed:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="p-6 text-sm" role="alert" data-testid="mobile-route-error">
          <p className="text-spectyn-text font-medium mb-1">這個畫面出了點問題</p>
          <p className="text-spectyn-muted mb-4 break-words">
            {this.state.error.message || "未知錯誤"}
          </p>
          <button
            className="px-4 py-2 rounded-lg bg-spectyn-card border border-spectyn-border text-spectyn-text"
            onClick={() => this.setState({ error: null })}
          >
            重試
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
