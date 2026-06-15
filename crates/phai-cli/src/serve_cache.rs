//! In-memory TTL cache for the `phai serve` read bridge.
//!
//! Every `GET /api/*` read re-queries the `FinanceStore` backend (BigQuery in
//! production), where a single request costs 1–4 s. There is no server-side
//! state between requests, so a browser reload pays the full cost again. This
//! cache holds the serialized JSON body for each read keyed by its full request
//! target (path + query string) and serves repeats in sub-millisecond time.
//!
//! Freshness is bounded two ways:
//! * a short TTL ([`CACHE_TTL`]) caps how long a stale entry can live, covering
//!   the out-of-band cron that refreshes BigQuery a few times a day;
//! * any successful write [`bust`](ReadCache::bust)s the whole cache, so a
//!   user-visible edit is reflected on the next read immediately.
//!
//! Only successful `200` bodies are cached — error responses are never stored.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// How long a cached read body stays fresh. Tuned against the cron that updates
/// BigQuery a few times a day: a few minutes of staleness is invisible to the
/// user, and any write busts the entry well before it expires anyway.
pub const CACHE_TTL: Duration = Duration::from_secs(300);

/// A cached read body and the instant it was stored.
#[derive(Clone)]
struct Entry {
    stored_at: Instant,
    body: Arc<[u8]>,
}

/// Thread-safe TTL cache of serialized read bodies, keyed by request target
/// (path plus query string). Cheap to [`clone`](Clone) — the map is shared.
#[derive(Clone, Default)]
pub struct ReadCache {
    inner: Arc<RwLock<HashMap<String, Entry>>>,
}

impl ReadCache {
    /// Build the cache key from a request path and raw query string. Two reads
    /// with different query params (e.g. `?month=2026-05`) get distinct keys.
    pub fn key(path: &str, query: Option<&str>) -> String {
        match query {
            Some(q) if !q.is_empty() => format!("{path}?{q}"),
            _ => path.to_string(),
        }
    }

    /// Return the cached body for `key` when present and still within
    /// [`CACHE_TTL`]. Expired entries are treated as a miss (and lazily
    /// overwritten on the next [`store`](Self::store)).
    pub fn get(&self, key: &str) -> Option<Arc<[u8]>> {
        let guard = self.inner.read().ok()?;
        let entry = guard.get(key)?;
        if entry.stored_at.elapsed() < CACHE_TTL {
            Some(entry.body.clone())
        } else {
            None
        }
    }

    /// Store a freshly computed body under `key`. Callers must only store
    /// successful (`200`) responses.
    pub fn store(&self, key: String, body: Arc<[u8]>) {
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(
                key,
                Entry {
                    stored_at: Instant::now(),
                    body,
                },
            );
        }
    }

    /// Drop every cached entry. Called after any successful write so the next
    /// read reflects the change immediately.
    pub fn bust(&self) {
        if let Ok(mut guard) = self.inner.write() {
            guard.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(bytes: &[u8]) -> Arc<[u8]> {
        Arc::from(bytes.to_vec().into_boxed_slice())
    }

    #[test]
    fn key_varies_by_query_string() {
        assert_eq!(ReadCache::key("/api/cards", None), "/api/cards");
        assert_eq!(ReadCache::key("/api/cards", Some("")), "/api/cards");
        assert_eq!(
            ReadCache::key("/api/cards", Some("month=2026-05")),
            "/api/cards?month=2026-05"
        );
        assert_ne!(
            ReadCache::key("/api/cards", Some("month=2026-05")),
            ReadCache::key("/api/cards", Some("month=2026-06"))
        );
    }

    #[test]
    fn hit_returns_stored_body() {
        let cache = ReadCache::default();
        let key = ReadCache::key("/api/accounts", None);
        assert!(cache.get(&key).is_none(), "cold cache must miss");
        cache.store(key.clone(), body(b"{\"rows\":[]}"));
        assert_eq!(cache.get(&key).as_deref(), Some(&b"{\"rows\":[]}"[..]));
    }

    #[test]
    fn entries_with_distinct_keys_do_not_collide() {
        let cache = ReadCache::default();
        let a = ReadCache::key("/api/cards", Some("month=2026-05"));
        let b = ReadCache::key("/api/cards", Some("month=2026-06"));
        cache.store(a.clone(), body(b"may"));
        cache.store(b.clone(), body(b"june"));
        assert_eq!(cache.get(&a).as_deref(), Some(&b"may"[..]));
        assert_eq!(cache.get(&b).as_deref(), Some(&b"june"[..]));
    }

    #[test]
    fn expired_entry_is_a_miss() {
        let cache = ReadCache::default();
        let key = ReadCache::key("/api/chart", None);
        // Insert an entry stamped far enough in the past to be expired.
        {
            let mut guard = cache.inner.write().unwrap();
            guard.insert(
                key.clone(),
                Entry {
                    stored_at: Instant::now() - (CACHE_TTL + Duration::from_secs(1)),
                    body: body(b"stale"),
                },
            );
        }
        assert!(
            cache.get(&key).is_none(),
            "an entry older than the TTL must re-query"
        );
    }

    #[test]
    fn bust_clears_all_entries() {
        let cache = ReadCache::default();
        let a = ReadCache::key("/api/accounts", None);
        let b = ReadCache::key("/api/categories", None);
        cache.store(a.clone(), body(b"a"));
        cache.store(b.clone(), body(b"b"));
        cache.bust();
        assert!(cache.get(&a).is_none());
        assert!(cache.get(&b).is_none());
    }
}
