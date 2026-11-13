#!/bin/sh
set -eu
cd "$(dirname "$0")/../.."
z80asm -o tools/bsrv/BSRV.COM tools/bsrv/bsrv.asm
z80asm -o tools/bsrv/install.bin tools/bsrv/install.asm
size=$(wc -c < tools/bsrv/BSRV.COM)
if [ "$size" -gt 640 ]; then
    echo 'BSRV.COM exceeds the five-record installer limit' >&2
    exit 1
fi
