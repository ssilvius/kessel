//! CLI wrapper around `kessel::scpt::parse_and_decrypt`.
//!
//! Walks every SCPT entry in the provided `.tor` archives, decrypts the body,
//! and writes the plaintext to `<output_dir>/<numeric_id>.scpt.dec`. See the
//! `kessel::scpt` module docs for format details.

use anyhow::Result;
use kessel::myp::Archive;
use kessel::scpt;
use std::path::PathBuf;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut input_dir = PathBuf::from(".");
    let mut output_dir: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" => {
                input_dir = PathBuf::from(&args[i + 1]);
                i += 2;
            }
            "-o" => {
                output_dir = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            _ => i += 1,
        }
    }
    let output_dir = output_dir.unwrap_or_else(|| PathBuf::from("/tmp/scpt-decrypted"));
    std::fs::create_dir_all(&output_dir)?;

    let tor_files: Vec<_> = std::fs::read_dir(&input_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
        .collect();

    let mut total = 0usize;
    for tor_path in &tor_files {
        let mut archive = match Archive::open(tor_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let entries: Vec<_> = match archive.entries() {
            Ok(e) => e.cloned().collect(),
            Err(_) => continue,
        };
        for entry in &entries {
            if entry.compressed_size < 40 {
                continue;
            }
            let data = match archive.read_entry(entry) {
                Ok(d) => d,
                Err(_) => continue,
            };
            if data.len() < 4 || &data[..4] != b"SCPT" {
                continue;
            }
            let (header, plaintext) = match scpt::parse_and_decrypt(&data) {
                Ok(out) => out,
                Err(e) => {
                    eprintln!("warn: {} -- {e}", entry.filename_hash);
                    continue;
                }
            };
            let out_path = output_dir.join(format!("{}.scpt.dec", header.numeric_id));
            std::fs::write(&out_path, &plaintext)?;
            total += 1;
        }
    }
    println!(
        "decrypted {} SCPT files into {}",
        total,
        output_dir.display()
    );
    Ok(())
}
