use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

use gstreamer::prelude::{Cast, ParamSpecBuilderExt, ToValue};
use gstreamer_base::traits::BaseSrcExt;
use gstreamer_video::VideoBufferPoolConfig;
use once_cell::sync::Lazy;

use gstreamer::subclass::prelude::*;
use gstreamer::{glib, prelude::BufferPoolExtManual};
use gstreamer_base::subclass::prelude::*;

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::{protocol::wl_registry, Connection, Dispatch, Proxy};
use wayland_client::{QueueHandle, Weak};

use crate::allocators::{DmaHeapMemoryAllocator, GbmMemoryAllocator, MemfdMemoryAllocator};
use crate::buffer_pool::{WaylandBufferMeta, WaylandBufferPool};
use crate::utils::{
    gst_video_format_from_drm_fourcc, gst_video_format_from_wl_shm, gst_video_format_to_drm_fourcc,
    gst_video_format_to_wl_shm,
};

static CAT: Lazy<gstreamer::DebugCategory> = Lazy::new(|| {
    gstreamer::DebugCategory::new(
        "wlrscreencopysrc",
        gstreamer::DebugColorFlags::empty(),
        Some("wlr-screencopy src"),
    )
});

#[derive(Debug, Default)]
struct Settings {
    wayland_display: Option<String>,
    output_name: Option<String>,
}

#[derive(Debug, Default)]
struct Mode {
    width: i32,
    height: i32,
    refresh: i32,
}

#[derive(Debug, Default)]
struct OutputInfo {
    name: String,
    description: String,
    mode: Mode,
    done: bool,
}

#[derive(Debug)]
struct FrameShmFormat {
    format: wayland_client::protocol::wl_shm::Format,
    width: u32,
    height: u32,
    stride: u32,
}

#[derive(Debug)]
struct FrameDmabufFormat {
    format: drm_fourcc::DrmFourcc,
    width: u32,
    height: u32,
}

#[derive(Debug)]
enum FrameState {
    Ready(std::time::Duration),
    Failed,
}

#[derive(Debug, Default)]
struct FrameInfo {
    shm_formats: Vec<FrameShmFormat>,
    dmabuf_formats: Vec<FrameDmabufFormat>,
    done: bool,
    state: Option<FrameState>,
    flags: Option<wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Flags>,
}

#[derive(Debug)]
struct WaylandState {
    wl_shm: wayland_client::protocol::wl_shm::WlShm,
    dmabuf: Option<wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1>,
    wlr_screencopy_manager: wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
    outputs: Vec<(wayland_client::protocol::wl_output::WlOutput, Option<wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::ZxdgOutputV1>, OutputInfo)>,
    current_frame: Option<(wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, FrameInfo)>,

    qhandle: QueueHandle<WaylandState>,
}

impl Dispatch<wayland_client::protocol::wl_output::WlOutput, ()> for WaylandState {
    fn event(
        state: &mut Self,
        proxy: &wayland_client::protocol::wl_output::WlOutput,
        event: <wayland_client::protocol::wl_output::WlOutput as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        let (_, _, output_info) = state
            .outputs
            .iter_mut()
            .find(|(output, _, _)| output == proxy)
            .expect("non existing output");

        match event {
            wayland_client::protocol::wl_output::Event::Geometry { .. } => {}
            wayland_client::protocol::wl_output::Event::Mode {
                flags,
                width,
                height,
                refresh,
            } => {
                if let Ok(flags) = flags.into_result() {
                    if flags.contains(wayland_client::protocol::wl_output::Mode::Current) {
                        output_info.mode.width = width;
                        output_info.mode.height = height;
                        output_info.mode.refresh = refresh;
                    }
                }
            }
            wayland_client::protocol::wl_output::Event::Done => {
                output_info.done = true;
            }
            wayland_client::protocol::wl_output::Event::Scale { .. } => {}
            wayland_client::protocol::wl_output::Event::Name { name } => output_info.name = name,
            wayland_client::protocol::wl_output::Event::Description { description } => {
                output_info.description = description
            }
            _ => unreachable!(),
        }
    }
}

impl Dispatch<wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1,
        _event: <wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        // No events to handle
    }
}

