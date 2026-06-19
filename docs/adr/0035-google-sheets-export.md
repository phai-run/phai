---
type: ADR
id: "0035"
title: "Open in Google Sheets via the Sheets API (OAuth)"
status: proposed
date: 2026-06-19
---

## Context

The sheet view already exports a month as CSV. A frequent ask is "Open in Google
Sheets" — land the current, filtered transactions straight into a live Google
spreadsheet for ad-hoc pivoting and sharing.

Two shapes were considered:

- **CSV → manual import** (no OAuth): download the CSV and open Google's import
  dialog. Zero setup, fully private, but clunky — the user does the import by hand.
- **Sheets API (chosen direction)**: create and populate a spreadsheet directly in
  the user's Google account. "Magic" UX, but requires OAuth and a Google Cloud
  client.

The owner chose the Sheets API path. This ADR records that decision and the work
it implies — including an **external, owner-only setup step** that gates the
feature, analogous to the paid Apple Developer ID for notarization
([notarization.md](../notarization.md)).

## Decision

**Add an "Open in Google Sheets" action that creates a spreadsheet via the Google
Sheets API using a per-user OAuth token. Ship it gated behind an owner-provided
Google Cloud OAuth client; without that client the action stays hidden and CSV
export remains the path.**

Properties:

- **OAuth, testing mode, family scale.** A Google Cloud project in *testing*
  publishing status with the household members added as test users avoids Google's
  app-verification process (only needed for public distribution). Scope:
  `https://www.googleapis.com/auth/spreadsheets` (create/write), ideally
  `drive.file` to limit Drive access to files the app creates.
- **Token handling.** The OAuth *authorization code* flow runs through the local
  bridge (loopback redirect to `http://localhost:<port>/api/google/callback`).
  The refresh token is stored in the existing config dir with `0600` perms, like
  the BigQuery service account ([ADR-0034](0034-self-contained-encrypted-invite.md)).
  Access tokens stay in memory.
- **Data path.** The bridge builds the same rows as the CSV export, calls
  `spreadsheets.create` then `spreadsheets.values.update`, and returns the new
  spreadsheet URL; the web app opens it in a new tab. The row-building is pure and
  unit-testable independent of the network.
- **Gating.** The client id/secret come from env (`GOOGLE_OAUTH_CLIENT_ID` /
  `GOOGLE_OAUTH_CLIENT_SECRET`) or config. `GET /api/status` reports whether Google
  export is available so the button only renders when configured.

## Owner setup (the external blocker)

The feature cannot function until the owner creates a Google Cloud OAuth client —
this is the one manual, external step:

1. In Google Cloud Console, create (or reuse) a project and **enable the Google
   Sheets API** (and Drive API if using `drive.file`).
2. Configure the **OAuth consent screen** in *testing* mode; add the household
   members as **test users**.
3. Create an **OAuth client ID** of type *Desktop app* (loopback redirect).
4. Provide the client id/secret to phai via env or config (never committed).

## Options considered

- **Sheets API + OAuth (chosen)**: native, direct create-in-account. Cost: OAuth
  flow, token storage, and the owner's Google Cloud client setup.
- **CSV → manual import**: zero setup and zero new surface, but the import is
  manual; rejected as the primary UX (kept as the always-available fallback).
- **Service-account-written sheet shared to the user**: avoids per-user OAuth, but
  a service account's Drive is not the user's, and sharing/ownership gets awkward;
  rejected.

## Consequences

- New OAuth + Sheets API client code in the bridge, plus a small Google API
  dependency. New dependency / external integration ⇒ this ADR.
- A loopback OAuth callback route and refresh-token-at-rest (`0600`) join the
  bridge's surface; both must respect the same-origin and secret-handling rules.
- Until the owner provides a Google OAuth client, the action is hidden and CSV
  export is unchanged. Re-evaluate the *testing*-mode limit (100 users / 7-day
  refresh-token expiry in some configs) if usage grows beyond the household.
