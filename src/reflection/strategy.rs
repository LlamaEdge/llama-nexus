//! Adaptive reflection strategy system.
//!
//! This module implements adaptive strategies for reflection that adjust
//! reflection depth and parameters based on historical success rates and
//! task characteristics.

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::types::ReflectionConfig;

// ============================================================================
// Task Category
// ============================================================================

/// Categories of tasks for tracking statistics.
#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub enum TaskCategory {
    /// Code generation tasks.
    CodeGeneration,
    /// Code review/analysis tasks.
    CodeReview,
    /// Documentation tasks.
    Documentation,
    /// Data processing tasks.
    DataProcessing,
    /// Research/analysis tasks.
    Research,
    /// General/uncategorized tasks.
    General,
}

impl TaskCategory {
    /// Infers the task category from the task description.
    pub fn from_description(description: &str) -> Self {
        let desc_lower = description.to_lowercase();

        if desc_lower.contains("write code")
            || desc_lower.contains("implement")
            || desc_lower.contains("create function")
            || desc_lower.contains("generate code")
        {
            TaskCategory::CodeGeneration
        } else if desc_lower.contains("review")
            || desc_lower.contains("analyze code")
            || desc_lower.contains("find bug")
        {
            TaskCategory::CodeReview
        } else if desc_lower.contains("document")
            || desc_lower.contains("readme")
            || desc_lower.contains("write doc")
        {
            TaskCategory::Documentation
        } else if desc_lower.contains("parse")
            || desc_lower.contains("transform")
            || desc_lower.contains("process data")
        {
            TaskCategory::DataProcessing
        } else if desc_lower.contains("research")
            || desc_lower.contains("investigate")
            || desc_lower.contains("find out")
        {
            TaskCategory::Research
        } else {
            TaskCategory::General
        }
    }

    /// Returns the category name as a string.
    pub fn name(&self) -> &'static str {
        match self {
            TaskCategory::CodeGeneration => "code_generation",
            TaskCategory::CodeReview => "code_review",
            TaskCategory::Documentation => "documentation",
            TaskCategory::DataProcessing => "data_processing",
            TaskCategory::Research => "research",
            TaskCategory::General => "general",
        }
    }
}

// ============================================================================
// Reflection Statistics
// ============================================================================

/// Statistics for a single task category.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CategoryStats {
    /// Total number of reflections performed.
    pub total_reflections: u64,
    /// Number of reflections that passed on first round.
    pub first_round_passes: u64,
    /// Number of reflections that required multiple rounds.
    pub multi_round_passes: u64,
    /// Number of reflections that failed.
    pub failures: u64,
    /// Total reflection rounds across all tasks.
    pub total_rounds: u64,
    /// Average confidence score.
    pub avg_confidence: f64,
    /// Average number of rounds needed.
    pub avg_rounds: f64,
}

impl CategoryStats {
    /// Records a reflection outcome.
    pub fn record(&mut self, passed: bool, rounds: u32, confidence: f64) {
        self.total_reflections += 1;
        self.total_rounds += rounds as u64;

        if passed {
            if rounds == 1 {
                self.first_round_passes += 1;
            } else {
                self.multi_round_passes += 1;
            }
        } else {
            self.failures += 1;
        }

        // Update running averages
        let n = self.total_reflections as f64;
        self.avg_confidence = self.avg_confidence * ((n - 1.0) / n) + confidence / n;
        self.avg_rounds = self.total_rounds as f64 / n;
    }

    /// Returns the first-round success rate.
    pub fn first_round_rate(&self) -> f64 {
        if self.total_reflections == 0 {
            return 0.0;
        }
        self.first_round_passes as f64 / self.total_reflections as f64
    }

    /// Returns the overall success rate.
    pub fn success_rate(&self) -> f64 {
        if self.total_reflections == 0 {
            return 0.0;
        }
        (self.first_round_passes + self.multi_round_passes) as f64 / self.total_reflections as f64
    }

    /// Returns a summary string.
    pub fn summary(&self) -> String {
        format!(
            "CategoryStats[total={}, success_rate={:.1}%, first_round_rate={:.1}%, avg_rounds={:.2}]",
            self.total_reflections,
            self.success_rate() * 100.0,
            self.first_round_rate() * 100.0,
            self.avg_rounds
        )
    }
}