impl Dispatch<wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        proxy: &wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: <wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &wayland_client::QueueHandle<Self>,
    ) {
        let (frame, frame_info) = state.current_frame.as_mut().expect("no frame");

        if frame != proxy {
            panic!("wrong frame");
        }

        match event {
            wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event::Buffer { format, width, height, stride } => {
                // TODO: Figure out how to handle the stride, currently we use the stride as defined by gstreamer for a format
                if let Ok(format) = format.into_result() {
                    frame_info.shm_formats.push(FrameShmFormat { format, width, height, stride });
                }
            },
            wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event::Flags { flags } => {
                let flags = flags.into_result().unwrap();
                frame_info.flags = Some(flags);
            },
            wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event::Ready { tv_sec_hi, tv_sec_lo, tv_nsec } => {
                let secs = (tv_sec_hi as u64) << 32 | tv_sec_lo as u64;
                frame_info.state = Some(FrameState::Ready(std::time::Duration::new(secs, tv_nsec)));
            },
            wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event::Failed => {
                frame_info.state = Some(FrameState::Failed);
            },
            wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event::Damage { .. } => {},
            wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event::LinuxDmabuf { format, width, height } => {
                if let Ok(format) = drm_fourcc::DrmFourcc::try_from(format) {
                    frame_info.dmabuf_formats.push(FrameDmabufFormat { format, width, height });
                }
            },
            wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::Event::BufferDone =>  frame_info.done = true,
            _ => todo!(),
        }
    }
}

#[derive(Debug, Default)]
pub struct WlrScreencopySrc {
    settings: Mutex<Settings>,
    wayland_state: Mutex<Option<WaylandState>>,
    _connection: Mutex<Option<wayland_client::Connection>>,
    event_queue: Mutex<Option<wayland_client::EventQueue<WaylandState>>>,
}

impl wayland_client::Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandState {
    fn event(
        _state: &mut WaylandState,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<WaylandState>,
    ) {
    }
}

impl wayland_client::Dispatch<wayland_client::protocol::wl_shm::WlShm, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_client::protocol::wl_shm::WlShm,
        _event: <wayland_client::protocol::wl_shm::WlShm as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // We completely ignore the formats and rely on the compositor to only send
        // shm frame formats it supports
    }
}

impl
    wayland_client::Dispatch<
        wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        (),
    > for WaylandState
{
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _event: <wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        // We completely ignore the formats and rely on the compositor to only send
        // dmabuf frame formats it supports
    }
}

impl wayland_client::Dispatch<wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1, ()> for WaylandState {
    fn event(
        _state: &mut Self,
        _proxy: &wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1,
        _event: <wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
       // No events 
    }
}

impl
    wayland_client::Dispatch<
        wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::ZxdgOutputV1,
        Weak<wayland_client::protocol::wl_output::WlOutput>,
    > for WaylandState
{
    fn event(
        state: &mut Self,
        _proxy: &wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::ZxdgOutputV1,
        event: <wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::ZxdgOutputV1 as Proxy>::Event,
        data: &Weak<wayland_client::protocol::wl_output::WlOutput>,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
        let (_, _, output_info) = state
            .outputs
            .iter_mut()
            .find(|(output, _, _)| output == data)
            .expect("non existing output");

        match event {
            wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::Event::LogicalPosition {.. } => {},
            wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::Event::LogicalSize { .. } => {},
            wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::Event::Done => {
                output_info.done = true;
            },
            wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::Event::Name { name } => {
                output_info.name = name;
            },
            wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::Event::Description { description } => {
                output_info.description = description;
            },
            _ => unreachable!(),
        }
    }
}

