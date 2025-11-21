//! Ouli CLI

use std::path::{Path, PathBuf};
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Ouli v{}", env!("CARGO_PKG_VERSION"));
        eprintln!();
        eprintln!("Usage: ouli <command> [options]");
        eprintln!();
        eprintln!("Commands:");
        eprintln!("  record    Start recording proxy");
        eprintln!("  replay    Start replay proxy");
        eprintln!("  stats     Show recording statistics");
        eprintln!();
        eprintln!("For more information, see: https://github.com/copyleftdev/ouli");
        process::exit(1);
    }

    let command = &args[1];

    match command.as_str() {
        "record" | "replay" => {
            eprintln!("Milestone 1: Core infrastructure implemented!");
            eprintln!("Network layer and engines coming in Milestones 2-4.");
            eprintln!();
            eprintln!("Current capabilities:");
            eprintln!("  ✓ Binary storage format with mmap");
            eprintln!("  ✓ Request fingerprinting (SHA-256)");
            eprintln!("  ✓ Configuration system");
            eprintln!();
            eprintln!("Run tests with: cargo test");
        }
        "stats" => {
            if args.len() < 3 {
                eprintln!("Usage: ouli stats <recording-dir>");
                process::exit(1);
            }

            let dir = PathBuf::from(&args[2]);
            show_stats(&dir);
        }
        _ => {
            eprintln!("Unknown command: {command}");
            eprintln!("Run 'ouli' for usage information.");
            process::exit(1);
        }
    }
}

fn show_stats(dir: &Path) {
    if !dir.exists() {
        eprintln!("Directory not found: {}", dir.display());
        process::exit(1);
    }

    println!("Recording directory: {}", dir.display());
    println!();
    println!("Stats functionality coming in Milestone 6 (Testing).");
    println!("For now, you can inspect recordings with hexdump or similar tools.");
}
