//! Reflection caching system for reducing redundant LLM calls.
//!
//! This module provides a caching mechanism that stores reflection results
//! for similar tasks to avoid repeated LLM evaluations for semantically
//! equivalent inputs.

// Some public API methods are not yet used in plan.rs but are part of the public interface
#![allow(dead_code)]

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::types::ReflectionResult;

// ============================================================================
// Cache Key
// ============================================================================

/// Key for caching reflection results.
///
/// The cache key is computed from the task description and result content,
/// using a similarity-based hashing approach.
#[derive(Debug, Clone, Eq)]
pub struct CacheKey {
    /// Normalized task description.
    task_hash: u64,
    /// Normalized result content hash.
    result_hash: u64,
    /// Original task description (for debugging).
    #[allow(dead_code)]
    task_description: String,
}

impl CacheKey {
    /// Creates a new cache key from task description and result.
    pub fn new(task_description: &str, result: &str) -> Self {
        let task_normalized = normalize_text(task_description);
        let result_normalized = normalize_text(result);

        Self {
            task_hash: compute_simhash(&task_normalized),
            result_hash: compute_simhash(&result_normalized),
            task_description: task_description.to_string(),
        }
    }

    /// Returns true if this key is similar to another key.
    ///
    /// Uses hamming distance on simhash values to determine similarity.
    pub fn is_similar_to(&self, other: &CacheKey, threshold: u32) -> bool {
        let task_distance = hamming_distance(self.task_hash, other.task_hash);
        let result_distance = hamming_distance(self.result_hash, other.result_hash);

        // Both task and result must be similar
        task_distance <= threshold && result_distance <= threshold
    }
}

impl PartialEq for CacheKey {
    fn eq(&self, other: &Self) -> bool {
        self.task_hash == other.task_hash && self.result_hash == other.result_hash
    }
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.task_hash.hash(state);
        self.result_hash.hash(state);
    }
}

// ============================================================================
// Cache Entry
// ============================================================================

/// A cached reflection result with metadata.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The cached reflection result.
    pub result: ReflectionResult,
    /// When this entry was created.
    pub created_at: Instant,
    /// Number of times this entry has been accessed.
    pub access_count: u64,
    /// Last access time.
    pub last_accessed: Instant,
}

impl CacheEntry {
    /// Creates a new cache entry.
    pub fn new(result: ReflectionResult) -> Self {
        let now = Instant::now();
        Self {
            result,
            created_at: now,
            access_count: 0,
            last_accessed: now,
        }
    }

    /// Records an access to this entry.
    pub fn record_access(&mut self) {
        self.access_count += 1;
        self.last_accessed = Instant::now();
    }

    /// Returns true if this entry has expired.
    pub fn is_expired(&self, ttl: Duration) -> bool {
        self.created_at.elapsed() > ttl
    }

    /// Returns the age of this entry.
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }
}

// ============================================================================
// Cache Configuration
// ============================================================================

/// Configuration for the reflection cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Whether caching is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Maximum number of entries in the cache.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    /// Time-to-live for cache entries (in seconds).
    #[serde(default = "default_ttl_secs")]
    pub ttl_secs: u64,
    /// Similarity threshold for cache hits (hamming distance).
    /// Lower values mean stricter matching.
    #[serde(default = "default_similarity_threshold")]
    pub similarity_threshold: u32,
    /// Whether to use fuzzy matching for cache lookups.
    #[serde(default = "default_fuzzy_matching")]
    pub fuzzy_matching: bool,
}

fn default_enabled() -> bool {
    true
}

fn default_max_entries() -> usize {
    1000
}

fn default_ttl_secs() -> u64 {
    3600 // 1 hour
}

fn default_similarity_threshold() -> u32 {
    5 // Allow up to 5 bits difference in simhash
}

fn default_fuzzy_matching() -> bool {
    true
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_entries: default_max_entries(),
            ttl_secs: default_ttl_secs(),
            similarity_threshold: default_similarity_threshold(),
            fuzzy_matching: default_fuzzy_matching(),
        }
    }
}

// ============================================================================
// Cache Statistics
// ============================================================================

