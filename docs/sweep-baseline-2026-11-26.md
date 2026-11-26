# Site sweep baseline, 2026-11-26

The daily-site sweep that set the beta order of 2026-11-26: the release
shell at b218a443d against Chrome/158.0.8127.37, 42 sites, one load a
site at 1280x800, each site under its own quiet lock
(`tools/sweep/sweep.mjs`, the list in silver-work's
`tooling/sweep-list.tsv`). Later sweeps diff against this table. The
screenshots and per-site reports are not kept here.

Columns: outcome, Silver/Chrome (S/C); load and settled, seconds; the
share of a core used while the page was left alone for 10 s after a
scroll (Silver: renderers and browser; Chrome: renderers, browser and
GPU); Silver's frames a second while left alone and what wanted most of
them; Silver's script tasks a second while left alone, by kind; the most
loop turns a second of any of its renderers ("-": the renderer did not
answer, its thread busy); private memory of all processes once left
alone, MiB; likeness of the top and scrolled screenshots to Chrome's.

Silver loaded 42 of 42 with no crash or hang. Chrome's load error on x
and hang on etsy are the harness's.

Chrome: challenged. On reddit, reddit-home, ai-chat, chatgpt and
pcpartpicker, automated Chrome (headless or windowed, a throwaway
profile) was shown a bot wall in place of the site: Reddit's "Prove
your humanity" reCAPTCHA page, Cloudflare's "Just a moment" check. Its
numbers there were the wall's and are left out (shown "-"); Silver's
stand. The sweep now flags such a row (`challenged` on Chrome's side).
Until a real reference exists, those sites are held to absolute
targets: idle at most 0.15 cores, settled within 3 s, the renderer's
peak at most 500 MiB.

| site | S/C outcome | load s S/C | settled s S/C | idle CPU S/C (renderers+browser, cores) | S idle frames/s | S frame cause | S idle tasks/s by kind | loop turns/s max | private MiB idle S/C | similarity top/scrolled |
|---|---|---|---|---|---|---|---|---|---|---|
| ai-chat | ok/challenged | 0.7/- | 0.8/- | 1.17/- | 3 | changed | timer6 event6 other4 messa1 anima0 fetch0 | - | 475/- | - |
| amazon | ok/ok | 1.0/0.6 | 1.0/0.6 | 0.00/0.01 | 0 | - | 0 | 0 | 265/282 | 0.72/0.73 |
| amazon-au | ok/ok | -/1.0 | -/2.8 | 0.97/0.06 | - | - | - | 0 | 554/454 | 0.71/0.72 |
| bbc | ok/ok | 1.9/0.9 | 6.3/1.2 | 1.12/0.61 | 48 | asked | timer49 other48 anima10 event3 fetch1 messa0 | 80 | 1088/610 | 0.87/0.80 |
| bing | ok/ok | 1.3/0.8 | 21.1/4.6 | 0.03/0.01 | 0 | - | timer4 | 4 | 360/331 | 0.76/0.77 |
| chatgpt | ok/challenged | 2.4/- | 2.5/- | 0.32/- | 1 | changed | event37 other3 messa2 timer2 fetch1 anima0 | - | 575/- | - |
| cloudflare | ok/ok | -/3.8 | -/- | 0.97/2.13 | 2 | changed | event118 other3 timer1 anima1 messa0 | - | 494/909 | 0.84/0.70 |
| cratesio | ok/ok | 1.0/0.9 | 4.5/2.2 | 0.00/0.01 | 0 | - | 0 | 0 | 184/387 | 0.85/0.84 |
| ddg | ok/ok | 2.5/1.4 | 2.5/1.7 | 0.71/0.01 | 71 | asked | other71 timer0 | 115 | 360/360 | 0.70/0.70 |
| docsrs | ok/ok | 1.0/0.4 | 1.0/0.7 | 0.00/0.01 | 0 | - | 0 | 0 | 176/299 | 0.85/0.84 |
| ebay | ok/ok | 0.5/0.4 | 0.5/0.5 | 0.00/0.01 | 0 | - | 0 | 0 | 127/267 | 1.00/1.00 |
| etsy | ok/hang | 3.0/1.4 | 3.0/- | 0.01/0.01 | 0 | - | 0 | 2 | 394/269 | 0.80/0.82 |
| gdocs | ok/ok | 1.2/0.7 | 1.4/1.0 | 0.00/0.03 | 0 | - | timer0 | 0 | 180/292 | 0.83/0.83 |
| gdrive | ok/ok | 1.5/0.6 | 1.6/1.0 | 0.00/0.02 | 0 | - | timer0 | 0 | 177/282 | 0.81/0.81 |
| gemini | ok/ok | 3.1/2.0 | 3.3/4.1 | 0.00/0.24 | 0 | - | 0 | 0 | 264/552 | 0.73/0.73 |
| github | ok/ok | 4.3/0.8 | 4.3/1.6 | 0.79/0.01 | 6 | changed | event53 other15 timer6 messa3 fetch1 anima1 | - | 431/387 | 0.82/0.83 |
| gmail | ok/ok | 1.1/1.1 | 1.6/1.4 | 0.00/0.02 | 0 | - | timer0 | 0 | 180/286 | 0.81/0.81 |
| google | ok/ok | 2.1/0.1 | 2.4/1.9 | 0.06/0.03 | 0 | - | timer39 event0 messa0 | 32 | 208/368 | 0.77/0.77 |
| google-images | ok/ok | 2.4/0.1 | 2.7/1.9 | 0.03/0.01 | 0 | - | timer39 event0 messa0 | 32 | 204/374 | 0.77/0.77 |
| grok | ok/ok | 5.8/0.9 | 5.8/7.1 | 0.00/0.02 | 0 | - | 0 | 0 | 321/898 | 0.93/0.93 |
| hn | ok/ok | 1.2/0.8 | 1.3/0.7 | 0.00/0.01 | 0 | - | 0 | 0 | 131/275 | 0.78/0.77 |
| huggingface | ok/ok | 3.1/1.2 | 6.3/2.6 | 1.02/0.27 | 23 | animation | timer23 event1 other0 fetch0 | 37 | 547/460 | 0.87/0.87 |
| maps | ok/ok | 2.6/0.6 | 7.2/3.9 | 1.38/0.02 | 22 | frameCallback | messa41 anima10 timer8 event6 other2 fetch2 | 52 | 474/828 | 0.85/0.85 |
| mdn | ok/ok | 2.5/0.3 | 3.8/2.3 | 1.02/0.01 | - | - | - | - | 328/419 | 0.78/0.74 |
| mslearn | ok/ok | 2.4/1.3 | 2.4/2.5 | 0.00/0.01 | 0 | - | timer0 | 0 | 229/295 | 0.85/0.85 |
| onepassword | ok/ok | 3.3/1.6 | 4.4/4.0 | 0.62/0.08 | 71 | animation | timer4 fetch2 event0 | 121 | 819/455 | 0.76/0.85 |
| pcpartpicker | ok/challenged | 0.6/- | 3.1/- | 0.21/- | 71 | animation | event5 timer1 | 118 | 159/- | - |
| plex | ok/ok | 2.9/0.9 | 10.5/5.7 | 0.98/0.53 | 62 | animation | event564 anima31 other31 timer0 | 26 | 385/585 | 0.73/0.73 |
| proton | ok/ok | -/2.3 | -/4.7 | 0.62/0.01 | 1 | changed | messa4 other1 event1 timer0 fetch0 | 7 | 267/369 | 0.77/0.74 |
| proton-drive | ok/ok | 3.1/2.5 | 3.1/9.1 | 0.00/0.01 | 0 | - | 0 | 0 | 164/378 | 0.72/0.72 |
| reddit | ok/challenged | 15.7/- | 17.6/- | 0.85/- | 56 | animation | timer2 anima0 event0 fetch0 messa0 | 89 | 734/- | - |
| reddit-home | ok/challenged | 8.8/- | 11.8/- | 0.70/- | 62 | animation | timer3 anima0 event0 messa0 | 102 | 638/- | - |
| stackoverflow | ok/ok | 0.6/1.7 | 7.0/4.7 | 1.16/0.06 | 2 | changed | event12 timer12 fetch4 anima0 messa0 other0 | - | 751/585 | 0.72/0.72 |
| twitch | ok/ok | 2.7/1.1 | 5.4/2.9 | 0.94/0.26 | 0 | changed | timer1 messa1 fetch0 anima0 event0 other0 | 1 | 576/735 | 0.67/0.86 |
| verge | ok/ok | 11.0/2.7 | 11.4/6.7 | 1.01/0.14 | 3 | animation | other33 timer21 event3 fetch1 messa0 | 1 | 974/765 | 0.76/0.64 |
| vimeo | ok/ok | 17.7/1.1 | 17.7/3.2 | 0.00/0.01 | 0 | - | timer4 | 3 | 515/738 | 0.86/0.82 |
| weather | ok/ok | 10.6/1.6 | 13.6/16.6 | 1.01/0.86 | 42 | animation | timer46 event36 fetch0 | 54 | 1600/- | 0.71/0.68 |
| wikipedia | ok/ok | 1.5/0.9 | 3.0/2.2 | 0.90/0.05 | 69 | animation | event0 | 89 | 207/340 | 0.81/0.78 |
| x | ok/load error | 4.6/- | 4.7/- | 0.00/- | 0 | - | timer0 fetch0 other0 | 1 | 246/- | - |
| yahoo-mail | ok/ok | 1.9/0.8 | 3.2/1.8 | 0.90/0.42 | 134 | changed | anima67 timer5 event4 other2 messa0 | 97 | 275/581 | 0.88/0.85 |
| youtube | ok/ok | 7.1/1.9 | 7.9/2.9 | 0.91/0.03 | 4 | asked | timer57 other12 fetch9 anima1 event0 messa0 | 5 | 545/545 | 0.75/0.83 |
| yt-watch | ok/ok | 10.5/2.3 | 14.1/4.3 | 0.98/0.03 | 1 | changed | timer10 event3 other1 anima1 fetch0 | 1 | 659/665 | 0.62/0.54 |

Ranked from it (the order of 2026-11-26):

1. Idle CPU: 18 sites at 0.6 to 1.4 cores left alone, Chrome at 0.3 or
   less. Animations ticked and restyled every frame off screen (9 sites,
   cause "animation"; Wikipedia's is an infinite transform and opacity
   pulse Chrome runs on its compositor); frames the browser asks for
   that nothing in the engine wants ("asked": ddg, bbc, youtube); and
   renderers busy in script (mdn, github, stackoverflow, ai-chat,
   chatgpt, cloudflare).
2. Load time on heavy sites: vimeo, reddit, verge, weather, yt-watch 5
   to 16 times Chrome's; proton, amazon-au and cloudflare never reach
   their load event.
3. Memory left alone: weather 1600 MiB; bbc, verge, onepassword,
   stackoverflow, reddit well over Chrome's. Light pages use less.
4. Likeness: reddit-home, yt-watch, twitch, reddit lowest.
