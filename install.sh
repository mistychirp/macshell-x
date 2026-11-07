#!/bin/sh
# Builds and installs makshell outside Nix: the shell, its KWin plugins, and a "makshell" login
# session. On NixOS, use the flake's module instead (see README.md).
#
#   ./install.sh build              as yourself: builds into target/ and build/
#   sudo ./install.sh install       copies it into PREFIX
#   sudo ./install.sh uninstall     removes what install copied
#
# PREFIX is /usr/local unless it's set, for both build and install. DESTDIR, if set, is put in front
# of every installed path, for packaging.
set -eu

PREFIX=${PREFIX:-/usr/local}
DESTDIR=${DESTDIR:-}
ROOT=$(cd "$(dirname "$0")" && pwd)
BUILD="$ROOT/build/kwin"
MANIFEST="$PREFIX/share/makshell/installed-files"

say() { printf '==> %s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

build() {
    [ "$(id -u)" -ne 0 ] || die "build as yourself, not as root; then install with sudo"
    command -v cargo >/dev/null || die "cargo is missing (install Rust 1.88 or newer, e.g. with rustup)"
    command -v cmake >/dev/null || die "cmake is missing"
    say "Building the shell"
    (cd "$ROOT" && cargo build --release --locked)
    say "Building the KWin plugins"
    cmake -S "$ROOT/kwin" -B "$BUILD" -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX="$PREFIX"
    cmake --build "$BUILD" --parallel
    say "Built. Now: sudo ./install.sh install"
}

# Copies a file into place, filling in @prefix@ and friends, and notes it for uninstall.
put() { # put MODE SOURCE DESTINATION
    mkdir -p "$DESTDIR$(dirname "$3")"
    sed -e "s|@prefix@|$PREFIX|g" -e "s|@makshell@|$PREFIX/bin/makshell|g" \
        -e "s|@qtplugins@|$QT_PLUGINS|g" "$2" > "$DESTDIR$3"
    chmod "$1" "$DESTDIR$3"
    echo "$3" >> "$DESTDIR$MANIFEST"
}

install_all() {
    [ -x "$ROOT/target/release/makshell" ] && [ -f "$BUILD/CMakeCache.txt" ] \
        || die "nothing built yet: run ./install.sh build first, as yourself"
    mkdir -p "$DESTDIR$(dirname "$MANIFEST")"
    : > "$DESTDIR$MANIFEST"

    say "Installing the KWin plugins"
    DESTDIR="$DESTDIR" cmake --install "$BUILD"
    # CMake's list doesn't end its last line.
    { sed "s|^$DESTDIR||" "$BUILD/install_manifest.txt"; echo; } | grep -v '^$' >> "$DESTDIR$MANIFEST"
    # Where Qt plugins went under PREFIX, which the session tells KWin.
    effect=$(grep '/kwin/effects/plugins/makshell\.so$' "$BUILD/install_manifest.txt" | head -n 1)
    [ -n "$effect" ] || die "the KWin effect wasn't installed"
    QT_PLUGINS=$(dirname "$(dirname "$(dirname "$(dirname "${effect#"$DESTDIR"}")")")")

    say "Installing the shell and the session"
    mkdir -p "$DESTDIR$PREFIX/bin"
    install -m 0755 "$ROOT/target/release/makshell" "$DESTDIR$PREFIX/bin/makshell"
    echo "$PREFIX/bin/makshell" >> "$DESTDIR$MANIFEST"
    printf '#!/bin/sh\nexec %s/bin/makshell settings "$@"\n' "$PREFIX" > "$DESTDIR$PREFIX/bin/makshell-settings"
    chmod 0755 "$DESTDIR$PREFIX/bin/makshell-settings"
    echo "$PREFIX/bin/makshell-settings" >> "$DESTDIR$MANIFEST"
    # KWin only lets the shell manage windows if a desktop entry naming its binary says so.
    put 0644 "$ROOT/contrib/makshell.desktop" "$PREFIX/share/applications/makshell.desktop"
    put 0644 "$ROOT/contrib/makshell-settings.desktop" "$PREFIX/share/applications/makshell-settings.desktop"
    put 0755 "$ROOT/contrib/session/makshell-session" "$PREFIX/bin/makshell-session"
    put 0755 "$ROOT/contrib/session/makshell-startup" "$PREFIX/libexec/makshell-startup"
    put 0644 "$ROOT/contrib/session/makshell.desktop" "$PREFIX/share/wayland-sessions/makshell.desktop"
    put 0644 "$ROOT/contrib/session/makshell-session.target" "$PREFIX/lib/systemd/user/makshell-session.target"

    say "Installed into $PREFIX. Log out, and choose \"makshell\" as the session."
}

uninstall_all() {
    [ -f "$DESTDIR$MANIFEST" ] || die "no record of an install in $PREFIX"
    while IFS= read -r file; do
        rm -f "$DESTDIR$file"
    done < "$DESTDIR$MANIFEST"
    rm -f "$DESTDIR$MANIFEST"
    # Folders this left empty, deepest first.
    for dir in "$PREFIX/share/makshell" "$PREFIX/share/plasma/shells/org.makshell.desktop/contents/lockscreen" \
               "$PREFIX/share/plasma/shells/org.makshell.desktop/contents/fonts" \
               "$PREFIX/share/plasma/shells/org.makshell.desktop/contents" "$PREFIX/share/plasma/shells/org.makshell.desktop" \
               "$PREFIX/share/kwin-wayland/tabbox/makshell/contents/ui" "$PREFIX/share/kwin-wayland/tabbox/makshell/contents" \
               "$PREFIX/share/kwin-wayland/tabbox/makshell"; do
        rmdir "$DESTDIR$dir" 2>/dev/null || true
    done
    say "Removed makshell from $PREFIX. Your settings in ~/.config/makshell stay."
}

case "${1:-}" in
    build) build ;;
    install) install_all ;;
    uninstall) uninstall_all ;;
    *) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
