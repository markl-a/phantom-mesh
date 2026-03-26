//! Observability — lightweight Prometheus-compatible metrics.
//! No external dependencies; uses atomic counters and lock-free histograms.
//!
//! Exposes a `GET /metrics` endpoint in Prometheus text format.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::Instant;

/// Global metrics registry
pub struct MetricsRegistry {
    counters: RwLock<HashMap<String, AtomicU64>>,
    gauges: RwLock<HashMap<String, AtomicU64>>,
    /// Histogram buckets (sum, count) per metric
    histograms: RwLock<HashMap<String, HistogramData>>,
}

/// Histogram with predefined buckets for latency tracking
pub struct HistogramData {
    /// Bucket upper bounds in milliseconds
    pub buckets: Vec<f64>,
    /// Count per bucket (cumulative)
    pub bucket_counts: Vec<AtomicU64>,
    /// Total sum of observed values
    pub sum: AtomicU64, // stored as microseconds
    /// Total count of observations
    pub count: AtomicU64,
}

impl HistogramData {
    fn new(buckets: Vec<f64>) -> Self {
        let bucket_counts = buckets.iter().map(|_| AtomicU64::new(0)).collect();
        Self {
            buckets,
            bucket_counts,
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    fn observe(&self, value_ms: f64) {
        let value_us = (value_ms * 1000.0) as u64;
        self.sum.fetch_add(value_us, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        for (i, bound) in self.buckets.iter().enumerate() {
            if value_ms <= *bound {
                self.bucket_counts[i].fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            counters: RwLock::new(HashMap::new()),
            gauges: RwLock::new(HashMap::new()),
            histograms: RwLock::new(HashMap::new()),
        }
    }

    /// Increment a counter by 1
    pub fn inc(&self, name: &str) {
        self.inc_by(name, 1);
    }

    /// Increment a counter by a given amount
    pub fn inc_by(&self, name: &str, n: u64) {
        let counters = self.counters.read().unwrap();
        if let Some(counter) = counters.get(name) {
            counter.fetch_add(n, Ordering::Relaxed);
            return;
        }
        drop(counters);

        let mut counters = self.counters.write().unwrap();
        counters
            .entry(name.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(n, Ordering::Relaxed);
    }

    /// Get counter value
    pub fn counter(&self, name: &str) -> u64 {
        let counters = self.counters.read().unwrap();
        counters.get(name).map(|c| c.load(Ordering::Relaxed)).unwrap_or(0)
    }

    /// Set a gauge to a specific value
    pub fn gauge_set(&self, name: &str, value: u64) {
        let gauges = self.gauges.read().unwrap();
        if let Some(gauge) = gauges.get(name) {
            gauge.store(value, Ordering::Relaxed);
            return;
        }
        drop(gauges);

        let mut gauges = self.gauges.write().unwrap();
        gauges
            .entry(name.to_string())
            .or_insert_with(|| AtomicU64::new(0))
            .store(value, Ordering::Relaxed);
    }

    /// Get gauge value
    pub fn gauge(&self, name: &str) -> u64 {
        let gauges = self.gauges.read().unwrap();
        gauges.get(name).map(|g| g.load(Ordering::Relaxed)).unwrap_or(0)
    }

    /// Register a histogram with default latency buckets (ms)
    pub fn register_histogram(&self, name: &str) {
        self.register_histogram_with_buckets(
            name,
            vec![10.0, 25.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10000.0],
        );
    }

    /// Register a histogram with custom buckets
    pub fn register_histogram_with_buckets(&self, name: &str, buckets: Vec<f64>) {
        let mut histograms = self.histograms.write().unwrap();
        histograms.entry(name.to_string()).or_insert_with(|| HistogramData::new(buckets));
    }

    /// Observe a value in a histogram
    pub fn observe(&self, name: &str, value_ms: f64) {
        let histograms = self.histograms.read().unwrap();
        if let Some(hist) = histograms.get(name) {
            hist.observe(value_ms);
        }
    }

    /// Get histogram count
    pub fn histogram_count(&self, name: &str) -> u64 {
        let histograms = self.histograms.read().unwrap();
        histograms.get(name).map(|h| h.count.load(Ordering::Relaxed)).unwrap_or(0)
    }

    /// Start a timer that automatically records duration when dropped
    pub fn start_timer(&self, histogram_name: &str) -> Timer<'_> {
        Timer {
            registry: self,
            name: histogram_name.to_string(),
            start: Instant::now(),
        }
    }

    /// Render all metrics in Prometheus text exposition format
    pub fn render_prometheus(&self) -> String {
        let mut output = String::new();

        // Counters
        let counters = self.counters.read().unwrap();
        let mut counter_names: Vec<_> = counters.keys().collect();
        counter_names.sort();
        for name in counter_names {
            let value = counters[name].load(Ordering::Relaxed);
            output.push_str(&format!("# TYPE {} counter\n", name));
            output.push_str(&format!("{} {}\n", name, value));
        }
        drop(counters);

        // Gauges
        let gauges = self.gauges.read().unwrap();
        let mut gauge_names: Vec<_> = gauges.keys().collect();
        gauge_names.sort();
        for name in gauge_names {
            let value = gauges[name].load(Ordering::Relaxed);
            output.push_str(&format!("# TYPE {} gauge\n", name));
            output.push_str(&format!("{} {}\n", name, value));
        }
        drop(gauges);

        // Histograms
        let histograms = self.histograms.read().unwrap();
        let mut hist_names: Vec<_> = histograms.keys().collect();
        hist_names.sort();
        for name in hist_names {
            let hist = &histograms[name];
            output.push_str(&format!("# TYPE {} histogram\n", name));
            for (i, bound) in hist.buckets.iter().enumerate() {
                let count = hist.bucket_counts[i].load(Ordering::Relaxed);
                output.push_str(&format!("{}_bucket{{le=\"{}\"}} {}\n", name, bound, count));
            }
            output.push_str(&format!(
                "{}_bucket{{le=\"+Inf\"}} {}\n",
                name,
                hist.count.load(Ordering::Relaxed)
            ));
            let sum_us = hist.sum.load(Ordering::Relaxed);
            output.push_str(&format!("{}_sum {:.3}\n", name, sum_us as f64 / 1000.0));
            output.push_str(&format!("{}_count {}\n", name, hist.count.load(Ordering::Relaxed)));
        }

        output
    }

    /// Render a JSON health summary (for /health and Telegram /status)
    pub fn render_health_json(&self) -> serde_json::Value {
        let counters = self.counters.read().unwrap();
        let gauges = self.gauges.read().unwrap();
        let histograms = self.histograms.read().unwrap();

        let mut counter_map = serde_json::Map::new();
        for (name, val) in counters.iter() {
            counter_map.insert(name.clone(), serde_json::json!(val.load(Ordering::Relaxed)));
        }

        let mut gauge_map = serde_json::Map::new();
        for (name, val) in gauges.iter() {
            gauge_map.insert(name.clone(), serde_json::json!(val.load(Ordering::Relaxed)));
        }

        let mut hist_map = serde_json::Map::new();
        for (name, hist) in histograms.iter() {
            let count = hist.count.load(Ordering::Relaxed);
            let sum_us = hist.sum.load(Ordering::Relaxed);
            let avg_ms = if count > 0 { (sum_us as f64 / 1000.0) / count as f64 } else { 0.0 };
            hist_map.insert(name.clone(), serde_json::json!({
                "count": count,
                "sum_ms": (sum_us as f64 / 1000.0 * 100.0).round() / 100.0,
                "avg_ms": (avg_ms * 100.0).round() / 100.0,
            }));
        }

        serde_json::json!({
            "counters": counter_map,
            "gauges": gauge_map,
            "histograms": hist_map,
        })
    }

    /// Register standard Phantom Mesh metrics
    pub fn register_defaults(&self) {
        // Histograms
        self.register_histogram("phantom_mesh_dispatch_duration_ms");
        self.register_histogram("phantom_mesh_tool_duration_ms");
        self.register_histogram("phantom_mesh_llm_duration_ms");
        // Default counters/gauges are created on first use
    }
}

/// Helper to create a pre-configured MetricsRegistry with standard Phantom Mesh metrics
pub fn default_metrics() -> MetricsRegistry {
    let m = MetricsRegistry::new();
    m.register_defaults();
    m
}

/// RAII timer — records elapsed time to a histogram when dropped
pub struct Timer<'a> {
    registry: &'a MetricsRegistry,
    name: String,
    start: Instant,
}

impl<'a> Timer<'a> {
    /// Manually stop the timer and record the duration (in ms)
    pub fn stop(self) -> f64 {
        let elapsed_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        self.registry.observe(&self.name, elapsed_ms);
        std::mem::forget(self); // prevent Drop from recording again
        elapsed_ms
    }
}

impl<'a> Drop for Timer<'a> {
    fn drop(&mut self) {
        let elapsed_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        self.registry.observe(&self.name, elapsed_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_basic() {
        let m = MetricsRegistry::new();
        assert_eq!(m.counter("requests"), 0);
        m.inc("requests");
        assert_eq!(m.counter("requests"), 1);
        m.inc_by("requests", 5);
        assert_eq!(m.counter("requests"), 6);
    }

    #[test]
    fn test_gauge_set() {
        let m = MetricsRegistry::new();
        m.gauge_set("active_agents", 3);
        assert_eq!(m.gauge("active_agents"), 3);
        m.gauge_set("active_agents", 1);
        assert_eq!(m.gauge("active_agents"), 1);
    }

    #[test]
    fn test_histogram_observe() {
        let m = MetricsRegistry::new();
        m.register_histogram("latency_ms");
        m.observe("latency_ms", 50.0);
        m.observe("latency_ms", 150.0);
        m.observe("latency_ms", 3000.0);
        assert_eq!(m.histogram_count("latency_ms"), 3);
    }

    #[test]
    fn test_histogram_buckets() {
        let m = MetricsRegistry::new();
        m.register_histogram_with_buckets("fast", vec![10.0, 50.0, 100.0]);
        m.observe("fast", 5.0);   // fits in 10, 50, 100
        m.observe("fast", 30.0);  // fits in 50, 100
        m.observe("fast", 80.0);  // fits in 100
        m.observe("fast", 200.0); // fits in none

        let histograms = m.histograms.read().unwrap();
        let h = &histograms["fast"];
        assert_eq!(h.bucket_counts[0].load(Ordering::Relaxed), 1); // <=10
        assert_eq!(h.bucket_counts[1].load(Ordering::Relaxed), 2); // <=50
        assert_eq!(h.bucket_counts[2].load(Ordering::Relaxed), 3); // <=100
        assert_eq!(h.count.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn test_prometheus_render() {
        let m = MetricsRegistry::new();
        m.inc("http_requests_total");
        m.gauge_set("agents_active", 2);
        m.register_histogram_with_buckets("request_duration_ms", vec![100.0, 500.0]);
        m.observe("request_duration_ms", 50.0);

        let output = m.render_prometheus();
        assert!(output.contains("# TYPE agents_active gauge"));
        assert!(output.contains("agents_active 2"));
        assert!(output.contains("# TYPE http_requests_total counter"));
        assert!(output.contains("http_requests_total 1"));
        assert!(output.contains("# TYPE request_duration_ms histogram"));
        assert!(output.contains("request_duration_ms_count 1"));
    }

    #[test]
    fn test_timer_records() {
        let m = MetricsRegistry::new();
        m.register_histogram("op_duration");
        {
            let _timer = m.start_timer("op_duration");
            // simulate work
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(m.histogram_count("op_duration"), 1);
    }

    #[test]
    fn test_health_json_basic() {
        let m = MetricsRegistry::new();
        m.inc("requests");
        m.gauge_set("workers", 4);
        let health = m.render_health_json();
        assert_eq!(health["counters"]["requests"], 1);
        assert_eq!(health["gauges"]["workers"], 4);
    }

    #[test]
    fn test_health_json_histograms() {
        let m = MetricsRegistry::new();
        m.register_histogram("latency");
        m.observe("latency", 100.0);
        m.observe("latency", 200.0);
        let health = m.render_health_json();
        assert_eq!(health["histograms"]["latency"]["count"], 2);
    }

    #[test]
    fn test_health_json_empty() {
        let m = MetricsRegistry::new();
        let health = m.render_health_json();
        assert!(health["counters"].as_object().unwrap().is_empty());
        assert!(health["gauges"].as_object().unwrap().is_empty());
    }

    #[test]
    fn test_register_defaults() {
        let m = MetricsRegistry::new();
        m.register_defaults();
        // Should be able to observe without panic
        m.observe("phantom_mesh_dispatch_duration_ms", 50.0);
        m.observe("phantom_mesh_tool_duration_ms", 25.0);
        m.observe("phantom_mesh_llm_duration_ms", 150.0);
        assert_eq!(m.histogram_count("phantom_mesh_dispatch_duration_ms"), 1);
    }

    #[test]
    fn test_default_metrics_fn() {
        let m = default_metrics();
        m.observe("phantom_mesh_dispatch_duration_ms", 10.0);
        assert_eq!(m.histogram_count("phantom_mesh_dispatch_duration_ms"), 1);
    }

    #[test]
    fn test_prometheus_render_sorted() {
        let m = MetricsRegistry::new();
        m.inc("z_last");
        m.inc("a_first");
        let output = m.render_prometheus();
        let z_pos = output.find("z_last").unwrap();
        let a_pos = output.find("a_first").unwrap();
        assert!(a_pos < z_pos, "Prometheus output should be sorted alphabetically");
    }

    #[test]
    fn test_health_json_avg_calculation() {
        let m = MetricsRegistry::new();
        m.register_histogram("test_lat");
        m.observe("test_lat", 100.0);
        m.observe("test_lat", 300.0);
        let health = m.render_health_json();
        // avg should be ~200
        let avg = health["histograms"]["test_lat"]["avg_ms"].as_f64().unwrap();
        assert!(avg > 150.0 && avg < 250.0, "avg_ms should be ~200, got {}", avg);
    }

    #[test]
    fn test_counter_concurrent() {
        use std::sync::Arc;
        let m = Arc::new(MetricsRegistry::new());
        let mut handles = vec![];
        for _ in 0..4 {
            let m = m.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    m.inc("concurrent_counter");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(m.counter("concurrent_counter"), 400);
    }
}
