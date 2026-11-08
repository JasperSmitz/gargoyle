//! The averaging window for CPU and load health (§8 "Windowed metrics").

use std::collections::VecDeque;
use std::time::Duration;

/// Upper bound on retained samples, so a long window with a short interval cannot grow without
/// limit (e.g. a 1h window at 1s is 3600 samples; beyond the cap the oldest are dropped early).
const MAX_SAMPLES: usize = 4096;

/// Timestamped samples covering the last `span` seconds; [`Window::mean`] is what CPU and load
/// health are evaluated on.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    span_s: i64,
    samples: VecDeque<(i64, f64)>,
}

impl Window {
    pub fn new(span: Duration) -> Self {
        Self {
            span_s: i64::try_from(span.as_secs()).unwrap_or(i64::MAX),
            samples: VecDeque::new(),
        }
    }

    /// Adds a sample taken at `ts` (unix seconds) and drops those that fell out of the window.
    ///
    /// Non-finite values are ignored. If the clock went backwards the old samples cannot be
    /// placed relative to the new one, so the window restarts.
    pub fn push(&mut self, ts: i64, value: f64) {
        if !value.is_finite() {
            return;
        }
        if self.samples.back().is_some_and(|&(last, _)| ts < last) {
            self.samples.clear();
        }
        self.samples.push_back((ts, value));
        let oldest_kept = ts.saturating_sub(self.span_s);
        while self.samples.front().is_some_and(|&(t, _)| t <= oldest_kept)
            || self.samples.len() > MAX_SAMPLES
        {
            self.samples.pop_front();
        }
    }

    /// Mean of the samples in the window, or `None` if there are none.
    pub fn mean(&self) -> Option<f64> {
        if self.samples.is_empty() {
            return None;
        }
        let sum: f64 = self.samples.iter().map(|&(_, v)| v).sum();
        Some(sum / self.samples.len() as f64)
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(span_s: u64) -> Window {
        Window::new(Duration::from_secs(span_s))
    }

    #[test]
    fn empty_window_has_no_mean() {
        assert_eq!(window(120).mean(), None);
    }

    #[test]
    fn mean_covers_only_the_span() {
        let mut w = window(30);
        for (ts, v) in [(0, 100.0), (10, 10.0), (20, 20.0), (30, 30.0)] {
            w.push(ts, v);
        }
        // ts 0 is exactly 30s old at ts 30 and falls out; 10, 20, 30 remain.
        assert_eq!(w.len(), 3);
        assert_eq!(w.mean(), Some(20.0));
    }

    #[test]
    fn a_single_spike_is_smoothed() {
        let mut w = window(120);
        for i in 0..12 {
            w.push(i * 10, if i == 11 { 100.0 } else { 10.0 });
        }
        assert_eq!(w.mean(), Some(17.5));
    }

    #[test]
    fn gap_longer_than_span_leaves_only_new_sample() {
        let mut w = window(60);
        w.push(0, 50.0);
        w.push(10, 50.0);
        w.push(1000, 5.0);
        assert_eq!(w.len(), 1);
        assert_eq!(w.mean(), Some(5.0));
    }

    #[test]
    fn clock_going_backwards_restarts() {
        let mut w = window(60);
        w.push(100, 50.0);
        w.push(50, 10.0);
        assert_eq!(w.len(), 1);
        assert_eq!(w.mean(), Some(10.0));
    }

    #[test]
    fn non_finite_values_are_ignored() {
        let mut w = window(60);
        w.push(0, f64::NAN);
        w.push(1, f64::INFINITY);
        assert!(w.is_empty());
    }

    #[test]
    fn size_is_capped() {
        let mut w = window(1_000_000);
        for ts in 0..(MAX_SAMPLES as i64 + 10) {
            w.push(ts, 1.0);
        }
        assert_eq!(w.len(), MAX_SAMPLES);
    }
}
