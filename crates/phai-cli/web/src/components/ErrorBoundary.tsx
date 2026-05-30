import { Component, type ReactNode } from "react";

interface Props {
	viewName: string;
	children: ReactNode;
}

interface State {
	hasError: boolean;
	error: Error | null;
}

export class ViewErrorBoundary extends Component<Props, State> {
	state: State = { hasError: false, error: null };

	static getDerivedStateFromError(error: Error): State {
		return { hasError: true, error };
	}

	componentDidCatch(error: Error, info: React.ErrorInfo) {
		console.error(
			`[phai] ${this.props.viewName} crashed:`,
			error,
			info.componentStack,
		);
	}

	render() {
		if (this.state.hasError) {
			return (
				<div
					style={{
						background: "var(--surface)",
						border: "1px solid var(--rose)",
						borderRadius: "var(--radius-lg)",
						padding: "var(--card-pad)",
						maxWidth: 600,
						margin: "40px auto",
						textAlign: "center",
					}}
				>
					<h2
						style={{
							fontFamily: "var(--font-display)",
							fontSize: "1.3rem",
							margin: "0 0 8px",
							color: "var(--rose)",
						}}
					>
						{this.props.viewName} · algo deu errado
					</h2>
					<pre
						className="mono"
						style={{
							fontSize: 12,
							color: "var(--muted)",
							background: "var(--bg)",
							padding: 12,
							borderRadius: "var(--radius-sm)",
							overflow: "auto",
							maxHeight: 160,
							textAlign: "left",
							whiteSpace: "pre-wrap",
							wordBreak: "break-word",
						}}
					>
						{this.state.error?.message ?? "Erro desconhecido"}
					</pre>
					<div
						style={{
							display: "flex",
							gap: 10,
							justifyContent: "center",
							marginTop: 16,
						}}
					>
						<button
							className="mono"
							onClick={() => this.setState({ hasError: false, error: null })}
							style={{
								background: "var(--purple)",
								color: "#fff",
								border: "none",
								borderRadius: "var(--radius-full)",
								padding: "8px 20px",
								fontSize: 13,
								cursor: "pointer",
							}}
						>
							tentar novamente
						</button>
						<button
							className="mono"
							onClick={() => window.location.reload()}
							style={{
								background: "transparent",
								color: "var(--muted)",
								border: "1px solid var(--border)",
								borderRadius: "var(--radius-full)",
								padding: "8px 20px",
								fontSize: 13,
								cursor: "pointer",
							}}
						>
							recarregar página
						</button>
					</div>
				</div>
			);
		}
		return this.props.children;
	}
}
