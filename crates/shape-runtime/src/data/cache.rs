//! Data cache for prefetched historical and live streaming data
//!
//! This module provides a caching layer that:
//! - Prefetches historical data asynchronously before execution
//! - Maintains a live data buffer updated by background tasks
//! - Provides synchronous access methods for the execution hot path

use super::{
    DataFrame, DataQuery, OwnedDataRow, SharedAsyncProvider, Timeframe,
    async_provider::AsyncDataError,
};
use crate::snapshot::{
    CacheKeySnapshot, CachedDataSnapshot, DEFAULT_CHUNK_LEN, DataCacheSnapshot, LiveBufferSnapshot,
    SnapshotStore, load_chunked_vec, store_chunked_vec,
};
use anyhow::Result as AnyResult;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use tokio::task::JoinHandle;

/// Cache key for data lookups
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct CacheKey {
    /// Identifier (symbol, sensor ID, etc)
    pub id: String,
    /// Timeframe
    pub timeframe: Timeframe,
}

impl CacheKey {
    /// Create a new cache key
    pub fn new(id: String, timeframe: Timeframe) -> Self {
        Self { id, timeframe }
    }
}

/// Cached data for a symbol/timeframe combination
#[derive(Debug, Clone)]
pub struct CachedData {
    /// Historical data (immutable after prefetch)
    pub historical: DataFrame,
    /// Current row index for iteration (used by ExecutionContext)
    pub current_index: usize,
}

impl CachedData {
    /// Create new cached data
    pub fn new(historical: DataFrame) -> Self {
        Self {
            historical,
            current_index: 0,
        }
    }

    /// Get total number of historical rows
    pub fn row_count(&self) -> usize {
        self.historical.row_count()
    }
}

/// Data cache with prefetched historical and live data buffers
///
/// The cache provides:
/// - Async prefetch of historical data (called before execution)
/// - Sync access to cached data (during execution)
/// - Live data streaming via background tasks
///
/// # Thread Safety
///
/// The live buffer uses `Arc<RwLock<...>>` to allow concurrent reads during
/// execution while background tasks write new bars.
///
/// Subscriptions use `Arc<Mutex<...>>` to allow shared ownership across clones.
#[derive(Clone)]
pub struct DataCache {
    /// Async data provider
    provider: SharedAsyncProvider,

    /// Prefetched historical data (populated by prefetch())
    /// Wrapped in Arc for cheap cloning
    historical: Arc<RwLock<HashMap<CacheKey, CachedData>>>,

    /// Live data buffer (updated by background tasks)
    /// RwLock allows many readers during execution, one writer in bg task
    live_buffer: Arc<RwLock<HashMap<CacheKey, Vec<OwnedDataRow>>>>,

    /// Active subscription handles
    /// Tracks background tasks so we can cancel them
    /// Mutex allows mutation through shared reference
    subscriptions: Arc<Mutex<HashMap<CacheKey, JoinHandle<()>>>>,

    /// Tokio runtime handle for spawning tasks
    runtime: tokio::runtime::Handle,
}

impl DataCache {
    /// Create a new data cache
    ///
    /// # Arguments
    ///
    /// * `provider` - Async data provider for loading data
    /// * `runtime` - Tokio runtime handle for spawning background tasks
    pub fn new(provider: SharedAsyncProvider, runtime: tokio::runtime::Handle) -> Self {
        Self {
            provider,
            historical: Arc::new(RwLock::new(HashMap::new())),
            live_buffer: Arc::new(RwLock::new(HashMap::new())),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            runtime,
        }
    }

    /// Create a DataCache pre-loaded with historical data (for tests).
    ///
    /// Uses a NullAsyncProvider and a temporary tokio runtime.
    #[cfg(test)]
    pub(crate) fn from_test_data(data: HashMap<CacheKey, DataFrame>) -> Self {
        let historical: HashMap<CacheKey, CachedData> = data
            .into_iter()
            .map(|(k, df)| (k, CachedData::new(df)))
            .collect();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test tokio runtime");
        Self {
            provider: Arc::new(super::async_provider::NullAsyncProvider),
            historical: Arc::new(RwLock::new(historical)),
            live_buffer: Arc::new(RwLock::new(HashMap::new())),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            runtime: rt.handle().clone(),
        }
    }

