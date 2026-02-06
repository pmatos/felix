// SPDX-License-Identifier: MIT
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};

use super::format::{EOF_MARKER, FORMAT_VERSION, MAGIC};
use crate::datasource::{DataSource, SessionMetadata};
use crate::recording::format::{FileHeader, Frame};
use crate::sampler::accumulator::ComputedFrame;

pub struct RecordingReader {
    metadata: SessionMetadata,
    frames: Vec<Frame>,
}

impl RecordingReader {
    /// Opens a recording file, validates the header, and reads all frames.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened, the header is invalid,
    /// or frame data is corrupted.
    pub fn open(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .with_context(|| format!("failed to open recording file: {}", path.display()))?;
        let buf_reader = BufReader::new(file);
        let mut decoder =
            zstd::Decoder::new(buf_reader).context("failed to create zstd decoder")?;

        let header = Self::read_header(&mut decoder)?;

        if header.magic != MAGIC {
            bail!("invalid magic bytes in recording file");
        }
        if header.format_version != FORMAT_VERSION {
            bail!(
                "unsupported format version {} (expected {FORMAT_VERSION})",
                header.format_version
            );
        }

        let frames = Self::read_all_frames(&mut decoder)?;

        Ok(Self {
            metadata: header.metadata,
            frames,
        })
    }

    #[must_use]
    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    #[must_use]
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    #[must_use]
    pub fn frame_at(&self, index: usize) -> Option<&Frame> {
        self.frames.get(index)
    }

    fn read_header(reader: &mut impl Read) -> Result<FileHeader> {
        let mut len_buf = [0u8; 4];
        reader
            .read_exact(&mut len_buf)
            .context("failed to read header length")?;
        let len = u32::from_le_bytes(len_buf) as usize;

        let mut data = vec![0u8; len];
        reader
            .read_exact(&mut data)
            .context("failed to read header data")?;

        postcard::from_bytes(&data).context("failed to deserialize file header")
    }

    fn read_all_frames(reader: &mut impl Read) -> Result<Vec<Frame>> {
        let mut frames = Vec::new();
        let mut len_buf = [0u8; 4];

        loop {
            match reader.read_exact(&mut len_buf) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => return Err(e).context("failed to read frame length"),
            }

            if len_buf == EOF_MARKER {
                break;
            }

            let len = u32::from_le_bytes(len_buf) as usize;
            let mut data = vec![0u8; len];
            reader
                .read_exact(&mut data)
                .context("failed to read frame data")?;

            let frame: Frame =
                postcard::from_bytes(&data).context("failed to deserialize frame")?;
            frames.push(frame);
        }

        Ok(frames)
    }
}

pub struct ReplaySource {
    reader: RecordingReader,
    current_index: usize,
    playback_speed: f64,
    last_emitted: Instant,
    paused: bool,
}

impl ReplaySource {
    #[must_use]
    pub fn new(reader: RecordingReader) -> Self {
        Self {
            reader,
            current_index: 0,
            playback_speed: 1.0,
            last_emitted: Instant::now(),
            paused: false,
        }
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.playback_speed = speed;
    }

    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            self.last_emitted = Instant::now();
        }
    }

    pub fn seek_to(&mut self, index: usize) {
        self.current_index = index.min(self.reader.frame_count());
        self.last_emitted = Instant::now();
    }

    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    #[must_use]
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn total_frames(&self) -> usize {
        self.reader.frame_count()
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn is_finished(&self) -> bool {
        self.current_index >= self.reader.frame_count()
    }
}

impl DataSource for ReplaySource {
    fn next_frame(&mut self) -> Option<ComputedFrame> {
        if self.paused {
            return None;
        }

        let frame = self.reader.frame_at(self.current_index)?;

        let sample_period_ns = frame.computed.sample_period_ns;
        #[allow(clippy::cast_precision_loss)]
        let required_ns = sample_period_ns as f64 / self.playback_speed;
        let elapsed_ns = self.last_emitted.elapsed().as_nanos();

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        if elapsed_ns < required_ns as u128 {
            return None;
        }

        let computed = frame.computed.clone();
        self.current_index += 1;
        self.last_emitted = Instant::now();
        Some(computed)
    }

    fn metadata(&self) -> &SessionMetadata {
        self.reader.metadata()
    }

    fn is_live(&self) -> bool {
        false
    }
}
