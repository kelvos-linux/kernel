#!/usr/bin/env sh
# Build the Rust static library and link its assembly bootstrap.
set -eu
cd "$(dirname "$0")"

cargo build --release --locked
${LD:-ld.lld} -T linker.ld -u _start \
    target/x86_64-unknown-none/release/libkelv_os.a \
    -o kernel.elf
