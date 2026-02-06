// SPDX-License-Identifier: MIT
use std::num::NonZeroUsize;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::{self, NonNull};

use anyhow::{Context, bail};
use nix::fcntl::OFlag;
use nix::sys::mman::{self, MapFlags, ProtFlags};
use nix::sys::stat::Mode;

use super::types::{AppType, ThreadStats, ThreadStatsHeader};

#[derive(Debug, Clone)]
pub struct HeaderSnapshot {
    pub version: u8,
    pub app_type: AppType,
    #[allow(dead_code)]
    pub thread_stats_size: u16,
    pub fex_version: String,
    pub head: u32,
    pub size: u32,
}

pub struct ShmReader {
    base: NonNull<u8>,
    fd: OwnedFd,
    size: usize,
}

// SAFETY: The mapped memory is read-only and only accessed through volatile reads.
unsafe impl Send for ShmReader {}

impl ShmReader {
    /// Opens the FEX shared memory segment for the given PID.
    ///
    /// # Errors
    ///
    /// Returns an error if the shared memory segment cannot be opened, is too
    /// small, or cannot be memory-mapped.
    pub fn open(pid: i32) -> anyhow::Result<Self> {
        let shm_name = format!("/fex-{pid}-stats");

        let fd = mman::shm_open(shm_name.as_str(), OFlag::O_RDONLY, Mode::empty())
            .with_context(|| format!("failed to open shared memory {shm_name}"))?;

        let stat =
            nix::sys::stat::fstat(fd.as_raw_fd()).context("failed to fstat shared memory")?;

        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        // st_size is non-negative for valid fds, and we target 64-bit only
        let file_size = stat.st_size as usize;
        let min_size = std::mem::size_of::<ThreadStatsHeader>();
        if file_size < min_size {
            bail!("shared memory too small: {file_size} bytes (minimum {min_size})");
        }

        let map_len = NonZeroUsize::new(file_size).context("shared memory has zero size")?;

        // SAFETY: We have a valid fd, PROT_READ only, MAP_SHARED is appropriate
        // for reading another process's shared memory.
        let mapped: NonNull<std::ffi::c_void> = unsafe {
            mman::mmap(
                None,
                map_len,
                ProtFlags::PROT_READ,
                MapFlags::MAP_SHARED,
                &fd,
                0,
            )
            .context("failed to mmap shared memory")?
        };

        let base = mapped.cast::<u8>();

        Ok(Self {
            base,
            fd,
            size: file_size,
        })
    }

    /// Reads the shared memory header using volatile reads.
    ///
    /// # Panics
    ///
    /// Panics if the mapped region is smaller than `ThreadStatsHeader`. This
    /// cannot happen because `open` validates the minimum size.
    #[must_use]
    pub fn read_header(&self) -> HeaderSnapshot {
        assert!(self.size >= std::mem::size_of::<ThreadStatsHeader>());

        // SAFETY: We validated that the mapping is at least as large as
        // ThreadStatsHeader. The pointer is aligned because mmap returns
        // page-aligned addresses. We use read_volatile because the other
        // process may update these fields concurrently.
        #[allow(clippy::cast_ptr_alignment)] // mmap guarantees page alignment
        let raw = unsafe { ptr::read_volatile(self.base.as_ptr().cast::<ThreadStatsHeader>()) };

        let version_len = raw
            .fex_version
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(raw.fex_version.len());
        let fex_version = String::from_utf8_lossy(&raw.fex_version[..version_len]).into_owned();

        let app_type = AppType::from_u8(raw.app_type).unwrap_or(AppType::Linux64);

        HeaderSnapshot {
            version: raw.version,
            app_type,
            thread_stats_size: raw.thread_stats_size,
            fex_version,
            head: raw.head,
            size: raw.size,
        }
    }

