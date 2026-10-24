// SPDX-License-Identifier: AGPL-3.0-only
#![allow(clippy::expect_used)]

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=migrations");
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .expect("Failed to execute command");

    let git_hash = String::from_utf8(output.stdout).expect("Unable to read git commit");

    println!("cargo:rustc-env=GIT_HASH={git_hash}");

    if let Ok(env) = std::env::var("DAEMON_DATABASE_URL") {
        println!("cargo:rustc-env=DATABASE_URL={env}");
    } else {
        println!("cargo:rustc-env=DATABASE_URL=mysql://root@localhost/daemon");
    }

    #[cfg(feature = "btrfs")]
    gen_btrfs_bindings();
}

#[cfg(feature = "btrfs")]
fn gen_btrfs_bindings() {
    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path =
        std::path::PathBuf::from(std::env::var("OUT_DIR").expect("Missing output directory"));
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
            std::fs::copy("btrfs/bindings_prebuilt.rs", target_path)
                .expect("Unable to copy prebuilt btrfs bindings");
        }
    }
}
