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

## User module and shared ring

GRUB supplies `user_init` as a Multiboot2 module. The kernel reserves the
module's physical pages, maps the image at ring 3, and maps one page containing
an eight-entry, single-producer/single-consumer submission/completion ring.
Userspace publishes a `SPAWN` entry and invokes `int 0x80`; the interrupt
handler drains the ring synchronously. There is no polling thread.

The kernel accepts the first SPAWN request by adding a child task to its
scheduler queue. Timer interrupts save all general-purpose registers and the
user return frame, switch kernel stacks, and restore the next task's context.
The initial task and one child can therefore execute in round-robin time
slices. They still share one address space, and the prototype uses fixed user
stacks and a static child kernel stack; it does not provide process isolation
or general task creation.

## Kernel build

From the workspace root:

```sh
make kernel
```

This creates `kernel/kernel.elf`. The workspace-level Makefile builds the
independent user image, packages both images into a GRUB ISO, and runs QEMU.