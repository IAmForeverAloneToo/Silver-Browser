# Site sweep log

Each run of `tools/sweep/sweep.mjs`: the release shell against the
installed Chrome, one load a site at 1280x800. Numbers only; the
screenshots and the full report stay in `target/sweep/<date>/`.

## 2026-11-13, 88ff12aa8, against Chrome/157.0.8103.52

| site | outcome | likeness top / scrolled | paint / load / settled (s) | Chrome settled (s) | missing | console errors | stack | G90 | CSP | failed requests |
|---|---|---|---|---|---|---|---|---|---|---|
| wikipedia | ok | 0.811 / 0.776 | 1.7 / 3.1 / 3.6 | 8.7 | 6 | 6 | 0 | 0 | 0 | 0 |
| mdn | ok | 0.779 / 0.799 | 1.2 / 2.5 / 4.1 | 5.0 | 0 | 7 | 0 | 0 | 0 | 0 |
| hn | ok | 0.776 / 0.772 | 1.6 / 1.7 / 1.9 | 1.5 | 0 | 0 | 0 | 0 | 0 | 0 |
| bbc | renderer crash | - / - | - / - / - | 2.9 | 0 | 0 | 0 | 0 | 0 | 0 |
| verge | renderer crash | - / - | - / - / - | - | 0 | 0 | 0 | 0 | 0 | 0 |
| stackoverflow | ok | 0.746 / 0.746 | 0.7 / 1.0 / 0.9 | 3.7 | 1 | 1 | 0 | 0 | 0 | 0 |
| github | ok | 0.818 / 0.792 | 5.0 / 5.0 / 0.3 | 16.6 | 0 | 6 | 0 | 0 | 0 | 0 |
| docsrs | ok | 0.848 / 0.837 | 1.0 / 1.2 / 1.4 | 7.3 | 0 | 0 | 0 | 0 | 0 | 0 |
| cratesio | ok | 0.851 / 0.839 | 1.6 / 1.6 / 4.3 | 3.0 | 0 | 0 | 0 | 0 | 0 | 0 |
| mslearn | ok | 0.652 / 0.845 | 2.7 / 3.3 / 0.3 | 2.5 | 0 | 1 | 0 | 0 | 0 | 0 |
| google | ok | 0.771 / 0.771 | 0.6 / 4.7 / 5.4 | 2.3 | 0 | 4 | 0 | 2 | 0 | 0 |
| ddg | ok | 0.694 / 0.704 | 1.8 / 4.3 / 4.4 | 4.3 | 0 | 4 | 0 | 0 | 0 | 0 |
| bing | ok | 0.845 / 0.783 | 1.9 / 2.7 / 2.7 | 5.1 | 1 | 3 | 0 | 0 | 0 | 0 |
| youtube | ok | 0.693 / 0.692 | 3.3 / 10.6 / 14.7 | 26.3 | 0 | 6 | 0 | 0 | 0 | 0 |
| twitch | ok | 0.738 / 0.62 | 0.6 / 3.0 / 3.0 | 0.8 | 4 | 4 | 0 | 0 | 0 | 0 |
| vimeo | renderer crash | - / - | - / - / - | 10.6 | 0 | 0 | 0 | 0 | 0 | 0 |
| maps | ok | 0.847 / 0.854 | 1.1 / 6.2 / 6.3 | 6.9 | 0 | 3 | 0 | 0 | 0 | 0 |
| reddit | hang | 0.688 / - | 1.0 / - / - | 6.9 | 0 | 2 | 0 | 0 | 0 | 0 |
| x | ok | - / - | 1.7 / 3.6 / 3.6 | - | 2 | 3 | 0 | 0 | 0 | 0 |
| gmail | ok | 0.806 / 0.806 | 2.3 / 3.4 / 3.6 | 8.5 | 0 | 0 | 0 | 5 | 0 | 0 |
| gdocs | ok | 0.831 / 0.831 | 2.1 / 3.1 / 3.3 | 1.5 | 0 | 0 | 0 | 5 | 0 | 0 |
| ai-chat | ok | 0.746 / 0.746 | 0.7 / 1.4 / 1.4 | 2.5 | 0 | 1 | 0 | 0 | 0 | 0 |
| amazon | ok | 0.567 / 0.557 | 2.0 / 26.6 / 26.6 | 6.9 | 0 | 0 | 0 | 0 | 0 | 0 |
| ebay | ok | 0.997 / 0.997 | 0.8 / 1.4 / 0.8 | 1.0 | 0 | 0 | 0 | 0 | 0 | 0 |
| weather | renderer crash | - / - | - / - / - | 14.8 | 0 | 0 | 0 | 0 | 0 | 0 |

