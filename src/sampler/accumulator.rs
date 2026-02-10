// SPDX-License-Identifier: MIT
use serde::{Deserialize, Serialize};

use super::thread_stats::SampleResult;
use crate::fex::smaps::MemSnapshot;

const NANOSECONDS_IN_SECOND: f64 = 1_000_000_000.0;

const HIGH_SMC_THRESHOLD: u64 = 500;
const HIGH_SIGBUS_THRESHOLD: u64 = 5_000;
const HIGH_SOFTFLOAT_THRESHOLD: u64 = 1_000_000;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CumulativeCountStats {
    pub sigbus: u64,
    pub smc: u64,
    pub float_fallback: u64,
    pub cache_miss: u64,
    pub jit: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ThreadLoad {
    pub tid: u32,
    pub load_percent: f32,
    pub total_cycles: u64,
}

#[allow(clippy::struct_excessive_bools)] // mirrors C++ histogram flags
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistogramEntry {
    pub load_percent: f32,
    pub high_jit_load: bool,
    pub high_invalidation_or_smc: bool,
    pub high_sigbus: bool,
    pub high_softfloat: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ComputedFrame {
    pub timestamp_ns: u64,
    pub sample_period_ns: u64,
    pub threads_sampled: usize,
    pub total_jit_time: u64,
    pub total_signal_time: u64,
    pub total_sigbus_count: u64,
    pub total_smc_count: u64,
    pub total_float_fallback_count: u64,
    pub total_cache_miss_count: u64,
    pub total_cache_read_lock_time: u64,
    pub total_cache_write_lock_time: u64,
    pub total_jit_count: u64,
    pub total_jit_invocations: u64,
    pub fex_load_percent: f64,
    pub thread_loads: Vec<ThreadLoad>,
    pub mem: MemSnapshot,
    pub histogram_entry: HistogramEntry,
    pub cumulative: CumulativeCountStats,
}

pub struct Accumulator {
    cycle_freq: f64,
    hardware_concurrency: usize,
}

impl Accumulator {
    #[must_use]
    pub fn new(cycle_freq: f64, hardware_concurrency: usize) -> Self {
        Self {
            cycle_freq,
            hardware_concurrency,
        }
    }

    #[must_use]
    pub fn compute_frame(
        &self,
        sample: &SampleResult,
        mem: &MemSnapshot,
        sample_period_ns: u64,
        total_jit_invocations: u64,
        cumulative: CumulativeCountStats,
    ) -> ComputedFrame {
        let mut frame = ComputedFrame {
            sample_period_ns,
            threads_sampled: sample.threads_sampled,
            total_jit_invocations,
            mem: mem.clone(),
            cumulative,
            ..ComputedFrame::default()
        };

        let mut per_thread_total_time: Vec<(u32, u64)> =
            Vec::with_capacity(sample.per_thread.len());

        for delta in &sample.per_thread {
            frame.total_jit_time += delta.jit_time;
            frame.total_signal_time += delta.signal_time;
            frame.total_sigbus_count += delta.sigbus_count;
            frame.total_smc_count += delta.smc_count;
            frame.total_float_fallback_count += delta.float_fallback_count;
            frame.total_cache_miss_count += delta.cache_miss_count;
            frame.total_cache_read_lock_time += delta.cache_read_lock_time;
            frame.total_cache_write_lock_time += delta.cache_write_lock_time;
            frame.total_jit_count += delta.jit_count;

            let total_time = delta.jit_time + delta.signal_time;
            per_thread_total_time.push((delta.tid, total_time));
        }

        per_thread_total_time.sort_by(|a, b| b.1.cmp(&a.1));

        let total_jit_time_all = frame.total_jit_time + frame.total_signal_time;

        #[allow(clippy::cast_precision_loss)]
        let sample_period_ns_f64 = sample_period_ns as f64;
        let max_cycles_in_sample_period =
            self.cycle_freq * (sample_period_ns_f64 / NANOSECONDS_IN_SECOND);

        let max_cores_threads = if sample.threads_sampled == 0 {
            1.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            let ts = sample.threads_sampled as f64;
            #[allow(clippy::cast_precision_loss)]
            let hc = self.hardware_concurrency as f64;
            ts.min(hc)
        };

        if max_cycles_in_sample_period > 0.0 {
            #[allow(clippy::cast_precision_loss)]
            let total_time_f64 = total_jit_time_all as f64;
            frame.fex_load_percent =
                (total_time_f64 / (max_cycles_in_sample_period * max_cores_threads)) * 100.0;
        }

        let cap = self.hardware_concurrency.min(per_thread_total_time.len());
        frame.thread_loads = per_thread_total_time[..cap]
            .iter()
            .map(|&(tid, total_cycles)| {
                #[allow(clippy::cast_possible_truncation)]
                let load_percent = if max_cycles_in_sample_period > 0.0 {
                    #[allow(clippy::cast_precision_loss)]
                    let tc = total_cycles as f64;
                    (tc / max_cycles_in_sample_period * 100.0) as f32
                } else {
                    0.0
                };
                ThreadLoad {
                    tid,
                    load_percent,
                    total_cycles,
                }
            })
            .collect();

        #[allow(clippy::cast_possible_truncation)]
        let load_pct_f32 = frame.fex_load_percent as f32;
        frame.histogram_entry = HistogramEntry {
            load_percent: load_pct_f32,
            high_jit_load: total_jit_time_all >= {
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                let v = max_cycles_in_sample_period as u64;
                v
            },
            high_invalidation_or_smc: frame.total_smc_count >= HIGH_SMC_THRESHOLD,
            high_sigbus: frame.total_sigbus_count >= HIGH_SIGBUS_THRESHOLD,
            high_softfloat: frame.total_float_fallback_count >= HIGH_SOFTFLOAT_THRESHOLD,
        };

        frame
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::sampler::thread_stats::ThreadDelta;

    fn make_sample(deltas: Vec<ThreadDelta>) -> SampleResult {
        let count = deltas.len();
        SampleResult {
            timestamp: Instant::now(),
            per_thread: deltas,
            threads_sampled: count,
        }
    }

    #[test]
    fn empty_sample_produces_zero_frame() {
        let acc = Accumulator::new(1_000_000_000.0, 4);
        let sample = make_sample(vec![]);
        let frame = acc.compute_frame(
            &sample,
            &MemSnapshot::default(),
            1_000_000_000,
            0,
            CumulativeCountStats::default(),
        );

        assert_eq!(frame.threads_sampled, 0);
        assert_eq!(frame.total_jit_time, 0);
        assert!(frame.fex_load_percent.abs() < f64::EPSILON);
    }

    #[test]
    fn single_thread_full_load() {
        let acc = Accumulator::new(1_000_000_000.0, 4);
        let delta = ThreadDelta {
            tid: 1,
            jit_time: 1_000_000_000,
            ..ThreadDelta::default()
        };
        let sample = make_sample(vec![delta]);
        let frame = acc.compute_frame(
            &sample,
            &MemSnapshot::default(),
            1_000_000_000,
            100,
            CumulativeCountStats::default(),
        );

        assert!((frame.fex_load_percent - 100.0).abs() < 0.01);
        assert_eq!(frame.thread_loads.len(), 1);
        assert!((f64::from(frame.thread_loads[0].load_percent) - 100.0).abs() < 0.01);
        assert!(frame.histogram_entry.high_jit_load);
    }

    #[test]
    fn histogram_thresholds() {
        let acc = Accumulator::new(1_000_000_000.0, 4);
        let delta = ThreadDelta {
            tid: 1,
            jit_time: 100,
            smc_count: 501,
            sigbus_count: 5001,
            float_fallback_count: 1_000_001,
            ..ThreadDelta::default()
        };
        let sample = make_sample(vec![delta]);
        let frame = acc.compute_frame(
            &sample,
            &MemSnapshot::default(),
            1_000_000_000,
            0,
            CumulativeCountStats::default(),
        );

        assert!(frame.histogram_entry.high_invalidation_or_smc);
        assert!(frame.histogram_entry.high_sigbus);
        assert!(frame.histogram_entry.high_softfloat);
        assert!(!frame.histogram_entry.high_jit_load);
    }

    #[test]
    fn thread_loads_capped_at_hardware_concurrency() {
        let acc = Accumulator::new(1_000_000_000.0, 2);
        let deltas = vec![
            ThreadDelta {
                tid: 1,
                jit_time: 300,
                ..ThreadDelta::default()
            },
            ThreadDelta {
                tid: 2,
                jit_time: 200,
                ..ThreadDelta::default()
            },
            ThreadDelta {
                tid: 3,
                jit_time: 100,
                ..ThreadDelta::default()
            },
        ];
        let sample = make_sample(deltas);
        let frame = acc.compute_frame(
            &sample,
            &MemSnapshot::default(),
            1_000_000_000,
            0,
            CumulativeCountStats::default(),
        );

        assert_eq!(frame.thread_loads.len(), 2);
        assert_eq!(frame.thread_loads[0].tid, 1);
        assert_eq!(frame.thread_loads[1].tid, 2);
    }

    #[test]
    fn totals_are_summed_across_threads() {
        let acc = Accumulator::new(1_000_000_000.0, 4);
        let deltas = vec![
            ThreadDelta {
                tid: 1,
                jit_time: 100,
                signal_time: 50,
                sigbus_count: 10,
                smc_count: 5,
                float_fallback_count: 1000,
                cache_miss_count: 20,
                cache_read_lock_time: 30,
                cache_write_lock_time: 40,
                jit_count: 60,
            },
            ThreadDelta {
                tid: 2,
                jit_time: 200,
                signal_time: 100,
                sigbus_count: 20,
                smc_count: 10,
                float_fallback_count: 2000,
                cache_miss_count: 40,
                cache_read_lock_time: 60,
                cache_write_lock_time: 80,
                jit_count: 120,
            },
        ];
        let sample = make_sample(deltas);
        let frame = acc.compute_frame(
            &sample,
            &MemSnapshot::default(),
            1_000_000_000,
            500,
            CumulativeCountStats::default(),
        );

        assert_eq!(frame.total_jit_time, 300);
        assert_eq!(frame.total_signal_time, 150);
        assert_eq!(frame.total_sigbus_count, 30);
        assert_eq!(frame.total_smc_count, 15);
        assert_eq!(frame.total_float_fallback_count, 3000);
        assert_eq!(frame.total_cache_miss_count, 60);
        assert_eq!(frame.total_cache_read_lock_time, 90);
        assert_eq!(frame.total_cache_write_lock_time, 120);
        assert_eq!(frame.total_jit_count, 180);
        assert_eq!(frame.total_jit_invocations, 500);
    }

    #[test]
    fn cumulative_stats_pass_through() {
        let acc = Accumulator::new(1_000_000_000.0, 4);
        let sample = make_sample(vec![]);
        let cumulative = CumulativeCountStats {
            sigbus: 100,
            smc: 200,
            float_fallback: 300,
            cache_miss: 400,
            jit: 500,
        };
        let frame = acc.compute_frame(
            &sample,
            &MemSnapshot::default(),
            1_000_000_000,
            0,
            cumulative,
        );

        assert_eq!(frame.cumulative.sigbus, 100);
        assert_eq!(frame.cumulative.smc, 200);
        assert_eq!(frame.cumulative.float_fallback, 300);
        assert_eq!(frame.cumulative.cache_miss, 400);
        assert_eq!(frame.cumulative.jit, 500);
    }
}