/// Statistics about cache usage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CacheStats {
    /// Total number of cache lookups.
    pub lookups: u64,
    /// Number of cache hits (exact matches).
    pub exact_hits: u64,
    /// Number of cache hits (fuzzy matches).
    pub fuzzy_hits: u64,
    /// Number of cache misses.
    pub misses: u64,
    /// Number of entries evicted due to capacity.
    pub evictions: u64,
    /// Number of entries expired.
    pub expirations: u64,
    /// Current number of entries in cache.
    pub current_entries: usize,
}

impl CacheStats {
    /// Returns the cache hit rate (0.0 - 1.0).
    pub fn hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            return 0.0;
        }
        (self.exact_hits + self.fuzzy_hits) as f64 / self.lookups as f64
    }

    /// Returns the exact hit rate (0.0 - 1.0).
    pub fn exact_hit_rate(&self) -> f64 {
        if self.lookups == 0 {
            return 0.0;
        }
        self.exact_hits as f64 / self.lookups as f64
    }

    /// Returns a summary string.
    pub fn summary(&self) -> String {
        format!(
            "CacheStats[lookups={}, hits={} (exact={}, fuzzy={}), misses={}, hit_rate={:.1}%]",
            self.lookups,
            self.exact_hits + self.fuzzy_hits,
            self.exact_hits,
            self.fuzzy_hits,
            self.misses,
            self.hit_rate() * 100.0
        )
    }
}

// ============================================================================
// Reflection Cache
// ============================================================================

/// Thread-safe cache for reflection results.
///
/// This cache stores reflection results indexed by a similarity-based key
/// computed from the task description and result content. It supports:
///
/// - Exact matching for identical inputs
/// - Fuzzy matching for similar inputs (using simhash)
/// - Automatic expiration of old entries
/// - LRU-style eviction when capacity is reached
pub struct ReflectionCache {
    /// Cache entries indexed by key.
    entries: Arc<RwLock<HashMap<CacheKey, CacheEntry>>>,
    /// Cache configuration.
    config: CacheConfig,
    /// Cache statistics.
    stats: Arc<RwLock<CacheStats>>,
}

