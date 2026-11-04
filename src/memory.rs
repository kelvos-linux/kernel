//! Multiboot2 physical memory map detection.
//!
//! Walks the boot information structure looking for the memory map tag
//! (type 6) and prints every region plus the total usable RAM.

use crate::serial_println;

const MULTIBOOT2_TAG_MEMORY_MAP: u32 = 6;

#[repr(C)]
struct BootInfoHeader {
    total_size: u32,
    reserved: u32,
}

#[repr(C)]
struct TagHeader {
    tag_type: u32,
    size: u32,
}

#[repr(C)]
struct MemoryMapEntry {
    base_addr: u64,
    length: u64,
    entry_type: u32,
    _reserved: u32,
}

fn entry_type_name(t: u32) -> &'static str {
    match t {
        1 => "usable",
        2 => "reserved",
        3 => "ACPI reclaimable",
        4 => "ACPI NVS",
        5 => "bad RAM",
        _ => "unknown",
    }
}

/// Parse the Multiboot2 boot information structure and print the BIOS
/// memory map. Valid because `boot.S` identity maps 0..4 GiB before
/// entering Rust, so physical addresses below 4 GiB are usable as-is.
pub fn init(boot_info_address: u32) {
    if boot_info_address == 0 {
        serial_println!("[WARN] null Multiboot2 info address");
        return;
    }

    // The Multiboot2 info structure is 8-byte aligned and placed below 4 GiB.
    let base = boot_info_address as u64 as *const u8;
    let total_size = unsafe { (&*base.cast::<BootInfoHeader>()).total_size } as u64;
    let mut offset = 8u64; // skip total_size + reserved

    while offset < total_size {
        let tag = unsafe { &*base.add(offset as usize).cast::<TagHeader>() };

        if tag.tag_type == 0 {
            break; // end tag
        }

        if tag.tag_type == MULTIBOOT2_TAG_MEMORY_MAP {
            // tag: type(4), size(4), entry_size(4), entry_version(4), entries…
            let entry_area = unsafe { base.add(offset as usize + 16) };
            let entry_count = (tag.size as u64 - 16) / 24;
            let mut usable_mib = 0u64;

            serial_println!("[MEM] BIOS physical memory map ({} entries)", entry_count);

            for i in 0..entry_count as usize {
                let entry = unsafe { &*entry_area.add(i * 24).cast::<MemoryMapEntry>() };
                if entry.entry_type == 1 {
                    usable_mib += entry.length >> 20;
                }
                serial_println!(
                    "[MEM]   {:#014x} - {:#014x}  {:>5} MiB  {}",
                    entry.base_addr,
                    entry.base_addr + entry.length,
                    entry.length >> 20,
                    entry_type_name(entry.entry_type)
                );
            }
            serial_println!("[MEM] total usable RAM: {} MiB", usable_mib);
            return;
        }

        // Advance to the next tag, 8-byte aligned.
        let size = ((tag.size as u64) + 7) & !7;
        if size == 0 {
            break;
        }
        offset += size;
    }

    serial_println!("[WARN] no memory map tag found in Multiboot2 info");
}