impl WlrScreencopySrc {
    fn connect_to_wl_display(&self, wayland_display: Option<&str>, output_name: Option<&str>) {
        let conn = if let Some(wayland_display) = wayland_display {
            let wayland_display = PathBuf::from_str(wayland_display).unwrap();

            let socket_path = if wayland_display.is_absolute() {
                wayland_display
            } else {
                let mut socket_path = std::env::var_os("XDG_RUNTIME_DIR")
                    .map(Into::<PathBuf>::into)
                    .unwrap();
                if !socket_path.is_absolute() {
                    panic!("oh no");
                }
                socket_path.push(wayland_display);
                socket_path
            };

            let stream = UnixStream::connect(socket_path).expect("oh no");
            Connection::from_socket(stream).unwrap()
        } else {
            Connection::connect_to_env().unwrap()
        };
        let (globals, mut event_queue) = registry_queue_init::<WaylandState>(&conn).unwrap();
        let qhandle = event_queue.handle();
        let wl_shm = globals
            .bind::<wayland_client::protocol::wl_shm::WlShm, _, _>(&qhandle, 1..=1, ())
            .expect("wl_shm missing");
        let zwp_linux_dmabuf = globals.bind::<wayland_protocols::wp::linux_dmabuf::zv1::client::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, _, _>(&qhandle, 2..=3, ()).ok();
        let wlr_screencopy_manager = globals.bind::<wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1, _, _>(&qhandle, 1..=3, ()).expect("not wlr screencopy");
        let xdg_output_manager = globals.bind::<wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1, _, _>(&qhandle, 2..=3, ()).ok();

        let mut wayland_state = WaylandState {
            current_frame: None,
            outputs: Vec::new(),
            wlr_screencopy_manager,
            wl_shm,
            dmabuf: zwp_linux_dmabuf,
            qhandle: qhandle.clone(),
        };

        globals.contents().with_list(|global_list| {
            for global in global_list
                .iter()
                .filter(|global| global.interface == "wl_output")
            {
                if global.version < 2 {
                    panic!("at least version 2 is required");
                }

                let version = std::cmp::min(global.version, 4);

                let output = globals
                    .registry()
                    .bind::<wayland_client::protocol::wl_output::WlOutput, _, _>(
                        global.name,
                        version,
                        &qhandle,
                        (),
                    );

                let zxdg_output = if version < 4 {
                    xdg_output_manager.as_ref().map(|xdg_output_manager| {
                        xdg_output_manager.get_xdg_output(&output, &qhandle, output.downgrade())
                    })
                } else {
                    None
                };

                wayland_state
                    .outputs
                    .push((output, zxdg_output, Default::default()));
            }
        });

        // roundtrip to get data for our output info
        while wayland_state.outputs.iter().any(|(_, _, info)| !info.done) {
            event_queue
                .blocking_dispatch(&mut wayland_state)
                .expect("failed to dispatch");
        }

        let (output, _, _) = if let Some(output_name) = output_name {
            wayland_state
                .outputs
                .iter()
                .find(|(_, _, info)| info.name == output_name)
                .unwrap_or_else(|| {
                    panic!(
                        "output {} not found, available outputs: {}",
                        output_name,
                        wayland_state
                            .outputs
                            .iter()
                            .map(|(_, _, info)| &info.name)
                            .fold("".to_owned(), |acc, item| { format!("{} {}", acc, item) })
                            .trim()
                    )
                })
        } else {
            wayland_state.outputs.first().expect("no outputs")
        };

        let frame = wayland_state
            .wlr_screencopy_manager
            .capture_output(0, output, &qhandle, ());
        wayland_state.current_frame = Some((frame, Default::default()));

        // third roundtrip to get frame info
        while !wayland_state
            .current_frame
            .as_ref()
            .map(|(_, info)| info.done)
            .unwrap_or(false)
        {
            event_queue
                .blocking_dispatch(&mut wayland_state)
                .expect("failed to dispatch");
        }

        *self.wayland_state.lock().unwrap() = Some(wayland_state);
        *self._connection.lock().unwrap() = Some(conn);
        *self.event_queue.lock().unwrap() = Some(event_queue);
    }
}

impl ObjectImpl for WlrScreencopySrc {
    fn properties() -> &'static [glib::ParamSpec] {
        static PROPERTIES: Lazy<Vec<glib::ParamSpec>> = Lazy::new(|| {
            vec![
                glib::ParamSpecString::builder("display")
                    .nick("Wayland Display")
                    .blurb("Wayland Display to use")
                    .construct()
                    .build(),
                glib::ParamSpecString::builder("output-name")
                    .nick("Wayland output name")
                    .blurb("Name of the output to capture")
                    .construct()
                    .build(),
            ]
        });

        PROPERTIES.as_ref()
    }

    fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
        match pspec.name() {
            "display" => {
                let mut settings = self.settings.lock().unwrap();
                let wayland_display = value
                    .get::<Option<String>>()
                    .expect("type checked upstream");
                settings.wayland_display = wayland_display;
            }
            "output-name" => {
                let mut settings = self.settings.lock().unwrap();
                let output_name = value
                    .get::<Option<String>>()
                    .expect("type checked upstream");
                settings.output_name = output_name;
            }
            _ => unreachable!(),
        }
    }

    fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
        match pspec.name() {
            "display" => {
                let settings = self.settings.lock().unwrap();
                settings.wayland_display.to_value()
            }
            "output-name" => {
                let settings = self.settings.lock().unwrap();
                settings.output_name.to_value()
            }
            _ => unreachable!(),
        }
    }

    fn constructed(&self) {
        self.parent_constructed();

        let obj = self.obj();
        obj.set_live(true);
        obj.set_format(gstreamer::Format::Time);
        // Replace this with frame finish timestamp
        obj.set_do_timestamp(true);
    }
}

