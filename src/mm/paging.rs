use x86_64::{
    VirtAddr,
    registers::control::Cr3,
    structures::paging::{
        Mapper, OffsetPageTable, Page, PageTable, PageTableFlags, PhysFrame, Size4KiB,
    },
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

    pub unsafe fn map_page(
        &mut self,
        page: Page<Size4KiB>,
        frame: PhysFrame<Size4KiB>,
        flags: PageTableFlags,
        allocator: &mut FrameAllocator,
    ) -> Result<(), &'static str> {
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
        let page_count = length.div_ceil(4096);
        let flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE | PageTableFlags::NO_EXECUTE;

        for i in 0..page_count {
            let page = Page::containing_address(start + (i * 4096) as u64);
            let frame = allocator.alloc_frame().ok_or("frame allocation failed")?;
            unsafe { self.mapper.map_to(page, frame, flags, allocator) }
                .map_err(|_| "page mapping failed")?
                .flush();
        }
        Ok(())
    }

    pub unsafe fn map_user_range(
        &mut self,
        start: VirtAddr,
        length: usize,
        allocator: &mut FrameAllocator,
    ) -> Result<(), &'static str> {
        let page_count = length.div_ceil(4096);
        let flags = PageTableFlags::PRESENT
            | PageTableFlags::WRITABLE
            | PageTableFlags::USER_ACCESSIBLE;

        for i in 0..page_count {
            let page = Page::containing_address(start + (i * 4096) as u64);
            let frame = allocator.alloc_frame().ok_or("frame allocation failed")?;
            unsafe { self.mapper.map_to(page, frame, flags, allocator) }
                .map_err(|_| "user page mapping failed")?
                .flush();
        }
        Ok(())
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