#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegionKind {
    Usable,
    Reserved,
    AcpiReclaim,
    AcpiNvs,
    KernelImage,
    Framebuffer,
    Bad,
}

#[derive(Clone, Copy, Debug)]
pub struct MemoryRegion {
    pub start: u64,
    pub len:   u64,
    pub kind:  RegionKind,
}

pub struct BootInfo {
    pub regions:        &'static [MemoryRegion],
    pub physmem_offset: u64,
    pub kernel_start:   u64,
    pub kernel_end:     u64,
    pub lapic_ids:      &'static [u32],
}

/// Normalize bootloader regions: page-align, merge adjacent Usable spans.
pub fn normalize(raw: &[(u64, u64, RegionKind)]) -> &'static [MemoryRegion] {
    static mut NORMALIZED: [MemoryRegion; 64] = [
        MemoryRegion { start: 0, len: 0, kind: RegionKind::Reserved }; 64
    ];

    unsafe {
        let slice = &mut NORMALIZED[..];
        let mut count = 0usize;

        for &(start, len, kind) in raw {
            if count >= slice.len() {
                break;
            }

            let aligned_start = start & !0xFFF;
            let aligned_end = (start + len + 0xFFF) & !0xFFF;
            let aligned_len = aligned_end.saturating_sub(aligned_start);

            if aligned_len == 0 {
                continue;
            }

            slice[count] = MemoryRegion { start: aligned_start, len: aligned_len, kind };
            count += 1;
        }

        &slice[..count]
    }
}