import { Component, type ErrorInfo, type ReactNode } from "react";
import { MobileErrorView } from "./MobileError";

// React error boundary for the mobile app. Catches render-time crashes that
// would otherwise blank the WebView and shows SPEC-34 Screen 16 (Error) so the
// user gets retry / report / reset instead of a white screen.

interface Props { children: ReactNode }
interface State { error: Error | null }

export default class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // Surface to the on-device diag log (installed in main.tsx) — handsets
    // have no remote console, so this is how we see boundary catches.
    const diag = (window as { spectynDiag?: (m: string, bg?: string) => void }).spectynDiag;
    diag?.(`[ErrorBoundary] ${error.message}`, "#f88");
    console.error("[ErrorBoundary]", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return <MobileErrorView code="RENDER" />;
    }
    return this.props.children;
  }
}
