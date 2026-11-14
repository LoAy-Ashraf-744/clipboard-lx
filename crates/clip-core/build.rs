use std::env;
use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let target_so = out_dir.join("libsqlite3.so");

    let candidates = [
        "/usr/lib/x86_64-linux-gnu/libsqlite3.so.0",
        "/lib/x86_64-linux-gnu/libsqlite3.so.0",
        "/usr/lib/libsqlite3.so.0",
    ];

    let source = candidates
        .iter()
        .find(|p| PathBuf::from(p).exists())
        .expect("Could not find libsqlite3.so.0 on system");

    if target_so.exists() || fs::symlink_metadata(&target_so).is_ok() {
        let _ = fs::remove_file(&target_so);
    }
    unix_fs::symlink(source, &target_so).expect("Failed to symlink libsqlite3.so");

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=dylib=sqlite3");
}
