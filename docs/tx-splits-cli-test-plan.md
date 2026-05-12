# Transaction Splits CLI Test Plan (BigQuery-only)

Status in this workspace: `finance-cli` exposes the split commands and local-backend unsupported behavior is covered by e2e tests. The remaining live BigQuery cases are intentionally gated because they require a configured dataset.

## Covered Local e2e Cases

1. `tx split preview` on backend `local`:
   - setup via `auth setup --backend local` + `admin migrate`
   - run `finance tx split preview --transaction-id <id> --payload missing.json`
   - expect non-zero exit and explicit "BigQuery-only"/"unsupported on local backend" message

2. `tx split apply` on backend `local`:
   - same local setup
   - run apply command
   - expect non-zero exit and same unsupported contract

3. `tx split show` on backend `local`:
   - expect unsupported message and non-zero exit

4. `tx split clear` on backend `local`:
   - expect unsupported message and non-zero exit

5. `report split-candidates` on backend `local`:
   - expect unsupported message and non-zero exit

6. `report item-prices` on backend `local`:
   - expect unsupported message and non-zero exit

## Planned BigQuery-gated Cases

1. BigQuery happy-path smoke:
   - setup `auth setup --backend bigquery` + `admin migrate`
   - seed one synthetic transaction fixture compatible with split flow
   - `tx split preview` returns deterministic totals and item rows
   - `tx split apply` persists idempotently
   - `tx split show` returns persisted split
   - `tx split clear` removes persisted split
   - `report split-candidates` and `report item-prices` include split-aware rows

2. BigQuery invariants:
   - applying split where line sum mismatches transaction amount should fail with validation error
   - receipt item totals can differ from transaction amount because items are telemetry-only
   - double-apply with same idempotency key should be no-op
   - `clear` on non-existent split should be deterministic (no crash, explicit status)
