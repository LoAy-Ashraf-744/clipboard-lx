use std::env;
use std::path::PathBuf;

extern "C" {
    fn getuid() -> u32;
}

pub fn current_uid() -> u32 {
    unsafe { getuid() }
}

pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR") {
        if !runtime_dir.is_empty() {
            return PathBuf::from(runtime_dir).join("clipboard-lx.sock");
        }
    }
    PathBuf::from(format!("/tmp/clipboard-lx-{}.sock", current_uid()))
}

pub fn default_db_path() -> PathBuf {
    if let Ok(data_home) = env::var("XDG_DATA_HOME") {
        if !data_home.is_empty() {
            return PathBuf::from(data_home)
                .join("clipboard-lx")
                .join("history.db");
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".local/share/clipboard-lx")
        .join("history.db")
}

pub fn default_settings_path() -> PathBuf {
    if let Ok(config_home) = env::var("XDG_CONFIG_HOME") {
        if !config_home.is_empty() {
            return PathBuf::from(config_home)
                .join("clipboard-lx")
                .join("settings.conf");
        }
    }
    let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home)
        .join(".config/clipboard-lx")
        .join("settings.conf")
}
