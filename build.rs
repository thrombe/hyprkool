use std::process::Command;

const VERSION: &str = env!("CARGO_PKG_VERSION");
fn main() {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .unwrap();

    let git_hash = String::from_utf8(output.stdout).unwrap();
    println!("cargo:rustc-env=GIT_HASH={}", git_hash);
    println!("cargo:rustc-env=VERSION={}-{}", VERSION, git_hash);
}
