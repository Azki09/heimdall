#!/bin/sh
# Installs Heimdall for the current user, so it shows up in the app menu.
# No sudo, nothing written outside your home folder, nothing downloaded.
# Run it from the unpacked .tar.gz:
#
#   ./install.sh
#
# Running it again updates an earlier install. To remove Heimdall, run
# ~/.local/share/heimdall/uninstall.sh (your vaults are never touched).
set -eu

if [ "$(id -u)" -eq 0 ]; then
    echo "Run this as your normal user, without sudo." >&2
    exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
for f in heimdall heimdall.desktop heimdall.png uninstall.sh; do
    if [ ! -f "$here/$f" ]; then
        echo "Missing $f next to install.sh. Run it from the unpacked .tar.gz." >&2
        exit 1
    fi
done

: "${HOME:?HOME is not set}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
case "$data" in /*) ;; *) data="$HOME/.local/share" ;; esac
bin="$HOME/.local/bin"

# The desktop entry runs the full path (~/.local/bin is not on every
# desktop's PATH). Refuse paths that would need escaping in it.
case "$bin" in
    *[\"\`\$\\%\|\&]*)
        echo "Your home folder path ($HOME) has characters this script can't handle." >&2
        exit 1
        ;;
esac

# install(1) replaces each file instead of writing into it, so updating
# while Heimdall is open is safe: the running copy keeps the old file.
install -D -m 755 "$here/heimdall" "$bin/heimdall"
install -D -m 644 "$here/heimdall.png" "$data/icons/hicolor/512x512/apps/heimdall.png"
install -D -m 755 "$here/uninstall.sh" "$data/heimdall/uninstall.sh"

mkdir -p "$data/applications"
entry="$data/applications/heimdall.desktop"
sed "s|^Exec=.*|Exec=\"$bin/heimdall\"|" "$here/heimdall.desktop" > "$entry.tmp"
chmod 644 "$entry.tmp"
mv -f "$entry.tmp" "$entry"

# Most desktops notice the new entry on their own; these only speed it up.
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "$data/applications" 2>/dev/null || true
fi
touch "$data/icons/hicolor" 2>/dev/null || true

echo "Heimdall is installed. Look for it in your app menu."
echo "If it doesn't appear yet, log out and back in."
echo "To remove it: $data/heimdall/uninstall.sh"
