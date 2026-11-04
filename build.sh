#!/usr/bin/env sh
# Build Kelv OS kernel.elf (Rust staticlib + assembly entry, linked with lld)
set -eu
cd "$(dirname "$0")"

cargo build --release

${AS:-as} --64 src/boot.S -o target/boot.o

${LD:-ld.lld} -T linker.ld \
    target/boot.o \
    target/x86_64-unknown-none/release/libkelv_os.a \
    -o kernel.elf

echo "built ./kernel.elf"