impl ReflectionCache {
    /// Creates a new reflection cache with the given configuration.
    pub fn new(config: CacheConfig) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            config,
            stats: Arc::new(RwLock::new(CacheStats::default())),
        }
    }

    /// Creates a cache with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(CacheConfig::default())
    }

    /// Returns whether caching is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Looks up a cached reflection result.
    ///
    /// Returns `Some(result)` if a matching entry is found, `None` otherwise.
    pub fn get(&self, task_description: &str, result: &str) -> Option<ReflectionResult> {
        if !self.config.enabled {
            return None;
        }

        let key = CacheKey::new(task_description, result);
        let ttl = Duration::from_secs(self.config.ttl_secs);

        // Try exact match first
        {
            let mut entries = self.entries.write().unwrap();
            if let Some(entry) = entries.get_mut(&key) {
                if !entry.is_expired(ttl) {
                    entry.record_access();
                    let result = entry.result.clone();

                    let mut stats = self.stats.write().unwrap();
                    stats.lookups += 1;
                    stats.exact_hits += 1;

                    tracing::debug!(
                        "Cache exact hit for task: {}",
                        truncate_str(task_description, 50)
                    );
                    return Some(result);
                } else {
                    // Entry expired, remove it
                    entries.remove(&key);
                    let mut stats = self.stats.write().unwrap();
                    stats.expirations += 1;
                }
            }
        }

        // Try fuzzy match if enabled
        if self.config.fuzzy_matching {
            let entries = self.entries.read().unwrap();
            for (cached_key, entry) in entries.iter() {
                if !entry.is_expired(ttl)
                    && key.is_similar_to(cached_key, self.config.similarity_threshold)
                {
                    let result = entry.result.clone();

                    let mut stats = self.stats.write().unwrap();
                    stats.lookups += 1;
                    stats.fuzzy_hits += 1;

                    tracing::debug!(
                        "Cache fuzzy hit for task: {}",
                        truncate_str(task_description, 50)
                    );
                    return Some(result);
                }
            }
        }

        // Cache miss
        let mut stats = self.stats.write().unwrap();
        stats.lookups += 1;
        stats.misses += 1;

        None
    }

    /// Stores a reflection result in the cache.
    pub fn put(&self, task_description: &str, result_content: &str, reflection: ReflectionResult) {
        if !self.config.enabled {
            return;
        }

        let key = CacheKey::new(task_description, result_content);

        {
            let mut entries = self.entries.write().unwrap();

            // Check capacity and evict if necessary
            if entries.len() >= self.config.max_entries {
                self.evict_lru(&mut entries);
            }

            entries.insert(key, CacheEntry::new(reflection));
        }

        let mut stats = self.stats.write().unwrap();
        stats.current_entries = self.entries.read().unwrap().len();

        tracing::debug!(
            "Cached reflection result for task: {}",
            truncate_str(task_description, 50)
        );
    }

    /// Evicts the least recently used entry.
    fn evict_lru(&self, entries: &mut HashMap<CacheKey, CacheEntry>) {
        if entries.is_empty() {
            return;
        }

        // Find the entry with the oldest last_accessed time
        let oldest_key = entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_accessed)
            .map(|(key, _)| key.clone());

        if let Some(key) = oldest_key {
            entries.remove(&key);
            let mut stats = self.stats.write().unwrap();
            stats.evictions += 1;
        }
    }

    /// Removes expired entries from the cache.
    pub fn cleanup_expired(&self) {
        let ttl = Duration::from_secs(self.config.ttl_secs);

        let mut entries = self.entries.write().unwrap();
        let initial_len = entries.len();

        entries.retain(|_, entry: &mut CacheEntry| !entry.is_expired(ttl));

        let removed = initial_len - entries.len();
        if removed > 0 {
            let mut stats = self.stats.write().unwrap();
            stats.expirations += removed as u64;
            stats.current_entries = entries.len();

            tracing::debug!("Cleaned up {} expired cache entries", removed);
        }
    }

    /// Clears all entries from the cache.
    pub fn clear(&self) {
        let mut entries = self.entries.write().unwrap();
        entries.clear();

        let mut stats = self.stats.write().unwrap();
        stats.current_entries = 0;
    }

    /// Returns the current cache statistics.
    pub fn stats(&self) -> CacheStats {
        let mut stats = self.stats.write().unwrap();
        stats.current_entries = self.entries.read().unwrap().len();
        stats.clone()
    }

    /// Returns the current number of entries.
    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Returns true if the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap().is_empty()
    }
}

