#!/bin/sh
# Captures every page in this directory from a headless browser into the
# PNG beside it: 800 by 600, device scale one, scrollbars hidden, sRGB.
# These are the fixtures tests/reference.rs compares the engine's render
# with, byte for byte, so they are committed; run this again only when a
# page changes, and look at the result before committing it.
#
# The browser is $BROWSER, else `chrome-headless-shell` on the PATH: the
# headless shell lays the page out in a viewport exactly the window's
# size, which is what makes the capture the page at 800 by 600. The full
# browser in its own headless mode does not: its window includes its
# toolbar, so the viewport comes out short. Any browser that takes these
# flags does; extra flags go in $BROWSER_FLAGS (a container running as
# root needs --no-sandbox, say).
set -eu
cd "$(dirname "$0")"
browser=${BROWSER:-chrome-headless-shell}
for page in *.html; do
  "$browser" --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --force-color-profile=srgb --window-size=800,600 ${BROWSER_FLAGS:-} \
    --screenshot="$PWD/${page%.html}.png" "file://$PWD/$page"
done
