use gstreamer::glib;

mod imp;

glib::wrapper! {
    pub struct MemfdMemoryAllocator(ObjectSubclass<imp::MemfdMemoryAllocator>) @extends gstreamer_allocators::FdAllocator, gstreamer::Allocator, gstreamer::Object;
}

impl Default for MemfdMemoryAllocator {
    fn default() -> Self {
        glib::Object::new()
    }
}
