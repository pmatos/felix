// SPDX-License-Identifier: MIT
pub mod format;
pub mod reader;
pub mod writer;

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use crate::datasource::SessionMetadata;
    use crate::fex::smaps::MemSnapshot;
    use crate::fex::types::AppType;
    use crate::recording::format::Frame;
    use crate::recording::reader::RecordingReader;
    use crate::recording::writer::RecordingWriter;
    use crate::sampler::accumulator::{ComputedFrame, HistogramEntry, ThreadLoad};
    use crate::sampler::thread_stats::ThreadDelta;

    fn make_metadata() -> SessionMetadata {
        SessionMetadata {
            pid: 1234,
            fex_version: "FEX-2501".to_string(),
            app_type: AppType::Linux64,
            stats_version: 3,
            cycle_counter_frequency: 1_000_000_000,
            hardware_concurrency: 8,
            recording_start: SystemTime::UNIX_EPOCH,
        }
    }

    fn make_frame(index: u64) -> Frame {
        Frame {
            computed: ComputedFrame {
                timestamp_ns: index * 1_000_000_000,
                sample_period_ns: 500_000_000,
                threads_sampled: 2,
                total_jit_time: 100 + index,
                total_signal_time: 50 + index,
                total_sigbus_count: index,
                total_smc_count: 0,
                total_float_fallback_count: 0,
                total_cache_miss_count: 10,
                total_cache_read_lock_time: 20,
                total_cache_write_lock_time: 30,
                total_jit_count: 40 + index,
                total_jit_invocations: 200 + index,
                fex_load_percent: 12.5,
                thread_loads: vec![
                    ThreadLoad {
                        tid: 1,
                        load_percent: 8.0,
                        total_cycles: 80_000,
                    },
                    ThreadLoad {
                        tid: 2,
                        load_percent: 4.5,
                        total_cycles: 45_000,
                    },
                ],
                mem: MemSnapshot::default(),
                histogram_entry: HistogramEntry {
                    load_percent: 12.5,
                    high_jit_load: false,
                    high_invalidation_or_smc: false,
                    high_sigbus: false,
                    high_softfloat: false,
                },
            },
            per_thread_deltas: vec![
                ThreadDelta {
                    tid: 1,
                    jit_time: 70 + index,
                    signal_time: 30 + index,
                    sigbus_count: index,
                    ..ThreadDelta::default()
                },
                ThreadDelta {
                    tid: 2,
                    jit_time: 30,
                    signal_time: 20,
                    ..ThreadDelta::default()
                },
            ],
        }
    }

    #[test]
    fn round_trip_write_then_read() {
        let dir = std::env::temp_dir().join("felix_recording_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_recording.felixr");

        let metadata = make_metadata();
        let frames: Vec<Frame> = (0..5).map(make_frame).collect();

        {
            let mut writer = RecordingWriter::create(&path, &metadata).unwrap();
            for frame in &frames {
                writer.write_frame(frame).unwrap();
            }
            writer.finish().unwrap();
        }

        let reader = RecordingReader::open(&path).unwrap();

        assert_eq!(reader.metadata().pid, metadata.pid);
        assert_eq!(reader.metadata().fex_version, metadata.fex_version);
        assert_eq!(reader.metadata().stats_version, metadata.stats_version);
        assert_eq!(
            reader.metadata().cycle_counter_frequency,
            metadata.cycle_counter_frequency
        );
        assert_eq!(
            reader.metadata().hardware_concurrency,
            metadata.hardware_concurrency
        );

        assert_eq!(reader.frame_count(), 5);

        for (i, expected) in frames.iter().enumerate() {
            let actual = reader.frame_at(i).expect("frame should exist");

            assert_eq!(actual.computed.timestamp_ns, expected.computed.timestamp_ns);
            assert_eq!(
                actual.computed.sample_period_ns,
                expected.computed.sample_period_ns
            );
            assert_eq!(
                actual.computed.threads_sampled,
                expected.computed.threads_sampled
            );
            assert_eq!(
                actual.computed.total_jit_time,
                expected.computed.total_jit_time
            );
            assert_eq!(
                actual.computed.total_signal_time,
                expected.computed.total_signal_time
            );
            assert_eq!(
                actual.computed.total_sigbus_count,
                expected.computed.total_sigbus_count
            );
            assert_eq!(
                actual.computed.total_jit_count,
                expected.computed.total_jit_count
            );
            assert_eq!(
                actual.computed.total_jit_invocations,
                expected.computed.total_jit_invocations
            );
            assert!(
                (actual.computed.fex_load_percent - expected.computed.fex_load_percent).abs()
                    < f64::EPSILON
            );

            assert_eq!(
                actual.computed.thread_loads.len(),
                expected.computed.thread_loads.len()
            );
            for (at, et) in actual
                .computed
                .thread_loads
                .iter()
                .zip(&expected.computed.thread_loads)
            {
                assert_eq!(at.tid, et.tid);
                assert!((at.load_percent - et.load_percent).abs() < f32::EPSILON);
                assert_eq!(at.total_cycles, et.total_cycles);
            }

            assert_eq!(
                actual.per_thread_deltas.len(),
                expected.per_thread_deltas.len()
            );
            for (ad, ed) in actual
                .per_thread_deltas
                .iter()
                .zip(&expected.per_thread_deltas)
            {
                assert_eq!(ad.tid, ed.tid);
                assert_eq!(ad.jit_time, ed.jit_time);
                assert_eq!(ad.signal_time, ed.signal_time);
                assert_eq!(ad.sigbus_count, ed.sigbus_count);
            }
        }

        assert!(reader.frame_at(5).is_none());

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn empty_recording_round_trip() {
        let dir = std::env::temp_dir().join("felix_recording_test_empty");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty_recording.felixr");

        let metadata = make_metadata();

        {
            let writer = RecordingWriter::create(&path, &metadata).unwrap();
            writer.finish().unwrap();
        }

        let reader = RecordingReader::open(&path).unwrap();
        assert_eq!(reader.frame_count(), 0);
        assert!(reader.frame_at(0).is_none());

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }
}
