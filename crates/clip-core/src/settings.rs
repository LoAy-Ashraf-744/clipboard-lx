use std::fs;
use std::path::Path;

use crate::paths::default_settings_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub auto_paste: bool,
    pub close_on_focus_loss: bool,
    pub render_markdown: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_paste: true,
            close_on_focus_loss: true,
            render_markdown: true,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::load_from(&default_settings_path())
    }

    pub fn save(&self) -> Result<(), String> {
        self.save_to(&default_settings_path())
    }

    pub fn load_from(path: &Path) -> Self {
        let mut settings = Self::default();
        let Ok(contents) = fs::read_to_string(path) else {
            return settings;
        };
        for line in contents.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim();
                let val = v.trim();
                if let Some(b) = parse_bool(val) {
                    match key {
                        "auto_paste" => settings.auto_paste = b,
                        "close_on_focus_loss" => settings.close_on_focus_loss = b,
                        "render_markdown" => settings.render_markdown = b,
                        _ => {}
                    }
                }
            }
        }
        settings
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| format!("create config dir: {}", e))?;
            }
        }
        let body = format!(
            "auto_paste = {}\nclose_on_focus_loss = {}\nrender_markdown = {}\n",
            self.auto_paste, self.close_on_focus_loss, self.render_markdown
        );
        fs::write(path, body).map_err(|e| format!("write settings: {}", e))
    }
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Some(true),
        "false" | "0" | "no" | "off" => Some(false),
        _ => None,
    }
}
