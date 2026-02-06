// SPDX-License-Identifier: MIT
use serde::{Deserialize, Serialize};

use crate::datasource::SessionMetadata;
use crate::sampler::accumulator::ComputedFrame;
use crate::sampler::thread_stats::ThreadDelta;

pub const MAGIC: [u8; 4] = *b"FLXR";
pub const FORMAT_VERSION: u8 = 1;
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
