//! Crash log: what a panic leaves behind.
//!
//! The shell installs a panic hook that hands every panic here. The report is
//! one plain-text block appended to `crash.log` in the logs directory, capped
//! so a panic loop cannot fill the disk. Nothing is sent anywhere; the file
//! stays on the device until the user chooses to share it.

use std::io::Write;
use std::path::Path;

/// Past this size the file is truncated before the next report is appended.
pub const MAX_BYTES: u64 = 256 * 1024;

/// One report block. Pure, so it can be tested without a panic.
pub fn format_report(version: &str, when: &str, location: &str, message: &str) -> String {
    format!(
        "--- panic ---\nversion: {version}\ntime: {when}\nat: {location}\nmessage: {message}\n\n"
    )
}

/// Append a report, truncating first when the file has outgrown [`MAX_BYTES`].
/// Best effort: a crash handler that can itself fail loudly helps nobody.
pub fn append(path: &Path, report: &str) -> std::io::Result<()> {
    let too_big = std::fs::metadata(path)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false);

    let mut options = std::fs::OpenOptions::new();

    options.create(true);

    if too_big {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }

    options.open(path)?.write_all(report.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_names_version_location_and_message() {
        let r = format_report("1.2.3", "t", "src/x.rs:1:2", "boom");

        assert!(r.contains("version: 1.2.3") && r.contains("src/x.rs:1:2") && r.contains("boom"));
    }

    #[test]
    fn reports_append_and_an_oversized_file_is_truncated() {
        let dir = std::env::temp_dir().join(format!("gs-crash-{}", std::process::id()));

        std::fs::create_dir_all(&dir).unwrap();

        let file = dir.join("crash.log");

        append(&file, "a\n").unwrap();
        append(&file, "b\n").unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "a\nb\n");

        std::fs::write(&file, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        append(&file, "fresh\n").unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "fresh\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
