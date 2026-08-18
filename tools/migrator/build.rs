//! Make cargo rebuild this crate when a migration changes.
//!
//! `sqlx::migrate!` reads the migrations directory at COMPILE time and bakes
//! the SQL into the binary. Cargo does not know that, so without this script
//! adding `migrations/0050_foo.sql` leaves the previously-built
//! `atlas-migrate` in place and running it reports "database is up to date;
//! nothing to apply" — the migration is simply not in the binary.
//!
//! That failure is silent and looks exactly like success, which is the same
//! shape as the initdb-mount problem this tool was built to replace. One
//! build script is a cheap price for never debugging it again.
//!
//! Both the directory and each file are declared: the directory's mtime
//! changes when a file is added or removed, but not when an existing one is
//! edited.

use std::path::Path;

fn main() {
    let dir = Path::new("../../migrations");
    println!("cargo:rerun-if-changed={}", dir.display());

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        // Don't fail the build here. If the directory is genuinely missing,
        // `sqlx::migrate!` produces a far better error than this script can.
        Err(e) => {
            println!("cargo:warning=cannot read {}: {e}", dir.display());
            return;
        }
    };

    for entry in entries.flatten() {
        println!("cargo:rerun-if-changed={}", entry.path().display());
    }
}
