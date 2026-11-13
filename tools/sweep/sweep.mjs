// The site sweep's scoreboard: the release shell against the installed
// Chrome on the beta list, one load a site, both browsers in the same
// minute, three sites at a time. Writes target/sweep/<date>/report.html
// (screenshots side by side and the top causes across sites) and
// appends the numbers alone to docs/sweep-log.md.
//
//   node tools/sweep/sweep.mjs [--only id,id] [--no-chrome] [--no-log]
//     [--silver <exe>] [--chrome <exe>] [--out <dir>]
//
// Needs Node 22 or later (its WebSocket) and nothing else. Never signs
// in, never types credentials; every permission is denied, consent
// banners are left alone, and no link is followed.

import { spawn, execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const WIDTH = 1280;
const HEIGHT = 800;
const CAP_MS = 30_000;
const SETTLE_MS = 2_000;
const PARALLEL = 3;
const SCROLL_SCREENS = 3;

// The beta list (the lead's, 2026-11-13): the login sites are their
// signed-out landings.
export const SITES = [
  ["wikipedia", "https://en.wikipedia.org/wiki/Web_browser"],
  ["mdn", "https://developer.mozilla.org/en-US/docs/Web/CSS/display"],
  ["hn", "https://news.ycombinator.com/"],
  ["bbc", "https://www.bbc.com/news"],
  ["verge", "https://www.theverge.com/"],
  ["stackoverflow", "https://stackoverflow.com/questions/tagged/rust"],
  ["github", "https://github.com/rust-lang/rust"],
  ["docsrs", "https://docs.rs/tokio/latest/tokio/"],
  ["cratesio", "https://crates.io/crates/serde"],
  ["mslearn", "https://learn.microsoft.com/en-us/windows/win32/"],
  ["google", "https://www.google.com/search?q=rust+programming"],
  ["ddg", "https://duckduckgo.com/?q=rust+programming"],
  ["bing", "https://www.bing.com/search?q=rust+programming"],
  ["youtube", "https://www.youtube.com/"],
  ["twitch", "https://www.twitch.tv/directory"],
  ["vimeo", "https://vimeo.com/"],
  ["maps", "https://www.google.com/maps"],
  ["reddit", "https://www.reddit.com/r/rust/"],
  ["x", "https://x.com/"],
  ["gmail", "https://mail.google.com/"],
  ["gdocs", "https://docs.google.com/"],
  ["claude", "https://claude.ai/"],
  ["amazon", "https://www.amazon.com/s?k=usb+c+cable"],
  ["ebay", "https://www.ebay.com/sch/i.html?_nkw=usb+c+cable"],
  ["weather", "https://weather.com/"],
];

// ---- Arguments ----------------------------------------------------------

function args(argv) {
  const out = { only: null, chrome: true, log: true };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--only") out.only = argv[++i].split(",");
    else if (a === "--no-chrome") out.chrome = false;
    else if (a === "--no-log") out.log = false;
    else if (a === "--silver") out.silver = argv[++i];
    else if (a === "--chrome") out.chromeExe = argv[++i];
    else if (a === "--out") out.out = argv[++i];
    else throw new Error(`unknown argument ${a}`);
  }
  return out;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---- PNG, read for the similarity -----------------------------------------

// The pixels of an 8-bit, non-interlaced PNG of RGB or RGBA, as RGBA.
export function readPng(bytes) {
  if (bytes.readUInt32BE(0) !== 0x89504e47) throw new Error("not a PNG");
  let pos = 8;
  let width = 0, height = 0, type = 0;
  const idat = [];
  while (pos < bytes.length) {
    const length = bytes.readUInt32BE(pos);
    const name = bytes.toString("latin1", pos + 4, pos + 8);
    const data = bytes.subarray(pos + 8, pos + 8 + length);
    if (name === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      if (data[8] !== 8 || data[12] !== 0) throw new Error("an unread PNG kind");
      type = data[9];
    } else if (name === "IDAT") idat.push(data);
    else if (name === "IEND") break;
    pos += 12 + length;
  }
  const channels = type === 6 ? 4 : type === 2 ? 3 : 0;
  if (!channels) throw new Error(`PNG colour type ${type}`);
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const out = Buffer.alloc(width * height * 4);
  let prev = Buffer.alloc(stride);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    const line = Buffer.from(raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1)));
    for (let x = 0; x < stride; x++) {
      const a = x >= channels ? line[x - channels] : 0;
      const b = prev[x];
      const c = x >= channels ? prev[x - channels] : 0;
      let v = line[x];
      if (filter === 1) v += a;
      else if (filter === 2) v += b;
      else if (filter === 3) v += (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v += pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      line[x] = v & 255;
    }
    for (let x = 0; x < width; x++) {
      const o = (y * width + x) * 4;
      out[o] = line[x * channels];
      out[o + 1] = line[x * channels + 1];
      out[o + 2] = line[x * channels + 2];
      out[o + 3] = channels === 4 ? line[x * channels + 3] : 255;
    }
    prev = line;
  }
  return { width, height, pixels: out };
}

