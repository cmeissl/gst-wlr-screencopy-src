use std::os::fd::AsRawFd;
use std::os::unix::io::{AsFd, BorrowedFd};
use std::sync::Mutex;

use gstreamer::glib;
use gstreamer::prelude::{Cast, ParamSpecBuilderExt, ToValue};
use gstreamer::subclass::prelude::*;
use gstreamer_allocators::subclass::prelude::*;
use gstreamer_allocators::DmaBufAllocator;
use nix::unistd;
use once_cell::sync::Lazy;

use crate::utils::gst_video_format_to_drm_fourcc;

/// A simple wrapper for a device node.
#[derive(Debug)]
pub struct Card(std::fs::File);

/// Implementing [`AsFd`] is a prerequisite to implementing the traits found
/// in this crate. Here, we are just calling [`File::as_fd()`] on the inner
/// [`File`].
impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

/// Simple helper methods for opening a `Card`.
impl Card {
    pub fn open(path: &str) -> Self {
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        options.write(true);
        Card(options.open(path).unwrap())
    }
}

#[derive(Debug, Default)]
struct Settings {
    device_path: Option<String>,
}

#[derive(Debug, Default)]
pub struct GbmMemoryAllocator {
    settings: Mutex<Settings>,
    device: Mutex<Option<gbm::Device<Card>>>,
}

impl GbmMemoryAllocator {
    pub fn alloc(
        &self,
        video_info: &gstreamer_video::VideoInfo,
    ) -> Result<gstreamer::Memory, glib::BoolError> {
        let obj = self.obj();
        let dmabuf_allocator: &DmaBufAllocator = obj.upcast_ref();

        let guard = self.device.lock().unwrap();
        let device = guard.as_ref().unwrap();

        let Some(format) = gst_video_format_to_drm_fourcc(video_info.format()) else {
            unreachable!()
        };

        let bo = device
            .create_buffer_object_with_modifiers2::<()>(
                video_info.width(),
                video_info.height(),
                format,
                [gbm::Modifier::Linear].into_iter(),
                gbm::BufferObjectFlags::RENDERING,
            )
            .expect("failed to create bo");
        let fd = bo.fd().expect("no fd");

        let fd_size = unistd::lseek(fd.as_raw_fd(), 0, unistd::Whence::SeekEnd).unwrap();
        let _ = unistd::lseek(fd.as_raw_fd(), 0, unistd::Whence::SeekSet);

        if (fd_size as usize) < video_info.size() {
            panic!("bo too small");
        }

        let memory = unsafe {
            dmabuf_allocator
                .alloc(fd, video_info.size())
                .expect("failed to allocate dmabuf memory")
        };

        Ok(memory)
    }
}

#[glib::object_subclass]
impl ObjectSubclass for GbmMemoryAllocator {
    const NAME: &'static str = "GbmMemoryAllocator";
    type Type = super::GbmMemoryAllocator;
    type ParentType = DmaBufAllocator;
    type Interfaces = ();
}

impl ObjectImpl for GbmMemoryAllocator {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
            vec![glib::ParamSpecString::builder("device")
                .nick("drm device")
                .blurb("device path to allocator buffers from")
                .default_value("/dev/dri/renderD128")
                .construct()
                .build()]
        });

        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        match pspec.name() {
            "device" => {
                let mut settings = self.settings.lock().unwrap();
                let device_path = value
                    .get::<Option<String>>()
                    .expect("type checked upstream");
                settings.device_path = device_path;
            }
            _ => unreachable!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "device" => {
                let settings = self.settings.lock().unwrap();
                settings.device_path.to_value()
            }
            _ => unreachable!(),
        }
    }

    fn constructed(&self) {
        let device_path = self.settings.lock().unwrap().device_path.clone().unwrap();
        *self.device.lock().unwrap() = Some(gbm::Device::new(Card::open(&device_path)).unwrap());
    }
}

impl GstObjectImpl for GbmMemoryAllocator {}

impl DmaBufAllocatorImpl for GbmMemoryAllocator {}
impl FdAllocatorImpl for GbmMemoryAllocator {}
impl AllocatorImpl for GbmMemoryAllocator {}
