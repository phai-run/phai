//! In-memory TTL cache for the `phai serve` read bridge.
//!
//! Every `GET /api/*` read re-queries the `FinanceStore` backend (BigQuery in
//! production), where a single request costs 1–4 s. There is no server-side
//! state between requests, so a browser reload pays the full cost again. This
//! cache holds the serialized JSON body for each read keyed by its full request
//! target (path + query string) and serves repeats in sub-millisecond time.
//!
//! Freshness is bounded three ways:
//! * a short TTL ([`CACHE_TTL`]) caps how long a stale entry can live, covering
//!   the out-of-band cron that refreshes BigQuery a few times a day;
//! * a bounded [`MAX_ENTRIES`] capacity with LRU eviction stops the map from
//!   growing without limit — cache keys embed same-origin-controllable query
//!   strings, so an unbounded map is a memory-pressure vector;
//! * any successful write invalidates the affected [`Resource`] families (see
//!   [`ReadCache::invalidate`]), so a user-visible edit is reflected on the next
//!   read of those families immediately.
//!
//! Only successful `200` bodies are cached — error responses are never stored.

use std::time::Duration;

use moka::sync::Cache;

/// How long a cached read body stays fresh. Tuned against the cron that updates
/// BigQuery a few times a day: a few minutes of staleness is invisible to the
/// user, and any relevant write invalidates the entry well before it expires.
pub const CACHE_TTL: Duration = Duration::from_secs(300);

/// Upper bound on the number of cached read bodies. Keys embed the request
/// query string (same-origin-controllable), so without a bound the map could
/// grow unboundedly under crafted or just varied requests. The read surface is
/// ~8 endpoints, each with a handful of realistic query variants; a few hundred
/// entries comfortably covers normal usage while capping worst-case memory.
/// moka evicts least-recently-used entries once this is exceeded.
pub const MAX_ENTRIES: u64 = 256;

/// A read-resource family. Each cached read belongs to exactly one family
/// (derived from its `/api/...` path), and each write invalidates only the
/// families it can actually change. Grouping by family keeps invalidation
/// granular: an account edit no longer nukes the cached chart, forecasts, etc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resource {
    /// `GET /api/transactions`
    Transactions,
    /// `GET /api/review-queue`
    ReviewQueue,
    /// `GET /api/chart`
    Chart,
    /// `GET /api/accounts`
    Accounts,
    /// `GET /api/cards`
    Cards,
    /// `GET /api/categories`
    Categories,
    /// `GET /api/forecasts`
    Forecasts,
    /// `GET /api/forecast-templates`
    Templates,
}

impl Resource {
    /// Map a request path to its resource family. Unknown paths fall back to
    /// `None` and are simply never cached/invalidated by family (they still get
    /// TTL/LRU eviction if stored). The match is on the leading path segment so
    /// query strings do not matter here.
    fn from_path(path: &str) -> Option<Self> {
        match path {
            "/api/transactions" => Some(Self::Transactions),
            "/api/review-queue" => Some(Self::ReviewQueue),
            "/api/chart" => Some(Self::Chart),
            "/api/accounts" => Some(Self::Accounts),
            "/api/cards" => Some(Self::Cards),
            "/api/categories" => Some(Self::Categories),
            "/api/forecasts" => Some(Self::Forecasts),
            "/api/forecast-templates" => Some(Self::Templates),
            _ => None,
        }
    }
}

/// Thread-safe TTL + LRU cache of serialized read bodies, keyed by request
/// target (path plus query string). Cheap to [`clone`](Clone) — the underlying
/// moka cache is shared (`Arc` internally).
#[derive(Clone)]
pub struct ReadCache {
    inner: Cache<String, CacheEntry>,
}

/// A cached read body tagged with its resource family so invalidation can be
/// targeted without re-parsing the key.
#[derive(Clone)]
struct CacheEntry {
    resource: Option<Resource>,
    body: std::sync::Arc<[u8]>,
}

impl Default for ReadCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ReadCache {
    /// Build a cache with the standard [`MAX_ENTRIES`] bound and [`CACHE_TTL`].
    pub fn new() -> Self {
        Self {
            inner: Cache::builder()
                .max_capacity(MAX_ENTRIES)
                .time_to_live(CACHE_TTL)
                .build(),
        }
    }

    /// Build the cache key from a request path and raw query string. Two reads
    /// with different query params (e.g. `?month=2026-05`) get distinct keys.
    pub fn key(path: &str, query: Option<&str>) -> String {
        match query {
            Some(q) if !q.is_empty() => format!("{path}?{q}"),
            _ => path.to_string(),
        }
    }

    /// Return the cached body for `key` when present and still within
    /// [`CACHE_TTL`]. Expired entries are treated as a miss (moka drops them
    /// lazily on access and proactively in the background).
    pub fn get(&self, key: &str) -> Option<std::sync::Arc<[u8]>> {
        self.inner.get(key).map(|entry| entry.body)
    }

    /// Store a freshly computed body under `key`. Callers must only store
    /// successful (`200`) responses. The resource family is derived from the
    /// path prefix of `key` so later [`invalidate`](Self::invalidate) calls can
    /// drop just this family.
    pub fn store(&self, key: String, body: std::sync::Arc<[u8]>) {
        let path = key.split('?').next().unwrap_or(&key);
        let resource = Resource::from_path(path);
        self.inner.insert(key, CacheEntry { resource, body });
    }

