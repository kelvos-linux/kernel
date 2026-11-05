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
    pub len: u64,
    pub kind: RegionKind,
}

pub struct BootInfo {
    pub regions: &'static [MemoryRegion],
    pub physmem_offset: u64,
    pub kernel_start: u64,
    pub kernel_end: u64,
    pub lapic_ids: &'static [u32],
}

/// Normalize bootloader regions to page boundaries and merge adjacent usable spans.
pub fn normalize(raw: &[(u64, u64, RegionKind)]) -> &'static [MemoryRegion] {
    static mut NORMALIZED: [MemoryRegion; 64] = [MemoryRegion {
        start: 0,
        len: 0,
        kind: RegionKind::Reserved,
    }; 64];

    unsafe {
        let slice = &mut NORMALIZED[..];
        let mut count = 0usize;

        for &(start, len, kind) in raw {
            if count == slice.len() {
                panic!("memory map exceeds supported region count");
            }

            let region_end = start.saturating_add(len);
            let (aligned_start, aligned_end) = if kind == RegionKind::Usable {
                (start.saturating_add(0xFFF) & !0xFFF, region_end & !0xFFF)
            } else {
                (start & !0xFFF, region_end.saturating_add(0xFFF) & !0xFFF)
            };
            let aligned_len = aligned_end.saturating_sub(aligned_start);

            if aligned_len == 0 {
                continue;
            }

            slice[count] = MemoryRegion {
                start: aligned_start,
                len: aligned_len,
                kind,
            };
            count += 1;
        }

        for i in 1..count {
            let mut j = i;
            while j > 0 && slice[j].start < slice[j - 1].start {
                slice.swap(j, j - 1);
                j -= 1;
            }
        }

        let mut merged = 0usize;
        for i in 0..count {
            let current = slice[i];
            if merged > 0 {
                let previous = &mut slice[merged - 1];
                if previous.kind == RegionKind::Usable
                    && current.kind == RegionKind::Usable
                    && previous.start.saturating_add(previous.len) == current.start
                {
                    previous.len = previous.len.saturating_add(current.len);
                    continue;
                }
            }
            slice[merged] = current;
            merged += 1;
        }

        &slice[..merged]
    }
}
