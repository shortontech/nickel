#!/bin/sh
set -eu

repository=/projects/nickel
release="$repository/target/release"
install_root=${NICKEL_INSTALL_ROOT:-}

for executable in nickel-login nickel-session nickel-ui; do
    if [ ! -x "$release/$executable" ]; then
        echo "Missing $release/$executable; build the release session first." >&2
        exit 1
    fi
done

install -Dm755 "$release/nickel-login" "$install_root/usr/local/bin/nickel-login"
install -Dm755 "$release/nickel-session" "$install_root/usr/local/bin/nickel-session"
install -Dm755 "$release/nickel-ui" "$install_root/usr/local/bin/nickel-ui"
install -Dm644 "$repository/packaging/nickel.desktop" \
    "$install_root/usr/share/wayland-sessions/nickel.desktop"

echo "Installed the Nickel SDDM session."
