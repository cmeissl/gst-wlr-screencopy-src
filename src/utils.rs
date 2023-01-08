use gstreamer_video::VideoFormat;

use wayland_client::protocol::wl_shm;

pub fn gst_video_format_from_wl_shm(format: wl_shm::Format) -> Option<VideoFormat> {
    let format = match format {
        wl_shm::Format::Abgr8888 => VideoFormat::Rgba,
        wl_shm::Format::Argb8888 => VideoFormat::Bgra,
        wl_shm::Format::Bgra8888 => VideoFormat::Argb,
        wl_shm::Format::Bgrx8888 => VideoFormat::Xrgb,
        wl_shm::Format::Rgba8888 => VideoFormat::Abgr,
        wl_shm::Format::Rgbx8888 => VideoFormat::Xbgr,
        wl_shm::Format::Xbgr8888 => VideoFormat::Rgbx,
        wl_shm::Format::Xrgb8888 => VideoFormat::Bgrx,
        _ => return None,
    };
    Some(format)
}

pub fn gst_video_format_to_wl_shm(format: VideoFormat) -> Option<wl_shm::Format> {
    let format = match format {
        VideoFormat::Abgr => wl_shm::Format::Rgba8888,
        VideoFormat::Argb => wl_shm::Format::Bgra8888,
        VideoFormat::Bgra => wl_shm::Format::Argb8888,
        VideoFormat::Bgrx => wl_shm::Format::Xrgb8888,
        VideoFormat::Rgba => wl_shm::Format::Abgr8888,
        VideoFormat::Rgbx => wl_shm::Format::Xbgr8888,
        VideoFormat::Xbgr => wl_shm::Format::Rgbx8888,
        VideoFormat::Xrgb => wl_shm::Format::Bgrx8888,
        _ => return None,
    };
    Some(format)
}

pub fn gst_video_format_from_drm_fourcc(format: drm_fourcc::DrmFourcc) -> Option<VideoFormat> {
    let format = match format {
        drm_fourcc::DrmFourcc::Abgr8888 => VideoFormat::Abgr,
        drm_fourcc::DrmFourcc::Argb8888 => VideoFormat::Argb,
        drm_fourcc::DrmFourcc::Bgra8888 => VideoFormat::Bgra,
        drm_fourcc::DrmFourcc::Bgrx8888 => VideoFormat::Bgrx,
        drm_fourcc::DrmFourcc::Rgba8888 => VideoFormat::Rgba,
        drm_fourcc::DrmFourcc::Rgbx8888 => VideoFormat::Rgbx,
        drm_fourcc::DrmFourcc::Xbgr8888 => VideoFormat::Xbgr,
        drm_fourcc::DrmFourcc::Xrgb8888 => VideoFormat::Xrgb,
        _ => return None,
    };
    Some(format)
}

pub fn gst_video_format_to_drm_fourcc(format: VideoFormat) -> Option<drm_fourcc::DrmFourcc> {
    let format = match format {
        gstreamer_video::VideoFormat::Abgr => drm_fourcc::DrmFourcc::Abgr8888,
        gstreamer_video::VideoFormat::Argb => drm_fourcc::DrmFourcc::Argb8888,
        gstreamer_video::VideoFormat::Bgra => drm_fourcc::DrmFourcc::Bgra8888,
        gstreamer_video::VideoFormat::Bgrx => drm_fourcc::DrmFourcc::Bgrx8888,
        gstreamer_video::VideoFormat::Rgba => drm_fourcc::DrmFourcc::Rgba8888,
        gstreamer_video::VideoFormat::Rgbx => drm_fourcc::DrmFourcc::Rgbx8888,
        gstreamer_video::VideoFormat::Xbgr => drm_fourcc::DrmFourcc::Xbgr8888,
        gstreamer_video::VideoFormat::Xrgb => drm_fourcc::DrmFourcc::Xrgb8888,
        _ => return None,
    };
    Some(format)
}
