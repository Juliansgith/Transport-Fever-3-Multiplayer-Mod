//! Rebuilds a retained snapshot's file from a chunk store:
//! `cargo run -p tpf3mp-snapshot --example export_root -- <store dir> <manifest id> <dest file>`.
//! Open a copy of a store, not one a running agent or server holds.
use std::path::Path;

use tpf3mp_snapshot::{ChunkStore, Manifest, StoreConfig};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: export_root <store dir> <manifest id> <dest file>");
        std::process::exit(2);
    }
    let dir = Path::new(&args[1]);
    let bytes = std::fs::read(dir.join("roots").join(&args[2])).expect("read the root");
    let manifest = Manifest::from_bytes(&bytes).expect("a manifest");
    let config = StoreConfig {
        max_bytes: u64::MAX / 2,
        compression_level: 3,
        sync_chunks: false,
        recompress_received: false,
    };
    let store = ChunkStore::open(dir, config).expect("open the store");
    store
        .assemble(&manifest, Path::new(&args[3]))
        .expect("assemble");
    println!("{} bytes to {}", manifest.total_size(), args[3]);
}
