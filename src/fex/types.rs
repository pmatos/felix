// SPDX-License-Identifier: MIT
use std::fmt;

use serde::{Deserialize, Serialize};

pub const STATS_VERSION: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum AppType {
    Linux32 = 0,
    Linux64 = 1,
    WinArm64ec = 2,
    WinWow64 = 3,
}

impl fmt::Display for AppType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Linux32 => write!(f, "Linux32"),
            Self::Linux64 => write!(f, "Linux64"),
            Self::WinArm64ec => write!(f, "arm64ec"),
            Self::WinWow64 => write!(f, "wow64"),
        }
    }
}

impl AppType {
    /// Converts a raw `u8` to an `AppType`, returning `None` for unknown values.
    #[must_use]
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::Linux32),
            1 => Some(Self::Linux64),
            2 => Some(Self::WinArm64ec),
            3 => Some(Self::WinWow64),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ThreadStatsHeader {
    pub version: u8,
    pub app_type: u8,
    pub thread_stats_size: u16,
    pub fex_version: [u8; 48],
    pub head: u32,
    pub size: u32,
    pub pad: u32,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[repr(C, align(16))]
pub struct ThreadStats {
    pub next: u32,
    pub tid: u32,
    pub accumulated_jit_time: u64,
    pub accumulated_signal_time: u64,
    pub sigbus_count: u64,
    pub smc_count: u64,
    pub float_fallback_count: u64,
    pub accumulated_cache_miss_count: u64,
    pub accumulated_cache_read_lock_time: u64,
    pub accumulated_cache_write_lock_time: u64,
    pub accumulated_jit_count: u64,
}

const _: () = assert!(
    std::mem::size_of::<ThreadStats>().is_multiple_of(16),
    "ThreadStats size must be a multiple of 16"
);

const _: () = assert!(
    std::mem::align_of::<ThreadStats>() == 16,
    "ThreadStats must be 16-byte aligned"
);
