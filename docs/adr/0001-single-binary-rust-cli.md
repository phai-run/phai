---
type: ADR
id: "0001"
title: "Single-binary Rust CLI as the product surface"
status: active
date: 2025-12-15
---

## Context

Finance OS needs to be installable, updatable, scriptable, and usable on a phone (via WhatsApp formatting). The author runs it daily in a personal pipeline that includes shell scripts, cron-style schedules, and AI agents. Long-running processes, browser UIs, and runtime dependencies all add friction the workflow cannot absorb.

Alternative shapes considered: a web dashboard (Next.js + Postgres), a desktop app (Tauri), a Python TUI, a Rust CLI.

## Decision

**Finance OS ships as a single statically-linked Rust binary, installed via `curl | bash` (`install.sh`) or `cargo install`.** The CLI is the only product surface. SQLite is bundled (`rusqlite` with `bundled`). Migrations are embedded at compile time via `include_str!`. There is no server, no daemon, no GUI.

## Options considered

- **Single Rust binary** (chosen): fastest cold start, trivial install, fits the WhatsApp pipeline, easy to script and chain with pipes. Decimal precision and async are first-class in the Rust ecosystem.
- **Web dashboard (Next.js + Postgres)**: rich UI, but introduces hosting, auth, browser context-switching, and a runtime database. None of that helps the actual workflow (read on phone, write from shell).
- **Tauri desktop app**: cross-platform native UI, but the user's actual reading surface is WhatsApp, not the desktop. Tauri pays for a webview the workflow never uses.
- **Python TUI**: faster prototype, but `Decimal` arithmetic is slower and the install story (venv, pip) loses to `curl | bash`.

## Consequences

- **Easier**: install, update, scripting, CI, decimal precision, agent integration (pipe in/out).
- **Harder**: rich UI work — anything visual happens in the consuming surface (WhatsApp, Sheets, an external dashboard fed by `--raw`).
- **Invariants for the codebase**: new features must not require a long-running process, a sidecar, or a build step on the user's machine. If a feature needs that, it's the wrong shape.
- **Re-evaluation triggers**: a sustained workflow change where the daily reading surface stops being WhatsApp/CLI; or a feature class (real-time alerts, OAuth-heavy aggregators) that genuinely needs a resident process.
