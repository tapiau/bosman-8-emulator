#!/bin/sh
# Renders the icon files from the SVG sources (needs inkscape and ImageMagick).
#   bosman8.svg        full design (ship's wheel with 8 spokes, brass hub with "8"), 48 px and up
#   bosman8-small.svg  simplified, 24 and 32 px
#   bosman8-tiny.svg   16 px (rim + hub, bold "8")
# Outputs: bosman8-<size>.png (hicolor icons for the .deb), bosman8.ico (Windows
# executable), bosman8-128.rgba (raw RGBA, window icon built into the program).
set -e
cd "$(dirname "$0")"
for s in 48 64 128 256; do inkscape bosman8.svg -o bosman8-$s.png -w $s -h $s; done
for s in 24 32; do inkscape bosman8-small.svg -o bosman8-$s.png -w $s -h $s; done
inkscape bosman8-tiny.svg -o bosman8-16.png -w 16 -h 16
magick bosman8-16.png bosman8-24.png bosman8-32.png bosman8-48.png bosman8-64.png \
  bosman8-128.png bosman8-256.png bosman8.ico
magick bosman8-128.png -depth 8 rgba:bosman8-128.rgba
