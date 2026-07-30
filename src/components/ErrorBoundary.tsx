import React from "react";

type Props = { children: React.ReactNode };
type State = { error: Error | null };

export default class ErrorBoundary extends React.Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("Tempo interface error", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <main className="fatal-error" role="alert">
        <div className="card card-pad">
          <h1>Tempo hit an interface error</h1>
          <p>Your tracker is still running in the background. Reload the interface to recover.</p>
          <pre>{this.state.error.message}</pre>
          <button className="btn btn-primary" onClick={() => window.location.reload()}>
            Reload Tempo
          </button>
        </div>
      </main>
    );
  }
}
