// SPDX-License-Identifier: MIT
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::fex::types::ThreadStats;

const DEFAULT_STALE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThreadDelta {
    pub tid: u32,
    pub jit_time: u64,
    pub signal_time: u64,
    pub sigbus_count: u64,
    pub smc_count: u64,
    pub float_fallback_count: u64,
    pub cache_miss_count: u64,
    pub cache_read_lock_time: u64,
    pub cache_write_lock_time: u64,
    pub jit_count: u64,
}

pub struct SampleResult {
    #[allow(dead_code)]
    pub timestamp: Instant,
    pub per_thread: Vec<ThreadDelta>,
    pub threads_sampled: usize,
}

pub struct ThreadSampler {
    previous: BTreeMap<u32, ThreadStats>,
    last_seen: BTreeMap<u32, Instant>,
    stale_timeout: Duration,
}

impl ThreadSampler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            previous: BTreeMap::new(),
            last_seen: BTreeMap::new(),
            stale_timeout: DEFAULT_STALE_TIMEOUT,
        }
    }

    pub fn sample(&mut self, raw_stats: &[ThreadStats], now: Instant) -> SampleResult {
        let mut deltas = Vec::with_capacity(raw_stats.len());

        for stat in raw_stats {
            let tid = stat.tid;
            self.last_seen.insert(tid, now);

            let delta = if let Some(prev) = self.previous.get(&tid) {
                ThreadDelta {
                    tid,
                    jit_time: stat
                        .accumulated_jit_time
                        .wrapping_sub(prev.accumulated_jit_time),
                    signal_time: stat
                        .accumulated_signal_time
                        .wrapping_sub(prev.accumulated_signal_time),
                    sigbus_count: stat.sigbus_count.wrapping_sub(prev.sigbus_count),
                    smc_count: stat.smc_count.wrapping_sub(prev.smc_count),
                    float_fallback_count: stat
                        .float_fallback_count
                        .wrapping_sub(prev.float_fallback_count),
                    cache_miss_count: stat
                        .accumulated_cache_miss_count
                        .wrapping_sub(prev.accumulated_cache_miss_count),
                    cache_read_lock_time: stat
                        .accumulated_cache_read_lock_time
                        .wrapping_sub(prev.accumulated_cache_read_lock_time),
                    cache_write_lock_time: stat
                        .accumulated_cache_write_lock_time
                        .wrapping_sub(prev.accumulated_cache_write_lock_time),
                    jit_count: stat
                        .accumulated_jit_count
                        .wrapping_sub(prev.accumulated_jit_count),
                }
            } else {
                ThreadDelta {
                    tid,
                    ..ThreadDelta::default()
                }
            };

            self.previous.insert(tid, *stat);
            deltas.push(delta);
        }

        let threads_sampled = deltas.len();

        self.last_seen
            .retain(|_, seen| now.duration_since(*seen) < self.stale_timeout);
        self.previous
            .retain(|tid, _| self.last_seen.contains_key(tid));

        SampleResult {
            timestamp: now,
            per_thread: deltas,
            threads_sampled,
        }
    }
}

impl Default for ThreadSampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_stats(tid: u32, jit_time: u64, signal_time: u64) -> ThreadStats {
        ThreadStats {
            tid,
            accumulated_jit_time: jit_time,
            accumulated_signal_time: signal_time,
            ..ThreadStats::default()
        }
    }

    #[test]
    fn first_sample_yields_zero_deltas() {
        let mut sampler = ThreadSampler::new();
        let now = Instant::now();
        let stats = vec![make_stats(1, 1000, 500)];
        let result = sampler.sample(&stats, now);

        assert_eq!(result.threads_sampled, 1);
        assert_eq!(result.per_thread.len(), 1);
        assert_eq!(result.per_thread[0].jit_time, 0);
        assert_eq!(result.per_thread[0].signal_time, 0);
    }

    #[test]
    fn second_sample_yields_correct_deltas() {
        let mut sampler = ThreadSampler::new();
        let t0 = Instant::now();
        sampler.sample(&[make_stats(1, 1000, 500)], t0);

        let t1 = t0 + Duration::from_secs(1);
        let result = sampler.sample(&[make_stats(1, 3000, 800)], t1);

        assert_eq!(result.per_thread[0].jit_time, 2000);
        assert_eq!(result.per_thread[0].signal_time, 300);
    }

    #[test]
    fn stale_threads_are_evicted() {
        let mut sampler = ThreadSampler::new();
        let t0 = Instant::now();
        sampler.sample(&[make_stats(1, 100, 50), make_stats(2, 200, 100)], t0);

        let t1 = t0 + Duration::from_secs(11);
        let result = sampler.sample(&[make_stats(1, 200, 60)], t1);

        assert_eq!(result.threads_sampled, 1);
        assert!(!sampler.previous.contains_key(&2));
    }

    #[test]
    fn multiple_threads_deltas() {
        let mut sampler = ThreadSampler::new();
        let t0 = Instant::now();
        sampler.sample(&[make_stats(10, 1000, 500), make_stats(20, 2000, 1000)], t0);

        let t1 = t0 + Duration::from_secs(1);
        let result = sampler.sample(&[make_stats(10, 1500, 600), make_stats(20, 3000, 1200)], t1);

        assert_eq!(result.per_thread.len(), 2);
        assert_eq!(result.per_thread[0].tid, 10);
        assert_eq!(result.per_thread[0].jit_time, 500);
        assert_eq!(result.per_thread[1].tid, 20);
        assert_eq!(result.per_thread[1].jit_time, 1000);
    }
}
