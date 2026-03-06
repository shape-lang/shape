//! Memoized transport wrapper that caches results of remote function calls.
//!
//! Caches results by `SHA-256(destination || payload)` using an LRU cache.
//! Intercepts [`Transport::send`] before forwarding to the inner transport,
//! returning the cached result when available.

use super::{Connection, Transport, TransportError};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

/// Configuration for the memoized transport.
#[derive(Debug, Clone)]
pub struct MemoConfig {
    /// Maximum number of cached entries before LRU eviction kicks in.
    pub max_entries: usize,
    /// Whether caching is enabled. When `false`, all calls pass through.
    pub enabled: bool,
}

impl Default for MemoConfig {
    fn default() -> Self {
        Self {
            max_entries: 1024,
            enabled: true,
        }
    }
}

/// A single entry in the memo cache.
#[derive(Debug, Clone)]
struct CacheEntry {
    result: Vec<u8>,
    hits: u64,
}

/// Interior mutable cache state, protected by a `Mutex` so we satisfy
/// the `Send + Sync` bound required by [`Transport`].
#[derive(Debug)]
struct CacheState {
    cache: HashMap<[u8; 32], CacheEntry>,
    stats: MemoStats,
    /// Insertion order for LRU eviction (oldest first).
    insertion_order: Vec<[u8; 32]>,
}

/// Memoized transport wrapper with LRU eviction.
///
/// Wraps any [`Transport`] implementation and caches the results of
/// one-shot `send` calls. Persistent connections (`connect`) are
/// forwarded directly to the inner transport without caching.
pub struct MemoizedTransport<T: Transport> {
    inner: T,
    config: MemoConfig,
    state: Mutex<CacheState>,
}

/// Cache hit/miss statistics.
#[derive(Debug, Default, Clone)]
pub struct MemoStats {
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub evictions: u64,
    pub total_requests: u64,
}

impl<T: Transport> MemoizedTransport<T> {
    /// Create a new memoized transport wrapping `inner` with the given config.
    pub fn new(inner: T, config: MemoConfig) -> Self {
        let state = CacheState {
            cache: HashMap::with_capacity(config.max_entries),
            stats: MemoStats::default(),
            insertion_order: Vec::new(),
        };
        Self {
            inner,
            config,
            state: Mutex::new(state),
        }
    }

    /// Compute the cache key as `SHA-256(destination || payload)`.
    pub fn compute_cache_key(destination: &str, payload: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(destination.as_bytes());
        hasher.update(payload);
        hasher.finalize().into()
    }

    /// Invalidate a specific cache entry by key.
    pub fn invalidate(&self, key: &[u8; 32]) {
        let mut state = self.state.lock().unwrap();
        if state.cache.remove(key).is_some() {
            state.insertion_order.retain(|k| k != key);
        }
    }

    /// Invalidate all cached entries.
    pub fn invalidate_all(&self) {
        let mut state = self.state.lock().unwrap();
        state.cache.clear();
        state.insertion_order.clear();
    }

    /// Return a snapshot of the current cache statistics.
    pub fn stats(&self) -> MemoStats {
        self.state.lock().unwrap().stats.clone()
    }

    /// Return the current number of cached entries.
    pub fn cache_len(&self) -> usize {
        self.state.lock().unwrap().cache.len()
    }
}

impl CacheState {
    /// Evict the oldest entry to make room for a new one.
    fn evict_oldest(&mut self) {
        if let Some(oldest_key) = self.insertion_order.first().copied() {
            self.cache.remove(&oldest_key);
            self.insertion_order.remove(0);
            self.stats.evictions += 1;
        }
    }
}

impl<T: Transport> Transport for MemoizedTransport<T> {
    fn send(&self, destination: &str, payload: &[u8]) -> Result<Vec<u8>, TransportError> {
        let key = MemoizedTransport::<T>::compute_cache_key(destination, payload);

        {
            let mut state = self.state.lock().unwrap();
            state.stats.total_requests += 1;

            if !self.config.enabled {
                // Drop the lock before calling inner.
                drop(state);
                return self.inner.send(destination, payload);
            }

            // Check cache.
            if let Some(entry) = state.cache.get_mut(&key) {
                let result = entry.result.clone();
                entry.hits += 1;
                state.stats.cache_hits += 1;
                return Ok(result);
            }

            state.stats.cache_misses += 1;
            // Drop lock before the potentially blocking inner send.
        }

        // Cache miss -- delegate to inner transport (lock not held).
        let result = self.inner.send(destination, payload)?;

        // Re-acquire lock to insert the result.
        {
            let mut state = self.state.lock().unwrap();

            // Evict if at capacity.
            if state.cache.len() >= self.config.max_entries {
                state.evict_oldest();
            }

            state.insertion_order.push(key);
            state.cache.insert(
                key,
                CacheEntry {
                    result: result.clone(),
                    hits: 0,
                },
            );
        }

        Ok(result)
    }