    /// Invalidate every cached entry belonging to any of `resources`. A write
    /// invalidates only the families it can change, so unrelated reads keep
    /// their cached bodies. Entries with no recognized family are left alone.
    pub fn invalidate(&self, resources: &[Resource]) {
        if resources.is_empty() {
            return;
        }
        // moka has no native "remove by predicate", so iterate the live keys.
        // The cache is small (<= MAX_ENTRIES) and writes are infrequent relative
        // to reads, so a linear scan is cheap and keeps invalidation precise.
        let doomed: Vec<String> = self
            .inner
            .iter()
            .filter_map(|(k, entry)| match entry.resource {
                Some(r) if resources.contains(&r) => Some((*k).clone()),
                _ => None,
            })
            .collect();
        for key in doomed {
            self.inner.invalidate(&key);
        }
    }

    /// Drop every cached entry. Reserved for writes that can touch any family
    /// (backend hot-swap on activate, a full Pluggy sync). Granular writes use
    /// [`invalidate`](Self::invalidate) instead.
    pub fn bust(&self) {
        self.inner.invalidate_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn body(bytes: &[u8]) -> Arc<[u8]> {
        Arc::from(bytes.to_vec().into_boxed_slice())
    }

    /// Force any pending lazy eviction/invalidation to settle. moka applies
    /// capacity eviction and `invalidate` asynchronously via internal queues;
    /// `run_pending_tasks` drains them so assertions are deterministic.
    fn settle(cache: &ReadCache) {
        cache.inner.run_pending_tasks();
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
        // A zero-TTL cache: anything stored is immediately stale.
        let cache = ReadCache {
            inner: Cache::builder()
                .max_capacity(MAX_ENTRIES)
                .time_to_live(Duration::from_nanos(1))
                .build(),
        };
        let key = ReadCache::key("/api/chart", None);
        cache.store(key.clone(), body(b"stale"));
        std::thread::sleep(Duration::from_millis(5));
        settle(&cache);
        assert!(
            cache.get(&key).is_none(),
            "an entry older than the TTL must re-query"
        );
    }

    #[test]
    fn capacity_bound_evicts_entries() {
        // Insert well past the capacity bound and assert the cache never holds
        // more than MAX_ENTRIES live entries — i.e. it does not grow unbounded.
        let cache = ReadCache::new();
        let overflow = MAX_ENTRIES + 64;
        for i in 0..overflow {
            cache.store(
                ReadCache::key("/api/transactions", Some(&format!("page={i}"))),
                body(format!("row-{i}").as_bytes()),
            );
        }
        settle(&cache);
        // The bound is the whole point: inserting `overflow` distinct keys must
        // NOT leave `overflow` live entries — the cache stays capped, so it can
        // never grow without limit (the P2 fix). moka's admission policy
        // (TinyLFU) chooses *which* entries to drop, so we assert the invariant
        // (count <= bound) rather than the identity of any single victim.
        let live = cache.inner.entry_count();
        assert!(
            live <= MAX_ENTRIES,
            "live entry count {live} must not exceed the bound {MAX_ENTRIES}"
        );
        assert!(
            live < overflow,
            "cache must have evicted entries under capacity pressure \
             (live {live} of {overflow} inserted)"
        );
    }

    #[test]
    fn invalidate_drops_only_named_families() {
        let cache = ReadCache::default();
        let accounts = ReadCache::key("/api/accounts", None);
        let chart = ReadCache::key("/api/chart", Some("monthsBack=6"));
        let categories = ReadCache::key("/api/categories", None);
        cache.store(accounts.clone(), body(b"acc"));
        cache.store(chart.clone(), body(b"chart"));
        cache.store(categories.clone(), body(b"cats"));

        // Invalidate only the accounts family.
        cache.invalidate(&[Resource::Accounts]);
        settle(&cache);

        assert!(
            cache.get(&accounts).is_none(),
            "accounts read must be invalidated"
        );
        assert_eq!(
            cache.get(&chart).as_deref(),
            Some(&b"chart"[..]),
            "unrelated chart read must survive"
        );
        assert_eq!(
            cache.get(&categories).as_deref(),
            Some(&b"cats"[..]),
            "unrelated categories read must survive"
        );
    }

    #[test]
    fn invalidate_multiple_families_at_once() {
        let cache = ReadCache::default();
        let forecasts = ReadCache::key("/api/forecasts", None);
        let chart = ReadCache::key("/api/chart", None);
        let cards = ReadCache::key("/api/cards", None);
        cache.store(forecasts.clone(), body(b"f"));
        cache.store(chart.clone(), body(b"c"));
        cache.store(cards.clone(), body(b"k"));

        cache.invalidate(&[Resource::Forecasts, Resource::Chart]);
        settle(&cache);

        assert!(cache.get(&forecasts).is_none());
        assert!(cache.get(&chart).is_none());
        assert_eq!(
            cache.get(&cards).as_deref(),
            Some(&b"k"[..]),
            "cards read is untouched by a forecast/chart write"
        );
    }

    #[test]
    fn empty_invalidate_is_a_noop() {
        let cache = ReadCache::default();
        let key = ReadCache::key("/api/accounts", None);
        cache.store(key.clone(), body(b"acc"));
        cache.invalidate(&[]);
        settle(&cache);
        assert_eq!(cache.get(&key).as_deref(), Some(&b"acc"[..]));
    }

    #[test]
    fn bust_clears_all_entries() {
        let cache = ReadCache::default();
        let a = ReadCache::key("/api/accounts", None);
        let b = ReadCache::key("/api/categories", None);
        cache.store(a.clone(), body(b"a"));
        cache.store(b.clone(), body(b"b"));
        cache.bust();
        settle(&cache);
        assert!(cache.get(&a).is_none());
        assert!(cache.get(&b).is_none());
    }
}
