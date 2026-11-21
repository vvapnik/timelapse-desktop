//! Splitting an ordered list of frames into separate sequences at long pauses.

use crate::scan::Frame;

/// Default pause between two shots that starts a new sequence (2 minutes),
/// same as `create_timelapse.sh`.
pub const DEFAULT_GAP_SECONDS: u64 = 120;

/// A run of frames without long pauses; rendered into its own video file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    pub frames: Vec<Frame>,
}

impl Sequence {
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Capture time of the first frame (ms since the Unix epoch).
    pub fn start_ms(&self) -> Option<i64> {
        self.frames.first().map(|f| f.time_ms)
    }

    /// Capture time of the last frame (ms since the Unix epoch).
    pub fn end_ms(&self) -> Option<i64> {
        self.frames.last().map(|f| f.time_ms)
    }
}

/// Splits ordered frames into sequences: a new sequence starts when the pause
/// between two consecutive frames is longer than `gap_seconds`.
///
/// Like the script, the pause is measured in whole seconds and a pause exactly
/// equal to the threshold does not split.
pub fn split_into_sequences(frames: Vec<Frame>, gap_seconds: u64) -> Vec<Sequence> {
    let gap = i64::try_from(gap_seconds).unwrap_or(i64::MAX);
    let mut sequences: Vec<Sequence> = Vec::new();
    let mut current: Vec<Frame> = Vec::new();
    let mut last_secs: Option<i64> = None;

    for frame in frames {
        let secs = frame.time_ms.div_euclid(1000);
        if let Some(last) = last_secs
            && secs.saturating_sub(last) > gap
        {
            sequences.push(Sequence {
                frames: std::mem::take(&mut current),
            });
        }
        last_secs = Some(secs);
        current.push(frame);
    }
    if !current.is_empty() {
        sequences.push(Sequence { frames: current });
    }
    sequences
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::TimeSource;
    use std::path::PathBuf;

    fn frames(times_secs: &[i64]) -> Vec<Frame> {
        times_secs
            .iter()
            .enumerate()
            .map(|(i, &t)| Frame {
                path: PathBuf::from(format!("IMG_{i:04}.JPG")),
                time_ms: t * 1000,
                time_source: TimeSource::FileModified,
            })
            .collect()
    }

    fn lengths(seqs: &[Sequence]) -> Vec<usize> {
        seqs.iter().map(Sequence::len).collect()
    }

    #[test]
    fn no_frames_no_sequences() {
        assert!(split_into_sequences(Vec::new(), 120).is_empty());
    }

    #[test]
    fn single_frame_is_one_sequence() {
        assert_eq!(lengths(&split_into_sequences(frames(&[0]), 120)), [1]);
    }

    #[test]
    fn regular_interval_is_one_sequence() {
        let seqs = split_into_sequences(frames(&[0, 60, 120, 180]), 120);
        assert_eq!(lengths(&seqs), [4]);
    }

    #[test]
    fn splits_on_gap_longer_than_threshold() {
        let seqs = split_into_sequences(frames(&[0, 10, 20, 500, 510, 2000]), 120);
        assert_eq!(lengths(&seqs), [3, 2, 1]);
        assert_eq!(seqs[1].start_ms(), Some(500_000));
        assert_eq!(seqs[1].end_ms(), Some(510_000));
    }

    #[test]
    fn gap_equal_to_threshold_does_not_split() {
        assert_eq!(lengths(&split_into_sequences(frames(&[0, 120]), 120)), [2]);
        assert_eq!(
            lengths(&split_into_sequences(frames(&[0, 121]), 120)),
            [1, 1]
        );
    }

    #[test]
    fn gap_is_measured_in_whole_seconds() {
        let mut f = frames(&[0, 0]);
        f[0].time_ms = 999; // second 0
        f[1].time_ms = 121_000; // second 121 -> 121 > 120
        assert_eq!(lengths(&split_into_sequences(f, 120)), [1, 1]);

        let mut f = frames(&[0, 0]);
        f[0].time_ms = 1_000; // second 1
        f[1].time_ms = 121_999; // second 121 -> 120, no split
        assert_eq!(lengths(&split_into_sequences(f, 120)), [2]);
    }

    #[test]
    fn custom_threshold() {
        let times = [0, 100, 200, 600];
        assert_eq!(lengths(&split_into_sequences(frames(&times), 300)), [3, 1]);
        assert_eq!(
            lengths(&split_into_sequences(frames(&times), 50)),
            [1, 1, 1, 1]
        );
    }
}
