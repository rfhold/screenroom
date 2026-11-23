#!/bin/sh
set -eu

if ! command -v gst-inspect-1.0 >/dev/null 2>&1; then
    echo "Missing gst-inspect-1.0 (Debian: gstreamer1.0-tools)." >&2
    exit 1
fi

# Inspect factories only; do not create a playback pipeline or update the registry cache.
export GST_REGISTRY_UPDATE=no
missing=0
for element in \
    playbin gtk4paintablesink souphttpsrc qtdemux avdec_h264 avdec_aac \
    h264parse aacparse tsdemux subparse subtitleoverlay textoverlay \
    autoaudiosink audioconvert audioresample
do
    if gst-inspect-1.0 "$element" >/dev/null 2>&1; then
        echo "Available: $element"
    else
        echo "Missing: $element" >&2
        missing=1
    fi
done
if gst-inspect-1.0 hlsdemux2 >/dev/null 2>&1; then
    echo "Available: hlsdemux2"
elif gst-inspect-1.0 hlsdemux >/dev/null 2>&1; then
    echo "Available: hlsdemux"
else
    echo "Missing: hlsdemux or hlsdemux2" >&2
    missing=1
fi
exit "$missing"