impl GstObjectImpl for WlrScreencopySrc {}

impl ElementImpl for WlrScreencopySrc {
    fn metadata() -> Option<&'static gstreamer::subclass::ElementMetadata> {
        static ELEMENT_METADATA: Lazy<gstreamer::subclass::ElementMetadata> = Lazy::new(|| {
            gstreamer::subclass::ElementMetadata::new(
                "Wayland Screencopy Src",
                "Source",
                "Copy wayland compositor output",
                "Christian Meissl <meissl.christian@gmail.com>",
            )
        });

        Some(&*ELEMENT_METADATA)
    }

    fn pad_templates() -> &'static [gstreamer::PadTemplate] {
        static PAD_TEMPLATES: Lazy<Vec<gstreamer::PadTemplate>> = Lazy::new(|| {
            let caps = gstreamer_video::VideoCapsBuilder::new()
                .format_list(gstreamer_video::VIDEO_FORMATS_ALL.iter().copied())
                .build();
            let mut dmabuf_caps = gstreamer_video::VideoCapsBuilder::new()
                .features(&[*gstreamer_allocators::CAPS_FEATURE_MEMORY_DMABUF])
                .format_list(gstreamer_video::VIDEO_FORMATS_ALL.iter().copied())
                .build();
            dmabuf_caps.merge(caps);
            let src_pad_template = gstreamer::PadTemplate::new(
                "src",
                gstreamer::PadDirection::Src,
                gstreamer::PadPresence::Always,
                &dmabuf_caps,
            )
            .unwrap();

            vec![src_pad_template]
        });

        PAD_TEMPLATES.as_ref()
    }

    fn change_state(
        &self,
        transition: gstreamer::StateChange,
    ) -> Result<gstreamer::StateChangeSuccess, gstreamer::StateChangeError> {
        if transition == gstreamer::StateChange::NullToReady {
            let settings = self.settings.lock().unwrap();
            self.connect_to_wl_display(
                settings.wayland_display.as_deref(),
                settings.output_name.as_deref(),
            );
            return Ok(gstreamer::StateChangeSuccess::Async);
        }

        self.parent_change_state(transition)
    }

    fn query(&self, query: &mut gstreamer::QueryRef) -> bool {
        ElementImplExt::parent_query(self, query)
    }
}

impl BaseSrcImpl for WlrScreencopySrc {
    fn query(&self, query: &mut gstreamer::QueryRef) -> bool {
        BaseSrcImplExt::parent_query(self, query)
    }

    fn caps(&self, filter: Option<&gstreamer::Caps>) -> Option<gstreamer::Caps> {
        let wayland_state = self.wayland_state.lock().unwrap();

        if let Some(state) = wayland_state.as_ref() {
            if let Some((_, frame_info)) = state.current_frame.as_ref() {
                let settings = self.settings.lock().unwrap();

                let (_, _, output_info) = if let Some(output_name) = settings.output_name.as_deref()
                {
                    state
                        .outputs
                        .iter()
                        .find(|(_, _, info)| info.name == output_name)
                        .unwrap_or_else(|| {
                            panic!(
                                "output {} not found, available outputs: {}",
                                output_name,
                                state
                                    .outputs
                                    .iter()
                                    .map(|(_, _, info)| &info.name)
                                    .fold("".to_owned(), |acc, item| {
                                        format!("{} {}", acc, item)
                                    })
                                    .trim()
                            )
                        })
                } else {
                    state.outputs.first().expect("no outputs")
                };

                let output_refresh = if output_info.mode.refresh > 0 {
                    gstreamer::Fraction::approximate_f64(
                        output_info.mode.refresh as f64 / 1_000_000f64,
                    )
                    .unwrap()
                } else {
                    gstreamer::Fraction::new(i32::MAX, 1)
                };

                let mut caps = gstreamer::Caps::new_empty();

                for dmabuf_format in frame_info.dmabuf_formats.iter() {
                    let Some(format) = gst_video_format_from_drm_fourcc(dmabuf_format.format) else {
                        continue;
                    };
                    let dmabuf_format_caps = gstreamer_video::video_make_raw_caps(&[format])
                        .width(dmabuf_format.width as i32)
                        .height(dmabuf_format.height as i32)
                        .framerate_range(..output_refresh)
                        .build();
                    caps.merge(dmabuf_format_caps);
                }

                for shm_format in frame_info.shm_formats.iter() {
                    let Some(format) = gst_video_format_from_wl_shm(shm_format.format) else {
                        continue;
                    };
                    let shm_format_caps = gstreamer_video::video_make_raw_caps(&[format])
                        .width(shm_format.width as i32)
                        .height(shm_format.height as i32)
                        .framerate_range(..output_refresh)
                        .build();
                    caps.merge(shm_format_caps);
                }

                // TODO: Apply the filter

                Some(caps)
            } else {
                self.parent_caps(filter)
            }
        } else {
            self.parent_caps(filter)
        }
    }

