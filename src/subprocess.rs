use std::os::unix::process::ExitStatusExt as _;

/// An error representing a subprocess that errored
///
/// This can be used to propogate a subprocesses exit status.
/// When this error is encountered the cli will exit with the status code
/// instead of printing an error,
#[derive(Debug)]
pub struct ExitStatusError {
    status: Option<i32>,
}

impl ExitStatusError {
    #[cfg(windows)]
    pub(crate) fn new(status: std::process::ExitStatus) -> Self {
        Self {
            status: status.code(),
        }
    }

    #[cfg(unix)]
    pub(crate) fn new(status: std::process::ExitStatus) -> Self {
        Self {
            status: status.code().or_else(|| status.signal()),
        }
    }

    pub fn code(&self) -> i32 {
        self.status.unwrap_or(1)
    }
}

impl std::error::Error for ExitStatusError {}

impl std::fmt::Display for ExitStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = write!(f, "subprocess exited with status: ");
        if let Some(status) = self.status {
            writeln!(f, "{status}")
        } else {
            writeln!(f, "unknown")
        }
    }
}
