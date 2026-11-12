//! Boot-time loader for fixed-address, static ELF64 user programs.

use alloc::vec::Vec;
use kelvos_shared_ring::{
    USER_IMAGE_ADDRESS, USER_IMAGE_CAPACITY, USER_STACK_SIZE, USER_STACK_TOP,
};
use x86_64::{
    structures::paging::{Page, PageTableFlags as Flags, PhysFrame, Size4KiB},
    VirtAddr,
};

use crate::mm::{frame::FrameAllocator, paging::PagingManager};

const PAGE_SIZE: usize = kelvos_shared_ring::layout::PAGE_SIZE as usize;
const ELF_HEADER_SIZE: usize = 64;
const PROGRAM_HEADER_SIZE: usize = 56;

/// A private address space and its initial userspace execution parameters.
pub struct LoadedImage {
    pub root: PhysFrame<Size4KiB>,
    pub entry: usize,
    pub stack_top: u64,
    writable_pages: Vec<ResetPage>,
}

struct ResetPage {
    frame: PhysFrame<Size4KiB>,
    initial: &'static [u8],
}

impl LoadedImage {
    /// # Safety
    /// The address space must be inactive and unreachable from any user thread.
    pub unsafe fn reset(&self) {
        for page in &self.writable_pages {
            let destination = page.frame.start_address().as_u64() as *mut u8;
            unsafe {
                destination.write_bytes(0, PAGE_SIZE);
                core::ptr::copy_nonoverlapping(
                    page.initial.as_ptr(),
                    destination,
                    page.initial.len(),
                );
            }
        }
    }
}

pub struct UserImage<'a> {
    bytes: &'a [u8],
    headers: &'a [u8],
    entry: usize,
}

