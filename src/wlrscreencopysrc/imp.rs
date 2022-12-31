use once_cell::sync::Lazy;

use gstreamer::glib;
use gstreamer::subclass::prelude::*;
use gstreamer_base::subclass::prelude::*;

#[derive(Debug, Default)]
pub struct WlrScreencopySrc {}

impl ObjectImpl for WlrScreencopySrc {}

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
            let caps = gstreamer::Caps::new_any();
            let src_pad_template = gstreamer::PadTemplate::new(
                "src",
                gstreamer::PadDirection::Src,
                gstreamer::PadPresence::Always,
                &caps,
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
        self.parent_change_state(transition)
    }
}

impl BaseSrcImpl for WlrScreencopySrc {}

impl PushSrcImpl for WlrScreencopySrc {}

#[glib::object_subclass]
impl ObjectSubclass for WlrScreencopySrc {
    const NAME: &'static str = "GstWlrScreencopySrc";
    type Type = super::WlrScreencopySrc;
    type ParentType = gstreamer_base::PushSrc;
    type Interfaces = ();
}
