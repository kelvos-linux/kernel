#!/usr/bin/env sh
# Boot Kelv OS through GRUB's Multiboot2 loader.
set -eu
cd "$(dirname "$0")"
sh ./make-iso.sh
exec qemu-system-x86_64 \
    -cdrom kelv-os.iso \
    -boot d \
    -serial stdio \
    -display none \
    -m 256M \
    -smp 4 \
    -no-reboot -no-shutdown