// Chrome on a probe page: the page's `window.__done` once the seconds
// are up, and its console, as JSON with the Chrome version it ran on.
//   node probe.mjs <page url> <out.json> <seconds>
// Headless, on a throwaway profile (never the user's), removed after.
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const [url, out, seconds] = process.argv.slice(2);
const exe = "C:/Program Files/Google/Chrome/Application/chrome.exe";
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "sse-chrome-"));
spawn(exe, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--disable-sync",
  "about:blank"], { stdio: "ignore" });
const portFile = path.join(profile, "DevToolsActivePort");
for (let i = 0; i < 200 && !fs.existsSync(portFile); i++) await sleep(50);
const [port, wsPath] = fs.readFileSync(portFile, "utf8").split("\n");
const ws = new WebSocket(`ws://127.0.0.1:${port}${wsPath}`);
await new Promise((res) => ws.addEventListener("open", res, { once: true }));
let next = 1;
const waiting = new Map();
const consoleLines = [];
ws.addEventListener("message", (event) => {
  const m = JSON.parse(event.data);
  if (m.id !== undefined) { const w = waiting.get(m.id); if (w) { waiting.delete(m.id); w(m.result || m.error); } }
  else if (m.method === "Log.entryAdded") consoleLines.push(`${m.params.entry.level} ${m.params.entry.source} ${m.params.entry.text}`);
  else if (m.method === "Runtime.consoleAPICalled") consoleLines.push("console " + m.params.args.map((a) => a.value).join(" "));
  else if (m.method === "Runtime.exceptionThrown") consoleLines.push("exception " + m.params.exceptionDetails.text);
});
const send = (method, params = {}, sessionId) => {
  const id = next++;
  ws.send(JSON.stringify({ id, method, params, sessionId }));
  return new Promise((res) => waiting.set(id, res));
};
const version = (await send("Browser.getVersion")).product;
const { targetId } = await send("Target.createTarget", { url: "about:blank" });
const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
await send("Log.enable", {}, sessionId);
await send("Runtime.enable", {}, sessionId);
await send("Page.navigate", { url }, sessionId);
await sleep(Number(seconds) * 1000);
const result = await send("Runtime.evaluate", { expression: "JSON.stringify(window.__done || null)", returnByValue: true }, sessionId);
fs.writeFileSync(out, JSON.stringify({ version, result: JSON.parse(result.result.value), console: consoleLines }, null, 1));
await send("Browser.close").catch(() => {});
await sleep(1500);
try { fs.rmSync(profile, { recursive: true, force: true }); } catch {}
process.exit(0);
