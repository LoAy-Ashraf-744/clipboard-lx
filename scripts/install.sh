#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
SYSTEMD_DIR="${HOME}/.config/systemd/user"
AUTOSTART_DIR="${HOME}/.config/autostart"
APPS_DIR="${HOME}/.local/share/applications"
EXT_DIR="${HOME}/.local/share/gnome-shell/extensions/clipboard-lx@local"

echo "[1/6] Building clipboard-lx (release)..."
cargo build --release --workspace --manifest-path "${ROOT_DIR}/Cargo.toml"

echo "[2/6] Installing binaries to ${BIN_DIR}..."
install -Dm755 "${ROOT_DIR}/target/release/clipd" "${BIN_DIR}/clipd"
install -Dm755 "${ROOT_DIR}/target/release/clip-ui" "${BIN_DIR}/clip-ui"
install -Dm755 "${ROOT_DIR}/target/release/clip-ctl" "${BIN_DIR}/clip-ctl"

echo "[3/6] Installing desktop entries..."
install -Dm644 "${ROOT_DIR}/data/clipboard-lx-daemon.desktop" "${AUTOSTART_DIR}/clipboard-lx-daemon.desktop"
install -Dm644 "${ROOT_DIR}/data/org.clipboardlx.UI.desktop" "${APPS_DIR}/org.clipboardlx.UI.desktop"
sed -i "s|/home/loay-ashraf|${HOME}|g" \
    "${AUTOSTART_DIR}/clipboard-lx-daemon.desktop" \
    "${APPS_DIR}/org.clipboardlx.UI.desktop"

echo "[4/6] Installing and starting systemd user service (clipd.service)..."
install -Dm644 "${ROOT_DIR}/data/clipd.service" "${SYSTEMD_DIR}/clipd.service"
systemctl --user import-environment DISPLAY WAYLAND_DISPLAY XDG_RUNTIME_DIR XAUTHORITY || true
systemctl --user daemon-reload
systemctl --user reenable clipd.service
systemctl --user restart clipd.service

echo "[5/6] Installing GNOME Shell 46 extension..."
mkdir -p "${EXT_DIR}"
install -Dm644 "${ROOT_DIR}/extension/clipboard-lx@local/metadata.json" "${EXT_DIR}/metadata.json"
install -Dm644 "${ROOT_DIR}/extension/clipboard-lx@local/extension.js" "${EXT_DIR}/extension.js"
if command -v gnome-extensions >/dev/null 2>&1; then
    gnome-extensions enable clipboard-lx@local >/dev/null 2>&1 || true
fi
python3 - <<'PY'
import ast, subprocess
try:
    cur = subprocess.check_output(
        ["gsettings", "get", "org.gnome.shell", "enabled-extensions"],
        text=True,
    ).strip()
    exts = [] if cur == "@as []" else ast.literal_eval(cur)
    if "clipboard-lx@local" not in exts:
        exts.append("clipboard-lx@local")
        subprocess.check_call(["gsettings", "set", "org.gnome.shell", "enabled-extensions", str(exts)])
except Exception as exc:
    print("Note: skipped enabled-extensions update:", exc)
PY

echo "[6/6] Binding Super+V and Super+. shortcuts to clip-ui..."
python3 - <<'PY'
import ast, os, subprocess

home = os.path.expanduser("~")
cmd = os.path.join(home, ".local", "bin", "clip-ui")
base_schema = "org.gnome.settings-daemon.plugins.media-keys"

# Ensure Super+V is free from GNOME message tray and Super+. is free from IBus emoji popup.
subprocess.call(["gsettings", "set", "org.gnome.shell.keybindings", "toggle-message-tray", "[]"])
try:
    cur_ibus = subprocess.check_output(
        ["gsettings", "get", "org.freedesktop.ibus.panel.emoji", "hotkey"],
        text=True,
    ).strip()
    keys = [] if cur_ibus == "@as []" else ast.literal_eval(cur_ibus)
    filtered = [k for k in keys if k != "<Super>period"]
    if filtered != keys:
        val = "@as []" if not filtered else str(filtered)
        subprocess.call(["gsettings", "set", "org.freedesktop.ibus.panel.emoji", "hotkey", val])
except Exception as exc:
    print("Note: skipped IBus emoji hotkey update:", exc)

cur = subprocess.check_output(
    ["gsettings", "get", base_schema, "custom-keybindings"],
    text=True,
).strip()
bindings = [] if cur == "@as []" else ast.literal_eval(cur)

def read_key(schema, key):
    try:
        raw = subprocess.check_output(["gsettings", "get", schema, key], text=True).strip()
        return ast.literal_eval(raw)
    except Exception:
        return ""

def ensure_binding(bindings_list, name, command, binding):
    target_slot = None
    for slot in bindings_list:
        schema = f"{base_schema}.custom-keybinding:{slot}"
        if read_key(schema, "name") == name or read_key(schema, "command") == command:
            target_slot = slot
            break
    if target_slot is None:
        idx = 0
        while True:
            candidate = f"/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/custom{idx}/"
            if candidate not in bindings_list:
                target_slot = candidate
                bindings_list.append(target_slot)
                break
            idx += 1

    schema = f"{base_schema}.custom-keybinding:{target_slot}"
    subprocess.check_call(["gsettings", "set", schema, "name", name])
    subprocess.check_call(["gsettings", "set", schema, "command", command])
    subprocess.check_call(["gsettings", "set", schema, "binding", binding])

ensure_binding(bindings, "Clipboard LX", cmd, "<Super>v")
ensure_binding(bindings, "Clipboard LX Emojis", f"{cmd} --emojis", "<Super>period")

val = "@as []" if not bindings else str(bindings)
subprocess.check_call(["gsettings", "set", base_schema, "custom-keybindings", val])
PY

echo "Done! Press Super+V for Clipboard LX or Super+. for Emojis."