Top 10 causes (sites, total):

- 4, 13: console: Uncaught TypeError: not a function at <url>
- 4, 4: outcome: renderer crash
- 3, 12: G90: same-origin frame refused
- 2, 5: missing: (not a function)
- 2, 2: console: Uncaught NotFoundError: The node is no longer there.
- 1, 6: console: Uncaught (in promise) ReferenceError: PerformanceObserver is not defined at <url>
- 1, 6: missing: PerformanceObserver
- 1, 4: console: Uncaught TypeError: Failed to execute 'insertBefore' on 'Node': parameter N is not of type 'Node'. at bootstrap:N:N
- 1, 3: console: Uncaught (in promise) SecurityError: Failed to read a named property 'JSON' from 'Window': Blocked a frame with origin "<url>" from accessing a cross-origin frame.
- 1, 2: console: TypeError: not a function

## 2026-11-13, 3a2e822f1, against Chrome/157.0.8103.52

| site | outcome | same host | likeness top / scrolled | paint / load / settled (s) | Chrome settled (s) | missing | console errors | stack | G90 | CSP | failed requests |
|---|---|---|---|---|---|---|---|---|---|---|---|
| wikipedia | ok | yes | 0.811 / 0.779 | 1.6 / 2.9 / 3.5 | 8.4 | 6 | 6 | 0 | 0 | 0 | 0 |
| mdn | ok | yes | 0.792 / 0.799 | 1.1 / 2.4 / 4.1 | 4.9 | 0 | 7 | 0 | 0 | 0 | 0 |
| hn | ok | yes | 0.774 / 0.771 | 1.3 / 1.4 / 1.6 | 1.4 | 0 | 0 | 0 | 0 | 0 | 0 |
| bbc | renderer crash | no | - / - | - / - / - | 2.7 | 0 | 0 | 0 | 0 | 0 | 0 |
| verge | renderer crash | no | - / - | - / - / - | 20.3 | 0 | 0 | 0 | 0 | 0 | 0 |
| stackoverflow | ok | yes | 0.746 / 0.746 | 0.9 / 1.4 / 1.4 | 3.4 | 0 | 1 | 0 | 0 | 0 | 0 |
| github | ok | yes | 0.818 / 0.792 | 6.2 / 6.7 / 6.7 | 3.6 | 0 | 6 | 0 | 0 | 0 | 0 |
| docsrs | ok | yes | 0.848 / 0.837 | 1.0 / 1.6 / 1.6 | 4.4 | 0 | 0 | 0 | 0 | 0 | 0 |
| cratesio | ok | yes | 0.615 / 0.732 | 1.4 / 1.4 / 3.2 | 10.2 | 0 | 1 | 0 | 0 | 0 | 0 |
| mslearn | ok | yes | 0.78 / 0.853 | 2.0 / 2.3 / 4.8 | 3.0 | 0 | 1 | 0 | 0 | 0 | 0 |
| google | ok | yes | 0.777 / 0.777 | 0.7 / 4.7 / 5.7 | 2.4 | 0 | 4 | 0 | 2 | 0 | 0 |
| ddg | ok | yes | 0.696 / 0.707 | 1.8 / 3.9 / 4.0 | 4.5 | 0 | 3 | 0 | 0 | 0 | 0 |
| bing | ok | yes | 0.846 / 0.861 | 2.0 / 2.9 / 3.0 | 5.0 | 1 | 3 | 0 | 0 | 0 | 0 |
| youtube | ok | yes | 0.693 / 0.692 | 2.7 / 8.8 / 13.2 | 3.6 | 0 | 7 | 0 | 0 | 0 | 0 |
| twitch | ok | yes | 0.606 / 0.606 | 0.8 / 0.1 / 3.2 | 4.7 | 4 | 4 | 0 | 0 | 0 | 0 |
| vimeo | renderer crash | no | - / - | - / - / - | 15.7 | 0 | 0 | 0 | 0 | 0 | 0 |
| maps | ok | yes | 0.887 / 0.866 | 1.5 / 8.6 / 20.0 | 4.5 | 0 | 11 | 0 | 0 | 0 | 0 |
| reddit | hang | yes | 0.688 / - | 0.6 / - / - | 2.6 | 0 | 2 | 0 | 0 | 0 | 0 |
| x | ok | - | - / - | 1.7 / 4.0 / 4.0 | - | 2 | 3 | 0 | 0 | 0 | 0 |
| gmail | ok | yes | 0.811 / 0.811 | 2.1 / 3.5 / 3.6 | 2.3 | 0 | 0 | 0 | 5 | 0 | 0 |
| gdocs | ok | yes | 0.831 / 0.831 | 2.2 / 3.2 / 3.3 | 15.1 | 0 | 0 | 0 | 5 | 0 | 0 |
| ai-chat | ok | yes | 0.747 / 0.733 | 0.8 / 1.5 / 1.5 | 2.8 | 0 | 4 | 0 | 0 | 0 | 0 |
| amazon | ok | no | 0.562 / 0.559 | 2.0 / 27.2 / 27.2 | 5.4 | 0 | 0 | 0 | 0 | 0 | 0 |
| ebay | ok | yes | 0.997 / 0.997 | 1.1 / 1.8 / 1.8 | 1.4 | 0 | 0 | 0 | 0 | 0 | 0 |
| weather | renderer crash | no | - / - | - / - / - | - | 0 | 0 | 0 | 0 | 0 | 0 |