    fn set_caps(&self, caps: &gstreamer::Caps) -> Result<(), gstreamer::LoggableError> {
        self.parent_set_caps(caps)
    }

    fn decide_allocation(
        &self,
        query: &mut gstreamer::query::Allocation,
    ) -> Result<(), gstreamer::LoggableError> {
        let guard = self.wayland_state.lock().unwrap();
        let state = guard.as_ref().unwrap();

        let (caps, _) = query.get_owned();
        let video_info =
            gstreamer_video::VideoInfo::from_caps(&caps).expect("failed to get video info");

        let is_dmabuf_format = state
            .current_frame
            .as_ref()
            .map(|(_, frame_info)| {
                let Some(format) = gst_video_format_to_drm_fourcc(video_info.format()) else {
                    return false
                };
                frame_info
                    .dmabuf_formats
                    .iter()
                    .any(|dmabuf_format| dmabuf_format.format == format)
            })
            .unwrap_or(false);

        let buffer_pool = WaylandBufferPool::new(&state.wl_shm, state.dmabuf.as_ref());
        let use_dmabuf_allocator = is_dmabuf_format && state.dmabuf.is_some();
        let (allocator, allocation_params, video_align) = if use_dmabuf_allocator {
            gstreamer::debug!(CAT, imp: self, "using dmabuf format");

            let allocator = if DmaHeapMemoryAllocator::is_available() {
                gstreamer::debug!(CAT, imp: self, "using dma-buf heap allocator");
                DmaHeapMemoryAllocator::default().upcast()
            } else {
                gstreamer::debug!(CAT, imp: self, "using gbm allocator");
                GbmMemoryAllocator::default().upcast()
            };
            // If we use dmabuf memory with a hardware encoder we need to align the memory
            // An alignment of 32bytes should work for most encoders
            let allocation_params =
                gstreamer::AllocationParams::new(gstreamer::MemoryFlags::empty(), 127, 0, 0);
            let video_align = gstreamer_video::VideoAlignment::new(0, 0, 0, 0, &[31, 0, 0, 0]);
            (allocator, Some(allocation_params), Some(video_align))
        } else {
            gstreamer::debug!(CAT, imp: self, "using shm format");

            let shm_format = state
                .current_frame
                .as_ref()
                .map(|(_, frame_info)| {
                    let format = gst_video_format_to_wl_shm(video_info.format()).unwrap();
                    frame_info
                        .shm_formats
                        .iter()
                        .find(|shm_format| shm_format.format == format)
                        .unwrap()
                })
                .unwrap();

            if video_info.stride()[0] != shm_format.stride as i32 {
                unimplemented!()
            }

            gstreamer::debug!(CAT, imp: self, "using memfd allocator");
            (MemfdMemoryAllocator::default().upcast(), None, None)
        };

        if let Some((_, _, min, max)) = query.allocation_pools().get(0) {
            let mut config = buffer_pool.config();
            config.set_allocator(Some(&allocator), allocation_params.as_ref());
            config.add_option(gstreamer_video::BUFFER_POOL_OPTION_VIDEO_META.as_ref());
            if let Some(video_align) = video_align.as_ref() {
                config.add_option(gstreamer_video::BUFFER_POOL_OPTION_VIDEO_ALIGNMENT.as_ref());
                config.set_video_alignment(video_align);
            }
            let size = video_info.size() as u32;
            config.set_params(Some(&caps), size, *min, *max);
            buffer_pool
                .set_config(config)
                .expect("failed to set config");
            query.set_nth_allocation_pool(0, Some(&buffer_pool), size, *min, *max);
        } else {
            let mut config = buffer_pool.config();
            config.set_allocator(Some(&allocator), allocation_params.as_ref());
            config.add_option(gstreamer_video::BUFFER_POOL_OPTION_VIDEO_META.as_ref());
            if let Some(video_align) = video_align.as_ref() {
                config.add_option(gstreamer_video::BUFFER_POOL_OPTION_VIDEO_ALIGNMENT.as_ref());
                config.set_video_alignment(video_align);
            }
            let (caps, _) = query.get_owned();
            let video_info =
                gstreamer_video::VideoInfo::from_caps(&caps).expect("failed to get video info");
            config.set_params(Some(&caps), video_info.size() as u32, 0, 0);
            buffer_pool
                .set_config(config)
                .expect("failed to set config");
            query.add_allocation_pool(Some(&buffer_pool), video_info.size() as u32, 0, 0);
        };

        Ok(())
    }
}

