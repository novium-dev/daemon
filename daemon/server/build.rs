use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .expect("Failed to execute command");

    let git_hash = String::from_utf8(output.stdout).unwrap();

    println!("cargo:rustc-env=GIT_HASH={}", git_hash);

    if let Ok(env) = std::env::var("DAEMON_DATABASE_URL") {
        println!("cargo:rustc-env=DATABASE_URL={}", env);
    } else {
        #[cfg(target_os = "linux")]
        {
            if let Ok(path) = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
                .join("../mysql/sockets/mysqld.sock")
                .canonicalize()
            {
                println!(
                    "cargo:rustc-env=DATABASE_URL=mysql://root@localhost/daemon?socket={}",
                    path.to_string_lossy()
                );
            }
        }

        #[cfg(target_os = "macos")]
        println!("cargo:rustc-env=DATABASE_URL=mysql://root@mysql.mac.orb.local/daemon");
    }

    #[cfg(feature = "btrfs")]
    gen_btrfs_bindings();
}

#[cfg(feature = "btrfs")]
fn gen_btrfs_bindings() {
    println!("cargo:rerun-if-changed=wrapper.h");

    let bindings = bindgen::Builder::default()
        .header("btrfs/wrapper.h")
        .derive_default(true)
        .ignore_functions()
        .clang_macro_fallback()
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .default_enum_style(bindgen::EnumVariation::ModuleConsts)
        .size_t_is_usize(true)
        .generate()
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("btrfs_bindings.rs"))
        .expect("Couldn't write bindings!");
}
