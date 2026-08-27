#!/bin/sh
set -eu

repository=/projects/nickel
release="$repository/target/release"
install_root=${NICKEL_INSTALL_ROOT:-}

for executable in nickel-login nickel-session nickel nickel-settings; do
    if [ ! -x "$release/$executable" ]; then
        echo "Missing $release/$executable; build the release session first." >&2
        exit 1
    fi
done

install -Dm755 "$release/nickel-login" "$install_root/usr/local/bin/nickel-login"
install -Dm755 "$release/nickel-session" "$install_root/usr/local/bin/nickel-session"
install -Dm755 "$release/nickel" "$install_root/usr/local/bin/nickel"
install -Dm755 "$release/nickel-settings" "$install_root/usr/local/bin/nickel-settings"
install -Dm644 "$repository/packaging/nickel.desktop" \
    "$install_root/usr/share/wayland-sessions/nickel.desktop"
install -Dm644 "$repository/packaging/nickel-settings.desktop" \
    "$install_root/usr/share/applications/nickel-settings.desktop"
install -Dm644 "$repository/assets/icons/nickel-settings.png" \
    "$install_root/usr/share/icons/hicolor/512x512/apps/nickel-settings.png"

echo "Installed the Nickel SDDM session."
