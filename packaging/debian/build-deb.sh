#!/usr/bin/env bash
# Build a Debian package from already compiled release binaries.
#
#   packaging/debian/build-deb.sh [VERSION] [BIN_DIR] [OUT_DIR]
#
#   VERSION  package version, default: workspace version from Cargo.toml
#            (a leading "v" is stripped, so a git tag like v0.1.0 works)
#   BIN_DIR  directory with bosman8, b8disk (default target/release)
#   OUT_DIR  where to put the .deb (default dist)
#
# Environment: DEB_MAINTAINER (default "tapiau <tapiau@users.noreply.github.com>"),
#              DEB_ARCH (default: dpkg --print-architecture)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

VERSION="${1:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
VERSION="${VERSION#v}"
BIN_DIR="${2:-target/release}"
OUT_DIR="${3:-dist}"
ARCH="${DEB_ARCH:-$(dpkg --print-architecture)}"
MAINTAINER="${DEB_MAINTAINER:-tapiau <tapiau@users.noreply.github.com>}"
PKG=bosman8-emulator

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
D="$WORK/$PKG"

install -d "$D/DEBIAN" "$D/etc" "$D/usr/bin" "$D/usr/share/$PKG/roms" "$D/usr/share/$PKG/disks" \
  "$D/usr/share/doc/$PKG/doc" "$D/usr/share/applications"
for b in bosman8 b8disk; do
  install -m 755 "$BIN_DIR/$b" "$D/usr/bin/$b"
  strip --strip-unneeded "$D/usr/bin/$b" 2>/dev/null || true
done
# The former terminal program: bosman8 called as "ang3001" runs mode "terminal".
ln -s bosman8 "$D/usr/bin/ang3001"
# roms/<machine>/ (bosman8, ang3001, later an2001)
for m in roms/*/; do
  m=$(basename "$m")
  install -d "$D/usr/share/$PKG/roms/$m"
  install -m 644 roms/$m/* "$D/usr/share/$PKG/roms/$m/"
done
# Old paths: /etc/bosman-8-emulator.conf is a conffile, so edited configs from
# earlier versions may still point at roms/bosman8.bin, roms/ang3001.bin (before
# 0.4.1) or roms/ang3001/ang3001.bin (0.4.1). They meant "the default ROM", so
# the terminal ones lead to the current default, ROM 90.09.11.
ln -s bosman8/bosman8.bin "$D/usr/share/$PKG/roms/bosman8.bin"
ln -s ang3001/ANG-3001_90.09.11.BIN "$D/usr/share/$PKG/roms/ang3001.bin"
ln -s ANG-3001_90.09.11.BIN "$D/usr/share/$PKG/roms/ang3001/ang3001.bin"
install -m 644 disks/* "$D/usr/share/$PKG/disks/"
install -m 644 README.md "$D/usr/share/doc/$PKG/"
install -m 644 doc/*.md "$D/usr/share/doc/$PKG/doc/"
install -d "$D/usr/share/doc/$PKG/tools"
cp -r tools/bsrv "$D/usr/share/doc/$PKG/tools/"
install -m 644 packaging/debian/*.desktop "$D/usr/share/applications/"
# Icon (ship's wheel with an "8"): hicolor theme, Icon=bosman8 in the .desktop files.
for s in 16 24 32 48 64 128 256; do
  install -Dm 644 packaging/icons/bosman8-$s.png "$D/usr/share/icons/hicolor/${s}x${s}/apps/bosman8.png"
done
install -Dm 644 packaging/icons/bosman8.svg "$D/usr/share/icons/hicolor/scalable/apps/bosman8.svg"
install -m 644 packaging/debian/bosman-8-emulator.conf "$D/etc/bosman-8-emulator.conf"
echo /etc/bosman-8-emulator.conf > "$D/DEBIAN/conffiles"

cat > "$D/usr/share/doc/$PKG/copyright" <<COPY
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: $PKG
Source: https://github.com/tapiau/bosman-8-emulator

Files: *
Copyright: 2026 bosman-8-emulator contributors
License: MIT

Files: usr/share/$PKG/roms/* usr/share/$PKG/disks/*
Comment: Original Bosman-8 / ANG-3001 EPROM images and CP/M utilities,
 included for preservation and emulation purposes.
COPY
chmod 644 "$D/usr/share/doc/$PKG/copyright"

# Debian changelog (required for native packages)
{
  echo "$PKG ($VERSION) unstable; urgency=medium"
  echo
  echo "  * Release $VERSION, see https://github.com/tapiau/bosman-8-emulator/releases"
  echo
  echo " -- $MAINTAINER  $(date -R)"
} | gzip -9n > "$D/usr/share/doc/$PKG/changelog.gz"
chmod 644 "$D/usr/share/doc/$PKG/changelog.gz"

# Library dependencies of the binaries (libc, libgcc, ...). GUI libraries
# (X11, Wayland, OpenGL, xkbcommon) are loaded at run time with dlopen, so
# they are listed explicitly below.
SHLIBS="libc6"
if command -v dpkg-shlibdeps >/dev/null; then
  mkdir -p "$WORK/shl/debian"
  echo "Source: $PKG" > "$WORK/shl/debian/control"
  SHLIBS="$(cd "$WORK/shl" && dpkg-shlibdeps -O --ignore-missing-info \
    "$D/usr/bin/bosman8" "$D/usr/bin/b8disk" 2>/dev/null \
    | sed -n 's/^shlibs:Depends=//p')"
fi

SIZE="$(du -sk "$D/usr" | cut -f1)"
cat > "$D/DEBIAN/control" <<CTRL
Package: $PKG
Version: $VERSION
Section: otherosfs
Priority: optional
Architecture: $ARCH
Maintainer: $MAINTAINER
Installed-Size: $SIZE
Depends: $SHLIBS
Recommends: libgl1, libx11-6, libxcursor1, libxrandr2, libxi6, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0, libwayland-cursor0, libwayland-egl1, xdg-desktop-portal
Homepage: https://github.com/tapiau/bosman-8-emulator
Description: emulators of the Bosman-8 computer and ANG-3001 terminal
 Bosman-8 (DZM-180, Z80A, CPM-R in EPROM) and its graphics terminal ANG-3001
 (Z80B, 512x256) in one program with three modes: "full" (computer and
 terminal in one window), "computer" (the computer only, its terminal line on
 TCP or a host serial port, e.g. for a real ANG-3001) and "terminal" (the
 terminal only, e.g. for a real Bosman; also run as "ang3001").
 .
 Includes bosman8 (GUI, or headless with --no-gui / terminal --headless),
 b8disk (CP/M floppy image tool), the ROM images and a floppy with CP/M
 utilities (/usr/share/bosman8-emulator/disks).
CTRL

mkdir -p "$OUT_DIR"
OUT="$OUT_DIR/${PKG}_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$D" "$OUT" >/dev/null
echo "$OUT"
