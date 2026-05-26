//! Save the PINF bytes to /tmp/pinf.bin for analysis.
use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::path::PathBuf;

fn main() -> Result<()> {
    let mut hd = HashDictionary::new();
    hd.load(&PathBuf::from(
        "/Users/seansilvius/.cache/kessel/hashes_filename.txt",
    ))?;
    let pinf_hash = hd
        .paths_matching("/resources/systemgenerated/prototypes.info")
        .into_iter()
        .map(|(h, _)| h)
        .next()
        .unwrap();
    for tor_path in std::fs::read_dir(&PathBuf::from("/Users/seansilvius/swtor/Assets"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "tor").unwrap_or(false))
    {
        let mut a = match Archive::open(&tor_path) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let entries: Vec<_> = match a.entries() {
            Ok(it) => it.cloned().collect(),
            Err(_) => continue,
        };
        for e in entries {
            if e.filename_hash == pinf_hash {
                let d = a.read_entry(&e)?;
                std::fs::write("/tmp/pinf.bin", &d)?;
                eprintln!("wrote /tmp/pinf.bin ({} bytes)", d.len());
                return Ok(());
            }
        }
    }
    Ok(())
}
