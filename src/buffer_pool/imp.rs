use std::sync::{Arc, Mutex};

use gstreamer::glib;
use gstreamer::prelude::Cast;
use gstreamer::subclass::prelude::*;

use gstreamer_video::VideoInfo;
use once_cell::sync::Lazy;
use wayland_client::backend::{ObjectData, ObjectId};
use wayland_client::protocol::wl_shm;
use wayland_client::{Proxy, WEnum};

use crate::allocators::{GbmMemoryAllocator, MemfdMemoryAllocator};

static CAT: Lazy<gstreamer::DebugCategory> = Lazy::new(|| {
    gstreamer::DebugCategory::new(
        "waylandbufferpool",
        gstreamer::DebugColorFlags::empty(),
        Some("Wayland Buffer Pool"),
    )
});

#[derive(Debug, Default)]
pub struct State {
    pub zwp_linux_dmabuf: Option<
        wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
    >,
    pub dmabuf_formats: Vec<gstreamer_video::VideoFormat>,
    pub wl_shm: Option<wayland_client::protocol::wl_shm::WlShm>,
    video_info: Option<VideoInfo>,
    allocator: Option<gstreamer::Allocator>,
}

#[derive(Debug)]
pub struct WaylandBufferPool {
    pub state: Mutex<State>,
    dummy_object_data: Arc<DummyObjectData>,
}

impl Default for WaylandBufferPool {
    fn default() -> Self {
        Self {
            state: Default::default(),
            dummy_object_data: DummyObjectData::new(),
        }
    }
}

#[glib::object_subclass]
impl ObjectSubclass for WaylandBufferPool {
    const NAME: &'static str = "WaylandBufferPool";
    type Type = super::WaylandBufferPool;
    type ParentType = gstreamer::BufferPool;
    type Interfaces = ();
}

impl ObjectImpl for WaylandBufferPool {}

impl GstObjectImpl for WaylandBufferPool {}

impl BufferPoolImpl for WaylandBufferPool {
    fn alloc_buffer(
        &self,
        params: Option<&gstreamer::BufferPoolAcquireParams>,
    ) -> Result<gstreamer::Buffer, gstreamer::FlowError> {
        let state = self.state.lock().unwrap();
        let video_info = state.video_info.as_ref().unwrap();
        let allocator = state.allocator.as_ref().unwrap();

        if let Some(gbm_allocator) = allocator.downcast_ref::<GbmMemoryAllocator>() {
            let mem = match gbm_allocator.alloc(video_info) {
                Ok(mem) => mem,
                Err(_) => {
                    return Err(gstreamer::FlowError::Error);
                }
            };

            let dmabuf_memory = mem
                .downcast_memory_ref::<gstreamer_allocators::DmaBufMemory>()
                .unwrap();
            let zwp_linux_dmabuf = state.zwp_linux_dmabuf.as_ref().unwrap();

            let params = zwp_linux_dmabuf.send_constructor::<wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1>(wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::Request::CreateParams {  }, self.dummy_object_data.clone()).expect("failed to create params");
            let modifier = u64::from(gbm::Modifier::Linear);
            let modifier_hi = (modifier >> 32) as u32;
            let modifier_lo = modifier as u32;
            params.add(
                dmabuf_memory.fd(),
                0,
                0,
                video_info.stride()[0] as u32,
                modifier_hi,
                modifier_lo,
            );

            let format = match video_info.format() {
                gstreamer_video::VideoFormat::Abgr => drm_fourcc::DrmFourcc::Abgr8888,
                gstreamer_video::VideoFormat::Argb => drm_fourcc::DrmFourcc::Argb8888,
                gstreamer_video::VideoFormat::Bgra => drm_fourcc::DrmFourcc::Bgra8888,
                gstreamer_video::VideoFormat::Bgrx => drm_fourcc::DrmFourcc::Bgrx8888,
                gstreamer_video::VideoFormat::Rgba => drm_fourcc::DrmFourcc::Rgba8888,
                gstreamer_video::VideoFormat::Rgbx => drm_fourcc::DrmFourcc::Rgbx8888,
                gstreamer_video::VideoFormat::Xbgr => drm_fourcc::DrmFourcc::Xbgr8888,
                gstreamer_video::VideoFormat::Xrgb => drm_fourcc::DrmFourcc::Xrgb8888,
                _ => {
                    params.destroy();
                    return Err(gstreamer::FlowError::Error);
                }
            };
            let wl_buffer = params.send_constructor::<wayland_client::protocol::wl_buffer::WlBuffer>(
                wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::Request::CreateImmed { 
                    width: video_info.width() as i32,
                    height: video_info.height() as i32,
                    format: format as u32,
                    flags: WEnum::Value(wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::Flags::empty())
                }, 
                self.dummy_object_data.clone()).expect("failed to create buffer");
            params.destroy();

            let mut buffer = gstreamer::Buffer::new();
            let buffer_mut = buffer.make_mut();
            super::meta::WaylandBufferMeta::add(buffer_mut, wl_buffer);
            buffer_mut.insert_memory(None, mem);
            buffer_mut.unset_flags(gstreamer::BufferFlags::TAG_MEMORY);
            return Ok(buffer);
        }

        // if we got here we have an fd based allocator
        let mut buffer = self.parent_alloc_buffer(params)?;
        let mem = buffer.memory(0).unwrap();

        if let Some(fd_memory) = mem.downcast_memory_ref::<gstreamer_allocators::FdMemory>() {
            let wl_shm = state.wl_shm.as_ref().unwrap();
            let pool = wl_shm
                .send_constructor::<wayland_client::protocol::wl_shm_pool::WlShmPool>(
                    wayland_client::protocol::wl_shm::Request::CreatePool {
                        fd: fd_memory.fd(),
                        size: buffer.size() as i32,
                    },
                    self.dummy_object_data.clone(),
                )
                .expect("failed to create pool");

            let format = match video_info.format() {
                gstreamer_video::VideoFormat::Abgr => wl_shm::Format::Abgr8888,
                gstreamer_video::VideoFormat::Argb => wl_shm::Format::Argb8888,
                gstreamer_video::VideoFormat::Bgra => wl_shm::Format::Bgra8888,
                gstreamer_video::VideoFormat::Bgrx => wl_shm::Format::Bgrx8888,
                gstreamer_video::VideoFormat::Rgba => wl_shm::Format::Rgba8888,
                gstreamer_video::VideoFormat::Rgbx => wl_shm::Format::Rgbx8888,
                gstreamer_video::VideoFormat::Xbgr => wl_shm::Format::Xbgr8888,
                gstreamer_video::VideoFormat::Xrgb => wl_shm::Format::Xrgb8888,
                _ => {
                    pool.destroy();
                    return Err(gstreamer::FlowError::Error);
                }
            };

            let wl_buffer = pool
                .send_constructor::<wayland_client::protocol::wl_buffer::WlBuffer>(
                    wayland_client::protocol::wl_shm_pool::Request::CreateBuffer {
                        offset: 0,
                        width: video_info.width() as i32,
                        height: video_info.height() as i32,
                        stride: video_info.stride()[0],
                        format: wayland_client::WEnum::Value(format),
                    },
                    self.dummy_object_data.clone(),
                )
                .expect("failed to create buffer");
            super::meta::WaylandBufferMeta::add(buffer.make_mut(), wl_buffer);
            pool.destroy();
            return Ok(buffer);
        }

        Err(gstreamer::FlowError::Error)
    }

