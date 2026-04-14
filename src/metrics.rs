//! Prometheus-style metrics registry.
//!
//! This is a dependency-free in-memory registry — we don't pull the
//! `prometheus` crate because the scrape surface is small, always-on, and
//! loopback-only. Counters are `AtomicU64`, histograms are fixed-bucket
//! with atomic counters per bucket. Output matches the Prometheus text
//! exposition format.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::RwLock;

/// Loopback-only metrics registry.
#[derive(Clone, Default)]
pub struct Metrics {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    counters: RwLock<BTreeMap<String, Arc<AtomicU64>>>,
    gauges: RwLock<BTreeMap<String, Arc<AtomicI64>>>,
    histograms: RwLock<BTreeMap<String, Arc<Histogram>>>,
}

/// Histogram with baked-in buckets.
///
/// Seconds buckets span a typical web-latency range. Everything over 10s
/// ends up in `+Inf`, which is visible in scrape output.
pub struct Histogram {
    buckets_upper: Vec<f64>,
    buckets_count: Vec<AtomicU64>,
    sum_nanos: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    const DEFAULT_UPPERS: &'static [f64] = &[
        0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
    ];

    fn new(uppers: &[f64]) -> Self {
        Self {
            buckets_upper: uppers.to_vec(),
            buckets_count: uppers.iter().map(|_| AtomicU64::new(0)).collect(),
            sum_nanos: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, seconds: f64) {
        let nanos = (seconds * 1_000_000_000.0) as u64;
        self.sum_nanos.fetch_add(nanos, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        for (i, upper) in self.buckets_upper.iter().enumerate() {
            if seconds <= *upper {
                self.buckets_count[i].fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner::default()),
        }
    }

    pub fn inc_counter(&self, name: &str, by: u64) {
        let map = self.inner.counters.read().expect("counters rwlock poisoned");
        if let Some(c) = map.get(name) {
            c.fetch_add(by, Ordering::Relaxed);
            return;
        }
        drop(map);
        let mut map = self.inner.counters.write().expect("counters rwlock poisoned");
        let c = map
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone();
        c.fetch_add(by, Ordering::Relaxed);
    }

    pub fn set_gauge(&self, name: &str, value: i64) {
        let map = self.inner.gauges.read().expect("gauges rwlock poisoned");
        if let Some(g) = map.get(name) {
            g.store(value, Ordering::Relaxed);
            return;
        }
        drop(map);
        let mut map = self.inner.gauges.write().expect("gauges rwlock poisoned");
        let g = map
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(AtomicI64::new(0)))
            .clone();
        g.store(value, Ordering::Relaxed);
    }

    pub fn observe_histogram(&self, name: &str, seconds: f64) {
        let map = self.inner.histograms.read().expect("hist rwlock poisoned");
        if let Some(h) = map.get(name) {
            h.observe(seconds);
            return;
        }
        drop(map);
        let mut map = self.inner.histograms.write().expect("hist rwlock poisoned");
        let h = map
            .entry(name.to_string())
            .or_insert_with(|| Arc::new(Histogram::new(Histogram::DEFAULT_UPPERS)))
            .clone();
        h.observe(seconds);
    }

    /// Render the whole registry as Prometheus text format.
    pub fn render(&self) -> String {
        let mut out = String::new();

        let counters = self.inner.counters.read().expect("counters rwlock poisoned");
        for (name, value) in counters.iter() {
            out.push_str(&format!("# TYPE {name} counter\n"));
            out.push_str(&format!("{name} {}\n", value.load(Ordering::Relaxed)));
        }
        drop(counters);

        let gauges = self.inner.gauges.read().expect("gauges rwlock poisoned");
        for (name, value) in gauges.iter() {
            out.push_str(&format!("# TYPE {name} gauge\n"));
            out.push_str(&format!("{name} {}\n", value.load(Ordering::Relaxed)));
        }
        drop(gauges);

        let hists = self.inner.histograms.read().expect("hist rwlock poisoned");
        for (name, h) in hists.iter() {
            out.push_str(&format!("# TYPE {name} histogram\n"));
            let mut cumulative: u64 = 0;
            for (i, upper) in h.buckets_upper.iter().enumerate() {
                cumulative += h.buckets_count[i].load(Ordering::Relaxed);
                out.push_str(&format!("{name}_bucket{{le=\"{upper}\"}} {cumulative}\n"));
            }
            let total = h.count.load(Ordering::Relaxed);
            out.push_str(&format!("{name}_bucket{{le=\"+Inf\"}} {total}\n"));
            let sum_sec =
                (h.sum_nanos.load(Ordering::Relaxed) as f64) / 1_000_000_000.0;
            out.push_str(&format!("{name}_sum {sum_sec}\n"));
            out.push_str(&format!("{name}_count {total}\n"));
        }
        drop(hists);

        out
    }
}
