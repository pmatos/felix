// SPDX-License-Identifier: MIT
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LargestAnon {
    pub begin: u64,
    pub end: u64,
    pub size: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemSnapshot {
    pub total_anon: u64,
    pub jit_code: u64,
    pub op_dispatcher: u64,
    pub frontend: u64,
    pub cpu_backend: u64,
    pub lookup: u64,
    pub lookup_l1: u64,
    pub thread_states: u64,
    pub block_links: u64,
    pub misc: u64,
    pub jemalloc: u64,
    pub unaccounted: u64,
    pub largest_anon: LargestAnon,
}

pub struct MemSampler {
    file: File,
    buf: String,
}

/// Identifies which sub-region accumulator an smaps region maps to.
#[derive(Clone, Copy)]
enum ActiveRegion {
    JitCode,
    OpDispatcher,
    Frontend,
    CpuBackend,
    Lookup,
    LookupL1,
    ThreadStates,
    BlockLinks,
    Misc,
    JeMalloc,
    Unaccounted,
}

impl MemSampler {
    /// Opens `/proc/{pid}/smaps` and keeps the fd open for repeated sampling.
    ///
    /// # Errors
    ///
    /// Returns an error if the smaps file cannot be opened.
    pub fn new(pid: i32) -> anyhow::Result<Self> {
        let path = format!("/proc/{pid}/smaps");
        let file = File::open(&path).with_context(|| format!("failed to open {path}"))?;
        Ok(Self {
            file,
            buf: String::with_capacity(256 * 1024),
        })
    }

    /// Reads and parses the full smaps file, returning a memory snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if seeking or reading fails.
    pub fn sample(&mut self) -> anyhow::Result<MemSnapshot> {
        self.buf.clear();
        self.file
            .seek(SeekFrom::Start(0))
            .context("failed to seek smaps")?;
        self.file
            .read_to_string(&mut self.buf)
            .context("failed to read smaps")?;

        Ok(parse_smaps(&self.buf))
    }
}

fn parse_smaps(content: &str) -> MemSnapshot {
    let mut snap = MemSnapshot::default();
    let mut active: Option<ActiveRegion> = None;
    let mut current_begin: u64 = 0;
    let mut current_end: u64 = 0;

    for line in content.lines() {
        // Region header lines look like:
        // 359519000-359918000 ---p 00000000 00:00 0    [anon:FEXMem]
        if line.contains("FEXMem") {
            if let Some((begin, end)) = parse_address_range(line) {
                current_begin = begin;
                current_end = end;
            }

            // Order matters: check more specific names before less specific ones.
            if line.contains("FEXMemJIT") {
                active = Some(ActiveRegion::JitCode);
            } else if line.contains("FEXMem_OpDispatcher") {
                active = Some(ActiveRegion::OpDispatcher);
            } else if line.contains("FEXMem_Frontend") {
                active = Some(ActiveRegion::Frontend);
            } else if line.contains("FEXMem_CPUBackend") {
                active = Some(ActiveRegion::CpuBackend);
            } else if line.contains("FEXMem_Lookup_L1") {
                active = Some(ActiveRegion::LookupL1);
            } else if line.contains("FEXMem_Lookup") {
                active = Some(ActiveRegion::Lookup);
            } else if line.contains("FEXMem_ThreadState") {
                active = Some(ActiveRegion::ThreadStates);
            } else if line.contains("FEXMem_BlockLinks") {
                active = Some(ActiveRegion::BlockLinks);
            } else if line.contains("FEXMem_Misc") {
                active = Some(ActiveRegion::Misc);
            } else {
                active = Some(ActiveRegion::Unaccounted);
            }
            continue;
        }

        if line.contains("JEMalloc") || line.contains("FEXAllocator") {
            active = Some(ActiveRegion::JeMalloc);
            if let Some((begin, end)) = parse_address_range(line) {
                current_begin = begin;
                current_end = end;
            }
            continue;
        }

        if line.contains("VmFlags") {
            active = None;
            continue;
        }

        if let Some(region) = active
            && let Some(rss_bytes) = parse_rss_line(line)
        {
            snap.total_anon += rss_bytes;
            let target = match region {
                ActiveRegion::JitCode => &mut snap.jit_code,
                ActiveRegion::OpDispatcher => &mut snap.op_dispatcher,
                ActiveRegion::Frontend => &mut snap.frontend,
                ActiveRegion::CpuBackend => &mut snap.cpu_backend,
                ActiveRegion::Lookup => &mut snap.lookup,
                ActiveRegion::LookupL1 => &mut snap.lookup_l1,
                ActiveRegion::ThreadStates => &mut snap.thread_states,
                ActiveRegion::BlockLinks => &mut snap.block_links,
                ActiveRegion::Misc => &mut snap.misc,
                ActiveRegion::JeMalloc => &mut snap.jemalloc,
                ActiveRegion::Unaccounted => &mut snap.unaccounted,
            };
            *target += rss_bytes;

            if matches!(region, ActiveRegion::JeMalloc) && rss_bytes > snap.largest_anon.size {
                snap.largest_anon = LargestAnon {
                    begin: current_begin,
                    end: current_end,
                    size: rss_bytes,
                };
            }
        }
    }

    snap
}

/// Parses an address range from the start of a mapping line.
/// Example: `359519000-359918000 ---p ...` -> Some((0x359519000, 0x359918000))
fn parse_address_range(line: &str) -> Option<(u64, u64)> {
    let addr_part = line.split_whitespace().next()?;
    let (begin_str, end_str) = addr_part.split_once('-')?;
    let begin = u64::from_str_radix(begin_str, 16).ok()?;
    let end = u64::from_str_radix(end_str, 16).ok()?;
    Some((begin, end))
}

/// Parses an `Rss:` line and returns the value in bytes.
/// Example: `Rss:                 560 kB` -> Some(573440)
fn parse_rss_line(line: &str) -> Option<u64> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("Rss:") {
        return None;
    }

    let value_part = trimmed.strip_prefix("Rss:")?;
    let mut parts = value_part.split_whitespace();
    let size_str = parts.next()?;
    let granule = parts.next()?;

    let size: u64 = size_str.parse().ok()?;

    if granule == "kB" {
        Some(size * 1024)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rss_line_valid() {
        assert_eq!(parse_rss_line("Rss:                 560 kB"), Some(573_440));
    }

    #[test]
    fn parse_rss_line_zero() {
        assert_eq!(parse_rss_line("Rss:                   0 kB"), Some(0));
    }

    #[test]
    fn parse_rss_line_not_rss() {
        assert_eq!(parse_rss_line("Pss:                 560 kB"), None);
    }

    #[test]
    fn parse_address_range_valid() {
        let line = "359519000-359918000 ---p 00000000 00:00 0                                [anon:FEXMem]";
        assert_eq!(
            parse_address_range(line),
            Some((0x3_5951_9000, 0x3_5991_8000))
        );
    }

    #[test]
    fn parse_smaps_basic() {
        let content = "\
359519000-359918000 ---p 00000000 00:00 0                                [anon:FEXMemJIT]
Size:               4096 kB
Rss:                 560 kB
Pss:                 560 kB
VmFlags: rd
400000000-400100000 ---p 00000000 00:00 0                                [anon:JEMalloc]
Size:               1024 kB
Rss:                 128 kB
Pss:                 128 kB
VmFlags: rd wr
";
        let snap = parse_smaps(content);
        assert_eq!(snap.jit_code, 560 * 1024);
        assert_eq!(snap.jemalloc, 128 * 1024);
        assert_eq!(snap.total_anon, (560 + 128) * 1024);
        assert_eq!(snap.largest_anon.size, 128 * 1024);
    }
}
