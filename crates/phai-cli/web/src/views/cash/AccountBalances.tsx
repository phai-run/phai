import { useEffect, useState } from "react";
import { api, type AccountRow } from "../../bridge/api";
import { formatMoney } from "../../lib/format";

/**
 * Per-checking-account balances under the consolidated cash hero. The hero
 * shows the total; this breaks it down by account (the `bank` accounts), so a
 * shared household can see "how much is in each account", not just the sum.
 */
export const AccountBalances = () => {
	const [accounts, setAccounts] = useState<AccountRow[]>([]);

	useEffect(() => {
		let live = true;
		api
			.accounts()
			.then((r) => live && setAccounts(r.rows))
			.catch(() => {});
		return () => {
			live = false;
		};
	}, []);

	// Pluggy types checking/savings as "bank"; legacy/local data may use
	// "checking". Match both (mirrors phai_core::models::is_checking_account_type).
	const checking = accounts.filter(
		(a) =>
			(a.accountType === "bank" || a.accountType === "checking") &&
			a.balance != null,
	);
	if (checking.length === 0) return null;

	return (
		<div
			style={{
				display: "flex",
				flexWrap: "wrap",
				gap: 8,
				marginTop: 10,
			}}
		>
			{checking.map((a) => (
				<div
					key={a.id}
					className="mono"
					title={a.label}
					style={{
						display: "flex",
						alignItems: "baseline",
						gap: 8,
						border: "1px solid var(--border)",
						borderRadius: "var(--radius-full)",
						padding: "5px 12px",
						background: "var(--card)",
						fontSize: 12,
					}}
				>
					<span
						style={{
							color: "var(--muted)",
							maxWidth: 180,
							overflow: "hidden",
							textOverflow: "ellipsis",
							whiteSpace: "nowrap",
						}}
					>
						🏦 {a.label}
					</span>
					<strong
						style={{
							color:
								Number(a.balance) >= 0 ? "var(--text)" : "var(--rose)",
							fontVariantNumeric: "tabular-nums",
						}}
					>
						{formatMoney(a.balance)}
					</strong>
				</div>
			))}
		</div>
	);
};
