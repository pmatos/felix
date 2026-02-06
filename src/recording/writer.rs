// SPDX-License-Identifier: MIT
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};

use super::format::{EOF_MARKER, FORMAT_VERSION, MAGIC};
use crate::datasource::SessionMetadata;
use crate::recording::format::{FileHeader, Frame};

pub struct RecordingWriter {
    encoder: zstd::Encoder<'static, BufWriter<File>>,
}

impl RecordingWriter {
    /// Creates a new recording file at `path` and writes the file header.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be created or the header cannot be written.
    pub fn create(path: &Path, metadata: &SessionMetadata) -> Result<Self> {
        let file = File::create(path)
            .with_context(|| format!("failed to create recording file: {}", path.display()))?;
        let buf_writer = BufWriter::new(file);
        let mut encoder =
            zstd::Encoder::new(buf_writer, 3).context("failed to create zstd encoder")?;

        let header = FileHeader {
            magic: MAGIC,
            format_version: FORMAT_VERSION,
            metadata: metadata.clone(),
        };

        let serialized = postcard::to_stdvec(&header).context("failed to serialize file header")?;

        #[allow(clippy::cast_possible_truncation)]
        let len = serialized.len() as u32;
        encoder
            .write_all(&len.to_le_bytes())
            .context("failed to write header length")?;
        encoder
            .write_all(&serialized)
            .context("failed to write header data")?;

        Ok(Self { encoder })
    }

    /// Writes a single frame to the recording.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization or writing fails.
    pub fn write_frame(&mut self, frame: &Frame) -> Result<()> {
        let serialized = postcard::to_stdvec(frame).context("failed to serialize frame")?;

        #[allow(clippy::cast_possible_truncation)]
        let len = serialized.len() as u32;
        self.encoder
            .write_all(&len.to_le_bytes())
            .context("failed to write frame length")?;
        self.encoder
            .write_all(&serialized)
            .context("failed to write frame data")?;

        Ok(())
    }

    /// Writes the EOF marker, finishes compression, and flushes the file.
    ///
    /// # Errors
    ///
    /// Returns an error if writing or flushing fails.
    pub fn finish(mut self) -> Result<()> {
        self.encoder
            .write_all(&EOF_MARKER)
            .context("failed to write EOF marker")?;
        let mut buf_writer = self
            .encoder
            .finish()
            .context("failed to finish zstd encoder")?;
        buf_writer
            .flush()
            .context("failed to flush recording file")?;
        Ok(())
    }
}