    /// Prefetch historical data for given queries (async)
    ///
    /// This loads all queries concurrently and populates the cache.
    /// Should be called before execution starts.
    ///
    /// # Arguments
    ///
    /// * `queries` - List of data queries to prefetch
    ///
    /// # Returns
    ///
    /// Ok if all queries loaded successfully, error otherwise.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let queries = vec![
    ///     DataQuery::new("AAPL", Timeframe::d1()).limit(1000),
    ///     DataQuery::new("MSFT", Timeframe::d1()).limit(1000),
    /// ];
    /// cache.prefetch(queries).await?;
    /// ```
    pub async fn prefetch(&self, queries: Vec<DataQuery>) -> Result<(), AsyncDataError> {
        use futures::future::join_all;

        // Load all queries concurrently
        let futures: Vec<_> = queries
            .iter()
            .map(|q| {
                let provider = self.provider.clone();
                let query = q.clone();
                async move {
                    let df = provider.load(&query).await?;
                    Ok::<_, AsyncDataError>((query, df))
                }
            })
            .collect();

        let results = join_all(futures).await;

        // Process results and populate cache
        let mut historical = self.historical.write().unwrap();
        for result in results {
            let (query, df) = result?;
            let key = CacheKey::new(query.id.clone(), query.timeframe);
            historical.insert(key, CachedData::new(df));
        }

        Ok(())
    }

    /// Get row at index (sync - reads from cache)
    ///
    /// This is the hot path - called frequently during execution.
    /// Reads are lock-free for historical data, read-locked for live data.
    ///
    /// # Arguments
    ///
    /// * `symbol` - Symbol to query
    /// * `timeframe` - Timeframe
    /// * `index` - Absolute row index
    ///
    /// # Returns
    ///
    /// The row if available, None otherwise.
    pub fn get_row(&self, id: &str, timeframe: &Timeframe, index: usize) -> Option<OwnedDataRow> {
        let key = CacheKey::new(id.to_string(), *timeframe);

        let historical = self.historical.read().unwrap();
        historical.get(&key).and_then(|cached| {
            let hist_len = cached.row_count();

            // First try historical data (no lock needed)
            if index < hist_len {
                if let Some(row) = cached.historical.get_row(index) {
                    return OwnedDataRow::from_data_row(&row);
                }
            }

            // Then check live buffer for newer data
            if let Ok(live) = self.live_buffer.read() {
                let live_index = index.saturating_sub(hist_len);
                if let Some(live_rows) = live.get(&key) {
                    return live_rows.get(live_index).cloned();
                }
            }

            None
        })
    }

    /// Get row range (sync - reads from cache)
    ///
    /// # Arguments
    ///
    /// * `symbol` - Symbol to query
    /// * `timeframe` - Timeframe
    /// * `start` - Start index (inclusive)
    /// * `end` - End index (exclusive)
    ///
    /// # Returns
    ///
    /// Vector of rows in the range. May be shorter than requested if data unavailable.
    pub fn get_row_range(
        &self,
        id: &str,
        timeframe: &Timeframe,
        start: usize,
        end: usize,
    ) -> Vec<OwnedDataRow> {
        let key = CacheKey::new(id.to_string(), *timeframe);
        let mut rows = Vec::new();

        let historical = self.historical.read().unwrap();
        if let Some(cached) = historical.get(&key) {
            let hist_len = cached.row_count();

            // Get rows from historical data
            for i in start..end.min(hist_len) {
                if let Some(row) = cached.historical.get_row(i) {
                    if let Some(owned) = OwnedDataRow::from_data_row(&row) {
                        rows.push(owned);
                    }
                }
            }

            // Get rows from live buffer if needed
            if end > hist_len {
                if let Ok(live) = self.live_buffer.read() {
                    if let Some(live_rows) = live.get(&key) {
                        let live_start = start.saturating_sub(hist_len);
                        let live_end = end - hist_len;
                        for row in live_rows
                            .iter()
                            .skip(live_start)
                            .take(live_end.saturating_sub(live_start))
                        {
                            rows.push(row.clone());
                        }
                    }
                }
            }
        }

        rows
    }

