//! CLI subcommand implementations. Each module owns one command and
//! returns `anyhow::Result<()>` so errors bubble up to `main` for unified
//! reporting.

pub mod deploy;
pub mod keys;
pub mod logs;
pub mod status;
pub mod validate;

/// Output mode shared by every command. Selected once via the global
/// `--json` flag in `main` and threaded into every command function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
}
