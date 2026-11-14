# clipboard-lx

A lightweight, native-feeling clipboard history manager for **Ubuntu 24.04 / GNOME 46 / Wayland**, written in **Rust** with **GTK4 + libadwaita**.

## Features

- **Native Ubuntu look & feel**: Built with GTK4 + libadwaita (`AdwApplicationWindow`, `GtkSearchEntry`, `GtkListBox`), automatically adapting to your Ubuntu Yaru/Adwaita light or dark theme.
- **Zero-RAM idle UI**: `clip-ui` runs on demand when you press `Super+V` and exits immediately after selection or `Esc`.
- **Tiny background daemon**: `clipd` uses event-driven XFixes selection monitoring (`XFixesSelectSelectionInput`) + GNOME Shell 46 extension forwarding (no polling), keeping idle RSS under ~5–10 MB.
- **Text, Markdown & Images**: Captures UTF-8 text (with automatic Markdown detection and native Pango formatted previews in the popup) and PNG images with dimensions, file sizes, deduplication, and inline thumbnail previews.
- **Fast search**: Instant hybrid substring + fuzzy search over clipboard history stored in SQLite (with FTS5 indexing).
- **Privacy aware**: Ignores password-manager concealed hints (`x-kde-passwordManagerHint`, `org.nspasteboard.ConcealedType`).
- **Start on boot**: Installs a `systemd --user` service (`clipd.service`), XDG autostart fallback, and GNOME Shell 46 extension.

## Architecture

- `crates/clip-core`: SQLite + FTS5 storage engine, text/image models, hybrid fuzzy search, and length-prefixed Unix socket IPC protocol.
- `crates/clipd`: Background clipboard monitor daemon and persistent clipboard setter.
- `crates/clip-ui`: Native GTK4 + libadwaita popup window bound to `Super+V`.
- `crates/clip-ctl`: Fast CLI tool to inspect, search, select, pin, delete, pause, or clear history.
- `extension/clipboard-lx@local`: GNOME Shell 46 ESM extension for Wayland clipboard owner-change forwarding.

## Keyboard Shortcuts (inside `clip-ui`)

| Key | Action |
|---|---|
| `Type` | Filter history instantly in the search box |
| `↑` / `↓` | Navigate clipboard items |
| `Enter` | Copy selected item to clipboard and close |
| `Ctrl+P` | Pin / unpin selected item |
| `Del` (or `Ctrl+Del`) | Delete selected item |
| `Esc` | Clear search text, or close window if empty |
| `Super+V` | Open (or toggle closed) the clipboard window |

## Build & Install

```bash
./scripts/install.sh
```

Or build and test manually:

```bash
cargo test --workspace
cargo build --release
```