    /// Start live data subscription (spawns background task)
    ///
    /// This subscribes to live bar updates and spawns a background task
    /// that appends new bars to the live buffer as they arrive.
    ///
    /// # Arguments
    ///
    /// * `symbol` - Symbol to subscribe to
    /// * `timeframe` - Timeframe for bars
    ///
    /// # Returns
    ///
    /// Ok if subscription started, error otherwise.
    /// Returns Ok without action if already subscribed.
    pub fn subscribe_live(&self, id: &str, timeframe: &Timeframe) -> Result<(), AsyncDataError> {
        let key = CacheKey::new(id.to_string(), *timeframe);

        // Don't subscribe twice
        {
            let subscriptions = self.subscriptions.lock().unwrap();
            if subscriptions.contains_key(&key) {
                return Ok(());
            }
        }

        let mut rx = self.provider.subscribe(id, timeframe)?;
        let live_buffer = self.live_buffer.clone();
        let key_clone = key.clone();

        // Spawn background task to receive bars
        let handle = self.runtime.spawn(async move {
            while let Some(df) = rx.recv().await {
                // Convert DataFrame rows to OwnedDataRow and append to buffer
                if let Ok(mut buffer) = live_buffer.write() {
                    let rows = buffer.entry(key_clone.clone()).or_insert_with(Vec::new);

                    for i in 0..df.row_count() {
                        if let Some(row) = df.get_row(i) {
                            if let Some(owned) = OwnedDataRow::from_data_row(&row) {
                                rows.push(owned);
                            }
                        }
                    }
                }
            }
        });

        let mut subscriptions = self.subscriptions.lock().unwrap();
        subscriptions.insert(key, handle);
        Ok(())
    }

    /// Stop live data subscription
    ///
    /// Cancels the background task and unsubscribes from the provider.
    ///
    /// # Arguments
    ///
    /// * `symbol` - Symbol to unsubscribe from
    /// * `timeframe` - Timeframe
    pub fn unsubscribe_live(&self, symbol: &str, timeframe: &Timeframe) {
        let key = CacheKey::new(symbol.to_string(), *timeframe);

        let mut subscriptions = self.subscriptions.lock().unwrap();
        if let Some(handle) = subscriptions.remove(&key) {
            handle.abort();
        }

        // Also tell the provider (best effort)
        let _ = self.provider.unsubscribe(symbol, timeframe);

        // Clear live buffer for this key
        if let Ok(mut buffer) = self.live_buffer.write() {
            buffer.remove(&key);
        }
    }

    /// Get total row count (historical + live)
    ///
    /// # Arguments
    ///
    /// * `symbol` - Symbol to query
    /// * `timeframe` - Timeframe
    ///
    /// # Returns
    ///
    /// Total number of rows available (historical + live).
    pub fn row_count(&self, id: &str, timeframe: &Timeframe) -> usize {
        let key = CacheKey::new(id.to_string(), *timeframe);

        let historical = self.historical.read().unwrap();
        let hist_count = historical.get(&key).map(|c| c.row_count()).unwrap_or(0);

        let live_count = self
            .live_buffer
            .read()
            .ok()
            .and_then(|b| b.get(&key).map(|v| v.len()))
            .unwrap_or(0);

        hist_count + live_count
    }

    /// Check if data is cached
    ///
    /// # Arguments
    ///
    /// * `symbol` - Symbol to check
    /// * `timeframe` - Timeframe to check
    ///
    /// # Returns
    ///
    /// true if historical data is cached for this key.
    pub fn has_cached(&self, symbol: &str, timeframe: &Timeframe) -> bool {
        let key = CacheKey::new(symbol.to_string(), *timeframe);
        let historical = self.historical.read().unwrap();
        historical.contains_key(&key)
    }

    /// Get list of cached symbols
    ///
    /// # Returns
    ///
    /// Vector of (symbol, timeframe) pairs that are cached.
    pub fn cached_keys(&self) -> Vec<(String, Timeframe)> {
        let historical = self.historical.read().unwrap();
        historical
            .keys()
            .map(|k| (k.id.clone(), k.timeframe))
            .collect()
    }

    /// Clear all cached data
    ///
    /// Stops all subscriptions and clears all cached data.
    pub fn clear(&self) {
        // Abort all background tasks
        let mut subscriptions = self.subscriptions.lock().unwrap();
        for (_, handle) in subscriptions.drain() {
            handle.abort();
        }
        drop(subscriptions);

        // Clear historical cache
        let mut historical = self.historical.write().unwrap();
        historical.clear();
        drop(historical);

        // Clear live buffer
        if let Ok(mut buffer) = self.live_buffer.write() {
            buffer.clear();
        }
    }