// A coarse likeness of two screenshots, 0 to 1: each scaled down to
// 32 by 20 blocks of mean luminance, compared block by block, and their
// edges (block-to-block steps) too, so that a blank page is not like a
// white one with text. For sorting only; the images are what is judged.
export function similarity(a, b) {
  const GW = 32, GH = 20;
  const grid = (img) => {
    const g = new Float64Array(GW * GH);
    const n = new Float64Array(GW * GH);
    for (let y = 0; y < img.height; y++) {
      const gy = Math.min(GH - 1, Math.floor((y * GH) / img.height));
      for (let x = 0; x < img.width; x++) {
        const gx = Math.min(GW - 1, Math.floor((x * GW) / img.width));
        const o = (y * img.width + x) * 4;
        const l = 0.2126 * img.pixels[o] + 0.7152 * img.pixels[o + 1] + 0.0722 * img.pixels[o + 2];
        g[gy * GW + gx] += l;
        n[gy * GW + gx] += 1;
      }
    }
    for (let i = 0; i < g.length; i++) g[i] = n[i] ? g[i] / n[i] : 0;
    return g;
  };
  const ga = grid(a), gb = grid(b);
  let lum = 0, edge = 0, edges = 0;
  for (let y = 0; y < GH; y++) {
    for (let x = 0; x < GW; x++) {
      const i = y * GW + x;
      lum += Math.abs(ga[i] - gb[i]);
      if (x + 1 < GW) {
        const ea = ga[i + 1] - ga[i], eb = gb[i + 1] - gb[i];
        edge += Math.abs(ea - eb);
        edges += Math.max(Math.abs(ea), Math.abs(eb));
      }
    }
  }
  const lumScore = 1 - lum / (GW * GH * 255);
  const edgeScore = edges > 0 ? 1 - Math.min(1, edge / (edges * 2)) : 1;
  return Math.round(1000 * (0.5 * lumScore + 0.5 * edgeScore)) / 1000;
}

// ---- Console, grouped ------------------------------------------------------

