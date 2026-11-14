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
        "libadwaita-1.so",
        &[
            "/usr/lib/x86_64-linux-gnu/libadwaita-1.so.0",
            "/usr/lib/libadwaita-1.so.0",
        ],
    );
    symlink_lib(
        &out_dir,
        "libgtk-4.so",
        &[
            "/usr/lib/x86_64-linux-gnu/libgtk-4.so.1",
            "/usr/lib/libgtk-4.so.1",
        ],
    );
    symlink_lib(
        &out_dir,
        "libgio-2.0.so",
        &[
            "/usr/lib/x86_64-linux-gnu/libgio-2.0.so.0",
            "/usr/lib/libgio-2.0.so.0",
        ],
    );
    symlink_lib(
        &out_dir,
        "libgobject-2.0.so",
        &[
            "/usr/lib/x86_64-linux-gnu/libgobject-2.0.so.0",
            "/usr/lib/libgobject-2.0.so.0",
        ],
    );
    symlink_lib(
        &out_dir,
        "libglib-2.0.so",
        &[
            "/usr/lib/x86_64-linux-gnu/libglib-2.0.so.0",
            "/usr/lib/libglib-2.0.so.0",
        ],
    );

    println!("cargo:rustc-link-search=native={}", out_dir.display());
    println!("cargo:rustc-link-lib=dylib=adwaita-1");
    println!("cargo:rustc-link-lib=dylib=gtk-4");
    println!("cargo:rustc-link-lib=dylib=gio-2.0");
    println!("cargo:rustc-link-lib=dylib=gobject-2.0");
    println!("cargo:rustc-link-lib=dylib=glib-2.0");
}
