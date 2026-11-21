//! Prints the sequences the core finds in a folder.
//!
//! Usage: cargo run -p timelapse-core --example sequences -- <folder> [gap_seconds]

use std::path::PathBuf;

use timelapse_core::{DEFAULT_GAP_SECONDS, scan_folder, split_into_sequences};

fn main() {
    let mut args = std::env::args().skip(1);
    let folder = PathBuf::from(
        args.next()
            .expect("usage: sequences <folder> [gap_seconds]"),
    );
    let gap = args
        .next()
        .map(|g| g.parse().expect("gap must be a number of seconds"))
        .unwrap_or(DEFAULT_GAP_SECONDS);

    let scan = match scan_folder(&folder) {
        Ok(scan) => scan,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    };
    let sequences = split_into_sequences(scan.frames, gap);
    println!("sequences: {}", sequences.len());
    for (i, seq) in sequences.iter().enumerate() {
        println!("sequence {}: {} frames", i + 1, seq.len());
        for frame in &seq.frames {
            println!("  {}", frame.path.display());
        }
    }
}
