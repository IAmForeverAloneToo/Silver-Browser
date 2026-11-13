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