/// Overall reflection statistics across all categories.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReflectionStats {
    /// Statistics per category.
    pub by_category: HashMap<TaskCategory, CategoryStats>,
    /// Global statistics.
    pub global: CategoryStats,
    /// When statistics collection started.
    #[serde(skip)]
    pub started_at: Option<Instant>,
}

impl ReflectionStats {
    /// Creates a new statistics tracker.
    pub fn new() -> Self {
        Self {
            by_category: HashMap::new(),
            global: CategoryStats::default(),
            started_at: Some(Instant::now()),
        }
    }

    /// Records a reflection outcome.
    pub fn record(&mut self, category: TaskCategory, passed: bool, rounds: u32, confidence: f64) {
        // Update category-specific stats
        self.by_category
            .entry(category)
            .or_default()
            .record(passed, rounds, confidence);

        // Update global stats
        self.global.record(passed, rounds, confidence);
    }

    /// Returns statistics for a specific category.
    pub fn get_category_stats(&self, category: &TaskCategory) -> Option<&CategoryStats> {
        self.by_category.get(category)
    }

    /// Returns the duration since statistics collection started.
    pub fn uptime(&self) -> Duration {
        self.started_at
            .map(|start| start.elapsed())
            .unwrap_or_default()
    }

    /// Returns a summary of all statistics.
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("Global: {}", self.global.summary())];

        for (category, stats) in &self.by_category {
            if stats.total_reflections > 0 {
                parts.push(format!("{}: {}", category.name(), stats.summary()));
            }
        }

        parts.join("; ")
    }
}

// ============================================================================
// Adaptive Strategy Configuration
// ============================================================================

/// Configuration for adaptive reflection strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveConfig {
    /// Whether adaptive strategy is enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Minimum number of samples before adapting.
    #[serde(default = "default_min_samples")]
    pub min_samples: u64,
    /// High success rate threshold (reduce reflection depth).
    #[serde(default = "default_high_success_threshold")]
    pub high_success_threshold: f64,
    /// Low success rate threshold (increase reflection depth).
    #[serde(default = "default_low_success_threshold")]
    pub low_success_threshold: f64,
    /// Minimum confidence threshold.
    #[serde(default = "default_min_confidence")]
    pub min_confidence_threshold: f64,
    /// Maximum confidence threshold.
    #[serde(default = "default_max_confidence")]
    pub max_confidence_threshold: f64,
    /// Minimum reflection rounds.
    #[serde(default = "default_min_rounds")]
    pub min_reflection_rounds: u32,
    /// Maximum reflection rounds.
    #[serde(default = "default_max_rounds")]
    pub max_reflection_rounds: u32,
}

fn default_enabled() -> bool {
    true
}

fn default_min_samples() -> u64 {
    10
}

fn default_high_success_threshold() -> f64 {
    0.9
}

fn default_low_success_threshold() -> f64 {
    0.6
}

fn default_min_confidence() -> f64 {
    0.5
}

fn default_max_confidence() -> f64 {
    0.9
}

fn default_min_rounds() -> u32 {
    1
}

fn default_max_rounds() -> u32 {
    5
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            min_samples: default_min_samples(),
            high_success_threshold: default_high_success_threshold(),
            low_success_threshold: default_low_success_threshold(),
            min_confidence_threshold: default_min_confidence(),
            max_confidence_threshold: default_max_confidence(),
            min_reflection_rounds: default_min_rounds(),
            max_reflection_rounds: default_max_rounds(),
        }
    }
}

// ============================================================================
// Adapted Parameters
// ============================================================================

/// Parameters adapted for a specific task.
#[derive(Debug, Clone)]
pub struct AdaptedParams {
    /// Confidence threshold for this task.
    pub confidence_threshold: f64,
    /// Maximum reflection rounds for this task.
    pub max_reflection_rounds: u32,
    /// Whether deep reflection should be skipped.
    pub skip_deep_reflection: bool,
    /// Reason for the adaptation.
    pub adaptation_reason: String,
}

