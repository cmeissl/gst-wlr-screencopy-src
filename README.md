## Testing locally

```sh
export GST_PLUGIN_PATH=$PWD/target/debug
gst-launch-1.0 wlrscreencopysrc ! vaapih264enc ! fakesink
```
