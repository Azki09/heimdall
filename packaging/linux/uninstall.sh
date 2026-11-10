#!/bin/sh
# Removes what install.sh put in your home folder: the program, its app
# menu entry and its icon. Your vaults (.heim files) are not touched.
set -eu

: "${HOME:?HOME is not set}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
case "$data" in /*) ;; *) data="$HOME/.local/share" ;; esac

rm -f "$HOME/.local/bin/heimdall" \
    "$data/applications/heimdall.desktop" \
    "$data/icons/hicolor/512x512/apps/heimdall.png" \
    "$data/heimdall/uninstall.sh"
rmdir "$data/heimdall" 2>/dev/null || true

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "$data/applications" 2>/dev/null || true
fi

config="${XDG_CONFIG_HOME:-$HOME/.config}"
case "$config" in /*) ;; *) config="$HOME/.config" ;; esac
config="$config/heimdall"
echo "Heimdall is removed. Your vaults were not touched."
if [ -d "$config" ]; then
    echo "The list of recent vaults is still in $config"
    echo "Delete that folder too if you want no trace left: rm -r \"$config\""
fi