impl Default for AdaptedParams {
    fn default() -> Self {
        Self {
            confidence_threshold: 0.7,
            max_reflection_rounds: 3,
            skip_deep_reflection: false,
            adaptation_reason: "Default parameters".to_string(),
        }
    }
}

// ============================================================================
// Adaptive Strategy
// ============================================================================

/// Adaptive reflection strategy that adjusts parameters based on history.
pub struct AdaptiveStrategy {
    /// Strategy configuration.
    config: AdaptiveConfig,
    /// Collected statistics.
    stats: Arc<RwLock<ReflectionStats>>,
}

impl AdaptiveStrategy {
    /// Creates a new adaptive strategy.
    pub fn new(config: AdaptiveConfig) -> Self {
        Self {
            config,
            stats: Arc::new(RwLock::new(ReflectionStats::new())),
        }
    }

    /// Creates a strategy with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(AdaptiveConfig::default())
    }

    /// Returns whether adaptive strategy is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Records a reflection outcome for learning.
    pub fn record_outcome(
        &self,
        task_description: &str,
        passed: bool,
        rounds: u32,
        confidence: f64,
    ) {
        let category = TaskCategory::from_description(task_description);

        let mut stats = self.stats.write().unwrap();
        stats.record(category, passed, rounds, confidence);

        tracing::debug!(
            "Recorded reflection outcome: passed={}, rounds={}, confidence={:.2}",
            passed,
            rounds,
            confidence
        );
    }

    /// Adapts reflection parameters for a specific task.
    pub fn adapt_for_task(
        &self,
        task_description: &str,
        base_config: &ReflectionConfig,
    ) -> AdaptedParams {
        if !self.config.enabled {
            return AdaptedParams {
                confidence_threshold: base_config.confidence_threshold,
                max_reflection_rounds: base_config.max_reflection_rounds,
                skip_deep_reflection: false,
                adaptation_reason: "Adaptive strategy disabled".to_string(),
            };
        }

        let category = TaskCategory::from_description(task_description);
        let stats = self.stats.read().unwrap();

        // Get category-specific stats, fall back to global
        let cat_stats = stats
            .get_category_stats(&category)
            .filter(|s| s.total_reflections >= self.config.min_samples);

        let (success_rate, avg_rounds, sample_count) = if let Some(cs) = cat_stats {
            (cs.success_rate(), cs.avg_rounds, cs.total_reflections)
        } else if stats.global.total_reflections >= self.config.min_samples {
            (
                stats.global.success_rate(),
                stats.global.avg_rounds,
                stats.global.total_reflections,
            )
        } else {
            // Not enough samples, use defaults
            return AdaptedParams {
                confidence_threshold: base_config.confidence_threshold,
                max_reflection_rounds: base_config.max_reflection_rounds,
                skip_deep_reflection: false,
                adaptation_reason: format!(
                    "Insufficient samples ({} < {})",
                    stats.global.total_reflections, self.config.min_samples
                ),
            };
        };

        // Adapt based on success rate
        let (confidence_threshold, max_rounds, skip_deep, reason) = if success_rate
            >= self.config.high_success_threshold
        {
            // High success rate: reduce reflection depth
            let new_threshold =
                (base_config.confidence_threshold - 0.1).max(self.config.min_confidence_threshold);
            let new_rounds = ((avg_rounds as u32).max(1)).min(base_config.max_reflection_rounds);

            (
                new_threshold,
                new_rounds,
                true,
                format!(
                    "High success rate ({:.1}% from {} samples): reduced depth",
                    success_rate * 100.0,
                    sample_count
                ),
            )
        } else if success_rate <= self.config.low_success_threshold {
            // Low success rate: increase reflection depth
            let new_threshold =
                (base_config.confidence_threshold + 0.1).min(self.config.max_confidence_threshold);
            let new_rounds =
                (base_config.max_reflection_rounds + 1).min(self.config.max_reflection_rounds);

            (
                new_threshold,
                new_rounds,
                false,
                format!(
                    "Low success rate ({:.1}% from {} samples): increased depth",
                    success_rate * 100.0,
                    sample_count
                ),
            )
        } else {
            // Normal success rate: use base config
            (
                base_config.confidence_threshold,
                base_config.max_reflection_rounds,
                false,
                format!(
                    "Normal success rate ({:.1}% from {} samples): using base config",
                    success_rate * 100.0,
                    sample_count
                ),
            )
        };

        AdaptedParams {
            confidence_threshold,
            max_reflection_rounds: max_rounds,
            skip_deep_reflection: skip_deep,
            adaptation_reason: reason,
        }
    }

    /// Returns the current statistics.
    pub fn stats(&self) -> ReflectionStats {
        self.stats.read().unwrap().clone()
    }

    /// Resets all statistics.
    pub fn reset_stats(&self) {
        let mut stats = self.stats.write().unwrap();
        *stats = ReflectionStats::new();
    }

    /// Returns a reference to the configuration.
    pub fn config(&self) -> &AdaptiveConfig {
        &self.config
    }
}

