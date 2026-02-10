// SPDX-License-Identifier: MIT
use serde::{Deserialize, Serialize};

use crate::datasource::SessionMetadata;
use crate::fex::smaps::MemSnapshot;
use crate::sampler::accumulator::{
    ComputedFrame, CumulativeCountStats, HistogramEntry, ThreadLoad,
};
use crate::sampler::thread_stats::ThreadDelta;

pub const MAGIC: [u8; 4] = *b"FLXR";
pub const FORMAT_VERSION: u8 = 2;
pub const EOF_MARKER: [u8; 4] = *b"FEOF";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileHeader {
    pub magic: [u8; 4],
    pub format_version: u8,
    pub metadata: SessionMetadata,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Frame {
    pub computed: ComputedFrame,
    pub per_thread_deltas: Vec<ThreadDelta>,
}

#[derive(Deserialize)]
pub struct LegacyComputedFrame {
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
}

#[derive(Deserialize)]
pub struct LegacyFrame {
    pub computed: LegacyComputedFrame,
    pub per_thread_deltas: Vec<ThreadDelta>,
}

impl From<LegacyFrame> for Frame {
    fn from(legacy: LegacyFrame) -> Self {
        let lc = legacy.computed;
        Self {
            computed: ComputedFrame {
                timestamp_ns: lc.timestamp_ns,
                sample_period_ns: lc.sample_period_ns,
                threads_sampled: lc.threads_sampled,
                total_jit_time: lc.total_jit_time,
                total_signal_time: lc.total_signal_time,
                total_sigbus_count: lc.total_sigbus_count,
                total_smc_count: lc.total_smc_count,
                total_float_fallback_count: lc.total_float_fallback_count,
                total_cache_miss_count: lc.total_cache_miss_count,
                total_cache_read_lock_time: lc.total_cache_read_lock_time,
                total_cache_write_lock_time: lc.total_cache_write_lock_time,
                total_jit_count: lc.total_jit_count,
                total_jit_invocations: lc.total_jit_invocations,
                fex_load_percent: lc.fex_load_percent,
                thread_loads: lc.thread_loads,
                mem: lc.mem,
                histogram_entry: lc.histogram_entry,
                cumulative: CumulativeCountStats::default(),
            },
            per_thread_deltas: legacy.per_thread_deltas,
        }
    }
}