    /// Walks the linked list of thread stats from the header and returns
    /// a snapshot of all thread stats entries.
    #[must_use]
    pub fn read_thread_stats(&self) -> Vec<ThreadStats> {
        let header = self.read_header();
        let mut result = Vec::new();
        let mut offset = header.head;

        while offset != 0 {
            let offset_usize = offset as usize;
            if offset_usize + std::mem::size_of::<ThreadStats>() > self.size {
                break;
            }

            // SAFETY: We just bounds-checked that offset + sizeof(ThreadStats)
            // fits within the mapped region. ThreadStats is repr(C, align(16))
            // and shm offsets from FEX are always 16-byte aligned.
            let stats = unsafe {
                let src = self.base.as_ptr().add(offset_usize);
                volatile_copy_thread_stats(src)
            };

            offset = stats.next;
            result.push(stats);
        }

        result
    }

    /// Re-checks the shared memory size and remaps if it has grown.
    ///
    /// # Errors
    ///
    /// Returns an error if the remap fails.
    pub fn check_resize(&mut self) -> anyhow::Result<()> {
        let header = self.read_header();

        let new_size = header.size as usize;
        if new_size == self.size || new_size == 0 {
            return Ok(());
        }

        // SAFETY: self.base was obtained from mmap and self.size is the
        // correct mapped length.
        unsafe {
            mman::munmap(self.base.cast::<std::ffi::c_void>(), self.size)
                .context("failed to munmap during resize")?;
        }

        let map_len = NonZeroUsize::new(new_size).context("new size is zero")?;

        // SAFETY: Valid fd, read-only mapping, shared.
        let mapped: NonNull<std::ffi::c_void> = unsafe {
            mman::mmap(
                None,
                map_len,
                ProtFlags::PROT_READ,
                MapFlags::MAP_SHARED,
                &self.fd,
                0,
            )
            .context("failed to remap shared memory")?
        };

        self.base = mapped.cast::<u8>();
        self.size = new_size;

        Ok(())
    }
}

impl Drop for ShmReader {
    fn drop(&mut self) {
        if self.size > 0 {
            // SAFETY: self.base was obtained from mmap with self.size length.
            // The fd is closed automatically by OwnedFd's Drop.
            let _ = unsafe { mman::munmap(self.base.cast::<std::ffi::c_void>(), self.size) };
        }
    }
}

/// Performs a volatile copy of a `ThreadStats` struct using naturally-aligned
/// chunk reads to take advantage of single-copy atomicity guarantees.
///
/// # Safety
///
/// `src` must point to a valid, readable memory region of at least
/// `size_of::<ThreadStats>()` bytes. The pointer must be 16-byte aligned.
unsafe fn volatile_copy_thread_stats(src: *const u8) -> ThreadStats {
    let mut dest = ThreadStats::default();

    #[cfg(target_arch = "aarch64")]
    {
        // ARMv8.4 guarantees single-copy atomicity for 128-bit aligned loads.
        let chunks = std::mem::size_of::<ThreadStats>() / std::mem::size_of::<u128>();
        #[allow(clippy::cast_ptr_alignment)] // caller guarantees 16-byte alignment
        let s = src.cast::<u128>();
        let d = ptr::from_mut(&mut dest).cast::<u128>();
        for i in 0..chunks {
            // SAFETY: Caller guarantees src is valid and aligned. d points to
            // our local dest which is also properly aligned.
            unsafe {
                ptr::write_volatile(d.add(i), ptr::read_volatile(s.add(i)));
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    {
        let chunks = std::mem::size_of::<ThreadStats>() / std::mem::size_of::<u64>();
        let s = src.cast::<u64>();
        let d = ptr::from_mut(&mut dest).cast::<u64>();
        for i in 0..chunks {
            // SAFETY: Caller guarantees src is valid and aligned. d points to
            // our local dest which is also properly aligned.
            unsafe {
                ptr::write_volatile(d.add(i), ptr::read_volatile(s.add(i)));
            }
        }
    }

    dest
}