impl Default for AdaptiveStrategy {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_category_inference() {
        assert_eq!(
            TaskCategory::from_description("Write code for a calculator"),
            TaskCategory::CodeGeneration
        );
        assert_eq!(
            TaskCategory::from_description("Implement the sorting algorithm"),
            TaskCategory::CodeGeneration
        );
        assert_eq!(
            TaskCategory::from_description("Review the pull request"),
            TaskCategory::CodeReview
        );
        assert_eq!(
            TaskCategory::from_description("Find bugs in the code"),
            TaskCategory::CodeReview
        );
        assert_eq!(
            TaskCategory::from_description("Write documentation for the API"),
            TaskCategory::Documentation
        );
        assert_eq!(
            TaskCategory::from_description("Parse the JSON file"),
            TaskCategory::DataProcessing
        );
        assert_eq!(
            TaskCategory::from_description("Research best practices"),
            TaskCategory::Research
        );
        assert_eq!(
            TaskCategory::from_description("Do something random"),
            TaskCategory::General
        );
    }

    #[test]
    fn test_task_category_name() {
        assert_eq!(TaskCategory::CodeGeneration.name(), "code_generation");
        assert_eq!(TaskCategory::CodeReview.name(), "code_review");
        assert_eq!(TaskCategory::General.name(), "general");
    }

    #[test]
    fn test_category_stats_record() {
        let mut stats = CategoryStats::default();

        stats.record(true, 1, 0.9);
        assert_eq!(stats.total_reflections, 1);
        assert_eq!(stats.first_round_passes, 1);
        assert_eq!(stats.failures, 0);

        stats.record(true, 3, 0.7);
        assert_eq!(stats.total_reflections, 2);
        assert_eq!(stats.multi_round_passes, 1);

        stats.record(false, 5, 0.3);
        assert_eq!(stats.total_reflections, 3);
        assert_eq!(stats.failures, 1);
    }

    #[test]
    fn test_category_stats_rates() {
        let mut stats = CategoryStats::default();

        // 2 first-round passes, 1 multi-round pass, 1 failure
        stats.record(true, 1, 0.9);
        stats.record(true, 1, 0.8);
        stats.record(true, 3, 0.7);
        stats.record(false, 5, 0.3);

        assert_eq!(stats.first_round_rate(), 0.5); // 2/4
        assert_eq!(stats.success_rate(), 0.75); // 3/4
    }

    #[test]
    fn test_category_stats_empty() {
        let stats = CategoryStats::default();
        assert_eq!(stats.first_round_rate(), 0.0);
        assert_eq!(stats.success_rate(), 0.0);
    }

    #[test]
    fn test_reflection_stats_record() {
        let mut stats = ReflectionStats::new();

        stats.record(TaskCategory::CodeGeneration, true, 1, 0.9);
        stats.record(TaskCategory::CodeGeneration, true, 2, 0.8);
        stats.record(TaskCategory::Documentation, false, 3, 0.5);

        assert_eq!(stats.global.total_reflections, 3);
        assert_eq!(
            stats
                .get_category_stats(&TaskCategory::CodeGeneration)
                .unwrap()
                .total_reflections,
            2
        );
        assert_eq!(
            stats
                .get_category_stats(&TaskCategory::Documentation)
                .unwrap()
                .total_reflections,
            1
        );
        assert!(stats.get_category_stats(&TaskCategory::Research).is_none());
    }