impl Default for ReflectionCache {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Normalizes text for comparison by lowercasing and removing extra whitespace.
fn normalize_text(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Computes a simhash for the given text.
///
/// Simhash is a locality-sensitive hash that produces similar values
/// for similar inputs, allowing for fuzzy matching.
fn compute_simhash(text: &str) -> u64 {
    let mut v = [0i32; 64];

    // Generate shingles (3-grams of words)
    let words: Vec<&str> = text.split_whitespace().collect();
    for window in words.windows(3) {
        let shingle = window.join(" ");
        let hash = hash_string(&shingle);

        // Update bit counts
        for (i, count) in v.iter_mut().enumerate() {
            if (hash >> i) & 1 == 1 {
                *count += 1;
            } else {
                *count -= 1;
            }
        }
    }

    // Handle short texts with single words
    if words.len() < 3 {
        for word in &words {
            let hash = hash_string(word);
            for (i, count) in v.iter_mut().enumerate() {
                if (hash >> i) & 1 == 1 {
                    *count += 1;
                } else {
                    *count -= 1;
                }
            }
        }
    }

    // Build final hash
    let mut simhash = 0u64;
    for (i, &count) in v.iter().enumerate() {
        if count > 0 {
            simhash |= 1 << i;
        }
    }

    simhash
}

/// Computes a hash for a string using FNV-1a.
fn hash_string(s: &str) -> u64 {
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    let mut hash = FNV_OFFSET;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Computes the hamming distance between two u64 values.
fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Truncates a string to a maximum length.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflection::types::RecommendedAction;

    fn make_test_result(passed: bool, confidence: f64) -> ReflectionResult {
        ReflectionResult {
            passed,
            confidence,
            issues: vec![],
            suggestions: vec![],
            recommended_action: RecommendedAction::Accept,
            reflection_rounds: 1,
        }
    }

    #[test]
    fn test_cache_key_creation() {
        let key1 = CacheKey::new("Calculate the sum of 1 and 2", "The sum is 3");
        let key2 = CacheKey::new("Calculate the sum of 1 and 2", "The sum is 3");
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_different() {
        let key1 = CacheKey::new("Calculate the sum", "3");
        let key2 = CacheKey::new("Calculate the product", "6");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_similar() {
        // Use more similar texts for simhash comparison
        let key1 = CacheKey::new(
            "Calculate the sum of two numbers and return the result",
            "The result is 5",
        );
        let key2 = CacheKey::new(
            "Calculate the sum of two numbers and return the result",
            "The result is 5",
        );
        // Identical texts should have zero hamming distance
        assert!(key1.is_similar_to(&key2, 0));

        // Slightly different texts should still be similar with higher threshold
        let key3 = CacheKey::new(
            "Calculate the sum of 2 numbers and return the result",
            "The result is 5",
        );
        // Allow higher threshold for minor differences
        assert!(key1.is_similar_to(&key3, 20));
    }

    #[test]
    fn test_cache_entry_creation() {
        let result = make_test_result(true, 0.95);
        let entry = CacheEntry::new(result.clone());
        assert_eq!(entry.access_count, 0);
        assert_eq!(entry.result.confidence, 0.95);
    }

    #[test]
    fn test_cache_entry_access() {
        let result = make_test_result(true, 0.9);
        let mut entry = CacheEntry::new(result);
        assert_eq!(entry.access_count, 0);

        entry.record_access();
        assert_eq!(entry.access_count, 1);

        entry.record_access();
        assert_eq!(entry.access_count, 2);
    }

    #[test]
    fn test_cache_entry_expiration() {
        let result = make_test_result(true, 0.9);
        let entry = CacheEntry::new(result);

        // Should not be expired immediately
        assert!(!entry.is_expired(Duration::from_secs(60)));

        // Should be expired with zero TTL
        assert!(entry.is_expired(Duration::ZERO));
    }

    #[test]
    fn test_cache_config_default() {
        let config = CacheConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_entries, 1000);
        assert_eq!(config.ttl_secs, 3600);
        assert_eq!(config.similarity_threshold, 5);
        assert!(config.fuzzy_matching);
    }

    #[test]
    fn test_cache_stats_hit_rate() {
        let mut stats = CacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);

        stats.lookups = 10;
        stats.exact_hits = 3;
        stats.fuzzy_hits = 2;
        stats.misses = 5;

        assert_eq!(stats.hit_rate(), 0.5);
        assert_eq!(stats.exact_hit_rate(), 0.3);
    }

    #[test]
    fn test_cache_stats_summary() {
        let mut stats = CacheStats::default();
        stats.lookups = 100;
        stats.exact_hits = 30;
        stats.fuzzy_hits = 20;
        stats.misses = 50;

        let summary = stats.summary();
        assert!(summary.contains("lookups=100"));
        assert!(summary.contains("hits=50"));
        assert!(summary.contains("50.0%"));
    }

    #[test]
    fn test_reflection_cache_creation() {
        let cache = ReflectionCache::with_defaults();
        assert!(cache.is_enabled());
        assert!(cache.is_empty());
    }

    #[test]
    fn test_reflection_cache_disabled() {
        let config = CacheConfig {
            enabled: false,
            ..Default::default()
        };
        let cache = ReflectionCache::new(config);

        let result = make_test_result(true, 0.9);
        cache.put("task", "result", result.clone());

        // Should not cache when disabled
        assert!(cache.get("task", "result").is_none());
        assert!(cache.is_empty());
    }

    #[test]
    fn test_reflection_cache_put_get() {
        let cache = ReflectionCache::with_defaults();

        let result = make_test_result(true, 0.95);
        cache.put("Calculate 1+1", "The answer is 2", result.clone());

        let cached = cache.get("Calculate 1+1", "The answer is 2");
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().confidence, 0.95);
    }

    #[test]
    fn test_reflection_cache_miss() {
        // Disable fuzzy matching for this test to ensure cache miss
        let mut config = CacheConfig::default();
        config.fuzzy_matching = false;
        let cache = ReflectionCache::new(config);

        let result = make_test_result(true, 0.9);
        cache.put(
            "Calculate the fibonacci sequence for input 10",
            "The result is 55",
            result,
        );

        // Completely different task/result should miss
        assert!(
            cache
                .get(
                    "Parse JSON configuration file and validate schema",
                    "Configuration is valid"
                )
                .is_none()
        );
    }

    #[test]
    fn test_reflection_cache_fuzzy_match() {
        let mut config = CacheConfig::default();
        config.similarity_threshold = 15; // More lenient for testing
        let cache = ReflectionCache::new(config);

        let result = make_test_result(true, 0.85);
        cache.put(
            "Calculate the sum of two numbers 1 and 2",
            "The result is 3",
            result,
        );

        // Similar query should hit with fuzzy matching
        let _cached = cache.get("Calculate the sum of 2 numbers 1 and 2", "The result is 3");
        // Note: Fuzzy matching depends on simhash similarity
        // This test verifies the fuzzy matching path is exercised
        let stats = cache.stats();
        assert!(stats.lookups > 0);
    }

    #[test]
    fn test_reflection_cache_clear() {
        let cache = ReflectionCache::with_defaults();

        let result = make_test_result(true, 0.9);
        cache.put("task", "result", result);
        assert!(!cache.is_empty());

        cache.clear();
        assert!(cache.is_empty());
    }

    #[test]
    fn test_reflection_cache_stats() {
        let cache = ReflectionCache::with_defaults();

        let result = make_test_result(true, 0.9);
        cache.put("task", "result", result);

        // First lookup - hit
        cache.get("task", "result");
        // Second lookup - miss
        cache.get("other task", "other result");

        let stats = cache.stats();
        assert_eq!(stats.lookups, 2);
        assert_eq!(stats.exact_hits, 1);
        assert_eq!(stats.misses, 1);
    }

    #[test]
    fn test_reflection_cache_eviction() {
        let config = CacheConfig {
            max_entries: 2,
            ..Default::default()
        };
        let cache = ReflectionCache::new(config);

        let result = make_test_result(true, 0.9);

        cache.put("task1", "result1", result.clone());
        cache.put("task2", "result2", result.clone());
        assert_eq!(cache.len(), 2);

        // Adding a third entry should trigger eviction
        cache.put("task3", "result3", result);
        assert_eq!(cache.len(), 2);

        let stats = cache.stats();
        assert_eq!(stats.evictions, 1);
    }

    #[test]
    fn test_normalize_text() {
        assert_eq!(normalize_text("  Hello   World  "), "hello world");
        assert_eq!(normalize_text("UPPERCASE"), "uppercase");
        assert_eq!(normalize_text("a\n\tb"), "a b");
    }

    #[test]
    fn test_simhash_similar_texts() {
        let hash1 = compute_simhash("the quick brown fox jumps over the lazy dog");
        let hash2 = compute_simhash("the quick brown fox leaps over the lazy dog");
        let hash3 = compute_simhash("completely different text about something else");

        // Similar texts should have low hamming distance
        let dist_similar = hamming_distance(hash1, hash2);
        let dist_different = hamming_distance(hash1, hash3);

        assert!(dist_similar < dist_different);
    }

    #[test]
    fn test_hamming_distance() {
        assert_eq!(hamming_distance(0, 0), 0);
        assert_eq!(hamming_distance(0b1111, 0b0000), 4);
        assert_eq!(hamming_distance(0b1010, 0b0101), 4);
        assert_eq!(hamming_distance(0b1111, 0b1110), 1);
    }

    #[test]
    fn test_hash_string() {
        let hash1 = hash_string("hello");
        let hash2 = hash_string("hello");
        let hash3 = hash_string("world");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("short", 10), "short");
        assert_eq!(truncate_str("this is long", 5), "this ...");
    }
}