// A console line with its URLs, numbers and ids taken out, so that one
// cause on many pages is one group.
export function normalize(text) {
  return String(text)
    .replace(/\b[a-z][a-z0-9+.-]*:\/\/[^\s'")]+/gi, "<url>")
    .replace(/\b[0-9a-f]{8,}\b/gi, "<id>")
    .replace(/[A-Za-z0-9_-]{24,}/g, "<id>")
    .replace(/\d+(\.\d+)?/g, "N")
    .replace(/\s+/g, " ")
    .trim()
    .slice(0, 200);
}

// ---- Chrome over its DevTools protocol -------------------------------------

class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.next = 1;
    this.waiting = new Map();
    this.listeners = [];
    ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id !== undefined) {
        const waiter = this.waiting.get(message.id);
        if (waiter) {
          this.waiting.delete(message.id);
          message.error ? waiter.reject(new Error(message.error.message)) : waiter.resolve(message.result);
        }
      } else {
        for (const listener of this.listeners) listener(message);
      }
    });
  }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", reject, { once: true });
    });
    return new Cdp(ws);
  }
  send(method, params = {}, sessionId) {
    const id = this.next++;
    this.ws.send(JSON.stringify({ id, method, params, sessionId }));
    return new Promise((resolve, reject) => {
      this.waiting.set(id, { resolve, reject });
      setTimeout(() => {
        if (this.waiting.delete(id)) reject(new Error(`${method} timed out`));
      }, 20_000);
    });
  }
  on(listener) {
    this.listeners.push(listener);
    return () => (this.listeners = this.listeners.filter((l) => l !== listener));
  }
  close() {
    this.ws.close();
  }
}

function chromeExe(given) {
  const candidates = [
    given,
    process.env.CHROME,
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/usr/bin/google-chrome",
  ];
  return candidates.find((c) => c && fs.existsSync(c));
}

async function startChrome(exe) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), "sweep-chrome-"));
  const child = spawn(exe, [
    "--headless=new",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    `--window-size=${WIDTH},${HEIGHT}`,
    "--force-device-scale-factor=1",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-extensions",
    "--disable-sync",
    "--disable-background-networking",
    "--disable-component-update",
    "--mute-audio",
    "about:blank",
  ], { stdio: "ignore" });
  const portFile = path.join(profile, "DevToolsActivePort");
  for (let i = 0; i < 200 && !fs.existsSync(portFile); i++) await sleep(50);
  const [port, wsPath] = fs.readFileSync(portFile, "utf8").split("\n");
  const cdp = await Cdp.connect(`ws://127.0.0.1:${port}${wsPath}`);
  const version = await cdp.send("Browser.getVersion");
  for (const name of ["geolocation", "notifications", "videoCapture", "audioCapture", "midi", "clipboardReadWrite"]) {
    await cdp.send("Browser.setPermission", { permission: { name }, setting: "denied" }).catch(() => {});
  }
  return {
    cdp,
    version: version.product,
    // Ends Chrome and removes its throwaway profile once it has let go.
    async stop() {
      const exited = new Promise((resolve) => child.once("exit", resolve));
      // Chrome ends its own helper processes when asked to close; a kill
      // of the one started would leave them.
      await cdp.send("Browser.close").catch(() => {});
      cdp.close();
      await Promise.race([exited, sleep(10_000)]);
      if (child.exitCode === null) child.kill();
      for (let i = 0; i < 10; i++) {
        try {
          fs.rmSync(profile, { recursive: true, force: true });
          return;
        } catch {
          await sleep(500);
        }
      }
      console.error(`sweep: could not remove ${profile}`);
    },
  };
}

