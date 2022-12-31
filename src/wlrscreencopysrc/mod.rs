use gstreamer::glib;
use gstreamer::prelude::*;

mod imp;

glib::wrapper! {
    pub struct WlrScreencopySrc(ObjectSubclass<imp::WlrScreencopySrc>) @extends gstreamer_base::PushSrc, gstreamer_base::BaseSrc, gstreamer::Element, gstreamer::Object;
}

pub fn register(plugin: &gstreamer::Plugin) -> Result<(), glib::BoolError> {
    gstreamer::Element::register(
        Some(plugin),
        "wlrscreencopysrc",
        gstreamer::Rank::Marginal,
        WlrScreencopySrc::static_type(),
    )
}
