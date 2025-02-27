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
            if let Ok(path) = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../mysql/sockets/mysqld.sock").canonicalize() {
                println!("cargo:rustc-env=DATABASE_URL=mysql://root@localhost/daemon?socket={}", path.to_string_lossy());
            }
        }

        #[cfg(target_os = "macos")]
        println!("cargo:rustc-env=DATABASE_URL=mysql://root@mysql.mac.orb.local/daemon");
    }
}
