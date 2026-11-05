use core::sync::atomic::{AtomicBool, Ordering};

use crate::mm::bootinfo::{MemoryRegion, RegionKind};
use x86_64::{
    structures::paging::{FrameAllocator as X86FrameAllocator, PhysFrame, Size4KiB},
    PhysAddr,
};

pub const PAGE_SIZE: u64 = 4096;
const MAX_PHYSICAL_ADDRESS: u64 = 1 << 32;
const MAX_FRAMES: usize = (MAX_PHYSICAL_ADDRESS / PAGE_SIZE) as usize;
const BITMAP_WORDS: usize = MAX_FRAMES.div_ceil(u64::BITS as usize);

static mut MANAGED_BITMAP: [u64; BITMAP_WORDS] = [0; BITMAP_WORDS];
static mut FREE_BITMAP: [u64; BITMAP_WORDS] = [0; BITMAP_WORDS];
static INITIALIZED: AtomicBool = AtomicBool::new(false);

pub struct FrameAllocator {
    managed_bitmap: &'static mut [u64],
    free_bitmap: &'static mut [u64],
    frame_limit: usize,
    total_frames: usize,
    free_frames: usize,
    hint: usize,
}

unsafe impl X86FrameAllocator<Size4KiB> for FrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.alloc_frame()
    }
}

impl FrameAllocator {
    /// Build a frame allocator from usable memory-map regions.
    ///
    /// Only fully usable pages below the boot identity-map limit are managed.
    /// `reserved_ranges` should include boot data and other memory not covered
    /// by the kernel image symbols.
    pub fn new(
        regions: &[MemoryRegion],
        reserved_ranges: &[(u64, u64)],
    ) -> Result<Self, &'static str> {
        INITIALIZED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "physical frame allocator is already initialized")?;

        let frame_limit = regions
            .iter()
            .map(|region| {
                region
                    .start
                    .saturating_add(region.len)
                    .min(MAX_PHYSICAL_ADDRESS)
            })
            .max()
            .unwrap_or(0)
            .div_ceil(PAGE_SIZE) as usize;
        if frame_limit == 0 {
            return Err("memory map contains no addressable frames");
        }

        let managed_bitmap = unsafe { &mut MANAGED_BITMAP[..] };
        let free_bitmap = unsafe { &mut FREE_BITMAP[..] };
        managed_bitmap.fill(0);
        free_bitmap.fill(0);

        let mut allocator = Self {
            managed_bitmap,
            free_bitmap,
            frame_limit,
            total_frames: 0,
            free_frames: 0,
            hint: 0,
        };

        for region in regions
            .iter()
            .filter(|region| region.kind == RegionKind::Usable)
        {
            allocator.mark_usable_range(region.start, region.len);
        }

        for region in regions
            .iter()
            .filter(|region| region.kind != RegionKind::Usable)
        {
            allocator.reserve_range(region.start, region.len);
        }
        allocator.reserve_range(0, 0x10_0000);
        for &(start, len) in reserved_ranges {
            allocator.reserve_range(start, len);
        }

        if allocator.free_frames == 0 {
            return Err("memory map contains no allocatable frames");
        }
        Ok(allocator)
    }

    fn bitmap_bit(frame: usize) -> (usize, u64) {
        (
            frame / u64::BITS as usize,
            1u64 << (frame % u64::BITS as usize),
        )
    }

    fn mark_usable_range(&mut self, start: u64, len: u64) {
        let first = start.saturating_add(PAGE_SIZE - 1) / PAGE_SIZE;
        let end = start.saturating_add(len) / PAGE_SIZE;
        let first = (first as usize).min(self.frame_limit);
        let end = (end as usize).min(self.frame_limit);

        for frame in first..end {
            let (word, mask) = Self::bitmap_bit(frame);
            if self.managed_bitmap[word] & mask == 0 {
                self.managed_bitmap[word] |= mask;
                self.free_bitmap[word] |= mask;
                self.total_frames += 1;
                self.free_frames += 1;
            }
        }
    }

    fn reserve_range(&mut self, start: u64, len: u64) {
        if len == 0 {
            return;
        }
        let first = (start / PAGE_SIZE) as usize;
        let end = start
            .saturating_add(len)
            .saturating_add(PAGE_SIZE - 1)
            .checked_div(PAGE_SIZE)
            .unwrap_or(u64::MAX) as usize;
        let end = end.min(self.frame_limit);

        for frame in first.min(end)..end {
            let (word, mask) = Self::bitmap_bit(frame);
            if self.managed_bitmap[word] & mask != 0 {
                self.managed_bitmap[word] &= !mask;
                if self.free_bitmap[word] & mask != 0 {
                    self.free_bitmap[word] &= !mask;
                    self.free_frames -= 1;
                }
                self.total_frames -= 1;
            }
        }
    }

    fn is_managed(&self, frame: usize) -> bool {
        if frame >= self.frame_limit {
            return false;
        }
        let (word, mask) = Self::bitmap_bit(frame);
        self.managed_bitmap[word] & mask != 0
    }

    fn is_free(&self, frame: usize) -> bool {
        if frame >= self.frame_limit {
            return false;
        }
        let (word, mask) = Self::bitmap_bit(frame);
        self.free_bitmap[word] & mask != 0
    }

    fn set_used(&mut self, frame: usize) {
        let (word, mask) = Self::bitmap_bit(frame);
        if self.free_bitmap[word] & mask != 0 {
            self.free_bitmap[word] &= !mask;
            self.free_frames -= 1;
        }
    }

    fn find_free(&self, start: usize, end: usize) -> Option<usize> {
        let mut word_index = start / u64::BITS as usize;
        let last_word = end.div_ceil(u64::BITS as usize);

        while word_index < last_word {
            let word_start = word_index * u64::BITS as usize;
            let lower_bit = start.saturating_sub(word_start);
            let upper_bit = (end - word_start).min(u64::BITS as usize);
            let lower_mask = u64::MAX << lower_bit;
            let upper_mask = if upper_bit == u64::BITS as usize {
                u64::MAX
            } else {
                (1u64 << upper_bit) - 1
            };
            let available = self.free_bitmap[word_index] & lower_mask & upper_mask;
            if available != 0 {
                return Some(word_start + available.trailing_zeros() as usize);
            }
            word_index += 1;
        }
        None
    }

    pub fn alloc_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let frame = self
            .find_free(self.hint.min(self.frame_limit), self.frame_limit)
            .or_else(|| self.find_free(0, self.hint.min(self.frame_limit)))?;
        self.set_used(frame);
        self.hint = if frame + 1 == self.frame_limit {
            0
        } else {
            frame + 1
        };
        Some(PhysFrame::containing_address(PhysAddr::new(
            frame as u64 * PAGE_SIZE,
        )))
    }

    /// Return a previously allocated frame to the allocator.
    ///
    /// Frames outside usable memory, reserved ranges, and duplicate frees are
    /// ignored.
    pub fn free_frame(&mut self, frame: PhysFrame<Size4KiB>) {
        let index = (frame.start_address().as_u64() / PAGE_SIZE) as usize;
        if !self.is_managed(index) || self.is_free(index) {
            return;
        }

        let (word, mask) = Self::bitmap_bit(index);
        self.free_bitmap[word] |= mask;
        self.free_frames += 1;
        self.hint = self.hint.min(index);
    }

    pub fn free_frames(&self) -> usize {
        self.free_frames
    }

    pub fn total_frames(&self) -> usize {
        self.total_frames
    }
}

pub type Frame = PhysFrame<Size4KiB>;