    fn set_config(&self, config: &mut gstreamer::BufferPoolConfigRef) -> bool {
        let (caps, size, min_buffers, max_buffers) = match config.params() {
            Some(params) => params,
            None => {
                gstreamer::warning!(CAT, imp: self, "no params");
                return false;
            }
        };

        let caps = match caps {
            Some(caps) => caps,
            None => {
                gstreamer::warning!(CAT, imp: self, "no caps config");
                return false;
            }
        };

        let video_info = match VideoInfo::from_caps(&caps) {
            Ok(info) => info,
            Err(err) => {
                gstreamer::warning!(
                    CAT,
                    imp: self,
                    "failed to get video info from caps: {}",
                    err
                );
                return false;
            }
        };

        let video_info_size = video_info.size();
        if (size as usize) < video_info_size {
            gstreamer::warning!(
                CAT,
                imp: self,
                "provided size is to small for the caps {} < {}",
                size,
                video_info_size,
            );
            return false;
        }

        let mut guard = self.state.lock().unwrap();

        let allocator: gstreamer::Allocator = if guard.dmabuf_formats.contains(&video_info.format())
            && guard.zwp_linux_dmabuf.is_some()
        {
            GbmMemoryAllocator::default().upcast()
        } else {
            MemfdMemoryAllocator::default().upcast()
        };

        guard.video_info = Some(video_info);

        config.set_allocator(Some(&allocator), None);
        config.set_params(
            Some(&caps),
            video_info_size as u32,
            min_buffers,
            max_buffers,
        );

        guard.allocator = Some(allocator);

        self.parent_set_config(config)
    }

    fn free_buffer(&self, buffer: gstreamer::Buffer) {
        if let Some(wayland_buffer_meta) = buffer.meta::<super::meta::WaylandBufferMeta>() {
            wayland_buffer_meta.wl_buffer().destroy();
        }
    }
}

#[derive(Debug)]
struct DummyObjectData;

impl DummyObjectData {
    fn new() -> Arc<Self> {
        Arc::new(DummyObjectData)
    }
}

impl ObjectData for DummyObjectData {
    fn event(
        self: Arc<Self>,
        _backend: &wayland_client::backend::Backend,
        _msg: wayland_client::backend::protocol::Message<
            ObjectId,
            wayland_client::backend::io_lifetimes::OwnedFd,
        >,
    ) -> Option<Arc<dyn ObjectData>> {
        None
    }

    fn destroyed(&self, _object_id: ObjectId) {}
}
