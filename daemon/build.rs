/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
#![allow(clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .expect("Failed to execute command");

    let git_hash = String::from_utf8(output.stdout).expect("Unable to read git commit");

    println!("cargo:rustc-env=GIT_HASH={git_hash}");

    if let Ok(env) = std::env::var("DAEMON_DATABASE_URL") {
        println!("cargo:rustc-env=DATABASE_URL={env}");
    } else {
        #[cfg(not(target_os = "macos"))]
        println!("cargo:rustc-env=DATABASE_URL=mysql://root@localhost/daemon");

        #[cfg(target_os = "macos")]
        println!("cargo:rustc-env=DATABASE_URL=mysql://root@mariadb.mac.orb.local/daemon");
    }

    #[cfg(feature = "btrfs")]
    gen_btrfs_bindings();
}

#[cfg(feature = "btrfs")]
fn gen_btrfs_bindings() {
    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(std::env::var("OUT_DIR").expect("Missing output directory"));
    let target_path = out_path.join("btrfs_bindings.rs");

    match bindgen::Builder::default()
        .header("btrfs/wrapper.h")
        .derive_default(true)
        .ignore_functions()
        .clang_macro_fallback()
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .size_t_is_usize(true)
        .generate()
    {
        Ok(bindings) => {
            bindings
                .write_to_file(target_path)
                .expect("Couldn't write bindings!");
        }
        Err(err) => {
            println!("Unable to generate btrfs bindings, using prebuilt instead: {err:?}");
            fs::copy("btrfs/bindings_prebuilt.rs", target_path)
                .expect("Unable to copy prebuilt btrfs bindings");
        }
    }
}
