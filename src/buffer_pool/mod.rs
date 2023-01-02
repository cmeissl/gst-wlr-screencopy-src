use gstreamer::{glib, subclass::prelude::ObjectSubclassIsExt};

mod imp;
mod meta;

pub use meta::WaylandBufferMeta;

glib::wrapper! {
    pub struct WaylandBufferPool(ObjectSubclass<imp::WaylandBufferPool>) @extends gstreamer::BufferPool, gstreamer::Object;
}

impl WaylandBufferPool {
    pub fn new(
        wl_shm: &wayland_client::protocol::wl_shm::WlShm,
        zwp_linux_dmabuf: Option<&wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
        dmabuf_formats: impl IntoIterator<Item = gstreamer_video::VideoFormat>,
    ) -> Self {
        let obj: WaylandBufferPool = glib::Object::new(&[]);
        let imp = obj.imp();
        let mut guard = imp.state.lock().unwrap();
        guard.wl_shm = Some(wl_shm.clone());
        guard.zwp_linux_dmabuf = zwp_linux_dmabuf.cloned();
        guard.dmabuf_formats = dmabuf_formats.into_iter().collect();
        std::mem::drop(guard);
        obj
    }
}
