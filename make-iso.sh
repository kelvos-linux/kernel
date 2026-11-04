#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")"

./build.sh
mkdir -p iso/boot/grub
cp kernel.elf iso/boot/kernel.elf
cp grub.cfg iso/boot/grub/grub.cfg
grub-mkimage \
	-O i386-pc-eltorito \
	-d /usr/lib/grub/i386-pc \
	-p /boot/grub \
	-o iso/boot/grub/eltorito.img \
	biosdisk iso9660 multiboot2 normal
xorriso -as mkisofs \
	-R \
	-b boot/grub/eltorito.img \
	-no-emul-boot \
	-boot-load-size 4 \
	-boot-info-table \
	-o kelv-os.iso \
	iso

echo "built ./kelv-os.iso"