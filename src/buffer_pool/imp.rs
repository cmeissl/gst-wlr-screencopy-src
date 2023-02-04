use std::sync::{Arc, Mutex};

use gstreamer::glib;
use gstreamer::prelude::Cast;
use gstreamer::subclass::prelude::*;

use gstreamer_video::{VideoInfo, VideoBufferPoolConfig};
use once_cell::sync::Lazy;
use wayland_client::backend::{ObjectData, ObjectId};
use wayland_client::{Proxy, WEnum};

use crate::allocators::{GbmMemoryAllocator, MemfdMemoryAllocator};
use crate::utils::{gst_video_format_to_wl_shm, gst_video_format_to_drm_fourcc};

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
    pub wl_shm: Option<wayland_client::protocol::wl_shm::WlShm>,
    video_info: Option<VideoInfo>,
    allocator: Option<gstreamer::Allocator>,
    allocation_params: Option<Option<gstreamer::AllocationParams>>,
    add_video_meta: bool,
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
    fn options() -> &'static [&'static str] {
        static OPTIONS: Lazy<Vec<&'static str>> = Lazy::new(|| vec![&*gstreamer_video::BUFFER_POOL_OPTION_VIDEO_META, &*gstreamer_video::BUFFER_POOL_OPTION_VIDEO_ALIGNMENT]);

        OPTIONS.as_ref()
    }

    fn alloc_buffer(
        &self,
        params: Option<&gstreamer::BufferPoolAcquireParams>,
    ) -> Result<gstreamer::Buffer, gstreamer::FlowError> {
        let state = self.state.lock().unwrap();
        let video_info = state.video_info.as_ref().unwrap();
        let allocator = state.allocator.as_ref().unwrap();

        let mut buffer = if let Some(gbm_allocator) = allocator.downcast_ref::<GbmMemoryAllocator>() {
            let mem = match gbm_allocator.alloc(video_info) {
                Ok(mem) => mem,
                Err(_) => {
                    return Err(gstreamer::FlowError::Error);
                }
            };

            let mut buffer = gstreamer::Buffer::new();
            let buffer_mut = buffer.make_mut();
            buffer_mut.insert_memory(None, mem);
            buffer
        } else {
            self.parent_alloc_buffer(params)?
        };

        let mem = buffer.memory(0).unwrap();

        if mem.downcast_memory_ref::<gstreamer_allocators::DmaBufMemory>().is_some() {
            let zwp_linux_dmabuf = state.zwp_linux_dmabuf.as_ref().unwrap();

            let params = zwp_linux_dmabuf.send_constructor::<wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_buffer_params_v1::ZwpLinuxBufferParamsV1>(wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::Request::CreateParams {  }, self.dummy_object_data.clone()).expect("failed to create params");
            
            for plane in 0..video_info.n_planes() {
                let offset = video_info.offset()[plane as usize];
                let stride= video_info.stride()[plane as usize];

                let (mem_idx, _, skip) = buffer.find_memory(offset, Some(1)).expect("memory does not seem to contain enough data for the specified format");
                let mem = buffer.peek_memory(mem_idx).downcast_memory_ref::<gstreamer_allocators::DmaBufMemory>().unwrap();
                params.add(
                    mem.fd(),
                    plane,
                    (mem.offset() + skip) as u32,
                    stride as u32,
                    0,
                    0,
                );
            }

            let Some(format) = gst_video_format_to_drm_fourcc(video_info.format()) else {
                params.destroy();
                return Err(gstreamer::FlowError::Error);
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

            let buffer_mut = buffer.make_mut();
            super::meta::WaylandBufferMeta::add(buffer_mut, wl_buffer);
            if state.add_video_meta {
                gstreamer_video::VideoMeta::add_full(
                    buffer_mut,
                    gstreamer_video::VideoFrameFlags::empty(),
                    video_info.format(),
                    video_info.width(),
                    video_info.height(),
                    video_info.offset(),
                    video_info.stride(),
                ).map_err(|err| {
                    gstreamer::warning!(CAT, imp: self, "failed to add video meta: {:?}", err);
                    gstreamer::FlowError::Error
                })?;
            }
            buffer_mut.unset_flags(gstreamer::BufferFlags::TAG_MEMORY);

            return Ok(buffer);
        }

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

            let Some(format) = gst_video_format_to_wl_shm(video_info.format()) else {
                pool.destroy();
                    return Err(gstreamer::FlowError::Error);
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
            pool.destroy();
            
            let buffer_mut = buffer.make_mut();
            super::meta::WaylandBufferMeta::add(buffer_mut, wl_buffer);
            if state.add_video_meta {
                gstreamer_video::VideoMeta::add_full(
                    buffer_mut,
                    gstreamer_video::VideoFrameFlags::empty(),
                    video_info.format(),
                    video_info.width(),
                    video_info.height(),
                    video_info.offset(),
                    video_info.stride(),
                ).map_err(|err| {
                    gstreamer::warning!(CAT, imp: self, "failed to add video meta: {:?}", err);
                    gstreamer::FlowError::Error
                })?;
            }
            buffer_mut.unset_flags(gstreamer::BufferFlags::TAG_MEMORY);
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

        let mut video_info = match VideoInfo::from_caps(&caps) {
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
                
        let (allocator, mut allocation_params) = if let Some((allocator, allocation_params)) = config.allocator() {
            let allocator = allocator.unwrap_or_else(|| MemfdMemoryAllocator::default().upcast());
            (allocator, Some(allocation_params))
        } else {
            (MemfdMemoryAllocator::default().upcast(), None)
        };

        let mut guard = self.state.lock().unwrap();
        guard.add_video_meta = config.has_option(gstreamer_video::BUFFER_POOL_OPTION_VIDEO_META.as_ref());
        let need_alignment = config.has_option(gstreamer_video::BUFFER_POOL_OPTION_VIDEO_ALIGNMENT.as_ref());

        if need_alignment && guard.add_video_meta {
            let video_align = config.video_alignment();

            if let Some(video_align) = video_align {
                let align = allocation_params.as_ref().map(|params| params.align()).unwrap_or_default();
                let mut max_align = align;

                for plane in 0..video_info.n_planes() {
                    max_align |= unsafe { *video_align.stride_align().get_unchecked(plane as usize) as usize };
                }

                let mut stride_align: [u32; gstreamer_video::ffi::GST_VIDEO_MAX_PLANES as usize] = [0; gstreamer_video::ffi::GST_VIDEO_MAX_PLANES as usize];
                for plane in 0..video_info.n_planes() {
                    stride_align[plane as usize] = max_align as u32;
                }

                let mut video_align = gstreamer_video::VideoAlignment::new(video_align.padding_top(), video_align.padding_bottom(), video_align.padding_left(), video_align.padding_right(), &stride_align);
                if let Err(err) = video_info.align(&mut video_align) {
                    gstreamer::warning!(CAT, imp: self, "failed to align video info: {}", err);
                    return false;
                }

                config.set_video_alignment(&video_align);

                if align < max_align {
                    gstreamer::warning!(CAT, imp: self, "allocation params alignment {} is smaller than the max specified video stride alignment {}, fixing", align, max_align);
                    allocation_params = allocation_params.as_ref().map(|params| gstreamer::AllocationParams::new(params.flags(), max_align, params.prefix(), params.padding()));
                    config.set_allocator(Some(&allocator), allocation_params.as_ref());
                }
            }
        }

        let size = std::cmp::max(size, video_info.size() as u32);
        guard.video_info = Some(video_info);

        config.set_params(
            Some(&caps),
            size,
            min_buffers,
            max_buffers,
        );

        guard.allocator = Some(allocator);
        guard.allocation_params = Some(allocation_params);

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