// One site in Chrome: its outcome, timings, console, screenshots.
async function captureChrome(chrome, url, dir) {
  const { cdp } = chrome;
  const result = { outcome: "ok", timings: {}, console: [], screenshots: {} };
  const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
  const send = (method, params) => cdp.send(method, params, sessionId);
  let start = 0;
  let crashed = false;
  const off = cdp.on((message) => {
    if (message.sessionId !== sessionId) return;
    const p = message.params;
    if (message.method === "Runtime.exceptionThrown") {
      const d = p.exceptionDetails;
      result.console.push({ kind: "exception", text: (d.exception && d.exception.description || d.text || "").split("\n")[0] });
    } else if (message.method === "Runtime.consoleAPICalled" && p.type === "error") {
      result.console.push({ kind: "error", text: p.args.map((a) => a.value ?? a.description ?? "").join(" ") });
    } else if (message.method === "Page.lifecycleEvent" && start) {
      if (p.name === "firstPaint" && result.timings.firstPaint === undefined) result.timings.firstPaint = Date.now() - start;
      if (p.name === "load" && result.timings.load === undefined) result.timings.load = Date.now() - start;
    } else if (message.method === "Inspector.targetCrashed") {
      crashed = true;
    }
  });
  try {
    await send("Page.enable");
    await send("Page.setLifecycleEventsEnabled", { enabled: true });
    await send("Runtime.enable");
    await send("Performance.enable");
    await send("Inspector.enable");
    await send("Emulation.setDeviceMetricsOverride", { width: WIDTH, height: HEIGHT, deviceScaleFactor: 1, mobile: false });
    start = Date.now();
    const nav = await send("Page.navigate", { url });
    if (nav.errorText) {
      result.outcome = "load error";
      result.error = nav.errorText;
      return result;
    }
    const layouts = async () => {
      const { metrics } = await send("Performance.getMetrics");
      return metrics.find((m) => m.name === "LayoutCount")?.value ?? 0;
    };
    let last = -1, since = Date.now();
    while (Date.now() - start < CAP_MS && !crashed) {
      await sleep(250);
      const count = await layouts().catch(() => last);
      if (count !== last) {
        last = count;
        since = Date.now();
      } else if (result.timings.firstPaint !== undefined && Date.now() - since >= SETTLE_MS) {
        result.timings.settled = since - start;
        break;
      }
    }
    if (crashed) {
      result.outcome = "renderer crash";
      return result;
    }
    if (result.timings.firstPaint === undefined) result.outcome = "hang";
    const shot = async (name) => {
      const { data } = await send("Page.captureScreenshot", { format: "png" });
      fs.writeFileSync(path.join(dir, name), Buffer.from(data, "base64"));
      result.screenshots[name.replace(/^chrome-|\.png$/g, "")] = name;
    };
    await shot("chrome-top.png");
    await send("Runtime.evaluate", { expression: `window.scrollBy(0, ${SCROLL_SCREENS * HEIGHT})` });
    let settled = Date.now();
    last = -1;
    while (Date.now() - settled < 6_000) {
      await sleep(250);
      const count = await layouts().catch(() => last);
      if (count !== last) last = count;
      else break;
    }
    await sleep(500);
    await shot("chrome-scrolled.png");
    return result;
  } catch (error) {
    result.outcome = result.outcome === "ok" ? "harness error" : result.outcome;
    result.error = String(error.message || error);
    return result;
  } finally {
    off();
    await cdp.send("Target.closeTarget", { targetId }).catch(() => {});
  }
}

// ---- Silver, by the shell's --sweep-site ----------------------------------

function silverExe(given) {
  const candidates = [
    given,
    path.join(ROOT, "target", "release", process.platform === "win32" ? "silver-browser.exe" : "silver-browser"),
  ];
  return candidates.find((c) => c && fs.existsSync(c));
}

function captureSilver(exe, url, dir) {
  return new Promise((resolve) => {
    const child = spawn(exe, ["--sweep-site", url, "--sweep-out", dir], { stdio: ["ignore", "ignore", "pipe"] });
    let stderr = "";
    child.stderr.on("data", (d) => (stderr += d).length > 20_000 && (stderr = stderr.slice(-20_000)));
    const timer = setTimeout(() => child.kill(), CAP_MS + 30_000);
    child.on("exit", (code) => {
      clearTimeout(timer);
      const file = path.join(dir, "silver.json");
      let record;
      try {
        record = JSON.parse(fs.readFileSync(file, "utf8"));
      } catch {
        record = { outcome: "harness error", error: `no record (exit ${code}): ${stderr.slice(-400)}` };
      }
      resolve(record);
    });
  });
}

// ---- The run ---------------------------------------------------------------

async function pool(items, size, work) {
  const results = new Array(items.length);
  let next = 0;
  await Promise.all(
    Array.from({ length: size }, async () => {
      while (next < items.length) {
        const i = next++;
        results[i] = await work(items[i], i);
      }
    }),
  );
  return results;
}

