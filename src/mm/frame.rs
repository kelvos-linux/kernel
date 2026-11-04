use crate::mm::bootinfo::{BootInfo, RegionKind};
use x86_64::{
    structures::paging::{FrameAllocator as X86FrameAllocator, PhysFrame, Size4KiB},
    PhysAddr,
};

pub const PAGE_SIZE: u64 = 4096;

static mut FRAME_BITMAP: [u64; 16384] = [0; 16384];

pub struct FrameAllocator {
    bitmap: &'static mut [u64],
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
    pub fn new_default() -> Self {
        let total = (1 << 20) as usize;
        let words = total.div_ceil(64).max(1);
        let bitmap = unsafe { &mut FRAME_BITMAP[..words] };

        let mut alloc = Self {
            bitmap,
            total_frames: total,
            free_frames: 0,
            hint: 0,
        };

        alloc.bitmap.fill(u64::MAX);
        alloc.free_frames = total;
        alloc
    }

    pub fn new(boot: &BootInfo) -> Self {
        let max_phys = boot
            .regions
            .iter()
            .map(|r| r.start.saturating_add(r.len))
            .max()
            .unwrap_or(0);

        let total = (max_phys / PAGE_SIZE) as usize;
        let words = total.div_ceil(64).max(1);
        let bitmap = unsafe { &mut FRAME_BITMAP[..words] };

        let mut alloc = Self {
            bitmap,
            total_frames: total,
            free_frames: 0,
            hint: 0,
        };

        alloc.bitmap.fill(u64::MAX);

        for region in boot.regions.iter().filter(|r| r.kind == RegionKind::Usable) {
            alloc.mark_range_free(region.start, region.len);
        }

        if boot.kernel_start < boot.kernel_end {
            alloc.mark_range_used(boot.kernel_start, boot.kernel_end - boot.kernel_start);
        }

        alloc
    }

    fn mark_range_free(&mut self, start: u64, len: u64) {
        let first = (start / PAGE_SIZE) as usize;
        let last = ((start + len + PAGE_SIZE - 1) / PAGE_SIZE) as usize;
        for i in first..last.min(self.total_frames) {
            self.set_free(i);
        }
    }

    fn mark_range_used(&mut self, start: u64, len: u64) {
        let first = (start / PAGE_SIZE) as usize;
        let last = ((start + len + PAGE_SIZE - 1) / PAGE_SIZE) as usize;
        for i in first..last.min(self.total_frames) {
            self.set_used(i);
        }
    }

    fn bit_index(frame: usize) -> (usize, u64) {
        (frame / 64, 1u64 << (frame % 64))
    }

    fn is_free(&self, frame: usize) -> bool {
        if frame >= self.total_frames {
            return false;
        }
        let (word_index, mask) = Self::bit_index(frame);
        (self.bitmap[word_index] & mask) != 0
    }

    fn set_free(&mut self, frame: usize) {
        if frame >= self.total_frames {
            return;
        }
        let (word_index, mask) = Self::bit_index(frame);
        self.bitmap[word_index] |= mask;
        self.free_frames += 1;
    }

    fn set_used(&mut self, frame: usize) {
        if frame >= self.total_frames {
            return;
        }
        let (word_index, mask) = Self::bit_index(frame);
        self.bitmap[word_index] &= !mask;
        self.free_frames = self.free_frames.saturating_sub(1);
    }

    pub fn alloc_frame(&mut self) -> Option<PhysFrame> {
        let start = self.hint.min(self.total_frames);
        for i in start..self.total_frames {
            if self.is_free(i) {
                self.set_used(i);
                self.hint = i + 1;
                return Some(PhysFrame::containing_address(PhysAddr::new(
                    (i as u64) * PAGE_SIZE,
                )));
            }
        }

        for i in 0..start {
            if self.is_free(i) {
                self.set_used(i);
                self.hint = i + 1;
                return Some(PhysFrame::containing_address(PhysAddr::new(
                    (i as u64) * PAGE_SIZE,
                )));
            }
        }

        None
    }

    pub fn free_frame(&mut self, f: PhysFrame) {
        let frame = (f.start_address().as_u64() / PAGE_SIZE) as usize;
        if frame < self.total_frames {
            self.set_free(frame);
        }
    }

    pub fn free_frames(&self) -> usize {
        self.free_frames
    }
    pub fn total_frames(&self) -> usize {
        self.total_frames
    }
}

pub type Frame = PhysFrame<Size4KiB>;