impl<'a> UserImage<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, &'static str> {
        if bytes.len() < ELF_HEADER_SIZE
            || bytes[..7] != *b"\x7fELF\x02\x01\x01"
            || word(bytes, 16) != 2
            || word(bytes, 18) != 62
            || dword(bytes, 20) != 1
            || word(bytes, 52) as usize != ELF_HEADER_SIZE
            || word(bytes, 54) as usize != PROGRAM_HEADER_SIZE
        {
            return Err("expected a static x86-64 ELF executable");
        }
        let entry = qword(bytes, 24) as usize;
        let start = qword(bytes, 32) as usize;
        let size = word(bytes, 56) as usize * PROGRAM_HEADER_SIZE;
        let headers = start
            .checked_add(size)
            .and_then(|end| bytes.get(start..end))
            .ok_or("truncated ELF program headers")?;
        let mut mapped = [false; USER_IMAGE_CAPACITY / PAGE_SIZE];
        let mut entry_is_executable = false;
        for header in headers.chunks_exact(PROGRAM_HEADER_SIZE) {
            match dword(header, 0) {
                1 => {}
                2 | 3 => return Err("dynamic ELF images are unsupported"),
                _ => continue,
            }
            let segment = Segment::parse(bytes, header)?;
            if segment.memory_len == 0 {
                continue;
            }
            let first_page = (segment.address - USER_IMAGE_ADDRESS as usize) / PAGE_SIZE;
            let page_count = segment.memory_len.div_ceil(PAGE_SIZE);
            for page in &mut mapped[first_page..first_page + page_count] {
                if *page {
                    return Err("ELF load segments overlap a page");
                }
                *page = true;
            }
            entry_is_executable |= segment.executable
                && (segment.address..segment.address + segment.file.len()).contains(&entry);
        }
        if !entry_is_executable {
            return Err("ELF entry is outside executable file data");
        }
        Ok(Self {
            bytes,
            headers,
            entry,
        })
    }

    /// # Safety
    /// Use the active boot mapper before AP startup. A load error is fatal
    /// during boot because partially mapped frames are retained by the allocator.
    pub unsafe fn instantiate(
        &self,
        kernel_paging: &mut PagingManager,
        allocator: &mut FrameAllocator,
    ) -> Result<LoadedImage, &'static str>
    where
        'a: 'static,
    {
        let mut writable_pages = Vec::new();
        let (root, mut paging) = unsafe { kernel_paging.new_user(allocator)? };
        for header in self.headers.chunks_exact(PROGRAM_HEADER_SIZE) {
            if dword(header, 0) != 1 {
                continue;
            }
            let segment = Segment::parse(self.bytes, header)?;
            let mut flags = Flags::PRESENT | Flags::USER_ACCESSIBLE;
            flags.set(Flags::WRITABLE, segment.writable);
            flags.set(Flags::NO_EXECUTE, !segment.executable);
            for offset in (0..segment.memory_len).step_by(PAGE_SIZE) {
                let frame = allocator.alloc_frame().ok_or("out of user image frames")?;
                let destination = frame.start_address().as_u64() as *mut u8;
                let data = &segment.file[offset.min(segment.file.len())..];
                let initial = &data[..data.len().min(PAGE_SIZE)];
                unsafe {
                    // Access the new frame through the supervisor-only boot mapping.
                    destination.write_bytes(0, PAGE_SIZE);
                    core::ptr::copy_nonoverlapping(initial.as_ptr(), destination, initial.len());
                    let address = VirtAddr::new((segment.address + offset) as u64);
                    paging.map_page(Page::containing_address(address), frame, flags, allocator)?;
                    kernel_paging.protect_user_frame_alias(frame, segment.writable, allocator)?;
                }
                if segment.writable {
                    writable_pages.push(ResetPage { frame, initial });
                }
            }
        }
        let stack_flags =
            Flags::PRESENT | Flags::WRITABLE | Flags::USER_ACCESSIBLE | Flags::NO_EXECUTE;
        for offset in (0..USER_STACK_SIZE).step_by(PAGE_SIZE) {
            let frame = allocator.alloc_frame().ok_or("out of user stack frames")?;
            let address = VirtAddr::new(USER_STACK_TOP - USER_STACK_SIZE as u64 + offset as u64);
            unsafe {
                (frame.start_address().as_u64() as *mut u8).write_bytes(0, PAGE_SIZE);
                paging.map_page(
                    Page::containing_address(address),
                    frame,
                    stack_flags,
                    allocator,
                )?;
                kernel_paging.protect_user_frame_alias(frame, true, allocator)?;
            }
            writable_pages.push(ResetPage {
                frame,
                initial: &[],
            });
        }
        Ok(LoadedImage {
            root,
            entry: self.entry,
            stack_top: USER_STACK_TOP,
            writable_pages,
        })
    }
}

struct Segment<'a> {
    address: usize,
    memory_len: usize,
    file: &'a [u8],
    writable: bool,
    executable: bool,
}

impl<'a> Segment<'a> {
    fn parse(bytes: &'a [u8], header: &[u8]) -> Result<Self, &'static str> {
        let flags = dword(header, 4);
        let offset = qword(header, 8) as usize;
        let address = qword(header, 16) as usize;
        let file_len = qword(header, 32) as usize;
        let memory_len = qword(header, 40) as usize;
        let base = USER_IMAGE_ADDRESS as usize;
        let writable = flags & 2 != 0;
        let executable = flags & 1 != 0;
        if flags & !7 != 0
            || flags & 4 == 0
            || (writable && executable)
            || address % PAGE_SIZE != 0
            || offset % PAGE_SIZE != 0
            || address < base
            || address > base + USER_IMAGE_CAPACITY
            || memory_len > base + USER_IMAGE_CAPACITY - address
            || file_len > memory_len
        {
            return Err("invalid ELF load segment");
        }
        let file = offset
            .checked_add(file_len)
            .and_then(|end| bytes.get(offset..end))
            .ok_or("truncated ELF load segment")?;
        Ok(Self {
            address,
            memory_len,
            file,
            writable,
            executable,
        })
    }
}

fn word(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn dword(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn qword(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