Top 10 causes (sites, total):

- 4, 13: console: Uncaught TypeError: not a function at <url>
- 4, 4: error status: document
- 4, 4: outcome: renderer crash
- 3, 12: G90: same-origin frame refused
- 3, 11: console: Uncaught NotFoundError: The node is no longer there.
- 2, 5: missing: (not a function)
- 2, 2: console: Uncaught TypeError: cannot read property 'getRandomValues' of undefined at <url>
- 1, 6: console: Uncaught (in promise) ReferenceError: PerformanceObserver is not defined at <url>
- 1, 6: missing: PerformanceObserver
- 1, 4: console: Uncaught TypeError: Failed to execute 'insertBefore' on 'Node': parameter N is not of type 'Node'. at bootstrap:N:N

## 2026-11-13, 9a8ff3116, against Chrome/157.0.8103.52

| site | outcome | same host | likeness top / scrolled | paint / load / settled (s) | Chrome settled (s) | missing | console errors | stack | G90 | CSP | failed requests |
|---|---|---|---|---|---|---|---|---|---|---|---|
| wikipedia | ok | yes | 0.811 / 0.776 | 1.8 / 3.5 / 4.0 | 9.1 | 6 | 6 | 0 | 0 | 0 | 0 |
| mdn | ok | yes | 0.779 / 0.799 | 1.2 / 2.6 / 4.3 | 5.6 | 0 | 7 | 0 | 0 | 0 | 0 |
| hn | ok | yes | 0.774 / 0.776 | 1.6 / 1.8 / 2.0 | 1.4 | 0 | 0 | 0 | 0 | 0 | 0 |
| bbc | hang | yes | 0.669 / - | 0.7 / - / - | 2.6 | 0 | 1 | 0 | 0 | 0 | 0 |
| verge | ok | yes | 0.838 / 0.704 | 2.0 / 7.0 / 11.8 | 13.0 | 2 | 3 | 0 | 0 | 0 | 0 |
| stackoverflow | ok | yes | 0.747 / 0.747 | 1.1 / 1.7 / 1.7 | 3.8 | 0 | 6 | 0 | 0 | 0 | 0 |
| github | ok | yes | 0.818 / 0.713 | 8.6 / 8.6 / 8.6 | 8.4 | 0 | 6 | 0 | 0 | 0 | 0 |
| docsrs | ok | yes | 0.848 / 0.837 | 1.6 / 2.1 / 2.1 | 2.3 | 0 | 0 | 0 | 0 | 0 | 0 |
| cratesio | ok | yes | 0.615 / 0.732 | 1.3 / 1.3 / 3.0 | - | 0 | 1 | 0 | 0 | 0 | 0 |
| mslearn | ok | yes | 0.652 / 0.845 | 3.3 / 3.6 / 3.6 | 3.0 | 0 | 1 | 0 | 0 | 0 | 0 |
| google | ok | yes | 0.776 / 0.776 | 1.2 / 5.6 / 5.9 | 3.5 | 0 | 4 | 0 | 2 | 0 | 0 |
| ddg | ok | yes | 0.694 / 0.705 | 1.7 / 4.0 / 4.0 | 6.6 | 0 | 14 | 0 | 0 | 0 | 0 |
| bing | ok | yes | 0.853 / 0.784 | 2.5 / 3.2 / 3.3 | - | 1 | 3 | 0 | 0 | 0 | 0 |
| youtube | ok | yes | 0.693 / 0.692 | 2.5 / 8.4 / 11.6 | 21.8 | 0 | 12 | 0 | 0 | 0 | 6 |
| twitch | ok | yes | 0.607 / 0.607 | 0.8 / 3.0 / 3.0 | 8.8 | 4 | 4 | 0 | 0 | 0 | 0 |
| vimeo | ok | yes | 0.776 / 0.761 | 3.6 / - / - | 19.4 | 1 | 2 | 0 | 0 | 0 | 0 |
| maps | hang | yes | 0.695 / - | 0.8 / - / - | 3.8 | 0 | 0 | 0 | 0 | 0 | 0 |
| reddit | hang | yes | 0.688 / - | 0.8 / - / - | 3.6 | 0 | 2 | 0 | 0 | 0 | 0 |
| x | ok | - | - / - | 3.3 / 8.3 / 8.3 | - | 2 | 3 | 0 | 0 | 0 | 0 |
| gmail | ok | yes | 0.812 / 0.806 | 2.6 / 4.2 / 4.2 | 2.8 | 0 | 0 | 0 | 5 | 0 | 0 |
| gdocs | ok | yes | 0.831 / 0.831 | 2.5 / 3.9 / 3.9 | 1.6 | 0 | 0 | 0 | 5 | 0 | 0 |
| ai-chat | ok | yes | 0.746 / 0.731 | 0.7 / 1.2 / 1.2 | 11.0 | 0 | 4 | 0 | 0 | 0 | 0 |
| amazon | ok | yes | 0.49 / 0.509 | 2.4 / 2.9 / 3.1 | 5.2 | 0 | 0 | 0 | 0 | 0 | 0 |
| ebay | ok | yes | 0.996 / 0.996 | 0.7 / 1.8 / 1.8 | 1.0 | 0 | 0 | 0 | 0 | 0 | 0 |
| weather | ok | yes | 0.722 / 0.629 | 1.2 / 19.6 / 19.7 | - | 1 | 5 | 0 | 20 | 0 | 1 |

