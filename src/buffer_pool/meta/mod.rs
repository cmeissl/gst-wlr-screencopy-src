use gstreamer::{glib, MetaAPI};
use wayland_client::protocol::wl_buffer::WlBuffer;

mod imp;

#[repr(transparent)]
pub struct WaylandBufferMeta(imp::WaylandBufferMeta);

unsafe impl Send for WaylandBufferMeta {}
unsafe impl Sync for WaylandBufferMeta {}

impl WaylandBufferMeta {
    // Add a new custom meta to the buffer with the given label.
    pub fn add(
        buffer: &mut gstreamer::BufferRef,
        wl_buffer: WlBuffer,
    ) -> gstreamer::MetaRefMut<Self, gstreamer::meta::Standalone> {
        unsafe {
            // Manually dropping because gst_buffer_add_meta() takes ownership of the
            // content of the struct.
            let mut params = std::mem::ManuallyDrop::new(imp::CustomMetaParams { wl_buffer });

            // The label is passed through via the params to custom_meta_init().
            let meta = gstreamer::ffi::gst_buffer_add_meta(
                buffer.as_mut_ptr(),
                imp::custom_meta_get_info(),
                &mut *params as *mut imp::CustomMetaParams as glib::ffi::gpointer,
            ) as *mut imp::WaylandBufferMeta;

            Self::from_mut_ptr(buffer, meta)
        }
    }

    // Retrieve the stored [`WlBuffer`].
    #[doc(alias = "get_dma_buffer")]
    pub fn wl_buffer(&self) -> &WlBuffer {
        &self.0.wl_buffer
    }
}

// Trait to allow using the gst::Buffer API with this meta.
unsafe impl MetaAPI for WaylandBufferMeta {
    type GstType = imp::WaylandBufferMeta;

    fn meta_api() -> glib::Type {
        imp::custom_meta_api_get_type()
    }
}

impl std::fmt::Debug for WaylandBufferMeta {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("WaylandBufferMeta")
            .field("wl_buffer", &self.0.wl_buffer)
            .finish()
    }
}
