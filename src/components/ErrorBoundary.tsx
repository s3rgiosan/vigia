import { Component, Fragment, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
}

interface State {
  failed: boolean;
  /** Changes on reload so the subtree mounts fresh. */
  attempt: number;
}

/** Catches render errors in its subtree and offers a reload that remounts it. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { failed: false, attempt: 0 };

  static getDerivedStateFromError(): Partial<State> {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error("view crashed", error, info.componentStack);
  }

  render(): ReactNode {
    if (this.state.failed) {
      return (
        <div className="error-boundary" role="alert">
          <p>Something went wrong in this view.</p>
          <button type="button" onClick={() => this.setState((s) => ({ failed: false, attempt: s.attempt + 1 }))}>
            Reload
          </button>
        </div>
      );
    }
    return <Fragment key={this.state.attempt}>{this.props.children}</Fragment>;
  }
}
