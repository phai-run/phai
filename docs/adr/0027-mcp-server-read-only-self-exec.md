# ADR-0027: MCP server — read-only v1, hand-rolled stdio loop, self-exec tools

- Status: active
- Date: 2026-06-12

## Context

Agents already reach phai by shelling out (the OpenClaw wrapper in
`integrations/openclaw/`). That works where an agent has shell access, but the
growing MCP ecosystem (Claude apps, IDEs, agent frameworks) speaks Model
Context Protocol and cannot exec arbitrary binaries. phai is LLM-neutral by
design; an MCP surface extends that neutrality to clients without a shell.

Three decisions needed: dependency strategy, execution strategy, and the v1
tool surface.

## Decision

`phai mcp` starts an MCP server on stdio (newline-delimited JSON-RPC 2.0).

1. **No SDK dependency — hand-rolled loop.** The server implements exactly
   `initialize`, `ping`, `tools/list` and `tools/call` (~150 lines on top of
   `serde_json`, already a workspace dependency). The official Rust SDK is
   young and the protocol surface we need is tiny; a new dependency would add
   audit/licensing surface for no leverage. Revisit when we want
   resources/prompts/streaming.
2. **Tools re-exec the current binary** (`std::env::current_exe()` +
   `--no-auto-update` + the same argv the CLI takes + `--raw`/`--json`) and
   return stdout as the tool result. One code path serves humans, scripts,
   the OpenClaw wrapper and MCP — the MCP layer cannot drift from the CLI,
   inherits every fix for free, and stays correct across self-updates (the
   re-exec resolves whatever binary is currently installed).
3. **v1 is read-only.** Tools mirror the report surface (`daily_pulse`,
   `monthly_spend`, `cashflow`, `card_summary`, `budget_status`,
   `installments`, `forecast_vs_actual`, `uncategorized`, `data_health`) plus
   `find_transactions`. No writes: an MCP client is an *analysis* surface
   until a human-in-the-loop write design exists (rules first, AI second —
   the LLM reads and proposes, it never silently decides).

Input validation happens before any subprocess spawns (e.g. `month` must be
`YYYY-MM`); tool failures come back as `isError: true` results, not protocol
errors, per the MCP spec.

## Consequences

- Any MCP client can point at `phai mcp` with zero configuration beyond the
  command itself; backend selection still comes from the same config dir/env
  the CLI uses.
- Each tool call pays one process spawn (~tens of ms on a local SQLite store,
  dominated by backend latency on BigQuery). Acceptable for an interactive
  analysis surface; in-process execution is the optimization path if it ever
  matters.
- Write tools (categorize, forecast upsert) need a follow-up ADR covering
  confirmation semantics before they are exposed.
- The protocol loop is unit-tested in-process and e2e-tested over real stdio
  against a seeded SQLite store (`mcp_serves_reports_over_stdio`).