Top 10 causes (sites, total):

- 5, 14: console: Uncaught TypeError: not a function at <url>
- 5, 7: error status: document
- 4, 32: G90: same-origin frame refused
- 3, 7: missing: (not a function)
- 3, 3: console: Uncaught Script error.
- 3, 3: missing: ReadableStream
- 3, 3: outcome: hang
- 2, 4: console: [Cloudflare Turnstile] Unhandled error: Uncaught TypeError: cannot read property 'getRandomValues' of undefined , with debug info:
- 2, 4: console: Uncaught TypeError: cannot read property 'getRandomValues' of undefined at <url>
- 2, 2: console: Uncaught (in promise) ReferenceError: ReadableStream is not defined at <url>

## 2026-11-13, eecdbf6c1, against Chrome/157.0.8103.52

Counting changed (errors from callbacks now counted): not comparable
with earlier lines. This line is the new baseline. Reddit hung here,
with the sweep's sites run side by side; alone, it loaded 3 of 3 (15.1
to 15.9 s) on this commit.

| site | outcome | same host | likeness top / scrolled | paint / load / settled (s) | Chrome settled (s) | missing | console errors | stack | G90 | CSP | failed requests |
|---|---|---|---|---|---|---|---|---|---|---|---|
| wikipedia | ok | yes | 0.809 / 0.776 | 2.1 / 4.2 / 5.0 | - | 6 | 6 | 0 | 0 | 0 | 0 |
| mdn | ok | yes | 0.779 / 0.799 | 1.6 / 3.9 / 6.2 | 6.5 | 0 | 7 | 0 | 0 | 0 | 0 |
| hn | ok | yes | 0.774 / 0.774 | 1.5 / 1.6 / 1.7 | 1.4 | 0 | 0 | 0 | 0 | 0 | 0 |
| bbc | hang | yes | 0.701 / - | 1.0 / - / - | 3.8 | 1 | 1 | 0 | 0 | 0 | 0 |
| verge | ok | yes | 0.798 / 0.701 | 2.2 / 8.8 / 8.8 | 8.8 | 2 | 3 | 0 | 0 | 0 | 0 |
| stackoverflow | ok | yes | 0.743 / 0.743 | 1.0 / 1.7 / 1.7 | - | 1 | 4 | 0 | 0 | 0 | 0 |
| github | ok | yes | 0.818 / 0.793 | 6.1 / 6.1 / 6.1 | 19.2 | 0 | 6 | 0 | 0 | 0 | 0 |
| docsrs | ok | yes | 0.848 / 0.837 | 1.1 / 1.6 / 1.6 | 15.0 | 0 | 0 | 0 | 0 | 0 | 0 |
| cratesio | ok | yes | 0.615 / 0.732 | 2.0 / 2.0 / 3.3 | 3.3 | 0 | 1 | 0 | 0 | 0 | 0 |
| mslearn | ok | yes | 0.652 / 0.845 | 2.3 / 2.7 / 2.7 | 6.3 | 0 | 1 | 0 | 0 | 0 | 0 |
| google | ok | yes | 0.77 / 0.77 | 1.0 / 5.4 / 6.0 | 2.3 | 1 | 4 | 0 | 2 | 0 | 0 |
| ddg | ok | yes | 0.695 / 0.701 | 1.8 / 4.4 / 4.4 | 2.8 | 0 | 3 | 0 | 0 | 0 | 0 |
| bing | ok | yes | 0.845 / 0.859 | 1.8 / 4.2 / 4.2 | 5.4 | 2 | 3 | 0 | 0 | 0 | 0 |
| youtube | ok | yes | 0.691 / 0.692 | 2.9 / 13.3 / 15.2 | 4.3 | 0 | 12 | 0 | 0 | 0 | 6 |
| twitch | ok | yes | 0.738 / 0.675 | 1.1 / 3.6 / 3.6 | 1.1 | 4 | 4 | 0 | 0 | 0 | 0 |
| vimeo | ok | yes | 0.722 / 0.543 | 8.8 / - / - | 9.0 | 1 | 2 | 0 | 0 | 0 | 0 |
| maps | hang | yes | 0.697 / - | 2.2 / - / - | 7.9 | 0 | 0 | 0 | 0 | 0 | 0 |
| reddit | hang | yes | 0.684 / - | 1.1 / - / - | 4.6 | 0 | 2 | 0 | 0 | 0 | 0 |
| x | ok | - | - / - | 4.2 / 8.5 / 8.5 | - | 3 | 3 | 0 | 0 | 0 | 0 |
| gmail | ok | yes | 0.806 / 0.806 | 2.6 / 3.7 / 3.9 | 12.6 | 0 | 0 | 0 | 5 | 0 | 0 |
| gdocs | ok | yes | 0.831 / 0.827 | 1.9 / 3.1 / 3.1 | 1.5 | 0 | 0 | 0 | 5 | 0 | 0 |
| ai-chat | ok | yes | 0.746 / 0.736 | 0.9 / 1.7 / 1.7 | 2.7 | 0 | 4 | 0 | 0 | 0 | 0 |
| amazon | ok | yes | 0.717 / 0.719 | 2.1 / - / - | 6.1 | 0 | 1 | 0 | 0 | 0 | 0 |
| ebay | ok | yes | 0.996 / 0.996 | 0.7 / 1.3 / 1.3 | 0.8 | 0 | 0 | 0 | 0 | 0 | 0 |
| weather | hang | yes | 0.622 / - | 1.1 / - / - | - | 0 | 0 | 0 | 0 | 0 | 1 |

Top 10 causes (sites, total):

- 5, 14: console: Uncaught TypeError: not a function at <url>
- 5, 10: missing: (not a function)
- 4, 6: error status: document
- 4, 4: outcome: hang
- 3, 12: G90: same-origin frame refused
- 2, 2: console: [Cloudflare Turnstile] Unhandled error: Uncaught TypeError: cannot read property 'getRandomValues' of undefined , with debug info:
- 2, 2: console: Uncaught Script error.
- 2, 2: console: Uncaught TypeError: cannot read property 'getRandomValues' of undefined at <url>
- 2, 2: missing: ReadableStream
- 1, 6: console: Failed to fetch <url>

