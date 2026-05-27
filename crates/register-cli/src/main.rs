//! Entry point for the `register` CLI binary.

#![forbid(unsafe_code)]
#![allow(
    clippy::redundant_pub_crate,
    reason = "pub(crate) は意図表現。unreachable_pub と redundant_pub_crate の衝突は前者を優先"
)]

use anyhow::Result;
use clap::Parser;
use mimalloc::MiMalloc;

/// `mimalloc` as the global allocator. The pipeline runs many small Vec
/// allocations per thread (per-page output buffers, packed PBM rows, etc.)
/// in parallel; mimalloc's per-thread heaps avoid the contention that
/// shows up under glibc's `malloc` on this kind of fan-out workload.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod cli;
mod logging;
mod runner;

fn main() -> Result<()> {
    logging::init();
    let args = cli::Args::parse();
    runner::run(&args)
}