function commit() {
  try {
    return execFileSync("git", ["rev-parse", "--short=9", "HEAD"], { cwd: ROOT }).toString().trim();
  } catch {
    return "unknown";
  }
}

export async function run(options) {
  const date = new Date().toISOString().slice(0, 10);
  const out = options.out || path.join(ROOT, "target", "sweep", date);
  fs.mkdirSync(out, { recursive: true });
  const silver = silverExe(options.silver);
  if (!silver) throw new Error("no release shell: cargo build --release -p silver-browser");
  const exe = options.chrome ? chromeExe(options.chromeExe) : null;
  if (options.chrome && !exe) throw new Error("no Chrome found");
  const chrome = exe ? await startChrome(exe) : null;
  const sites = SITES.filter(([id]) => !options.only || options.only.includes(id));
  const began = Date.now();
  const rows = await pool(sites, PARALLEL, async ([id, url]) => {
    const dir = path.join(out, id);
    fs.rmSync(dir, { recursive: true, force: true });
    fs.mkdirSync(dir, { recursive: true });
    const [s, c] = await Promise.all([
      captureSilver(silver, url, dir),
      chrome ? captureChrome(chrome, url, dir) : Promise.resolve(null),
    ]);
    const row = { id, url, silver: s, chrome: c, similarity: {} };
    for (const which of ["top", "scrolled"]) {
      const a = path.join(dir, `silver-${which}.png`), b = path.join(dir, `chrome-${which}.png`);
      if (fs.existsSync(a) && fs.existsSync(b)) {
        try {
          row.similarity[which] = similarity(readPng(fs.readFileSync(a)), readPng(fs.readFileSync(b)));
        } catch (error) {
          row.similarity[which] = null;
        }
      }
    }
    fs.writeFileSync(path.join(dir, "row.json"), JSON.stringify(row, null, 1));
    console.log(`${id}: silver ${s.outcome}, chrome ${c ? c.outcome : "-"}, similarity ${row.similarity.top ?? "-"}`);
    return row;
  });
  if (chrome) await chrome.stop();
  const meta = {
    date,
    commit: commit(),
    chrome: chrome ? chrome.version : null,
    seconds: Math.round((Date.now() - began) / 1000),
  };
  const causes = topCauses(rows);
  fs.writeFileSync(path.join(out, "report.html"), report(meta, rows, causes));
  fs.writeFileSync(path.join(out, "run.json"), JSON.stringify({ meta, rows, causes }, null, 1));
  if (options.log) appendLog(meta, rows, causes);
  return { out, meta, rows, causes };
}

// Each cause with the sites it hit: the names missing, the console's
// groups, and the counters, most sites first.
export function topCauses(rows) {
  const causes = new Map();
  const hit = (key, kind, site, amount = 1) => {
    if (!amount) return;
    const cause = causes.get(key) || { key, kind, sites: new Set(), total: 0 };
    cause.sites.add(site);
    cause.total += amount;
    causes.set(key, cause);
  };
  for (const row of rows) {
    const s = row.silver || {};
    const counts = s.counters || {};
    for (const [name, n] of Object.entries(counts.missing || {})) hit(`missing: ${name}`, "unimplemented", row.id, n);
    for (const [name, n] of Object.entries(counts.mediaUnsupported || {})) hit(`media: ${name}`, "media type", row.id, n);
    hit("stack: Maximum call stack", "counter", row.id, counts.stackOverflows || 0);
    hit("script stopped: too long", "counter", row.id, counts.budgetStops || 0);
    hit("G90: same-origin frame refused", "counter", row.id, counts.frameRefusedSameOrigin || 0);
    hit("G90: same-origin contentDocument null", "counter", row.id, counts.frameDocumentSameOrigin || 0);
    hit("policy: CSP refusal", "counter", row.id, counts.policyRefusals || 0);
    const net = s.network || {};
    for (const [kind, n] of Object.entries(net.failedByKind || {})) hit(`network: ${kind}`, "network", row.id, n);
    for (const [dest, n] of Object.entries(net.failedByDestination || {})) hit(`failed ${dest}`, "subresource", row.id, n);
    const groups = new Map();
    for (const entry of s.console || []) {
      if (entry.source !== "exception" && entry.level !== "error") continue;
      const key = `console: ${normalize(entry.text)}`;
      groups.set(key, (groups.get(key) || 0) + 1);
    }
    for (const [key, n] of groups) hit(key, "console", row.id, n);
    if (s.outcome && s.outcome !== "ok") hit(`outcome: ${s.outcome}`, "outcome", row.id);
  }
  return [...causes.values()]
    .map((c) => ({ key: c.key, kind: c.kind, sites: c.sites.size, siteList: [...c.sites], total: c.total }))
    .sort((a, b) => b.sites - a.sites || b.total - a.total || a.key.localeCompare(b.key));
}

