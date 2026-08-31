// SPDX-License-Identifier: AGPL-3.0-only
#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

fn main() {
    println!("cargo:rerun-if-changed=src/service");

    let mut files = Vec::new();
    collect(Path::new("src/service"), &mut files);
    files.sort();

    let mut hash = OFFSET;
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
        let relative = file
            .strip_prefix("src/service")
            .expect("collected paths live under src/service");
        update(&mut hash, relative.to_string_lossy().as_bytes());
        update(
            &mut hash,
            &fs::read(&file).expect("service definition is readable"),
        );
    }

    println!("cargo:rustc-env=API_HASH={hash:016x}");
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn update(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(PRIME);
    }
}
