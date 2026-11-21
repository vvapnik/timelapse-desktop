//! Output file names, matching `create_timelapse.sh`.

use std::path::{Path, PathBuf};

/// Container format of the output video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Mp4,
    Mov,
    Avi,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Mp4 => "mp4",
            OutputFormat::Mov => "mov",
            OutputFormat::Avi => "avi",
        }
    }
}

/// Default output files for `sequence_count` sequences found in `input_folder`:
/// `{folder}_timelapse.{ext}` for one sequence, otherwise
/// `{folder}_timelapse_part1.{ext}`, `…_part2.{ext}`, … — saved in `output_dir`
/// (the script uses the input folder).
pub fn default_output_paths(
    input_folder: &Path,
    output_dir: &Path,
    format: OutputFormat,
    sequence_count: usize,
) -> Vec<PathBuf> {
    let folder_name = input_folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "timelapse".to_owned());
    let ext = format.extension();
    match sequence_count {
        0 => Vec::new(),
        1 => vec![output_dir.join(format!("{folder_name}_timelapse.{ext}"))],
        n => (1..=n)
            .map(|i| output_dir.join(format!("{folder_name}_timelapse_part{i}.{ext}")))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_sequence_has_no_part_suffix() {
        let input = Path::new("/Volumes/SD/DCIM");
        assert_eq!(
            default_output_paths(input, input, OutputFormat::Mp4, 1),
            [PathBuf::from("/Volumes/SD/DCIM/DCIM_timelapse.mp4")]
        );
    }

    #[test]
    fn multiple_sequences_are_numbered_from_one() {
        let input = Path::new("/photos/garden");
        assert_eq!(
            default_output_paths(input, Path::new("/out"), OutputFormat::Mov, 3),
            [
                PathBuf::from("/out/garden_timelapse_part1.mov"),
                PathBuf::from("/out/garden_timelapse_part2.mov"),
                PathBuf::from("/out/garden_timelapse_part3.mov"),
            ]
        );
    }

    #[test]
    fn no_sequences_no_files() {
        let input = Path::new("/photos");
        assert!(default_output_paths(input, input, OutputFormat::Avi, 0).is_empty());
    }
}
