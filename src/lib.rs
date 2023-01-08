#![allow(clippy::non_send_fields_in_send_ty, unused_doc_comments)]

use gstreamer::glib;

mod allocators;
mod buffer_pool;
mod wlrscreencopysrc;
mod utils;

fn plugin_init(plugin: &gstreamer::Plugin) -> Result<(), glib::BoolError> {
    wlrscreencopysrc::register(plugin)
}

gstreamer::plugin_define!(
    wlrscreencopy,
    env!("CARGO_PKG_DESCRIPTION"),
    plugin_init,
    concat!(env!("CARGO_PKG_VERSION"), "-", env!("COMMIT_ID")),
    "MIT/X11",
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_NAME"),
    env!("CARGO_PKG_REPOSITORY"),
    env!("BUILD_REL_DATE")
);
