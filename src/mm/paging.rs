use x86_64::{
    registers::control::Cr3,
    structures::paging::{
        Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
    },
    VirtAddr,
};

use crate::mm::frame::FrameAllocator;

pub static PHYS_OFFSET: spin::Once<VirtAddr> = spin::Once::new();

pub struct PagingManager {
    mapper: OffsetPageTable<'static>,
}

impl PagingManager {
    /// Initialize the current CRT page table in identity-mapped mode.
    /// The current boot path identity-maps the first 4 GiB, so a zero offset is
    /// valid for early kernel setup and simplified table walks.
    pub unsafe fn new() -> Self {
        let phys_offset = VirtAddr::new(0);
        let mapper = unsafe { init(phys_offset) };
        Self { mapper }
    }

    /// Map one virtual page to a caller-selected physical frame.
    ///
    /// # Safety
    /// The caller must ensure the mapping does not invalidate live references
    /// or create invalid aliases, and that the flags are safe for the mapping.
    pub unsafe fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        frame: PhysFrame<Size4KiB>,
        flags: PageTableFlags,
        allocator: &mut FrameAllocator,
    ) -> Result<(), &'static str> {
        if !flags.contains(PageTableFlags::PRESENT) {
            return Err("mapped pages must be present");
        }
        unsafe { self.mapper.map_to(page, frame, flags, allocator) }
            .map_err(|_| "page mapping failed")?
            .flush();
        Ok(())
    }

    pub unsafe fn map_range(
        &mut self,
        start: VirtAddr,
        length: usize,
        allocator: &mut FrameAllocator,
    ) -> Result<(), &'static str> {
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;
        unsafe { self.map_range_with_flags(start, length, flags, allocator) }
    }

    /// Map a virtual range to consecutive physical frames with the supplied flags.
    ///
    /// The physical frames remain owned by the caller. The virtual start must
    /// be page-aligned; a partial final page is mapped in full.
    ///
    /// # Safety
    /// The caller must ensure that these physical frames can safely be mapped
    /// at the requested virtual addresses and with the requested permissions.
    pub unsafe fn map_range_to(
        &mut self,
        start: VirtAddr,
        first_frame: PhysFrame<Size4KiB>,
        length: usize,
        flags: PageTableFlags,
        allocator: &mut FrameAllocator,
    ) -> Result<(), &'static str> {
        let page_count = Self::validated_page_count(start, length, flags)?;
        if page_count == 0 {
            return Ok(());
        }

        let first_pfn = first_frame.start_address().as_u64() / 4096;
        let last_pfn = first_pfn
            .checked_add((page_count - 1) as u64)
            .ok_or("physical mapping range overflow")?;
        PhysFrame::<Size4KiB>::try_from_pfn(last_pfn)
            .map_err(|_| "invalid physical frame in mapping range")?;

        for i in 0..page_count {
            let page = Page::containing_address(start + (i * 4096) as u64);
            let frame = PhysFrame::try_from_pfn(first_pfn + i as u64)
                .map_err(|_| "invalid physical frame in mapping range")?;
            unsafe { self.map_page(page, frame, flags, allocator)? };
        }
        Ok(())
    }

    /// Allocate frames and map them with the supplied page-table flags.
    ///
    /// # Safety
    /// The caller must ensure that the requested virtual range is safe to map.
    pub unsafe fn map_range_with_flags(
        &mut self,
        start: VirtAddr,
        length: usize,
        flags: PageTableFlags,
        allocator: &mut FrameAllocator,
    ) -> Result<(), &'static str> {
        let page_count = Self::validated_page_count(start, length, flags)?;
        for i in 0..page_count {
            let page = Page::containing_address(start + (i * 4096) as u64);
            let frame = allocator.alloc_frame().ok_or("frame allocation failed")?;
            if let Err(error) = unsafe { self.map_page(page, frame, flags, allocator) } {
                allocator.free_frame(frame);
                return Err(error);
            }
        }
        Ok(())
    }

    pub unsafe fn map_user_range(
        &mut self,
        start: VirtAddr,
        length: usize,
        allocator: &mut FrameAllocator,
    ) -> Result<(), &'static str> {
        let flags =
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::USER_ACCESSIBLE;
        unsafe { self.map_range_with_flags(start, length, flags, allocator) }
    }

    fn validated_page_count(
        start: VirtAddr,
        length: usize,
        flags: PageTableFlags,
    ) -> Result<usize, &'static str> {
        if start.as_u64() % 4096 != 0 {
            return Err("virtual mapping start is not page-aligned");
        }
        if !flags.contains(PageTableFlags::PRESENT) {
            return Err("mapped pages must be present");
        }
        if length == 0 {
            return Ok(0);
        }

        let page_count = (length - 1) / 4096 + 1;
        let mapped_size = page_count
            .checked_mul(4096)
            .ok_or("virtual mapping range overflow")?;
        let last_address = start
            .as_u64()
            .checked_add(mapped_size as u64 - 1)
            .ok_or("virtual mapping range overflow")?;
        VirtAddr::try_new(last_address).map_err(|_| "virtual mapping range is not canonical")?;
        if start.as_u64() >> 47 != last_address >> 47 {
            return Err("virtual mapping range crosses the canonical address gap");
        }

        Ok(page_count)
    }
}

/// # Safety
/// `phys_offset` must be the HHDM base; caller must not create a second
/// live `&mut PageTable` for the same CR3.
pub unsafe fn init(phys_offset: VirtAddr) -> OffsetPageTable<'static> {
    PHYS_OFFSET.call_once(|| phys_offset);
    let (l4_frame, _) = Cr3::read();
    let l4_phys = l4_frame.start_address();
    let l4_virt = phys_offset + l4_phys.as_u64();
    let l4 = unsafe { &mut *(l4_virt.as_mut_ptr::<PageTable>()) };
    unsafe { OffsetPageTable::new(l4, phys_offset) }
}