    /// Get the async provider
    ///
    /// Returns a clone of the SharedAsyncProvider for use in other components.
    pub fn provider(&self) -> SharedAsyncProvider {
        self.provider.clone()
    }

    /// Create a snapshot of the data cache (historical + live buffers).
    ///
    /// Per ADR-006 §2.7.4 (snapshot rebuild ruling), the DataFrame
    /// (de)serializers were deleted alongside the broader nanboxed-slot
    /// snapshot helpers. The kind-threaded replacement lands in the
    /// Phase 2c snapshot rebuild session; until then, snapshotting the
    /// data cache `todo!()`s rather than emit a placeholder serializer
    /// that silently corrupts persisted rows.
    pub fn snapshot(&self, store: &SnapshotStore) -> AnyResult<DataCacheSnapshot> {
        let _ = (
            store,
            &self.historical,
            &self.live_buffer,
            DEFAULT_CHUNK_LEN,
        );
        let _: Option<CacheKeySnapshot> = None;
        let _: Option<CachedDataSnapshot> = None;
        let _: Option<LiveBufferSnapshot> = None;
        let _ = store_chunked_vec::<u8>;
        todo!("phase-2c snapshot rebuild — see snapshot.rs:648 deferral")
    }

    /// Restore data cache contents from a snapshot.
    ///
    /// See [`Self::snapshot`] — Phase 2c rebuild deferral.
    pub fn restore_from_snapshot(
        &self,
        _snapshot: DataCacheSnapshot,
        _store: &SnapshotStore,
    ) -> AnyResult<()> {
        let _ = load_chunked_vec::<OwnedDataRow>;
        todo!("phase-2c snapshot rebuild — see snapshot.rs:648 deferral")
    }
}

impl Drop for DataCache {
    fn drop(&mut self) {
        // Clean shutdown: abort all background tasks
        let mut subscriptions = self.subscriptions.lock().unwrap();
        for (_, handle) in subscriptions.drain() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{DataQuery, NullAsyncProvider};
    use crate::snapshot::SnapshotStore;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    // Note: Full tests require a mock provider and tokio runtime
    // These tests verify basic structure

    #[test]
    fn test_cache_key() {
        let key1 = CacheKey::new("AAPL".to_string(), Timeframe::d1());
        let key2 = CacheKey::new("AAPL".to_string(), Timeframe::d1());
        let key3 = CacheKey::new("MSFT".to_string(), Timeframe::d1());

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }

    #[test]
    fn test_cached_data() {
        let df = DataFrame::new("TEST", Timeframe::d1());
        let cached = CachedData::new(df);

        assert_eq!(cached.row_count(), 0);
        assert_eq!(cached.current_index, 0);
    }

    #[derive(Clone)]
    struct TestAsyncProvider {
        frames: Arc<HashMap<CacheKey, DataFrame>>,
        load_calls: Arc<AtomicUsize>,
    }

    impl crate::data::AsyncDataProvider for TestAsyncProvider {
        fn load<'a>(
            &'a self,
            query: &'a DataQuery,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<DataFrame, crate::data::AsyncDataError>>
                    + Send
                    + 'a,
            >,
        > {
            let key = CacheKey::new(query.id.clone(), query.timeframe);
            let frames = self.frames.clone();
            let calls = self.load_calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                frames
                    .get(&key)
                    .cloned()
                    .ok_or_else(|| crate::data::AsyncDataError::SymbolNotFound(query.id.clone()))
            })
        }

        fn has_data(&self, symbol: &str, timeframe: &Timeframe) -> bool {
            let key = CacheKey::new(symbol.to_string(), *timeframe);
            self.frames.contains_key(&key)
        }

        fn symbols(&self) -> Vec<String> {
            self.frames.keys().map(|k| k.id.clone()).collect()
        }
    }

    // `test_data_cache_snapshot_roundtrip_no_refetch` deleted — see
    // `DataCache::snapshot` doc comment. Phase 2c rebuilds the snapshot
    // helpers and the test returns alongside.
    #[allow(dead_code)]
    fn _unused_test_imports(
        _provider: TestAsyncProvider,
        _df: DataFrame,
        _query: DataQuery,
        _kind: NullAsyncProvider,
        _store: SnapshotStore,
        _arc: Arc<()>,
        _atomic: AtomicUsize,
        _ordering: Ordering,
    ) {
        let _ = (SystemTime::UNIX_EPOCH, UNIX_EPOCH);
    }
}
