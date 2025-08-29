//! Common CLI code and constants

use clap::builder::{styling::AnsiColor, Styles};

/// Clap [`Styles`] for Spin CLI and plugins.
pub const CLAP_STYLES: Styles = Styles::styled()
    .header(AnsiColor::Yellow.on_default())
    .usage(AnsiColor::Green.on_default())
    .literal(AnsiColor::Green.on_default())
    .placeholder(AnsiColor::Green.on_default());
