use anyhow::Result;
use kessel::hash::HashDictionary;
use kessel::myp::Archive;
use std::path::PathBuf;
fn main() -> Result<()> {
    let mut hd = HashDictionary::new();
    hd.load(&PathBuf::from(
        "/Users/seansilvius/.cache/kessel/hashes_filename.txt",
    ))?;
    let hash = hd
        .paths_matching("/resources/systemgenerated/client.gom")
        .into_iter()
        .map(|(h, _)| h)
        .next()
        .expect("client.gom not in dict");
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
            Ok(e) => e.cloned().collect(),
            Err(_) => continue,
        };
        for entry in entries {
            if entry.filename_hash == hash {
                let data = a.read_entry(&entry)?;
                std::fs::write("/tmp/client.gom.bin", &data)?;
                eprintln!("wrote /tmp/client.gom.bin ({} bytes)", data.len());
                return Ok(());
            }
        }
    }
    Ok(())
}
