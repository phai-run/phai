import { makeWorker } from '@livestore/adapter-web/worker'
import { schema } from './schema'

// Client-only: no sync backend. The eventlog is persisted locally (OPFS); the
// Rust bridge reconciles writes with BigQuery/SQLite out of band.
makeWorker({ schema })