const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const ms = (v) => (v === undefined || v === null ? "-" : `${(v / 1000).toFixed(1)}s`);
const mib = (v) => (v ? `${Math.round(v / 1048576)}` : "-");

function report(meta, rows, causes) {
  const siteRows = rows
    .map((r) => {
      const s = r.silver || {}, c = r.chrome || {};
      const pic = (who, which) => {
        const name = `${r.id}/${who}-${which}.png`;
        return `<figure><img loading=lazy src="${esc(name)}" alt=""><figcaption>${who} ${which}</figcaption></figure>`;
      };
      const counts = s.counters || {};
      const missing = Object.entries(counts.missing || {}).map(([k, v]) => `${esc(k)}&times;${v}`).join(", ");
      return `<section id="${esc(r.id)}"><h2>${esc(r.id)} <small>${esc(r.url)}</small></h2>
<p><b>Silver</b> ${esc(s.outcome)}${s.error ? ` (${esc(s.error)})` : ""}${s.dump ? ` dump ${esc(s.dump)}` : ""};
paint ${ms(s.timings?.firstPaint)}, load ${ms(s.timings?.load)}, settled ${ms(s.timings?.settled)};
renderers peak ${mib(s.memory?.rendererPeakWorkingSet)} MiB / commit ${mib(s.memory?.rendererPeakCommit)} MiB, browser ${mib(s.memory?.browserPeakWorkingSet)} / ${mib(s.memory?.browserPeakCommit)} MiB.
<b>Chrome</b> ${esc(c.outcome)}${c.error ? ` (${esc(c.error)})` : ""}; paint ${ms(c.timings?.firstPaint)}, load ${ms(c.timings?.load)}, settled ${ms(c.timings?.settled)}.
<b>Likeness</b> top ${r.similarity.top ?? "-"}, scrolled ${r.similarity.scrolled ?? "-"}.</p>
<p>Missing: ${missing || "none"}; stack ${counts.stackOverflows || 0}; G90 ${(counts.frameRefusedSameOrigin || 0) + (counts.frameDocumentSameOrigin || 0)}; CSP ${counts.policyRefusals || 0}; console errors ${(s.console || []).filter((e) => e.level === "error").length} (Chrome ${(c.console || []).length}).</p>
<div class=pair>${pic("silver", "top")}${pic("chrome", "top")}</div>
<div class=pair>${pic("silver", "scrolled")}${pic("chrome", "scrolled")}</div></section>`;
    })
    .join("\n");
  const causeRows = causes
    .map((c) => `<tr><td>${c.sites}</td><td>${c.total}</td><td>${esc(c.kind)}</td><td>${esc(c.key)}</td><td>${esc(c.siteList.join(" "))}</td></tr>`)
    .join("\n");
  const summary = rows
    .map((r) => `<tr><td><a href="#${esc(r.id)}">${esc(r.id)}</a></td><td>${esc(r.silver?.outcome)}</td><td>${esc(r.chrome?.outcome ?? "-")}</td><td>${r.similarity.top ?? "-"}</td><td>${r.similarity.scrolled ?? "-"}</td><td>${ms(r.silver?.timings?.settled)}</td><td>${ms(r.chrome?.timings?.settled)}</td></tr>`)
    .join("\n");
  return `<!DOCTYPE html><meta charset=utf-8><title>Site sweep ${esc(meta.date)}</title>
<style>body{font:14px system-ui,sans-serif;margin:16px;max-width:1400px}table{border-collapse:collapse}td,th{border:1px solid #ccc;padding:2px 6px;text-align:left}
.pair{display:flex;gap:8px}.pair figure{margin:0;flex:1}.pair img{width:100%;border:1px solid #999}small{color:#666;font-weight:normal}section{border-top:2px solid #333;margin-top:24px}</style>
<h1>Site sweep ${esc(meta.date)}</h1>
<p>Silver ${esc(meta.commit)} against ${esc(meta.chrome ?? "no reference")}, ${rows.length} sites in ${meta.seconds}s, ${WIDTH}x${HEIGHT} at 1x.</p>
<h2>Sites</h2><table><tr><th>site</th><th>Silver</th><th>Chrome</th><th>likeness top</th><th>scrolled</th><th>settled</th><th>Chrome settled</th></tr>${summary}</table>
<h2>Top causes</h2><table><tr><th>sites</th><th>total</th><th>kind</th><th>cause</th><th>where</th></tr>${causeRows}</table>
${siteRows}`;
}

