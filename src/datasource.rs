// SPDX-License-Identifier: MIT
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::fex::types::AppType;
use crate::sampler::accumulator::ComputedFrame;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub pid: i32,
    pub fex_version: String,
    pub app_type: AppType,
    pub stats_version: u8,
    pub cycle_counter_frequency: u64,
    pub hardware_concurrency: usize,
    pub recording_start: SystemTime,
}

pub trait DataSource {
    fn next_frame(&mut self) -> Option<ComputedFrame>;
    #[allow(dead_code)]
    fn metadata(&self) -> &SessionMetadata;
    #[allow(dead_code)]
    fn is_live(&self) -> bool;
}
