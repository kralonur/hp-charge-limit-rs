//! The running limiter's next check, readable by an unprivileged `status`.

use crate::paths;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(super) fn publish(interval: Duration) {
    if let Some(deadline) = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| elapsed.checked_add(interval))
        .and_then(|deadline| u64::try_from(deadline.as_millis()).ok())
    {
        let _ = write(&paths::at(paths::NEXT_CHECK), std::process::id(), deadline);
    }
}

pub(super) fn clear() {
    let path = paths::at(paths::NEXT_CHECK);
    if read(&path, std::process::id()).is_some() {
        let _ = fs::remove_file(path);
    }
}

pub(crate) fn next_check_unix_ms(pid: u32) -> Option<u64> {
    read(&paths::at(paths::NEXT_CHECK), pid)
}

fn write(path: &Path, pid: u32, deadline: u64) -> io::Result<()> {
    // Publish the PID and deadline together so readers never see a partial update.
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().expect("schedule has a directory"))?;
    writeln!(file, "{pid} {deadline}")?;
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    file.persist(path)?;
    Ok(())
}

fn read(path: &Path, pid: u32) -> Option<u64> {
    let line = fs::read_to_string(path).ok()?;
    let (owner, deadline) = line.trim().split_once(' ')?;
    if owner.parse::<u32>().ok()? != pid {
        return None;
    }
    deadline.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_is_only_reported_for_the_running_process() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("next-check");
        assert_eq!(read(&path, 12), None);
        write(&path, 12, 1_000).unwrap();
        assert_eq!(read(&path, 12), Some(1_000));
        assert_eq!(read(&path, 13), None);
        write(&path, 13, 2_000).unwrap();
        assert_eq!(read(&path, 12), None);
        assert_eq!(read(&path, 13), Some(2_000));
        fs::write(&path, "13 partial").unwrap();
        assert_eq!(read(&path, 13), None);
    }
}