    #[test]
    fn test_adaptive_config_default() {
        let config = AdaptiveConfig::default();
        assert!(config.enabled);
        assert_eq!(config.min_samples, 10);
        assert_eq!(config.high_success_threshold, 0.9);
        assert_eq!(config.low_success_threshold, 0.6);
    }

    #[test]
    fn test_adapted_params_default() {
        let params = AdaptedParams::default();
        assert_eq!(params.confidence_threshold, 0.7);
        assert_eq!(params.max_reflection_rounds, 3);
        assert!(!params.skip_deep_reflection);
    }

    #[test]
    fn test_adaptive_strategy_creation() {
        let strategy = AdaptiveStrategy::with_defaults();
        assert!(strategy.is_enabled());
    }

    #[test]
    fn test_adaptive_strategy_disabled() {
        let config = AdaptiveConfig {
            enabled: false,
            ..Default::default()
        };
        let strategy = AdaptiveStrategy::new(config);

        let base_config = ReflectionConfig::default();
        let params = strategy.adapt_for_task("Write code", &base_config);

        assert_eq!(
            params.confidence_threshold,
            base_config.confidence_threshold
        );
        assert!(params.adaptation_reason.contains("disabled"));
    }

    #[test]
    fn test_adaptive_strategy_insufficient_samples() {
        let strategy = AdaptiveStrategy::with_defaults();

        // Record fewer than min_samples
        for _ in 0..5 {
            strategy.record_outcome("Write code", true, 1, 0.9);
        }

        let base_config = ReflectionConfig::default();
        let params = strategy.adapt_for_task("Write code", &base_config);

        assert!(params.adaptation_reason.contains("Insufficient samples"));
    }

    #[test]
    fn test_adaptive_strategy_high_success() {
        let mut config = AdaptiveConfig::default();
        config.min_samples = 5;
        let strategy = AdaptiveStrategy::new(config);

        // Record high success rate
        for _ in 0..10 {
            strategy.record_outcome("Write code", true, 1, 0.95);
        }

        let base_config = ReflectionConfig::default();
        let params = strategy.adapt_for_task("Write code", &base_config);

        // Should reduce reflection depth
        assert!(params.skip_deep_reflection);
        assert!(params.adaptation_reason.contains("High success rate"));
    }

    #[test]
    fn test_adaptive_strategy_low_success() {
        let mut config = AdaptiveConfig::default();
        config.min_samples = 5;
        let strategy = AdaptiveStrategy::new(config);

        // Record low success rate
        for _ in 0..10 {
            strategy.record_outcome("Write code", false, 5, 0.3);
        }

        let base_config = ReflectionConfig::default();
        let params = strategy.adapt_for_task("Write code", &base_config);

        // Should increase reflection depth
        assert!(!params.skip_deep_reflection);
        assert!(params.adaptation_reason.contains("Low success rate"));
    }

    #[test]
    fn test_adaptive_strategy_stats() {
        let strategy = AdaptiveStrategy::with_defaults();

        strategy.record_outcome("Write code", true, 1, 0.9);
        strategy.record_outcome("Review code", false, 3, 0.5);

        let stats = strategy.stats();
        assert_eq!(stats.global.total_reflections, 2);
    }

    #[test]
    fn test_adaptive_strategy_reset() {
        let strategy = AdaptiveStrategy::with_defaults();

        strategy.record_outcome("task", true, 1, 0.9);
        assert_eq!(strategy.stats().global.total_reflections, 1);

        strategy.reset_stats();
        assert_eq!(strategy.stats().global.total_reflections, 0);
    }

    #[test]
    fn test_category_stats_summary() {
        let mut stats = CategoryStats::default();
        stats.record(true, 1, 0.9);
        stats.record(true, 2, 0.8);

        let summary = stats.summary();
        assert!(summary.contains("total=2"));
        assert!(summary.contains("success_rate=100.0%"));
    }

    #[test]
    fn test_reflection_stats_summary() {
        let mut stats = ReflectionStats::new();
        stats.record(TaskCategory::CodeGeneration, true, 1, 0.9);

        let summary = stats.summary();
        assert!(summary.contains("Global:"));
        assert!(summary.contains("code_generation"));
    }
}
