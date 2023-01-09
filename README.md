# Gstreamer plugin for wlr-screencopy

WIP

## Testing locally

### Software Encoding

```sh
export GST_PLUGIN_PATH="$PWD/target/debug"
gst-launch-1.0 wlrscreencopysrc display="wayland-1" ! videoconvert ! openh264enc ! openh264dec ! videoconvert ! queue ! waylandsink
```

### Gstreamer VA-API

```sh
export GST_PLUGIN_PATH="$PWD/target/debug"
gst-launch-1.0 wlrscreencopysrc display="wayland-1" ! vaapipostproc ! vaapih264enc ! vaapih264dec ! vaapipostproc ! queue ! waylandsink
```

### Gstreamer VA (plugins-bad)

Note: This requires as least gstreamer 1.21 which is not released, if you build from source
you can override the plugin paths with:

```sh
export GST_PLUGIN_SYSTEM_PATH="/usr/local/lib64/gstreamer-1.0"
export LD_LIBRARY_PATH=/usr/local/lib64/:$LD_LIBRARY_PATH
```

```sh
export GST_PLUGIN_PATH="$PWD/target/debug"
gst-launch-1.0 wlrscreencopysrc display="wayland-1" ! glupload ! glcolorconvert ! gldownload ! vah264enc ! vah264dec ! vapostproc ! queue ! waylandsink
```

### Recording

Recording ~10s from output with 60Hz

#### Software

```sh
gst-launch-1.0 -m wlrscreencopysrc display="wayland-1" num-buffers=600 ! videoconvert ! openh264enc ! h264parse ! mp4mux ! filesink location="record.mp4"
```

#### VA-API

```sh
gst-launch-1.0 -m wlrscreencopysrc display="wayland-1" num-buffers=600 ! vaapipostproc ! vaapih264enc ! h264parse ! mp4mux ! filesink location="record.mp4"
```