// The numbers alone, appended to docs/sweep-log.md: no screenshots and
// no page text but the causes' normalized names.
function appendLog(meta, rows, causes) {
  const file = path.join(ROOT, "docs", "sweep-log.md");
  const lines = [];
  if (!fs.existsSync(file)) {
    lines.push(
      "# Site sweep log",
      "",
      "Each run of `tools/sweep/sweep.mjs`: the release shell against the",
      "installed Chrome, one load a site at 1280x800. Numbers only; the",
      "screenshots and the full report stay in `target/sweep/<date>/`.",
      "",
    );
  }
  lines.push(`## ${meta.date}, ${meta.commit}, against ${meta.chrome ?? "no reference"}`, "");
  lines.push("| site | outcome | likeness top / scrolled | paint / load / settled (s) | Chrome settled (s) | missing | console errors | stack | G90 | CSP | failed requests |");
  lines.push("|---|---|---|---|---|---|---|---|---|---|---|");
  const s1 = (v) => (v === undefined || v === null ? "-" : (v / 1000).toFixed(1));
  for (const r of rows) {
    const s = r.silver || {};
    const k = s.counters || {};
    const failed = Object.values(s.network?.failedByDestination || {}).reduce((a, b) => a + b, 0);
    lines.push(
      `| ${r.id} | ${s.outcome ?? "-"} | ${r.similarity.top ?? "-"} / ${r.similarity.scrolled ?? "-"} | ${s1(s.timings?.firstPaint)} / ${s1(s.timings?.load)} / ${s1(s.timings?.settled)} | ${s1(r.chrome?.timings?.settled)} | ${Object.values(k.missing || {}).reduce((a, b) => a + b, 0)} | ${(s.console || []).filter((e) => e.level === "error").length} | ${k.stackOverflows || 0} | ${(k.frameRefusedSameOrigin || 0) + (k.frameDocumentSameOrigin || 0)} | ${k.policyRefusals || 0} | ${failed} |`,
    );
  }
  lines.push("", "Top 10 causes (sites, total):", "");
  for (const cause of causes.slice(0, 10)) lines.push(`- ${cause.sites}, ${cause.total}: ${cause.key.replace(/\|/g, "/")}`);
  lines.push("");
  fs.appendFileSync(file, lines.join("\n") + "\n");
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  run(args(process.argv.slice(2)))
    .then(({ out }) => console.log(`report: ${path.join(out, "report.html")}`))
    .catch((error) => {
      console.error(`sweep: ${error.stack || error}`);
      process.exit(1);
    });
}
