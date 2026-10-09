#!/bin/sh
# Captures every page in this directory from a headless browser into the
# PNG beside it: 800 by 600 unless the page says otherwise, device scale
# one, scrollbars hidden, sRGB. These are the fixtures tests/reference.rs
# compares the engine's render with, byte for byte, so they are
# committed; run this again only when a page changes, and look at the
# result before committing it. Pages named as arguments are captured
# alone; with none, every page is.
#
# A page may ask, in its head, for what it is captured under (M20), as
# tests/reference.rs renders it:
#   <meta name="reference-size" content="640x480">: the viewport's size;
#   <meta name="reference-media" content="print">: the print medium on
#     the screen, the browser's mediaTypeOverride;
#   <meta name="reference-color-scheme" content="dark">: a dark scheme
#     preferred, its preferredColorScheme;
#   <meta name="reference-reduced-motion" content="reduce">: reduced
#     motion preferred.
#
# A page's scripts run, as they do in any page the browser loads; the
# pages that have some say <meta name="reference-scripts" content="on">,
# which tests/reference.rs reads to run them too (M24).
#
# A page's animations are captured at their first frame (M22). A page
# that asks for changes after it, <meta name="reference-change"> and
# <meta name="reference-time">, makes them itself by loading
# reference-change.js, which holds the animations at the times asked:
# the capture needs nothing more.
#
# The browser is $BROWSER, else `chrome-headless-shell` on the PATH: the
# headless shell lays the page out in a viewport exactly the window's
# size, which is what makes the capture the page at the size asked. The
# full browser in its own headless mode does not: its window includes
# its toolbar, so the viewport comes out short. Any browser that takes
# these flags does; extra flags go in $BROWSER_FLAGS (a container
# running as root needs --no-sandbox, say).
set -eu
cd "$(dirname "$0")"
browser=${BROWSER:-chrome-headless-shell}
# The content of the page's <meta name="$2">, if it has one.
meta() {
  sed -n "s/.*<meta name=\"$2\" content=\"\([^\"]*\)\".*/\1/p" "$1" | head -n 1
}
[ $# -gt 0 ] || set -- *.html
for page in "$@"; do
  size=$(meta "$page" reference-size)
  size=${size:-800x600}
  settings=
  case $(meta "$page" reference-media) in
    print) settings=mediaTypeOverride=print ;;
  esac
  case $(meta "$page" reference-color-scheme) in
    dark) settings=${settings:+$settings,}preferredColorScheme=0 ;;
  esac
  motion=
  case $(meta "$page" reference-reduced-motion) in
    reduce) motion=--force-prefers-reduced-motion ;;
  esac
  "$browser" --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --force-color-profile=srgb --window-size="${size%x*},${size#*x}" \
    ${settings:+--blink-settings=$settings} $motion ${BROWSER_FLAGS:-} \
    --screenshot="$PWD/${page%.html}.png" "file://$PWD/$page"
done
