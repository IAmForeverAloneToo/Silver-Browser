// Chrome on a page: the text of its #out once the seconds are up.
// Headless, on a throwaway profile (never the user's), removed after.
//   node read-out.mjs <page url> <out file> <seconds> [time zone]
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const [url, out, seconds, zone] = process.argv.slice(2);
const exe = "C:/Program Files/Google/Chrome/Application/chrome.exe";
const profile = fs.mkdtempSync(path.join(os.tmpdir(), "out-chrome-"));
const chrome = spawn(exe, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
  "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--disable-sync",
  "about:blank"], { stdio: "ignore" });
const portFile = path.join(profile, "DevToolsActivePort");
for (let i = 0; i < 200 && !fs.existsSync(portFile); i++) await sleep(50);
const [port, wsPath] = fs.readFileSync(portFile, "utf8").split("\n");
const ws = new WebSocket(`ws://127.0.0.1:${port}${wsPath}`);
await new Promise((res) => ws.addEventListener("open", res, { once: true }));
let next = 1;
const waiting = new Map();
ws.addEventListener("message", (event) => {
  const m = JSON.parse(event.data);
  if (m.id !== undefined) { const w = waiting.get(m.id); if (w) { waiting.delete(m.id); w(m.result || m.error); } }
});
const send = (method, params = {}, sessionId) => {
  const id = next++;
  ws.send(JSON.stringify({ id, method, params, sessionId }));
  return new Promise((res) => waiting.set(id, res));
};
const version = (await send("Browser.getVersion")).product;
const { targetId } = await send("Target.createTarget", { url: "about:blank" });
const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
if (zone) {
  const r = await send("Emulation.setTimezoneOverride", { timezoneId: zone }, sessionId);
  if (r && r.code) throw new Error("time zone override: " + r.message);
}
await send("Page.navigate", { url }, sessionId);
await sleep(Number(seconds) * 1000);
const r = await send("Runtime.evaluate", { expression: "document.getElementById('out').textContent", returnByValue: true }, sessionId);
fs.writeFileSync(out, r.result.value);
console.log(version);
await send("Browser.close").catch(() => {});
await sleep(1500);
chrome.kill();
try { fs.rmSync(profile, { recursive: true, force: true }); } catch {}
process.exit(0);
