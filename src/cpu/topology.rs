use crate::cpu::detection::CpuFeatures;

pub const MAX_CPUS: usize = 64;

#[derive(Clone, Copy, Debug)]
pub struct CpuTopology {
    apic_ids: [u32; MAX_CPUS],
    pub cpu_count: usize,
    pub bsp_index: usize,
    pub bsp_apic_id: u32,
    pub local_apic_address: u64,
    pub from_acpi: bool,
    pub unsupported_cpu_count: usize,
}

impl CpuTopology {
    fn bsp_only(bsp_apic_id: u32, local_apic_address: u64) -> Self {
        let mut apic_ids = [u32::MAX; MAX_CPUS];
        apic_ids[0] = bsp_apic_id;
        Self {
            apic_ids,
            cpu_count: 1,
            bsp_index: 0,
            bsp_apic_id,
            local_apic_address,
            from_acpi: false,
            unsupported_cpu_count: 0,
        }
    }

    pub fn apic_ids(&self) -> &[u32] {
        &self.apic_ids[..self.cpu_count]
    }

    pub fn logical_id(&self, apic_id: u32) -> Option<usize> {
        self.apic_ids().iter().position(|id| *id == apic_id)
    }

    fn add_apic_id(&mut self, apic_id: u32) -> bool {
        if self.apic_ids().contains(&apic_id) {
            return true;
        }
        if self.cpu_count == MAX_CPUS {
            return false;
        }
        self.apic_ids[self.cpu_count] = apic_id;
        self.cpu_count += 1;
        true
    }
}

pub static TOPOLOGY: spin::Once<CpuTopology> = spin::Once::new();

const MULTIBOOT2_TAG_ACPI_OLD: u32 = 14;
const MULTIBOOT2_TAG_ACPI_NEW: u32 = 15;
const MULTIBOOT2_TAG_END: u32 = 0;
const LAPIC_ADDRESS_DEFAULT: u64 = kelvos_shared_ring::layout::LAPIC_DEFAULT;

#[repr(C)]
struct TagHeader {
    tag_type: u32,
    size: u32,
}

fn checksum_is_valid(bytes: &[u8]) -> bool {
    bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)) == 0
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn find_rsdp(boot_info: *const u8, total_size: usize) -> Option<&'static [u8]> {
    let mut offset = 8usize;
    while offset.checked_add(8)? <= total_size {
        let header =
            unsafe { core::ptr::read_unaligned(boot_info.add(offset).cast::<TagHeader>()) };
        let size = usize::try_from(header.size).ok()?;
        if size < 8 || size > total_size - offset {
            return None;
        }
        if header.tag_type == MULTIBOOT2_TAG_END {
            break;
        }
        if header.tag_type == MULTIBOOT2_TAG_ACPI_NEW || header.tag_type == MULTIBOOT2_TAG_ACPI_OLD
        {
            let bytes = unsafe { core::slice::from_raw_parts(boot_info.add(offset + 8), size - 8) };
            if bytes.len() >= 20
                && bytes.get(..8) == Some(b"RSD PTR ")
                && checksum_is_valid(&bytes[..20])
            {
                if header.tag_type == MULTIBOOT2_TAG_ACPI_NEW {
                    if bytes.len() < 36 {
                        return None;
                    }
                    let length = usize::try_from(read_u32(bytes, 20)?).ok()?;
                    if length < 36 || length > bytes.len() || !checksum_is_valid(&bytes[..length]) {
                        return None;
                    }
                    return Some(&bytes[..length]);
                }
                return Some(&bytes[..20]);
            }
        }
        offset = offset.checked_add(size.checked_add(7)? & !7)?;
    }
    None
}

fn table_checksum_is_valid(address: u64, length: usize) -> bool {
    let Some(end) = address.checked_add(length as u64) else {
        return false;
    };
    if end > kelvos_shared_ring::layout::BOOT_IDENTITY_END {
        return false;
    }
    let bytes = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
    checksum_is_valid(bytes)
}

