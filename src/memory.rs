//! Multiboot2 physical memory map detection.

use crate::{
    mm::bootinfo::{self, MemoryRegion, RegionKind},
    serial_println,
};

const MULTIBOOT2_TAG_MEMORY_MAP: u32 = 6;
const MULTIBOOT2_TAG_MODULE: u32 = 3;
const MAX_MEMORY_REGIONS: usize = 64;

pub struct BootMemoryMap {
    pub regions: &'static [MemoryRegion],
    pub boot_info_start: u64,
    pub boot_info_len: u64,
    pub user_module_start: u64,
    pub user_module_len: u64,
}

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

fn region_kind(entry_type: u32) -> RegionKind {
    match entry_type {
        1 => RegionKind::Usable,
        3 => RegionKind::AcpiReclaim,
        4 => RegionKind::AcpiNvs,
        5 => RegionKind::Bad,
        _ => RegionKind::Reserved,
    }
}

fn entry_type_name(entry_type: u32) -> &'static str {
    match entry_type {
        1 => "usable",
        2 => "reserved",
        3 => "ACPI reclaimable",
        4 => "ACPI NVS",
        5 => "bad RAM",
        _ => "unknown",
    }
}

/// Parse and print the Multiboot2 memory map.
///
/// The returned map is page-aligned. The frame allocator further restricts
/// usable spans to fully contained pages and to the boot identity-map limit.
pub fn init(boot_info_address: u32) -> Result<BootMemoryMap, &'static str> {
    if boot_info_address == 0 {
        return Err("null Multiboot2 info address");
    }

    // Multiboot2 places this structure below 4 GiB, which boot.S identity maps.
    let base = boot_info_address as u64 as *const u8;
    let total_size =
        unsafe { core::ptr::read_unaligned(base.cast::<BootInfoHeader>()) }.total_size as usize;
    if total_size < core::mem::size_of::<BootInfoHeader>() {
        return Err("invalid Multiboot2 info size");
    }
    if (boot_info_address as u64).saturating_add(total_size as u64) > (1u64 << 32) {
        return Err("Multiboot2 info structure exceeds the boot identity map");
    }

    let mut raw = [(0, 0, RegionKind::Reserved); MAX_MEMORY_REGIONS];
    let mut raw_count = 0usize;
    let mut offset = 8usize;
    let mut found_map = false;
    let mut user_module = None;

    while offset.checked_add(8).is_some_and(|end| end <= total_size) {
        let tag = unsafe { core::ptr::read_unaligned(base.add(offset).cast::<TagHeader>()) };
        let tag_size = tag.size as usize;
        if tag_size < 8 || tag_size > total_size - offset {
            return Err("invalid Multiboot2 tag size");
        }

        if tag.tag_type == 0 {
            break;
        }

        if tag.tag_type == MULTIBOOT2_TAG_MEMORY_MAP {
            if tag_size < 16 {
                return Err("invalid Multiboot2 memory map tag");
            }
            let entry_size =
                unsafe { core::ptr::read_unaligned(base.add(offset + 8).cast::<u32>()) } as usize;
            if entry_size < core::mem::size_of::<MemoryMapEntry>() {
                return Err("invalid Multiboot2 memory map entry size");
            }

            let entries_size = tag_size - 16;
            if entries_size % entry_size != 0 {
                return Err("truncated Multiboot2 memory map entries");
            }
            let entry_count = entries_size / entry_size;
            if entry_count > MAX_MEMORY_REGIONS {
                return Err("memory map exceeds supported region count");
            }

            let mut usable_mib = 0u64;
            serial_println!("[MEM] BIOS physical memory map ({} entries)", entry_count);
            for i in 0..entry_count {
                let entry_ptr = unsafe { base.add(offset + 16 + i * entry_size) };
                let entry =
                    unsafe { core::ptr::read_unaligned(entry_ptr.cast::<MemoryMapEntry>()) };

                if entry.entry_type == 1 {
                    usable_mib = usable_mib.saturating_add(entry.length >> 20);
                }
                serial_println!(
                    "[MEM]   {:#014x} - {:#014x}  {:>5} MiB  {}",
                    entry.base_addr,
                    entry.base_addr.saturating_add(entry.length),
                    entry.length >> 20,
                    entry_type_name(entry.entry_type)
                );

                raw[i] = (entry.base_addr, entry.length, region_kind(entry.entry_type));
            }

            raw_count = entry_count;
            found_map = true;
            serial_println!("[MEM] total usable RAM: {} MiB", usable_mib);
        } else if tag.tag_type == MULTIBOOT2_TAG_MODULE {
            if tag_size < 17 {
                return Err("invalid Multiboot2 module tag");
            }
            let module_start =
                unsafe { core::ptr::read_unaligned(base.add(offset + 8).cast::<u32>()) };
            let module_end =
                unsafe { core::ptr::read_unaligned(base.add(offset + 12).cast::<u32>()) };
            let command =
                unsafe { core::slice::from_raw_parts(base.add(offset + 16), tag_size - 16) };
            let command_len = command
                .iter()
                .position(|byte| *byte == 0)
                .ok_or("unterminated Multiboot2 module command line")?;

            if &command[..command_len] == b"user_init" {
                if user_module.is_some() {
                    return Err("multiple user_init Multiboot2 modules");
                }
                if module_start >= module_end || u64::from(module_end) > (1u64 << 32) {
                    return Err("invalid user_init Multiboot2 module range");
                }
                user_module = Some((module_start, module_end));
            }
        }

        let next_offset = tag_size
            .checked_add(7)
            .map(|size| size & !7)
            .and_then(|size| offset.checked_add(size))
            .ok_or("Multiboot2 tag offset overflow")?;
        if next_offset <= offset {
            return Err("invalid Multiboot2 tag alignment");
        }
        offset = next_offset;
    }

    if !found_map {
        return Err("Multiboot2 memory map tag not found");
    }
    let (module_start, module_end) = user_module.ok_or("user_init Multiboot2 module not found")?;

    Ok(BootMemoryMap {
        regions: bootinfo::normalize(&raw[..raw_count]),
        boot_info_start: boot_info_address as u64,
        boot_info_len: total_size as u64,
        user_module_start: u64::from(module_start),
        user_module_len: u64::from(module_end - module_start),
    })
}
