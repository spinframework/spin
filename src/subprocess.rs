/// An error representing a subprocess that errored
///
/// This can be used to propogate a subprocesses exit status.
/// When this error is encountered the cli will exit with the status code
/// instead of printing an error,
#[derive(Debug)]
pub enum ExitStatusError {
    ExitCode(i32),
    Signal(i32),
    Unknown,
}

impl ExitStatusError {
    #[cfg(unix)]
    pub(crate) fn new(status: std::process::ExitStatus) -> Self {
        use std::os::unix::process::ExitStatusExt as _;

        if let Some(code) = status.code() {
            Self::ExitCode(code)
        } else if let Some(signal) = status.signal() {
            Self::Signal(signal)
        } else {
            Self::Unknown
        }
    }

    #[cfg(not(unix))]
    pub(crate) fn new(status: std::process::ExitStatus) -> Self {
        if let Some(code) = status.code() {
            Self::ExitCode(code)
        } else {
            Self::Unknown
        }
    }

    pub fn code(&self) -> i32 {
        match self {
            Self::ExitCode(code) => *code,
            Self::Signal(signal) => 128 + *signal,
            Self::Unknown => 1,
        }
        .min(255)
    }
}

impl std::error::Error for ExitStatusError {}

impl std::fmt::Display for ExitStatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = write!(f, "subprocess exited with status: ");

        match self {
            Self::ExitCode(code) => writeln!(f, "{code}"),
            Self::Signal(signal) => writeln!(f, "signal {signal}"),
            Self::Unknown => writeln!(f, "unknown"),
        }
    }
}
