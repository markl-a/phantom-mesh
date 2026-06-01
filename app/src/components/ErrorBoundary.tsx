import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  // Reset the boundary when this changes (e.g. the route path) so navigating
  // away from a crashed screen recovers automatically.
  resetKey?: string;
}

interface State {
  // React error boundaries catch ANY thrown value, not just Error
  // (a component can `throw null` / a string). Keep it unknown and narrow
  // before reading .message so the fallback itself can't crash.
  error: unknown;
}

// Isolates a render crash to one screen instead of blanking the whole app.
// Without this, a single bad component (e.g. a peer status value outside the
// expected union) unmounts the entire React tree — taking the nav bar with it,
// so the user can't even navigate away. See peerBadge.tsx for the bug that
// motivated this.
export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: unknown): State {
    return { error };
  }

  componentDidUpdate(prev: Props) {
    if (prev.resetKey !== this.props.resetKey && this.state.error) {
      this.setState({ error: null });
    }
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    // eslint-disable-next-line no-console
    console.error("[ErrorBoundary]", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      const err = this.state.error;
      const msg = err instanceof Error ? err.message : String(err);
      return (
        <div className="p-6 text-center text-sm text-phantom-muted" role="alert">
          <p className="text-phantom-text mb-2">這個畫面發生錯誤</p>
          <p className="text-xs break-words mb-4">{msg}</p>
          <button
            onClick={() => this.setState({ error: null })}
            className="px-4 py-2 bg-phantom-card border border-phantom-border rounded-lg text-phantom-text"
          >
            重試
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
