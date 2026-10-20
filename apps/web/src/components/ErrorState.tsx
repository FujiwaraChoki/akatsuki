// Error states (ARCHITECTURE §15): every error is shown through the copy table in @akatsuki/ui (`errorCopy`),
// never with provider text, and offers the remedy the code calls for (retry, sign in, reconnect, upgrade).
import { Button, errorCopy, Heading } from "@akatsuki/ui";
import { toErrorPayload } from "@akatsuki/wire/errors";
import { Component, type ErrorInfo, type ReactNode } from "react";

export function codeOf(err: unknown): string {
  return toErrorPayload(err).code;
}

export interface ErrorStateProps {
  error: unknown;
  onRetry?: () => void;
  onSignIn?: () => void;
  onReconnect?: () => void;
  onUpgrade?: () => void;
  /** Smaller inline variant (inside a message, a pane). */
  inline?: boolean;
}

export function ErrorState({ error, onRetry, onSignIn, onReconnect, onUpgrade, inline }: ErrorStateProps) {
  const code = codeOf(error);
  const copy = errorCopy(code);
  const action =
    copy.remedy === "retry" && onRetry ? (
      <Button onClick={onRetry}>Try again</Button>
    ) : copy.remedy === "sign_in" && onSignIn ? (
      <Button variant="primary" onClick={onSignIn}>
        Sign in
      </Button>
    ) : copy.remedy === "reconnect" && onReconnect ? (
      <Button variant="primary" onClick={onReconnect}>
        Reconnect
      </Button>
    ) : copy.remedy === "upgrade" && onUpgrade ? (
      <Button variant="primary" onClick={onUpgrade}>
        See plans
      </Button>
    ) : onRetry ? (
      <Button onClick={onRetry}>Try again</Button>
    ) : null;
  if (inline)
    return (
      <div className="ak-web-error ak-web-error--inline" role="alert" data-code={code}>
        <span>{copy.title}</span>
        {action}
      </div>
    );
  return (
    <div className="ak-web-error" role="alert" data-code={code}>
      <Heading level={2} variant="heading">
        {copy.title}
      </Heading>
      {action}
    </div>
  );
}

interface BoundaryProps {
  children: ReactNode;
  /** Changing this resets the boundary (e.g. the route). */
  resetKey?: string;
  fallback: (error: unknown, reset: () => void) => ReactNode;
}

/** Catches errors thrown by Convex queries (ConvexError codes) and render failures. */
export class ErrorBoundary extends Component<BoundaryProps, { error: unknown; key?: string }> {
  override state: { error: unknown; key?: string } = { error: null };

  static getDerivedStateFromError(error: unknown) {
    return { error };
  }

  static getDerivedStateFromProps(props: BoundaryProps, state: { error: unknown; key?: string }) {
    if (props.resetKey !== state.key) return { error: null, key: props.resetKey };
    return null;
  }

  override componentDidCatch(error: unknown, info: ErrorInfo) {
    // Codes only: never log message text that could hold user data (§14.2).
    console.error("render error", codeOf(error), info.componentStack?.split("\n")[1]?.trim());
  }

  reset = () => this.setState({ error: null });

  override render() {
    if (this.state.error) return this.props.fallback(this.state.error, this.reset);
    return this.props.children;
  }
}
