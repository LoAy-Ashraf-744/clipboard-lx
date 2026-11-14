use std::env;
use std::fs;
use std::os::unix::fs as unix_fs;
use std::path::PathBuf;

fn symlink_lib(out_dir: &PathBuf, link_name: &str, candidates: &[&str]) {
    let target = out_dir.join(link_name);
    let source = candidates
        .iter()
        .find(|p| PathBuf::from(p).exists())
        .unwrap_or_else(|| panic!("Could not find {} on system", link_name));
    if target.exists() || fs::symlink_metadata(&target).is_ok() {
        let _ = fs::remove_file(&target);
    }
    unix_fs::symlink(source, &target)
        .unwrap_or_else(|e| panic!("Failed to symlink {}: {}", link_name, e));
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    symlink_lib(
        &out_dir,
        "libX11.so",
        &[
            "/usr/lib/x86_64-linux-gnu/libX11.so.6",
            "/lib/x86_64-linux-gnu/libX11.so.6",
            "/usr/lib/libX11.so.6",
        ],
    );
    symlink_lib(
        &out_dir,
        "libXfixes.so",
        &[
            "/usr/lib/x86_64-linux-gnu/libXfixes.so.3",
            "/lib/x86_64-linux-gnu/libXfixes.so.3",
            "/usr/lib/libXfixes.so.3",
        ],
    );

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=dylib=X11");
    println!("cargo:rustc-link-lib=dylib=Xfixes");
}