fn parse_madt(address: u64, bsp_apic_id: u32) -> Option<CpuTopology> {
    if address == 0 || address.checked_add(44)? > kelvos_shared_ring::layout::BOOT_IDENTITY_END {
        return None;
    }
    let header = unsafe { core::slice::from_raw_parts(address as *const u8, 44) };
    if header.get(..4)? != b"APIC" {
        return None;
    }
    let length = usize::try_from(read_u32(header, 4)?).ok()?;
    if length < 44 || !table_checksum_is_valid(address, length) {
        return None;
    }

    let bytes = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
    let mut topology = CpuTopology::bsp_only(bsp_apic_id, read_u32(bytes, 36)? as u64);
    let mut offset = 44usize;
    while offset.checked_add(2)? <= length {
        let entry_type = *bytes.get(offset)?;
        let entry_length = usize::from(*bytes.get(offset + 1)?);
        if entry_length < 2 || entry_length > length - offset {
            return None;
        }

        match entry_type {
            0 if entry_length >= 8 => {
                let apic_id = u32::from(*bytes.get(offset + 3)?);
                let flags = read_u32(bytes, offset + 4)?;
                if flags & 0b11 != 0 && !topology.add_apic_id(apic_id) {
                    topology.unsupported_cpu_count += 1;
                }
            }
            0 => return None,
            5 if entry_length >= 12 => {
                topology.local_apic_address = read_u64(bytes, offset + 4)?;
            }
            5 => return None,
            9 if entry_length >= 16 => {
                let apic_id = read_u32(bytes, offset + 4)?;
                let flags = read_u32(bytes, offset + 8)?;
                if flags & 0b11 != 0 {
                    if apic_id > u8::MAX.into() || !topology.add_apic_id(apic_id) {
                        topology.unsupported_cpu_count += 1;
                    }
                }
            }
            9 => return None,
            _ => {}
        }
        offset += entry_length;
    }
    if offset != length {
        return None;
    }

    if topology.local_apic_address == 0
        || topology.local_apic_address & 0xfff != 0
        || topology.local_apic_address >= kelvos_shared_ring::layout::BOOT_IDENTITY_END
    {
        return None;
    }

    let bsp_position = topology.logical_id(bsp_apic_id)?;
    if bsp_position != 0 {
        topology.apic_ids.swap(0, bsp_position);
    }
    topology.bsp_index = 0;
    topology.bsp_apic_id = bsp_apic_id;
    topology.from_acpi = true;
    Some(topology)
}

fn parse_root_table(address: u64, bsp_apic_id: u32) -> Option<CpuTopology> {
    if address == 0 || address.checked_add(36)? > kelvos_shared_ring::layout::BOOT_IDENTITY_END {
        return None;
    }
    let header = unsafe { core::slice::from_raw_parts(address as *const u8, 36) };
    let entry_width = match header.get(..4)? {
        b"XSDT" => 8,
        b"RSDT" => 4,
        _ => return None,
    };
    let length = usize::try_from(read_u32(header, 4)?).ok()?;
    if length < 36
        || address.checked_add(length as u64)? > kelvos_shared_ring::layout::BOOT_IDENTITY_END
        || (length - 36) % entry_width != 0
        || !table_checksum_is_valid(address, length)
    {
        return None;
    }
    let bytes = unsafe { core::slice::from_raw_parts(address as *const u8, length) };
    for entry in bytes[36..].chunks_exact(entry_width) {
        let table_address = if entry_width == 8 {
            u64::from_le_bytes(entry.try_into().ok()?)
        } else {
            u64::from(u32::from_le_bytes(entry.try_into().ok()?))
        };
        if let Some(topology) = parse_madt(table_address, bsp_apic_id) {
            return Some(topology);
        }
    }
    None
}

fn detect(boot_info_address: u32, features: CpuFeatures) -> CpuTopology {
    let bsp_apic_id = features.initial_apic_id;
    if !features.local_apic {
        return CpuTopology::bsp_only(bsp_apic_id, LAPIC_ADDRESS_DEFAULT);
    }
    let boot_info = boot_info_address as u64 as *const u8;
    let total_size = unsafe { core::ptr::read_unaligned(boot_info.cast::<u32>()) } as usize;

    if total_size >= 8
        && (boot_info_address as u64).saturating_add(total_size as u64)
            <= kelvos_shared_ring::layout::BOOT_IDENTITY_END
    {
        if let Some(rsdp) = find_rsdp(boot_info, total_size) {
            let revision = rsdp.get(15).copied().unwrap_or(0);
            let xsdt = (revision >= 2)
                .then(|| read_u64(rsdp, 24).unwrap_or(0))
                .unwrap_or(0);
            let rsdt = u64::from(read_u32(rsdp, 16).unwrap_or(0));
            for root_address in [xsdt, rsdt] {
                if let Some(topology) = parse_root_table(root_address, bsp_apic_id) {
                    return topology;
                }
            }
        }
    }

    CpuTopology::bsp_only(bsp_apic_id, LAPIC_ADDRESS_DEFAULT)
}

pub fn init(boot_info_address: u32) {
    TOPOLOGY.call_once(|| detect(boot_info_address, CpuFeatures::detect()));
}
