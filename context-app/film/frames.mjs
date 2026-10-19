/* frames.mjs - render exact frames of film.html through the cached headless
 * Chrome over CDP, as the platform film's frames.mjs does. It serves the
 * context-app folder itself (film.html reads ../crates/freebyrd/ui/ over
 * http), and uses Node 22+'s own WebSocket.
 *
 *   node frames.mjs OUT_DIR board 0.25,1.3,2.1      storyboard stills
 *   node frames.mjs OUT_DIR board seq:0:5:30        storyboard sequence (png)
 *   node frames.mjs OUT_DIR flat  seq:0:5:30        texture sequence + camera.json
 *
 * board: 1600x720 viewport (20:9) at --dsf (default 2), JPEG.
 * flat:  the 1512x982 screen alone at --scale (default 4), PNG.
 * --ui=DIR serves the notch's files from DIR instead of crates/freebyrd/ui
 * (a snapshot, so edits made meanwhile don't land mid-sequence).
 * --page=film2.html films another chapter (default film.html, chapter 1).
 */
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdirSync, readFileSync, writeFileSync, existsSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, "..");
const CHROME = `${process.env.HOME}/Library/Caches/ms-playwright/chromium_headless_shell-1228/chrome-headless-shell-mac-arm64/chrome-headless-shell`;

const [out, mode = "board", times = "0"] = process.argv.slice(2);
const arg = (name, def) => { const a = process.argv.find(x => x.startsWith(`--${name}=`)); return a ? a.slice(name.length + 3) : def; };
const SCALE = +arg("scale", 4);
const BOARD_DSF = +arg("dsf", 2);
const [WIN_W, WIN_H] = [1512, 982];
const [VP_W, VP_H] = arg("vp", "1600x720").split("x").map(Number);
const UI = arg("ui", null);
const PAGE = arg("page", "film.html");
const SEQ = times.startsWith("seq:");
mkdirSync(out, { recursive: true });

let list;
if (SEQ) {
  const [, a, b, fps] = times.split(":").map(Number);
  list = [];
  for (let f = 0; a + f / fps < b - 1e-9; f++) list.push(a + f / fps);
} else list = times.split(",").map(Number);

/* the folder over http; /crates/freebyrd/ui/ from --ui when given */
const TYPES = { ".html": "text/html", ".css": "text/css", ".js": "text/javascript", ".mjs": "text/javascript", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png" };
const server = createServer((req, res) => {
  let rel = decodeURIComponent(new URL(req.url, "http://x").pathname);
  let file = path.join(ROOT, rel);
  if (UI && rel.startsWith("/crates/freebyrd/ui/")) file = path.join(path.resolve(UI), rel.slice("/crates/freebyrd/ui/".length));
  if (!file.startsWith(ROOT) && !(UI && file.startsWith(path.resolve(UI)))) { res.writeHead(403); res.end(); return; }
  if (!existsSync(file) || statSync(file).isDirectory()) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream", "cache-control": "no-store" });
  res.end(readFileSync(file));
});
await new Promise(r => server.listen(0, "127.0.0.1", r));
const HTTP = `http://127.0.0.1:${server.address().port}`;

const port = 9400 + Math.floor(Math.random() * 400);
const chrome = spawn(CHROME, ["--headless", `--remote-debugging-port=${port}`, "--hide-scrollbars", "--force-color-profile=srgb", "--font-render-hinting=none", "about:blank"], { stdio: "ignore" });
const sleep = ms => new Promise(r => setTimeout(r, ms));
const quit = code => { try { chrome.kill(); } catch {} server.close(); if (code !== undefined) process.exit(code); };
process.on("exit", () => quit());
for (const sig of ["SIGINT", "SIGTERM"]) process.on(sig, () => quit(1));
process.on("uncaughtException", e => { console.error(e); quit(1); });
process.on("unhandledRejection", e => { console.error(e); quit(1); });

let target;
for (let i = 0; i < 60 && !target; i++) {
  await sleep(200);
  try { target = (await (await fetch(`http://127.0.0.1:${port}/json`)).json()).find(t => t.type === "page"); } catch {}
}
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise(r => ws.addEventListener("open", r, { once: true }));
let id = 0;
const pending = new Map();
ws.addEventListener("message", m => { const d = JSON.parse(m.data); if (d.id && pending.has(d.id)) { pending.get(d.id)(d); pending.delete(d.id); } });
const send = (method, params = {}) => new Promise(r => { const i = ++id; pending.set(i, r); ws.send(JSON.stringify({ id: i, method, params })); });
const evaluate = async expr => {
  const d = await send("Runtime.evaluate", { expression: expr, awaitPromise: true, returnByValue: true });
  if (d.result?.exceptionDetails) throw new Error(JSON.stringify(d.result.exceptionDetails).slice(0, 800));
  return d.result?.result?.value;
};

const flat = mode === "flat" || mode === "eval";
await send("Emulation.setDeviceMetricsOverride", flat
  ? { width: WIN_W, height: WIN_H, deviceScaleFactor: SCALE, mobile: false }
  : { width: VP_W, height: VP_H, deviceScaleFactor: BOARD_DSF, mobile: false });
await send("Runtime.enable");
await send("Page.enable");
await send("Page.navigate", { url: `${HTTP}/film/${PAGE}?mode=${mode === "eval" ? "flat" : mode}` });
for (let i = 0; i < 100; i++) { await sleep(100); if (await evaluate("window.filmReady === true").catch(() => false)) break; }
if (!(await evaluate("window.filmReady === true"))) throw new Error("film.html did not get ready");
if (mode === "eval") {                     /* node frames.mjs OUT_DIR eval "EXPR": print EXPR's value, in flat */
  console.log(JSON.stringify(await evaluate(times)));
  ws.close();
  quit(0);
}

for (const [i, t] of list.entries()) {
  await evaluate(`(seek(${t}), filmFrame())`);
  const shot = await send("Page.captureScreenshot", flat
    ? { format: "png", clip: { x: 0, y: 0, width: WIN_W, height: WIN_H, scale: 1 }, captureBeyondViewport: false }
    : SEQ ? { format: "png" } : { format: "jpeg", quality: 92 });
  const name = flat || SEQ ? `f${String(i).padStart(4, "0")}.png` : `t${t.toFixed(2)}.jpg`;
  writeFileSync(path.join(out, name), Buffer.from(shot.result.data, "base64"));
  if (!SEQ || i % 30 === 0) console.log(name);
}
if (flat) {
  /* the camera the film defines, sampled at every rendered frame, so Blender
   * films exactly the move the storyboard shows (screen px: centre + width) */
  const cams = await evaluate(`JSON.stringify(${JSON.stringify(list)}.map(t => camAt(t)))`);
  writeFileSync(path.join(out, "camera.json"), JSON.stringify({ window: [WIN_W, WIN_H], scale: SCALE, times: list, cam: JSON.parse(cams) }));
  console.log("camera.json");
}
ws.close();
quit(0);
