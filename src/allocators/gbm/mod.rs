use std::path::Path;

use gstreamer::{glib, subclass::prelude::ObjectSubclassIsExt};

mod imp;

glib::wrapper! {
    pub struct GbmMemoryAllocator(ObjectSubclass<imp::GbmMemoryAllocator>) @extends gstreamer_allocators::DmaBufAllocator, gstreamer_allocators::FdAllocator, gstreamer::Allocator, gstreamer::Object;
}

impl GbmMemoryAllocator {
    pub fn new<P: AsRef<Path>>(device_path: Option<P>) -> Self {
        let device_path = device_path.map(|p| p.as_ref().to_str().unwrap().to_string());
        glib::Object::new(&[("device", &device_path)])
    }

    pub fn alloc(
        &self,
        video_info: &gstreamer_video::VideoInfo,
    ) -> Result<gstreamer::Memory, glib::BoolError> {
        self.imp().alloc(video_info)
    }
}

impl Default for GbmMemoryAllocator {
    fn default() -> Self {
        glib::Object::new(&[])
    }
}
