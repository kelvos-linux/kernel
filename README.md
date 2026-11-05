# Kelv OS Kernel

Kelv OS is an experimental x86_64 kernel written in Rust with an assembly boot entry. It boots through GRUB using the Multiboot2 protocol and runs in QEMU.

## Prerequisites

- Linux on an x86_64 host
- Rust and Cargo via [rustup](https://rustup.rs/)
- GNU Make, GNU assembler (`as`), and LLVM linker (`ld.lld`)
- GRUB's i386-pc modules and image builder (`grub-mkimage`)
- `xorriso`
- QEMU system emulator for x86_64 (`qemu-system-x86_64`)

On Debian or Ubuntu, the system packages can be installed with:

```sh
sudo apt update
sudo apt install build-essential lld grub-pc-bin xorriso qemu-system-x86
```

Install the nightly Rust toolchain and its source component:

```sh
rustup toolchain install nightly --component rust-src
```

The repository's `rust-toolchain.toml` selects nightly automatically. Cargo is configured to build the required core libraries for the `x86_64-unknown-none` target.

## Kernel heap

The kernel registers a locked linked-list allocator over a 16 MiB heap mapped during paging initialization. Startup validates allocation and reuse with `Box`, `Vec`, and `BTreeMap` before initializing subsystems that use `alloc`.

## Physical memory and paging

Startup builds a 4 KiB frame bitmap from the Multiboot2 memory map, allocates only fully usable pages below the boot identity-map limit, and reserves the kernel image, boot information, and low 1 MiB. `PagingManager` can map allocated frames with custom flags or map consecutive caller-owned physical frames into a virtual range.

## Build

From the repository root:

```sh
make build
```

This creates `kernel.elf`.

## Run in QEMU

```sh
make qemu
```

This builds the kernel, creates a BIOS-bootable GRUB ISO, and starts QEMU with 256 MiB of memory. Kernel serial output is connected to the terminal. `make run` is an equivalent command.

To create the bootable ISO without starting QEMU:

```sh
make iso
```

The generated `kernel.elf`, `kelv-os.iso`, `iso/`, and Cargo `target/` directory are local build artifacts and are excluded from Git.

## Clean

```sh
make clean
```