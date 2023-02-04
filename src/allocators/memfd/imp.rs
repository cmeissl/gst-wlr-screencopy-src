use std::os::unix::io::IntoRawFd;

use gstreamer::glib;
use gstreamer::prelude::Cast;
use gstreamer::subclass::prelude::*;
use gstreamer_allocators::{subclass::prelude::FdAllocatorImpl, FdAllocator, FdMemoryFlags};

#[derive(Debug)]
pub struct MemfdMemoryAllocator {
    mem_fd_opts: memfd::MemfdOptions,
}

impl Default for MemfdMemoryAllocator {
    fn default() -> Self {
        Self {
            mem_fd_opts: memfd::MemfdOptions::default()
                .allow_sealing(true)
                .close_on_exec(true),
        }
    }
}

#[glib::object_subclass]
impl ObjectSubclass for MemfdMemoryAllocator {
    const NAME: &'static str = "MemfdMemoryAllocator";
    type Type = super::MemfdMemoryAllocator;
    type ParentType = FdAllocator;
    type Interfaces = ();
}

impl ObjectImpl for MemfdMemoryAllocator {}

impl GstObjectImpl for MemfdMemoryAllocator {}

impl AllocatorImpl for MemfdMemoryAllocator {
    fn alloc(
        &self,
        size: usize,
        _params: Option<&gstreamer::AllocationParams>,
    ) -> Result<gstreamer::Memory, glib::BoolError> {
        let obj = self.obj();
        let fd_allocator: &FdAllocator = obj.upcast_ref();

        let mem_fd = self
            .mem_fd_opts
            .create("gst-shm-memory-allocator")
            .expect("failed to create memfd");

        mem_fd
            .as_file()
            .set_len(size as u64)
            .expect("failed to set size");

        let mut seals = memfd::SealsHashSet::new();
        seals.insert(memfd::FileSeal::SealShrink);
        let _ = mem_fd.add_seals(&seals);
        let _ = mem_fd.add_seal(memfd::FileSeal::SealSeal);

        // FIXME: if alloc fails we will have a dangling fd
        unsafe {
            FdAllocator::alloc(
                fd_allocator,
                mem_fd.into_raw_fd(),
                size,
                FdMemoryFlags::NONE,
            )
        }
    }

    fn free(&self, memory: gstreamer::Memory) {
        self.parent_free(memory)
    }
}

impl FdAllocatorImpl for MemfdMemoryAllocator {}
