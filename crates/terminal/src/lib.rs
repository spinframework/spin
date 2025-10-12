//! A helper library for terminal output.
//!
//! This library is used by Spin to print out messages in an appropriate format
//! that is easy for users to read. This is not meant as a general purpose library.

use std::{io::IsTerminal, sync::OnceLock};
use termcolor::{ColorSpec, StandardStream, StandardStreamLock, WriteColor};

static COLOR_OUT: OnceLock<StandardStream> = OnceLock::new();
static COLOR_ERR: OnceLock<StandardStream> = OnceLock::new();

/// A wrapper around a standard stream lock that resets the color on drop
pub struct ColorText(StandardStreamLock<'static>);

impl ColorText {
    /// Create a `ColorText` tied to stdout
    pub fn stdout(spec: ColorSpec) -> ColorText {
        let stream =
            COLOR_OUT.get_or_init(|| StandardStream::stdout(color_choice(std::io::stdout())));
        set_color(stream, spec)
    }

    /// Create a `ColorText` tied to stderr
    pub fn stderr(spec: ColorSpec) -> ColorText {
        let stream =
            COLOR_ERR.get_or_init(|| StandardStream::stderr(color_choice(std::io::stderr())));
        set_color(stream, spec)
    }
}

impl std::io::Write for ColorText {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl WriteColor for ColorText {
    fn supports_color(&self) -> bool {
        self.0.supports_color()
    }

    fn set_color(&mut self, spec: &ColorSpec) -> std::io::Result<()> {
        self.0.set_color(spec)
    }

    fn reset(&mut self) -> std::io::Result<()> {
        self.0.reset()
    }
}

impl Drop for ColorText {
    fn drop(&mut self) {
        let _ = self.reset();
    }
}

fn set_color(stream: &'static StandardStream, spec: ColorSpec) -> ColorText {
    let mut lock = stream.lock();
    let _ = lock.set_color(&spec);
    ColorText(lock)
}

fn color_choice(stream: impl IsTerminal) -> termcolor::ColorChoice {
    if stream.is_terminal() {
        termcolor::ColorChoice::Auto
    } else {
        termcolor::ColorChoice::Never
    }
}

#[macro_export]
macro_rules! step {
    ($step:expr, $($arg:tt)*) => {{
        $crate::cprint!($crate::colors::bold_green(), $step);
        print!(" ");
        println!($($arg)*);
    }};
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{
        $crate::ceprint!($crate::colors::bold_yellow(), "Warning");
        eprint!(": ");
        eprintln!($($arg)*);
    }};
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {{
        $crate::ceprint!($crate::colors::bold_red(), "Error");
        eprint!(": ");
        eprintln!($($arg)*);
    }};
}

#[macro_export]
macro_rules! einfo {
    ($highlight:expr, $($arg:tt)*) => {{
        $crate::ceprint!($crate::colors::bold_cyan(), $highlight);
        eprint!(" ");
        eprintln!($($arg)*);
    }};
}

#[macro_export]
macro_rules! cprint {
    ($color:expr, $($arg:tt)*) => {{
        use std::io::Write;
        let mut out = $crate::ColorText::stdout($color);
        let _ = write!(out, $($arg)*);
        drop(out); // Reset colors
    }};
}

#[macro_export]
macro_rules! ceprint {
    ($color:expr, $($arg:tt)*) => {{
        use std::io::Write;
        let mut out = $crate::ColorText::stderr($color);
        let _ = write!(out, $($arg)*);
        drop(out); // Reset color}s
    }};
}

#[macro_export]
macro_rules! ceprintln {
    ($color:expr, $($arg:tt)*) => {{
        use std::io::Write;
        let mut out = $crate::ColorText::stderr($color);
        let _ = writeln!(out, $($arg)*);
        drop(out); // Reset colors
    }};
}

pub mod colors {
    use termcolor::{Color, ColorSpec};

    pub fn bold_red() -> ColorSpec {
        new(Color::Red, true)
    }

    pub fn bold_green() -> ColorSpec {
        new(Color::Green, true)
    }

    pub fn bold_cyan() -> ColorSpec {
        new(Color::Cyan, true)
    }

    pub fn bold_yellow() -> ColorSpec {
        new(Color::Yellow, true)
    }

    fn new(color: Color, bold: bool) -> ColorSpec {
        let mut s = ColorSpec::new();
        s.set_fg(Some(color)).set_bold(bold);
        s
    }
}
