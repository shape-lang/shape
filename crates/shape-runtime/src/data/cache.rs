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
    SnapshotStore, deserialize_dataframe, load_chunked_vec, serialize_dataframe, store_chunked_vec,
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
    pub fn snapshot(&self, store: &SnapshotStore) -> AnyResult<DataCacheSnapshot> {
        let historical_guard = self.historical.read().unwrap();
        let mut historical = Vec::with_capacity(historical_guard.len());
        for (key, cached) in historical_guard.iter() {
            let key_snapshot = CacheKeySnapshot {
                id: key.id.clone(),
                timeframe: key.timeframe,
            };
            historical.push(CachedDataSnapshot {
                key: key_snapshot,
                historical: serialize_dataframe(&cached.historical, store)?,
                current_index: cached.current_index,
            });
        }
        historical.sort_by(|a, b| {
            a.key
                .id
                .cmp(&b.key.id)
                .then(a.key.timeframe.cmp(&b.key.timeframe))
        });

        let live_guard = self.live_buffer.read().unwrap();
        let mut live_buffer = Vec::with_capacity(live_guard.len());
        for (key, rows) in live_guard.iter() {
            let key_snapshot = CacheKeySnapshot {
                id: key.id.clone(),
                timeframe: key.timeframe,
            };
            let rows_blob = store_chunked_vec(rows, DEFAULT_CHUNK_LEN, store)?;
            live_buffer.push(LiveBufferSnapshot {
                key: key_snapshot,
                rows: rows_blob,
            });
        }
        live_buffer.sort_by(|a, b| {
            a.key
                .id
                .cmp(&b.key.id)
                .then(a.key.timeframe.cmp(&b.key.timeframe))
        });

        Ok(DataCacheSnapshot {
            historical,
            live_buffer,
        })
    }

    /// Restore data cache contents from a snapshot.
    pub fn restore_from_snapshot(
        &self,
        snapshot: DataCacheSnapshot,
        store: &SnapshotStore,
    ) -> AnyResult<()> {
        self.clear();

        let mut historical_guard = self.historical.write().unwrap();
        for entry in snapshot.historical.into_iter() {
            let key = CacheKey::new(entry.key.id, entry.key.timeframe);
            let df = deserialize_dataframe(entry.historical, store)?;
            historical_guard.insert(
                key,
                CachedData {
                    historical: df,
                    current_index: entry.current_index,
                },
            );
        }
        drop(historical_guard);

        let mut live_guard = self.live_buffer.write().unwrap();
        for entry in snapshot.live_buffer.into_iter() {
            let key = CacheKey::new(entry.key.id, entry.key.timeframe);
            let rows: Vec<OwnedDataRow> = load_chunked_vec(&entry.rows, store)?;
            live_guard.insert(key, rows);
        }
        Ok(())
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

    fn temp_snapshot_root(name: &str) -> std::path::PathBuf {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        std::env::temp_dir().join(format!("shape_snapshot_test_{}_{}", name, ts))
    }

    fn make_df(id: &str, timeframe: Timeframe) -> DataFrame {
        let mut df = DataFrame::new(id, timeframe);
        df.timestamps = vec![1, 2, 3];
        df.add_column("a", vec![10.0, 11.0, 12.0]);
        df.add_column("b", vec![20.0, 21.0, 22.0]);
        df
    }

    #[tokio::test]
    async fn test_data_cache_snapshot_roundtrip_no_refetch() {
        let tf = Timeframe::d1();
        let df = make_df("TEST", tf);
        let mut frames = HashMap::new();
        frames.insert(CacheKey::new("TEST".to_string(), tf), df);
        let load_calls = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(TestAsyncProvider {
            frames: Arc::new(frames),
            load_calls: load_calls.clone(),
        });

        let cache = DataCache::new(provider, tokio::runtime::Handle::current());
        cache
            .prefetch(vec![DataQuery::new("TEST", tf)])
            .await
            .unwrap();

        // Inject live buffer rows and tweak current index for snapshot fidelity
        let key = CacheKey::new("TEST".to_string(), tf);
        if let Some(entry) = cache.historical.write().unwrap().get_mut(&key) {
            entry.current_index = 2;
        }
        cache.live_buffer.write().unwrap().insert(
            key.clone(),
            vec![OwnedDataRow::new_generic(
                4,
                HashMap::from([("a".to_string(), 13.0)]),
            )],
        );

        let store = SnapshotStore::new(temp_snapshot_root("data_cache")).unwrap();
        let snapshot = cache.snapshot(&store).unwrap();

        // Restore into a cache with a provider that always fails (proves no refetch)
        let fail_provider = Arc::new(NullAsyncProvider::default());
        let restored = DataCache::new(fail_provider, tokio::runtime::Handle::current());
        restored.restore_from_snapshot(snapshot, &store).unwrap();

        let row = restored
            .get_row("TEST", &tf, 0)
            .expect("row should be cached");
        assert_eq!(row.timestamp, 1);
        assert_eq!(row.fields.get("a"), Some(&10.0));

        let live_rows = restored
            .live_buffer
            .read()
            .unwrap()
            .get(&key)
            .cloned()
            .unwrap_or_default();
        assert_eq!(live_rows.len(), 1);
        assert_eq!(live_rows[0].timestamp, 4);

        let restored_index = restored
            .historical
            .read()
            .unwrap()
            .get(&key)
            .map(|c| c.current_index)
            .unwrap_or(0);
        assert_eq!(restored_index, 2);

        // Ensure we only loaded once during prefetch
        assert_eq!(load_calls.load(Ordering::SeqCst), 1);
    }
}
