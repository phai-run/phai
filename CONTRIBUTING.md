# Contributing to Finance OS

Thank you for your interest in contributing!

## Prerequisites

- Rust 1.90+ (`rustup update stable`)
- SQLite 3.x (for local backend tests)

## Getting Started

```bash
git clone https://github.com/<owner>/finance-os.git
cd finance-os
cargo build
cargo test --workspace
```

## Development Workflow

1. Fork and create a feature branch from `main`
2. Make your changes
3. Ensure all checks pass:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --all-targets --all-features
   cargo test --workspace
   ```
4. Open a pull request against `main`

## Code Conventions

- Use `anyhow::Result` with `.context()` for error propagation — no `.unwrap()` in production code
- All monetary amounts use `rust_decimal::Decimal` — never `f64`
- SQL parameters must be bound, never interpolated (except table identifiers validated against an allowlist)
- Every write operation must emit an `AuditEvent`
- New migrations must be idempotent (safe to re-run)

## Testing

End-to-end tests run against the SQLite (local) backend using temporary directories:

```bash
cargo test --package finance-cli
```

## Reporting Issues

Please include:
- Your Rust version (`rustc --version`)
- The backend you are using (local or bigquery)
- Steps to reproduce
