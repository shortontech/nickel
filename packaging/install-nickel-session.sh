#!/bin/sh
set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository=$(dirname -- "$script_directory")
release=${NICKEL_RELEASE_DIR:-"$repository/target/release"}
install_root=${NICKEL_INSTALL_ROOT:-}
install_mode=${NICKEL_INSTALL_MODE:-}

if [ -z "$install_mode" ]; then
    if [ -z "$install_root" ]; then
        install_mode=symlink
    else
        install_mode=copy
    fi
fi

for executable in nickel-login nickel-session nickel nickel-settings; do
    if [ ! -x "$release/$executable" ]; then
        echo "Missing $release/$executable; build the release session first." >&2
        exit 1
    fi
done

if ! "$release/nickel-session" --available-backends | grep -qx udev; then
    echo "The release nickel-session lacks the native udev backend." >&2
    echo "Rebuild it with: cargo build --release -p nickel-session --no-default-features --features backend-udev" >&2
    exit 1
fi

for executable in nickel-login nickel-session nickel nickel-settings; do
    destination="$install_root/usr/local/bin/$executable"
    if [ "$install_mode" = symlink ]; then
        mkdir -p "$(dirname "$destination")"
        ln -sfn "$release/$executable" "$destination"
    elif [ "$install_mode" = copy ]; then
        install -Dm755 "$release/$executable" "$destination"
    else
        echo "NICKEL_INSTALL_MODE must be 'symlink' or 'copy'." >&2
        exit 1
    fi
done
install -Dm644 "$repository/packaging/nickel.desktop" \
    "$install_root/usr/share/wayland-sessions/nickel.desktop"
install -Dm644 "$repository/packaging/nickel-settings.desktop" \
    "$install_root/usr/share/applications/nickel-settings.desktop"
install -Dm644 "$repository/packaging/nickel-portals.conf" \
    "$install_root/usr/share/xdg-desktop-portal/nickel-portals.conf"
install -Dm644 "$repository/assets/icons/nickel-settings.png" \
    "$install_root/usr/share/icons/hicolor/512x512/apps/nickel-settings.png"

echo "Installed the Nickel SDDM session."