impl PushSrcImpl for WlrScreencopySrc {
    fn create(
        &self,
        _buffer: Option<&mut gstreamer::BufferRef>,
    ) -> Result<gstreamer_base::subclass::base_src::CreateSuccess, gstreamer::FlowError> {
        let pool = self
            .obj()
            .buffer_pool()
            .expect("buffer_pool set in decide_allocation");
        let buffer_pool_aquire_params = gstreamer::BufferPoolAcquireParams::with_flags(
            gstreamer::BufferPoolAcquireFlags::empty(),
        );
        let new_buffer = pool.acquire_buffer(Some(&buffer_pool_aquire_params))?;
        let wl_buffer_meta = new_buffer
            .meta::<WaylandBufferMeta>()
            .expect("no wayland buffer meta");
        let wl_buffer = wl_buffer_meta.wl_buffer();
        let mut event_queue_guard = self.event_queue.lock().unwrap();
        let mut state_guard = self.wayland_state.lock().unwrap();
        let state = state_guard.as_mut().unwrap();
        let settings = self.settings.lock().unwrap();

        // first finish the current frame
        let frame = state
            .current_frame
            .as_ref()
            .map(|(frame, _)| frame)
            .unwrap();
        frame.copy(wl_buffer);

        while !state
            .current_frame
            .as_ref()
            .map(|(_, info)| info.state.is_some())
            .unwrap_or(false)
        {
            event_queue_guard
                .as_mut()
                .unwrap()
                .blocking_dispatch(state)
                .expect("failed to dispatch");
        }

        let (frame, frame_info) = state.current_frame.take().unwrap();
        frame.destroy();
        let frame_state = frame_info.state.unwrap();

        // then shedule the next frame
        let (output, _, _) = if let Some(output_name) = settings.output_name.as_deref() {
            state
                .outputs
                .iter()
                .find(|(_, _, info)| info.name == output_name)
                .unwrap_or_else(|| {
                    panic!(
                        "output {} not found, available outputs: {}",
                        output_name,
                        state
                            .outputs
                            .iter()
                            .map(|(_, _, info)| &info.name)
                            .fold("".to_owned(), |acc, item| { format!("{} {}", acc, item) })
                            .trim()
                    )
                })
        } else {
            state.outputs.first().expect("no outputs")
        };

        let frame = state
            .wlr_screencopy_manager
            .capture_output(0, output, &state.qhandle, ());
        state.current_frame = Some((frame, Default::default()));

        while !state
            .current_frame
            .as_ref()
            .map(|(_, info)| info.done)
            .unwrap_or(false)
        {
            event_queue_guard
                .as_mut()
                .unwrap()
                .blocking_dispatch(state)
                .expect("failed to dispatch");
        }

        match frame_state {
            FrameState::Ready(_timestamp) => {
                // TODO: Set the buffer pts from the duration (and figure out how to transform the time base correctly)
                // remove base.set_do_timestamp(true) when ready
                Ok(gstreamer_base::subclass::base_src::CreateSuccess::NewBuffer(new_buffer))
            }
            FrameState::Failed => Err(gstreamer::FlowError::Error),
        }
    }
}

#[glib::object_subclass]
impl ObjectSubclass for WlrScreencopySrc {
    const NAME: &'static str = "GstWlrScreencopySrc";
    type Type = super::WlrScreencopySrc;
    type ParentType = gstreamer_base::PushSrc;
    type Interfaces = ();
}