    fn connect(&self, destination: &str) -> Result<Box<dyn Connection>, TransportError> {
        // Persistent connections are not cacheable; delegate directly.
        self.inner.connect(destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A mock transport that echoes the payload back with a call counter.
    struct EchoTransport {
        call_count: Arc<AtomicU64>,
    }

    impl EchoTransport {
        fn new() -> (Self, Arc<AtomicU64>) {
            let counter = Arc::new(AtomicU64::new(0));
            (
                Self {
                    call_count: counter.clone(),
                },
                counter,
            )
        }
    }

    impl Transport for EchoTransport {
        fn send(&self, _destination: &str, payload: &[u8]) -> Result<Vec<u8>, TransportError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(payload.to_vec())
        }

        fn connect(&self, _destination: &str) -> Result<Box<dyn Connection>, TransportError> {
            Err(TransportError::ConnectionFailed(
                "not supported".to_string(),
            ))
        }
    }

    #[test]
    fn test_cache_hit() {
        let (echo, counter) = EchoTransport::new();
        let memo = MemoizedTransport::new(echo, MemoConfig::default());

        let r1 = Transport::send(&memo, "host:1234", b"hello").unwrap();
        let r2 = Transport::send(&memo, "host:1234", b"hello").unwrap();

        assert_eq!(r1, r2);
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        let stats = memo.stats();
        assert_eq!(stats.cache_hits, 1);
        assert_eq!(stats.cache_misses, 1);
        assert_eq!(stats.total_requests, 2);
    }

    #[test]
    fn test_cache_miss_different_payload() {
        let (echo, counter) = EchoTransport::new();
        let memo = MemoizedTransport::new(echo, MemoConfig::default());

        Transport::send(&memo, "host:1234", b"aaa").unwrap();
        Transport::send(&memo, "host:1234", b"bbb").unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_cache_miss_different_destination() {
        let (echo, counter) = EchoTransport::new();
        let memo = MemoizedTransport::new(echo, MemoConfig::default());

        Transport::send(&memo, "host-a:1234", b"same").unwrap();
        Transport::send(&memo, "host-b:1234", b"same").unwrap();

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_lru_eviction() {
        let (echo, _counter) = EchoTransport::new();
        let memo = MemoizedTransport::new(
            echo,
            MemoConfig {
                max_entries: 2,
                enabled: true,
            },
        );

        Transport::send(&memo, "a", b"1").unwrap();
        Transport::send(&memo, "b", b"2").unwrap();
        // This should evict the entry for ("a", "1").
        Transport::send(&memo, "c", b"3").unwrap();

        assert_eq!(memo.stats().evictions, 1);
        assert_eq!(memo.cache_len(), 2);

        // "a"/"1" should have been evicted.
        let key_a = MemoizedTransport::<EchoTransport>::compute_cache_key("a", b"1");
        assert!(!memo.state.lock().unwrap().cache.contains_key(&key_a));
    }

    #[test]
    fn test_disabled_passthrough() {
        let (echo, counter) = EchoTransport::new();
        let memo = MemoizedTransport::new(
            echo,
            MemoConfig {
                max_entries: 1024,
                enabled: false,
            },
        );

        Transport::send(&memo, "host", b"x").unwrap();
        Transport::send(&memo, "host", b"x").unwrap();

        // Both should go through to the inner transport.
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_invalidate() {
        let (echo, counter) = EchoTransport::new();
        let memo = MemoizedTransport::new(echo, MemoConfig::default());

        Transport::send(&memo, "host", b"data").unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);

        let key = MemoizedTransport::<EchoTransport>::compute_cache_key("host", b"data");
        memo.invalidate(&key);

        // After invalidation the next call should miss.
        Transport::send(&memo, "host", b"data").unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn test_invalidate_all() {
        let (echo, counter) = EchoTransport::new();
        let memo = MemoizedTransport::new(echo, MemoConfig::default());

        Transport::send(&memo, "a", b"1").unwrap();
        Transport::send(&memo, "b", b"2").unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 2);

        memo.invalidate_all();
        assert_eq!(memo.cache_len(), 0);

        Transport::send(&memo, "a", b"1").unwrap();
        Transport::send(&memo, "b", b"2").unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn test_compute_cache_key_deterministic() {
        let k1 = MemoizedTransport::<EchoTransport>::compute_cache_key("host", b"payload");
        let k2 = MemoizedTransport::<EchoTransport>::compute_cache_key("host", b"payload");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_compute_cache_key_distinct() {
        let k1 = MemoizedTransport::<EchoTransport>::compute_cache_key("host", b"aaa");
        let k2 = MemoizedTransport::<EchoTransport>::compute_cache_key("host", b"bbb");
        assert_ne!(k1, k2);
    }
}
