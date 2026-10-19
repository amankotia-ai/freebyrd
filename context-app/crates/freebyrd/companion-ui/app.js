// Freebyrd for iPad: the frame, the calls to the Mac, and what every screen
// shares (cards, rows, Markdown, drafts). The screens are in screens.js.
//
// Everything shown comes from the Mac's memory (messages, pages, meetings,
// Claude's answers) and is set as text, never parsed as HTML.

"use strict";

const $ = (id) => document.getElementById(id);
const main = $("main");

// ---------- Small helpers ----------

// An element with a class and children; strings become text.
function el(tag, className, ...children) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    node.append(typeof child === "string" || typeof child === "number" ? String(child) : child);
  }
  return node;
}

// A solid icon; inner shapes of `path` are cut out of the outer one.
function icon(path) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("class", "icon");
  const shape = document.createElementNS("http://www.w3.org/2000/svg", "path");
  shape.setAttribute("d", path);
  shape.setAttribute("fill-rule", "evenodd");
  svg.append(shape);
  return svg;
}

function button(label, className, onClick) {
  const node = el("button", className ? `btn ${className}` : "btn");
  node.type = "button";
  if (typeof label === "string") node.textContent = label;
  else node.append(...label);
  if (onClick) node.addEventListener("click", onClick);
  return node;
}

function status() {
  return el("span", "status");
}

function showError(node, err) {
  node.textContent = err?.message ?? String(err);
  node.classList.add("error");
}

let toastTimer = 0;
function toast(text) {
  const node = $("toast");
  node.textContent = text;
  node.classList.add("on");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => node.classList.remove("on"), 2600);
}

const pad2 = (n) => String(n).padStart(2, "0");
const clock = (date = new Date()) => `${pad2(date.getHours())}:${pad2(date.getMinutes())}`;

// "5 min ago", "2 h ago", "yesterday".
function ago(ms) {
  const minutes = Math.round((Date.now() - ms) / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.round(hours / 24);
  return days === 1 ? "yesterday" : `${days} days ago`;
}

// ---------- Solid icons ----------

const P = {
  sun: "M12 7a5 5 0 1 1 0 10a5 5 0 0 1 0-10Z M11 1.5h2v3.5h-2Z M11 19h2v3.5h-2Z M1.5 11H5v2H1.5Z M19 11h3.5v2H19Z M3.5 4.9L4.9 3.5 7.4 6 6 7.4Z M17 6l2.5-2.5 1.4 1.4-2.5 2.5Z M6 16.6l1.4 1.4-2.5 2.5-1.4-1.4Z M16.6 18L18 16.6l2.5 2.5-1.4 1.4Z",
  tray: "M6.2 3h11.6a2 2 0 0 1 1.85 1.24L22 10v8a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3v-8l2.35-5.76A2 2 0 0 1 6.2 3Z M4.9 10H8a1 1 0 0 1 1 1 3 3 0 0 0 6 0 1 1 0 0 1 1-1h3.1l-1.85-5H6.75Z",
  check: "M6 3h12a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3Z M16.7 8.3a1 1 0 0 0-1.4 0l-4.8 4.8-1.8-1.8a1 1 0 1 0-1.4 1.4l2.5 2.5a1 1 0 0 0 1.4 0l5.5-5.5a1 1 0 0 0 0-1.4Z",
  circleCheck: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M16.7 8.3a1 1 0 0 0-1.4 0l-4.8 4.8-1.8-1.8a1 1 0 1 0-1.4 1.4l2.5 2.5a1 1 0 0 0 1.4 0l5.5-5.5a1 1 0 0 0 0-1.4Z",
  people: "M9 3.5a4 4 0 1 1 0 8a4 4 0 0 1 0-8Z M2 20.5a7 7 0 0 1 14 0v.5H2Z M17 4.5a3.5 3.5 0 1 1 0 7a3.5 3.5 0 0 1 0-7Z M18.2 13.2a5.5 5.5 0 0 1 3.8 5.3V21h-4.3v-.5a8.8 8.8 0 0 0-1.6-5.1 5.5 5.5 0 0 1 2.1-2.2Z",
  calendar: "M8 2a1 1 0 0 1 1 1v1h6V3a1 1 0 1 1 2 0v1h1a3 3 0 0 1 3 3v11a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3h1V3a1 1 0 0 1 1-1Z M5 9.5h14V11H5Z M7.5 13.25h3v3h-3Z",
  chat: "M5 3h14a3 3 0 0 1 3 3v9a3 3 0 0 1-3 3h-7l-5 4v-4H5a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3Z",
  mic: "M12 2a3.5 3.5 0 0 1 3.5 3.5v6a3.5 3.5 0 0 1-7 0v-6A3.5 3.5 0 0 1 12 2Z M5 11a1 1 0 0 1 1 1 6 6 0 0 0 12 0 1 1 0 1 1 2 0 8 8 0 0 1-7 7.94V21h2a1 1 0 1 1 0 2H9a1 1 0 1 1 0-2h2v-1.06A8 8 0 0 1 4 12a1 1 0 0 1 1-1Z",
  pencil: "M16.59 3.41a2 2 0 0 1 2.82 0l1.18 1.18a2 2 0 0 1 0 2.82L9.4 18.6a2 2 0 0 1-.88.51l-4.27 1.22a.8.8 0 0 1-.99-.99l1.22-4.27a2 2 0 0 1 .51-.88L16.59 3.41Z",
  book: "M2.5 4.6c3-.9 6-.5 8.5 1.2v14.4c-2.5-1.6-5.5-2-8.5-1.1Z M13 5.8c2.5-1.7 5.5-2.1 8.5-1.2v14.5c-3-.9-6-.5-8.5 1.1Z",
  video: "M3.5 5.5h10a2.5 2.5 0 0 1 2.5 2.5v8a2.5 2.5 0 0 1-2.5 2.5h-10A2.5 2.5 0 0 1 1 16V8a2.5 2.5 0 0 1 2.5-2.5Z M17.5 10l4-2.7a1 1 0 0 1 1.5.8v7.8a1 1 0 0 1-1.5.8l-4-2.7Z",
  term: "M4 3h16a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H4a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3Z M6.3 8.3a1 1 0 0 0 0 1.4L8.6 12l-2.3 2.3a1 1 0 1 0 1.4 1.4l3-3a1 1 0 0 0 0-1.4l-3-3a1 1 0 0 0-1.4 0Z M12.5 14.5a1 1 0 1 0 0 2h5a1 1 0 1 0 0-2Z",
  moon: "M20.6 14.2A8.6 8.6 0 0 1 9.8 3.4a.7.7 0 0 0-.9-.86A9.6 9.6 0 1 0 21.46 15.1a.7.7 0 0 0-.86-.9Z",
  bars: "M3 12.5h4.5V21H3Z M9.75 7.5h4.5V21h-4.5Z M16.5 3h4.5v18h-4.5Z",
  stand: "M5 2h14a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2Z M10 18.5h4l.9 2.5H17a.5.5 0 0 1 0 1H7a.5.5 0 0 1 0-1h2.1Z",
  gear: "M10.3 2h3.4l.5 2.6a8 8 0 0 1 1.9 1.1l2.5-.9 1.7 3-2 1.7a8 8 0 0 1 0 2.2l2 1.7-1.7 3-2.5-.9a8 8 0 0 1-1.9 1.1l-.5 2.6h-3.4l-.5-2.6a8 8 0 0 1-1.9-1.1l-2.5.9-1.7-3 2-1.7a8 8 0 0 1 0-2.2l-2-1.7 1.7-3 2.5.9a8 8 0 0 1 1.9-1.1Z M12 8.5a3.5 3.5 0 1 0 0 7a3.5 3.5 0 0 0 0-7Z",
  send: "M12 3a1.2 1.2 0 0 1 .85.35l6 6a1.2 1.2 0 0 1-1.7 1.7l-3.95-3.95V20a1.2 1.2 0 0 1-2.4 0V7.1l-3.95 3.95a1.2 1.2 0 0 1-1.7-1.7l6-6A1.2 1.2 0 0 1 12 3Z",
  copy: "M9 2h9a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2h-1V8a3 3 0 0 0-3-3H7V4a2 2 0 0 1 2-2Z M6 7h8a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2Z",
  dismiss: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M8.7 7.3a1 1 0 0 0-1.4 1.4l3.3 3.3-3.3 3.3a1 1 0 1 0 1.4 1.4l3.3-3.3 3.3 3.3a1 1 0 0 0 1.4-1.4L13.4 12l3.3-3.3a1 1 0 0 0-1.4-1.4L12 10.6 8.7 7.3Z",
  back: "M14.7 6.3a1 1 0 0 1 0 1.4L10.4 12l4.3 4.3a1 1 0 0 1-1.4 1.4l-5-5a1 1 0 0 1 0-1.4l5-5a1 1 0 0 1 1.4 0Z",
  chevron: "M9.3 6.3a1 1 0 0 1 1.4 0l5 5a1 1 0 0 1 0 1.4l-5 5a1 1 0 0 1-1.4-1.4L13.6 12 9.3 7.7a1 1 0 0 1 0-1.4Z",
  film: "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z M6 6v2h2V6H6Z M6 11v2h2v-2H6Z M6 16v2h2v-2H6Z M16 6v2h2V6h-2Z M16 11v2h2v-2h-2Z M16 16v2h2v-2h-2Z",
  list: "M3.5 5.5a1.5 1.5 0 1 1 3 0a1.5 1.5 0 0 1-3 0Z M9 4.5h11a1 1 0 1 1 0 2H9a1 1 0 0 1 0-2Z M3.5 12a1.5 1.5 0 1 1 3 0a1.5 1.5 0 0 1-3 0Z M9 11h11a1 1 0 1 1 0 2H9a1 1 0 1 1 0-2Z M3.5 18.5a1.5 1.5 0 1 1 3 0a1.5 1.5 0 0 1-3 0Z M9 17.5h11a1 1 0 1 1 0 2H9a1 1 0 1 1 0-2Z",
  wait: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M9 8a1 1 0 0 0-1 1v6a1 1 0 1 0 2 0V9a1 1 0 0 0-1-1Z M15 8a1 1 0 0 0-1 1v6a1 1 0 1 0 2 0V9a1 1 0 0 0-1-1Z",
  error: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M11 6.5v7h2v-7h-2Z M11 15.5v2h2v-2h-2Z",
  spark: "M11 2.5l2.3 6.2 6.2 2.3-6.2 2.3L11 19.5l-2.3-6.2L2.5 11l6.2-2.3L11 2.5Z M18.5 14.5l1 2.5 2.5 1-2.5 1-1 2.5-1-2.5-2.5-1 2.5-1 1-2.5Z",
  camera: "M9 3h6l1.5 2H19a3 3 0 0 1 3 3v10a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3V8a3 3 0 0 1 3-3h2.5Z M12 8.5a4.5 4.5 0 1 0 0 9a4.5 4.5 0 0 0 0-9Z",
  play: "M8 5.6v12.8a1 1 0 0 0 1.53.85l10.2-6.4a1 1 0 0 0 0-1.7L9.53 4.75A1 1 0 0 0 8 5.6Z",
  laptop: "M5 4h14a2 2 0 0 1 2 2v9H3V6a2 2 0 0 1 2-2Z M1 16.5h22v.5a2.5 2.5 0 0 1-2.5 2.5h-17A2.5 2.5 0 0 1 1 17Z",
  undo: "M9.7 4.3a1 1 0 0 1 0 1.4L7.4 8H14a6 6 0 1 1 0 12h-3a1 1 0 1 1 0-2h3a4 4 0 1 0 0-8H7.4l2.3 2.3a1 1 0 0 1-1.4 1.4l-4-4a1 1 0 0 1 0-1.4l4-4a1 1 0 0 1 1.4 0Z",
};

// Each kind of card has a side of the notch, a solid icon and a color, as
// in the notch (ui/app.js): left for what happened, right for what needs
// the person.
const KINDS = {
  recap: { label: "Recap", side: "left", color: "#64d2ff", path: "M7 2h7.59a2 2 0 0 1 1.41.59l3.41 3.41A2 2 0 0 1 20 7.41V20a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2Z M8.75 7a1 1 0 0 0 0 2h3.5a1 1 0 1 0 0-2h-3.5Z M8.75 11a1 1 0 1 0 0 2h7.5a1 1 0 1 0 0-2h-7.5Z M8.75 15a1 1 0 1 0 0 2h7.5a1 1 0 1 0 0-2h-7.5Z" },
  night: { label: "Night shift", side: "left", color: "#5e5ce6", path: P.moon },
  nightplan: { label: "Tonight", side: "right", color: "#5e5ce6", path: P.moon },
  // The easy stop (night/stop.rs): near the usual stop, what is open handed
  // to the night shift with one press.
  stop: { label: "Stop for tonight", side: "right", color: "#5e5ce6", path: P.moon },
  job: { label: "Job", side: "left", color: "#5e5ce6", path: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M12 6.5a1 1 0 0 0-1 1v4.91a1 1 0 0 0 .29.7l3 3a1 1 0 0 0 1.42-1.41L13 12v-4.5a1 1 0 0 0-1-1Z" },
  due: { label: "Due", side: "right", color: "#ff9f0a", path: P.calendar },
  draft: { label: "Draft", side: "right", color: "#30d158", path: P.pencil },
  suggestion: { label: "Suggestion", side: "right", color: "#bf5af2", path: "M12 2.5l2.04 6.12a2 2 0 0 0 1.26 1.26L21.5 12l-6.2 2.12a2 2 0 0 0-1.26 1.26L12 21.5l-2.04-6.12a2 2 0 0 0-1.26-1.26L2.5 12l6.2-2.12a2 2 0 0 0 1.26-1.26L12 2.5Z" },
  event: { label: "Event", side: "right", color: "#ff375f", path: "M8 2a1 1 0 0 1 1 1v1h6V3a1 1 0 1 1 2 0v1h1a3 3 0 0 1 3 3v11a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3h1V3a1 1 0 0 1 1-1Z M12 10.5a1 1 0 0 0-1 1V13H9.5a1 1 0 1 0 0 2H11v1.5a1 1 0 1 0 2 0V15h1.5a1 1 0 1 0 0-2H13v-1.5a1 1 0 0 0-1-1Z" },
  block: { label: "Plan", side: "right", color: "#0a84ff", path: "M9 3h6a2 2 0 0 1 2 2v1h2a3 3 0 0 1 3 3v9a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3V9a3 3 0 0 1 3-3h2V5a2 2 0 0 1 2-2Z M9 5v1h6V5H9Z" },
  question: { label: "Question", side: "right", color: "#ffd60a", path: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M12 6.5a3.5 3.5 0 0 0-3.5 3.5 1 1 0 1 0 2 0 1.5 1.5 0 1 1 2.12 1.37c-.79.35-1.62 1.05-1.62 2.13v.5a1 1 0 1 0 2 0v-.37c0-.08.07-.2.44-.37A3.5 3.5 0 0 0 12 6.5Z M12 15.75a1.25 1.25 0 1 0 0 2.5 1.25 1.25 0 0 0 0-2.5Z" },
  idea: { label: "Idea", side: "right", color: "#ac8e68", path: "M12 2a7 7 0 0 0-4 12.74V17a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1v-2.26A7 7 0 0 0 12 2Z M9 19.5h6a.5.5 0 0 1 .5.5 2 2 0 0 1-2 2h-3a2 2 0 0 1-2-2 .5.5 0 0 1 .5-.5Z" },
};
const kindOf = (name) => KINDS[name] ?? { label: name, side: "right", color: "#ffffff", path: "M12 6a6 6 0 1 1 0 12a6 6 0 0 1 0-12Z" };

// A day plan's blocks, by what the time is for.
const BLOCKS = {
  work: { label: "Work", color: "#0a84ff", path: KINDS.block.path },
  break: { label: "Break", color: "#63e6e2", path: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M9 8a1 1 0 0 0-1 1v6a1 1 0 1 0 2 0V9a1 1 0 0 0-1-1Z M15 8a1 1 0 0 0-1 1v6a1 1 0 1 0 2 0V9a1 1 0 0 0-1-1Z" },
  life: { label: "Life", color: "#ff7a59", path: "M12 21s-7.5-4.6-9.6-9.4C.9 8.2 2.9 4.5 6.6 4.5c2.1 0 3.6 1.1 5.4 3.2 1.8-2.1 3.3-3.2 5.4-3.2 3.7 0 5.7 3.7 4.2 7.1C19.5 16.4 12 21 12 21Z" },
};
// A suggestion once answered (`state` in its data, as in the notch): the
// header says where the fix stands. Set up is a green check that stays lit.
const ANSWERED = {
  added: { label: "Skill added", color: "#30d158", path: P.circleCheck, lit: true },
  scheduled: { label: "Scheduled", color: "#30d158", path: P.circleCheck, lit: true },
  paused: { label: "Paused", color: "#ffffff", path: P.wait },
  removed: { label: "Removed", color: "#ffffff", path: P.dismiss },
  declined: { label: "Declined", color: "#ffffff", path: P.dismiss },
};
// Pause and Remove are never the white button, even first.
const QUIET_ACTIONS = new Set(["pause", "remove"]);
const cardKind = (card) =>
  (card.kind === "block" && BLOCKS[card.data?.category]) ||
  (card.kind === "suggestion" && ANSWERED[card.data?.state]) ||
  kindOf(card.kind);

function kindLabel(kind, text, lit = false) {
  const node = el("span", lit ? "kind lit" : "kind", icon(kind.path), text ?? kind.label);
  node.style.setProperty("--kind", kind.color);
  return node;
}

// ---------- Talking to the Mac ----------

const TOKEN_KEY = "freebyrd.token";
const keep = {
  get(key) {
    try { return localStorage.getItem(key); } catch { return null; }
  },
  set(key, value) {
    try { localStorage.setItem(key, value); } catch {}
  },
  remove(key) {
    try { localStorage.removeItem(key); } catch {}
  },
};

let token = keep.get(TOKEN_KEY);

class Unpaired extends Error {}

// One call to the Mac: `/api/call/<name>`, the notch's own calls where it
// has them. Throws the Mac's own words when it says no.
async function call(name, args = {}) {
  let response;
  try {
    response = await fetch(`/api/call/${name}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body: JSON.stringify(args),
    });
  } catch {
    setReachable(false);
    throw new Error("Your Mac isn't answering. It may be asleep, or this iPad is off its network.");
  }
  setReachable(true);
  if (response.status === 401) {
    forget();
    throw new Unpaired("This iPad is no longer paired.");
  }
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body.error || `Your Mac answered ${response.status}.`);
  return body;
}

// A screen's data: fresh from the Mac, or what it sent last if it can't
// be reached now (`stale` is when that was).
async function load(name, args = {}) {
  const key = `freebyrd.cache.${name}`;
  try {
    const data = await call(name, args);
    keep.set(key, JSON.stringify({ at: Date.now(), data }));
    return { data, stale: null };
  } catch (err) {
    if (err instanceof Unpaired) throw err;
    const kept = keep.get(key);
    if (kept) {
      const { at, data } = JSON.parse(kept);
      return { data, stale: at };
    }
    throw err;
  }
}

function staleBanner(at) {
  const time = new Date(at);
  return el("div", "banner", el("span", "dot"), `Your Mac isn't answering. This is what it sent at ${clock(time)}${Date.now() - at > 86400000 ? `, ${time.toLocaleDateString()}` : ""}; buttons wait until it's back.`);
}

// A picture or a reel, which needs the token, as a local address.
const blobs = new Map();
async function mediaUrl(path) {
  if (blobs.has(path)) return blobs.get(path);
  const response = await fetch(path, { headers: { Authorization: `Bearer ${token}` } });
  if (!response.ok) throw new Error("It could not be loaded.");
  const url = URL.createObjectURL(await response.blob());
  blobs.set(path, url);
  return url;
}

function forget() {
  token = null;
  keep.remove(TOKEN_KEY);
  tellWorker();
}

// ---------- The frame ----------

const PLACES = [
  ["today", "Today", P.sun],
  ["cards", "Cards", P.tray],
  ["todo", "To-do", P.check],
  ["people", "People", P.people],
  ["day", "Day plan", P.calendar],
  null,
  ["ask", "Ask", P.chat],
  ["capture", "Capture", P.mic],
  ["sketch", "Sketch", P.pencil],
  ["reading", "Reading", P.book],
  null,
  ["meetings", "Meetings", P.video],
  ["work", "Work board", P.term],
  ["night", "Night shift", P.moon],
  ["week", "Week", P.bars],
  null,
  ["desk", "Desk mode", P.stand],
  ["settings", "Settings", P.gear],
];

// Screens register here (screens.js): name -> async (root, arg, alive).
const SCREENS = {};
const counts = {};
let current = { name: null, arg: null };
let renderCount = 0;

function buildNav() {
  const side = $("side");
  side.replaceChildren(
    ...PLACES.map((place) => {
      if (!place) return el("div", "nav-gap");
      const [name, label, path] = place;
      const link = el("a", "nav", icon(path), el("span", "nav-label", label), el("span", "n"));
      link.href = `#${name}`;
      link.dataset.place = name;
      return link;
    }),
  );
}

function updateNav() {
  for (const link of document.querySelectorAll(".nav")) {
    const name = link.dataset.place;
    if (name === current.name) link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
    const n = link.querySelector(".n");
    const count = counts[name];
    n.textContent = count?.value ? String(count.value) : "";
    n.classList.toggle("warn", Boolean(count?.warn));
  }
}

function setCount(name, value, warn = false) {
  counts[name] = { value, warn };
  updateNav();
}

async function route() {
  if (!token) {
    showPairing();
    return;
  }
  const [rawName, arg] = (location.hash.slice(1) || "today").split("/");
  const name = SCREENS[rawName] ? rawName : "today";
  current = { name, arg: arg ?? null };
  document.body.classList.toggle("desk", name === "desk");
  if (name !== "desk") releaseWake();
  updateNav();
  const mine = ++renderCount;
  const alive = () => mine === renderCount;
  const root = el("div", "screen");
  main.replaceChildren(root);
  main.scrollTop = 0;
  try {
    await SCREENS[name](root, current.arg, alive);
  } catch (err) {
    if (err instanceof Unpaired) {
      showPairing();
      return;
    }
    if (alive()) root.append(el("p", "empty", err.message ?? String(err)));
  }
}

function go(hash) {
  if (location.hash === hash) route();
  else location.hash = hash;
}

// Re-reads the screen in view, keeping where it was scrolled.
async function refresh() {
  if (!token || !current.name || current.name === "ask" || current.name === "sketch" || current.name === "capture") return;
  const top = main.scrollTop;
  await route();
  main.scrollTop = top;
}

// ---------- The top: the Mac, the island, the to-do count ----------

let macName = keep.get("freebyrd.mac") || "Your Mac";
let reachable = null;
let pausedUntil = null;

function setReachable(ok) {
  if (reachable === ok) return;
  reachable = ok;
  paintMac();
}

function paintMac() {
  const dot = document.querySelector("#mac .dot");
  dot.className = "dot";
  if (reachable === false) {
    dot.classList.add("warn");
    $("mac-text").textContent = `${macName} isn't answering`;
  } else if (pausedUntil && pausedUntil > Date.now()) {
    dot.classList.add("warn");
    $("mac-text").textContent = `Watching paused until ${clock(new Date(pausedUntil))}`;
  } else if (reachable) {
    dot.classList.add("ok");
    $("mac-text").textContent = macName;
  } else {
    dot.classList.add("busy");
    $("mac-text").textContent = "Reaching your Mac…";
  }
}

// What waits, beside the island; the to-do count; the icon's badge.
async function glance() {
  if (!token) return;
  try {
    const g = await call("glance");
    macName = g.mac || macName;
    keep.set("freebyrd.mac", macName);
    pausedUntil = g.pausedUntil;
    paintMac();
    paintIsland(g.waiting ?? [], g.waitingCount ?? 0);
    const badge = g.badge ?? {};
    const count = $("todo-count");
    count.textContent = badge.count ? String(badge.count) : "";
    $("todo-button").classList.toggle("overdue", Boolean(badge.overdue));
    setCount("todo", badge.count, badge.overdue);
    setCount("cards", g.waitingCount);
    setCount("night", g.nightNeeds, Boolean(g.nightNeeds));
    if ("setAppBadge" in navigator) {
      const total = (g.waitingCount ?? 0) + (badge.count ?? 0);
      (total ? navigator.setAppBadge(total) : navigator.clearAppBadge()).catch(() => {});
    }
  } catch (err) {
    if (err instanceof Unpaired) showPairing();
  }
}

function paintIsland(kinds, count) {
  const island = document.getElementById("island");
  if (!island) return;
  const left = kinds.map(kindOf).find((kind) => kind.side === "left");
  const right = kinds.map(kindOf).find((kind) => kind.side === "right");
  const wing = (node, kind) => {
    node.replaceChildren(kind ? icon(kind.path) : "");
    node.style.color = kind?.color ?? "";
  };
  wing(island.querySelector(".wing:first-child"), left);
  wing(island.querySelector(".wing:last-child"), right);
  island.querySelector(".says").textContent = count ? (count === 1 ? "1 waiting" : `${count} waiting`) : clock();
  island.classList.toggle("glow", count > 0);
  islandCount = count;
}

// Idle, the island shows the time.
let islandCount = 0;
setInterval(() => {
  if (!islandCount) document.querySelector("#island .says").textContent = clock();
}, 15000);

$("island").addEventListener("click", () => go("#cards"));
$("todo-button").addEventListener("click", () => go("#todo"));
$("mac").addEventListener("click", () => go("#settings"));

// ---------- Pairing ----------

function showPairing() {
  document.body.classList.remove("desk");
  $("app").hidden = true;
  let box = document.querySelector(".pairing");
  if (box) return;
  const code = el("input", "code-input");
  code.inputMode = "numeric";
  code.autocomplete = "one-time-code";
  code.maxLength = 7;
  code.placeholder = "000000";
  code.setAttribute("aria-label", "The code on your Mac");
  const name = el("input", "field");
  name.value = /iPad/.test(navigator.userAgent) || navigator.maxTouchPoints > 1 ? "iPad" : "This device";
  name.setAttribute("aria-label", "A name for this device");
  name.style.marginTop = "10px";
  name.style.textAlign = "center";
  const note = status();
  const pair = button("Pair", "primary", async () => {
    note.classList.remove("error");
    note.textContent = "Pairing…";
    pair.disabled = true;
    try {
      const response = await fetch("/api/pair", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ code: code.value, name: name.value }),
      });
      const body = await response.json().catch(() => ({}));
      if (!response.ok) throw new Error(body.error || "That did not work.");
      token = body.token;
      keep.set(TOKEN_KEY, token);
      macName = body.mac || macName;
      keep.set("freebyrd.mac", macName);
      tellWorker();
      box.remove();
      $("app").hidden = false;
      start();
    } catch (err) {
      showError(note, err);
      pair.disabled = false;
    }
  });
  pair.style.marginTop = "16px";
  pair.style.width = "100%";
  pair.style.height = "48px";
  code.addEventListener("keydown", (event) => {
    if (event.key === "Enter") pair.click();
  });
  box = el(
    "div",
    "pairing",
    el("span", "mark"),
    el("h1", "", "Pair with your Mac"),
    el("p", "", "On your Mac, open Freebyrd's menu-bar icon and choose Pair an iPad. Type the code it shows."),
    code,
    name,
    pair,
    el("p", "small", note),
    el("p", "faint small", "Everything stays between this iPad and your Mac. Nothing is kept anywhere else."),
  );
  document.body.append(box);
  setTimeout(() => code.focus(), 50);
}

// ---------- Service worker: offline and notifications ----------

async function tellWorker() {
  if (!("serviceWorker" in navigator) || !window.isSecureContext) return;
  try {
    const registration = await navigator.serviceWorker.ready;
    registration.active?.postMessage({ type: "token", token });
  } catch {}
}

if ("serviceWorker" in navigator && window.isSecureContext) {
  navigator.serviceWorker.register("/sw.js").then(tellWorker).catch(() => {});
  navigator.serviceWorker.addEventListener("message", (event) => {
    if (event.data?.type === "go") go(new URL(event.data.url, location.origin).hash || "#today");
  });
}

// ---------- Keeping the screen on (desk mode) ----------

let wake = null;
async function holdWake() {
  try {
    wake = await navigator.wakeLock?.request("screen");
  } catch {
    wake = null;
  }
}

function releaseWake() {
  wake?.release().catch(() => {});
  wake = null;
}

// ---------- Text from the Mac ----------

// Claude answers in Markdown. The common parts become elements whose text
// is set directly, so nothing is ever parsed as HTML. A ```draft block is a
// draft to send, with Copy, and an ```event block an event to add to the
// calendar (`drafts`).
function markdown(text, drafts = false, onRework = null) {
  const out = document.createDocumentFragment();
  let paragraph = null;
  let list = null;
  let quote = null;
  let code = null;
  // The lines of an ```event block, while inside one.
  let offered = null;
  const block = (tag) => {
    paragraph = list = quote = null;
    return out.appendChild(document.createElement(tag));
  };
  for (const raw of String(text ?? "").split("\n")) {
    const fence = /^\s*```/.test(raw);
    if (offered) {
      if (fence) {
        out.append(offeredEvent(offered.join("\n")));
        offered = null;
      } else {
        offered.push(raw);
      }
      continue;
    }
    if (code) {
      if (fence) code = null;
      else code.append(code.hasChildNodes() ? `\n${raw}` : raw);
      continue;
    }
    const line = raw.trimEnd();
    let m;
    if (fence && drafts && /^\s*```event\s*$/.test(raw)) {
      paragraph = list = quote = null;
      offered = [];
    } else if (fence) {
      code = drafts && /^\s*```draft\s*$/.test(raw)
        ? draftBlock(block("div"), onRework)
        : block("pre").appendChild(document.createElement("code"));
    } else if (!line.trim()) {
      paragraph = list = quote = null;
    } else if ((m = line.match(/^ {0,3}#{1,6}\s+(.*?)(?:\s+#+)?$/))) {
      inline(block("h3"), m[1]);
    } else if (/^ {0,3}([-*_])(?:\s*\1){2,}$/.test(line)) {
      block("hr");
    } else if ((m = line.match(/^\s*>\s?(.*)$/))) {
      if (quote) quote.append(document.createElement("br"));
      else quote = block("blockquote");
      inline(quote, m[1]);
    } else if ((m = line.match(/^(\s*)(?:[-*+•]|(\d{1,9})[.)])\s+(.*)$/))) {
      const tag = m[2] ? "ol" : "ul";
      if (list?.localName !== tag) {
        list = block(tag);
        if (m[2] && m[2] !== "1") list.start = Number(m[2]);
      }
      const item = list.appendChild(document.createElement("li"));
      if (m[1].length >= 2) item.className = "sub";
      inline(item, m[3]);
    } else if (list && /^\s/.test(line)) {
      const item = list.lastElementChild;
      item.append(document.createElement("br"));
      inline(item, line.trim());
    } else {
      if (paragraph) paragraph.append(document.createElement("br"));
      else paragraph = block("p");
      inline(paragraph, line);
    }
  }
  // An event block still arriving.
  if (offered) out.append(el("p", "pending", "Filling in the event…"));
  return out;
}

// Events answers offered, by their JSON, so a repaint keeps the fields
// being edited, and the card once added.
const offeredNodes = new Map();

// An event an answer offered because the person asked for something to go
// on their calendar (claude.rs): filled in, to change, and Add to calendar,
// the notch's `add_event`. Nothing is added before the press. Values from
// the answer are only ever set as text and field values.
function offeredEvent(json) {
  const key = json.trim();
  const kept = offeredNodes.get(key);
  if (kept) return kept;
  let event = null;
  try {
    event = JSON.parse(key);
  } catch {
    // Shown as it came, below.
  }
  const text = (value, max) => (typeof value === "string" ? value.trim().slice(0, max) : "");
  if (!event || typeof event !== "object" || !text(event.title, 160) || !/^\d{4}-\d{2}-\d{2}$/.test(event.date ?? "")) {
    return el("pre", "", el("code", "", json));
  }
  const count = (value, max) => (Number.isInteger(value) && value >= 0 && value <= max ? value : null);
  const time = /^\d{2}:\d{2}$/.test(event.time ?? "") ? event.time : null;
  const form = eventForm({
    title: text(event.title, 160),
    date: event.date,
    time,
    minutes: count(event.minutes, 24 * 60) || 30,
    location: text(event.location, 200) || null,
    alertMinutes: time ? count(event.alertMinutes, 24 * 60) ?? 30 : null,
  });
  const source = text(event.source, 120);
  const notes = text(event.notes, 300);
  const when = el("strong", "", whenLabel(event.date, time));
  form.node.addEventListener("input", () => {
    const read = form.read();
    when.textContent = whenLabel(read.date, read.time);
  });
  const about = [notes, source && `From ${source}`].filter(Boolean).join(" \u00b7 ");
  // Its icon in color while it waits for the press, like a fresh nudge.
  const node = el(
    "article",
    "card offered new",
    el("header", "", kindLabel(KINDS.event, null), when),
    el("div", "card-body", form.node, about ? el("p", "note-line", about) : null),
  );
  const note = status();
  const add = button("Add to calendar", "primary", async () => {
    add.disabled = true;
    note.classList.remove("error");
    note.textContent = WORKING.add ?? "Adding…";
    try {
      const card = await call("add_event", {
        offer: { ...form.read(), source: source || null, notes: notes || null },
      });
      const done = cardElement({ ...card, seenAt: card.seenAt ?? Date.now() });
      done.classList.add("offered");
      offeredNodes.set(key, done);
      node.replaceWith(done);
      glance();
    } catch (err) {
      showError(note, err);
      add.disabled = false;
    }
  });
  node.append(el("div", "actions", add, note));
  offeredNodes.set(key, node);
  return node;
}

// "Fri 16 Oct, 23:30", or "Fri 16 Oct" for all day.
function whenLabel(date, time) {
  const day = new Date(`${date}T00:00`);
  if (Number.isNaN(day.getTime())) return time ?? "";
  const label = day.toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short" }).replace(",", "");
  return time ? `${label}, ${time}` : label;
}

const INLINE =
  /`([^`]+)`|\*\*(.+?)\*\*|(?<![\w*])\*(?![\s*])(.+?)(?<![\s*])\*(?![\w*])|(?<!\w)_(?![\s_])(.+?)(?<![\s_])_(?!\w)|\[([^\]]+)\]\(([^)\s]+)\)/g;

// `code`, **bold**, *italic*, and [label](url). A web link opens in Safari
// when tapped, its site shown beside it; any other address stays text.
function inline(parent, text) {
  let at = 0;
  for (const m of text.matchAll(INLINE)) {
    parent.append(text.slice(at, m.index));
    at = m.index + m[0].length;
    const [, code, bold, star, underscore, label, href] = m;
    if (code !== undefined) {
      parent.append(el("code", "", code));
    } else if (label !== undefined) {
      parent.append(link(label, href));
    } else {
      const node = document.createElement(bold !== undefined ? "strong" : "em");
      inline(node, bold ?? star ?? underscore);
      parent.append(node);
    }
  }
  parent.append(text.slice(at));
}

function link(label, href) {
  let url = null;
  try {
    url = new URL(href);
  } catch {}
  if (!url || (url.protocol !== "https:" && url.protocol !== "http:")) return el("span", "", label);
  const node = el("a", "", label, " ", el("small", "", url.hostname.replace(/^www\./, "")));
  node.href = url.href;
  node.target = "_blank";
  node.rel = "noopener noreferrer";
  return node;
}

async function copyText(text) {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {}
  // Plain HTTP (the Wi-Fi): a hidden field and the copy command.
  const field = el("textarea", "");
  field.value = text;
  field.setAttribute("readonly", "");
  field.style.position = "fixed";
  field.style.opacity = "0";
  document.body.append(field);
  field.select();
  field.setSelectionRange(0, text.length);
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {}
  field.remove();
  return ok;
}

function copyButton(read, label = "Copy") {
  const node = button([icon(P.copy), el("span", "", label)], "small", async () => {
    const said = node.querySelector("span");
    said.textContent = (await copyText(read())) ? "Copied" : "Could not copy";
    setTimeout(() => (said.textContent = label), 1500);
  });
  return node;
}

// A draft in an answer: the text alone, with Copy, and rework buttons.
const REWORK = [
  ["Shorter", "Make the draft shorter."],
  ["More formal", "Rewrite the draft to be more formal."],
  ["Warmer", "Rewrite the draft to be warmer."],
];

function draftBlock(node, onRework) {
  node.className = "draft";
  const text = el("p", "draft-text");
  const head = el("header", "", kindLabel(KINDS.draft, "Draft", true), copyButton(() => text.textContent));
  node.append(head, text);
  if (onRework) {
    const row = el("div", "chips rework");
    row.style.marginTop = "10px";
    for (const [label, ask] of REWORK) {
      const chip = el("button", "chip", label);
      chip.type = "button";
      chip.addEventListener("click", () => onRework(ask));
      row.append(chip);
    }
    node.append(row);
  }
  return text;
}

// A draft prepared before something is due (drafts.rs), inside its card.
function preparedDraft(data) {
  const what = [`${data.kind === "update" ? "Update" : "Draft"}${data.to ? ` to ${data.to}` : ""}`, data.channel].filter(Boolean);
  return el(
    "div",
    "draft",
    el("header", "", kindLabel(KINDS.draft, what.join(" · "), true), copyButton(() => data.draft)),
    el("p", "draft-text", data.draft),
  );
}

// Long text cut to a few lines, with More to read it all.
function clamped(node, lines = 4) {
  node.classList.add("clamp");
  node.style.setProperty("--lines", String(lines));
  const more = el("button", "toggle-link", "More");
  more.type = "button";
  more.hidden = true;
  more.addEventListener("click", () => {
    const open = node.classList.toggle("clamp");
    more.textContent = open ? "More" : "Less";
  });
  requestAnimationFrame(() => {
    more.hidden = node.scrollHeight <= node.clientHeight + 2;
  });
  return [node, more];
}

function revealer(showLabel, hideLabel, content) {
  content.hidden = true;
  const toggle = el("button", "toggle-link", showLabel);
  toggle.type = "button";
  toggle.addEventListener("click", () => {
    content.hidden = !content.hidden;
    toggle.textContent = content.hidden ? showLabel : hideLabel;
  });
  return [toggle, content];
}

// ---------- Asking, from anywhere ----------

// What the Ask screen should ask when it opens: a question, how to show it
// in the thread, and what was pressed (sent as memory, never in the words).
let pendingAsk = null;

function askAbout(question, shown, focus = null) {
  pendingAsk = { question, shown: shown ?? question, focus };
  go("#ask");
}

// ---------- Cards ----------

const WORKING = {
  add: "Adding…",
  yes: "Setting it up…",
  run: "Starting…",
  pause: "Pausing…",
  resume: "Resuming…",
  remove: "Removing…",
  undo: "Undoing…",
  redo: "Gathering your work…",
  done: "Marking it done…",
};

const LENGTHS = [15, 30, 45, 60, 90, 120];

function lengthLabel(minutes) {
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (!hours) return `${rest} min`;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

// Fields for an event to add: title; date, time (empty for all day) and
// length; place.
function eventForm(event) {
  const field = (tag, className, attributes = {}) => {
    const input = document.createElement(tag);
    input.className = className;
    for (const [key, value] of Object.entries(attributes)) input.setAttribute(key, value);
    return input;
  };
  const title = field("input", "ev-title", { type: "text", "aria-label": "Title" });
  title.value = event.title ?? "";
  const date = field("input", "ev-date", { type: "date", "aria-label": "Date" });
  date.value = event.date ?? "";
  const time = field("input", "ev-time", { type: "time", "aria-label": "Time (empty for all day)" });
  time.value = event.time ?? "";
  const length = field("select", "ev-length", { "aria-label": "Length" });
  const minutes = event.minutes ?? 30;
  for (const value of [...new Set([...LENGTHS, minutes])].sort((a, b) => a - b)) {
    const option = el("option", "", lengthLabel(value));
    option.value = String(value);
    option.selected = value === minutes;
    length.append(option);
  }
  const place = field("input", "ev-place", { type: "text", placeholder: "Where", "aria-label": "Where" });
  place.value = event.location ?? "";
  // When to be reminded, for an event an answer offered.
  const alert = "alertMinutes" in event ? field("select", "ev-alert", { "aria-label": "Alert" }) : null;
  if (alert) {
    const before = event.alertMinutes ?? 30;
    for (const value of [...new Set([...ALERTS, before])].sort((a, b) => a - b)) {
      const option = el("option", "", value ? `Alert ${lengthLabel(value)} before` : "Alert at start");
      option.value = String(value);
      option.selected = value === before;
      alert.append(option);
    }
  }
  const sync = () => {
    length.disabled = !time.value;
    if (alert) alert.disabled = !time.value;
  };
  time.addEventListener("input", sync);
  sync();
  const node = el(
    "div",
    "event-form",
    title,
    el("div", "ev-row", date, time, length),
    alert ? el("div", "ev-row ev-where", place, alert) : place,
  );
  return {
    node,
    read: () => ({
      title: title.value.trim(),
      date: date.value,
      time: time.value || null,
      minutes: time.value ? Number(length.value) : null,
      location: place.value.trim() || null,
      ...(alert && time.value ? { alertMinutes: Number(alert.value) } : {}),
    }),
  };
}

// Minutes before the start, for an offered event's alert.
const ALERTS = [0, 15, 30, 60, 90, 120, 180];

// A card, with its buttons acting on the Mac. `after` hears the card as it
// is after a button (or null once put away).
function cardElement(card, after = null) {
  const node = el("article", "card");
  node.classList.toggle("new", !card.seenAt);
  node.classList.toggle("block", card.kind === "block");
  const kind = cardKind(card);
  const label = kindLabel(kind, null, Boolean(kind.lit));
  label.style.setProperty("--kind", kind.color);
  const dismiss = el("button", "dismiss", icon(P.dismiss));
  dismiss.type = "button";
  dismiss.setAttribute("aria-label", "Put away");
  dismiss.addEventListener("click", async () => {
    node.style.opacity = "0.4";
    try {
      await call("dismiss_card", { id: card.id });
      node.remove();
      after?.(null);
      glance();
    } catch (err) {
      node.style.opacity = "";
      toast(err.message);
    }
  });
  node.append(el("header", "", label, el("strong", "", card.title), dismiss));

  const body = el("div", "card-body");
  const editable = card.kind === "event" || card.kind === "block";
  const form = editable && card.data?.date ? eventForm(card.data) : null;
  const what = form && card.kind === "block" ? el("p", "", el("strong", "", card.data.title ?? "")) : null;
  if (what) {
    form.node.hidden = true;
    body.append(what);
  }
  if (form) body.append(form.node);
  body.append(markdown(card.body));
  node.append(body);
  if (card.kind === "suggestion" && typeof card.data?.draft === "string") {
    const what = !card.data.state ? "draft" : card.data.fix === "schedule" ? "job" : "skill";
    node.append(...revealer(`Show the ${what}`, `Hide the ${what}`, el("pre", "box", card.data.draft)));
  }
  if (card.kind === "draft" && typeof card.data?.draft === "string") {
    node.append(preparedDraft(card.data));
    if (card.data.done?.length) {
      const done = el("ul", "", ...card.data.done.map((line) => el("li", "", line)));
      done.style.margin = "8px 0 0";
      done.style.color = "var(--text-2)";
      node.append(...revealer("What’s done", "Hide what’s done", done));
    }
  }

  if (card.actions?.length) {
    const row = el("div", "actions");
    const note = status();
    card.actions.forEach((action, index) => {
      const primary = index === 0 && !QUIET_ACTIONS.has(action.id);
      const press = button(action.label, primary ? "primary" : "", async () => {
        if (card.kind.startsWith("night")) {
          call("dismiss_card", { id: card.id }).catch(() => {});
          go(card.kind === "nightplan" ? "#night/tonight" : "#night");
          return;
        }
        if (card.kind === "draft" && action.id === "keep") {
          if (card.data?.plan && card.data?.question) {
            askAbout(card.data.question, `Keep working: ${card.data.title ?? "the draft"}`, `plan:${card.data.plan}`);
          }
          return;
        }
        for (const other of row.querySelectorAll("button")) other.disabled = true;
        note.classList.remove("error");
        const agenda = card.kind === "block" && card.data?.category === "work";
        note.textContent = action.id === "add" && agenda ? "Writing its agenda…" : (WORKING[action.id] ?? "");
        try {
          const updated = await call("card_action", {
            id: card.id,
            action: action.id,
            template: form && action.id === "add" ? form.read() : null,
          });
          const next = cardElement({ ...updated, seenAt: updated.seenAt ?? Date.now() }, after);
          node.replaceWith(next);
          after?.(updated);
          glance();
        } catch (err) {
          showError(note, err);
          for (const other of row.querySelectorAll("button")) other.disabled = false;
        }
      });
      row.append(press);
    });
    if (what) {
      const change = button("Change", "", () => {
        const opening = form.node.hidden;
        form.node.hidden = !opening;
        what.hidden = opening;
        change.textContent = opening ? "Close" : "Change";
        if (!opening) what.querySelector("strong").textContent = form.read().title;
      });
      row.append(change);
    }
    row.append(note);
    node.append(row);
  }
  return node;
}

// ---------- To-dos ----------

// A to-do as a row. Tapping it shows where it came from and its buttons.
// On a person's page (`without` is theirs), their name is left off.
function todoRow(item, { without = null, onList } = {}) {
  const row = el("div", "row");
  const done = item.actions.find((action) => action.id === "done" || action.id === "undo");
  row.classList.toggle("finished", done?.id === "undo");
  const check = el("button", "check", icon(P.circleCheck));
  check.type = "button";
  check.setAttribute("aria-label", done?.label ?? "Done");
  check.addEventListener("click", async () => {
    if (!done) return;
    row.classList.toggle("finished", done.id === "done");
    try {
      onList?.(await call("todo_action", { id: item.id, action: done.id, value: null }));
      glance();
    } catch (err) {
      row.classList.toggle("finished", done.id !== "done");
      toast(err.message);
    }
  });
  const main = el("button", "row-main", el("span", "row-title", item.title));
  main.type = "button";
  const meta = el("span", "row-meta");
  if (item.person && item.person.id !== without) {
    const who = el("span", "person-chip", item.person.name);
    meta.append(who);
  } else if (item.who) {
    meta.append(el("span", "", item.who));
  }
  if (item.when) meta.append(el("span", item.overdue ? "overdue" : "", item.when));
  if (meta.hasChildNodes()) main.append(meta);
  // What last moved it, as the notch shows it.
  if (item.latest) main.append(el("span", "row-latest", `${item.latest.when} \u00b7 ${item.latest.note}`));
  row.append(check, main, el("span", ""));
  let more = null;
  main.addEventListener("click", () => {
    if (more) {
      more.remove();
      more = null;
      row.classList.remove("open");
      return;
    }
    more = todoDetails(item, onList);
    row.append(more);
    row.classList.add("open");
  });
  return row;
}

function todoDetails(item, onList) {
  const box = el("div", "row-more", el("p", "quiet small", `From ${item.source}`));
  box.firstChild.style.margin = "0";
  const row = el("div", "actions");
  const note = status();
  for (const action of item.actions) {
    if (action.id === "done" || action.id === "undo") continue;
    row.append(
      button(action.label, action.id === "ask" ? "primary small" : "small", async () => {
        if (action.id === "ask") {
          askAbout(item.question, `${action.label}: ${item.title}`, item.focus);
          return;
        }
        if (action.id === "date") {
          row.replaceWith(datePicker(item, onList));
          return;
        }
        if (item.person && action.id === "person") return;
        for (const other of row.querySelectorAll("button")) other.disabled = true;
        note.classList.remove("error");
        note.textContent = action.id === "calendar" ? "Adding it, with an agenda…" : "";
        try {
          onList?.(await call("todo_action", { id: item.id, action: action.id, value: null }));
          glance();
        } catch (err) {
          showError(note, err);
          for (const other of row.querySelectorAll("button")) other.disabled = false;
        }
      }),
    );
  }
  if (item.person) row.append(button("Their page", "small", () => go(`#people/${item.person.id}`)));
  row.append(note);
  box.append(row);
  return box;
}

function datePicker(item, onList) {
  const input = el("input", "field");
  input.type = "date";
  input.style.width = "auto";
  input.style.height = "36px";
  const today = new Date();
  input.min = `${today.getFullYear()}-${pad2(today.getMonth() + 1)}-${pad2(today.getDate())}`;
  input.value = input.min;
  const note = status();
  const row = el(
    "div",
    "actions",
    input,
    button("Set", "primary small", async () => {
      try {
        onList?.(await call("todo_action", { id: item.id, action: "date", value: input.value }));
      } catch (err) {
        showError(note, err);
      }
    }),
    note,
  );
  return row;
}

// ---------- Starting ----------

function start() {
  $("app").hidden = false;
  buildNav();
  paintMac();
  route();
  glance();
}

window.addEventListener("hashchange", route);
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState !== "visible") return;
  glance();
  refresh();
  if (current.name === "desk") holdWake();
});
setInterval(() => {
  if (document.visibilityState === "visible") glance();
}, 60000);

window.addEventListener("load", () => {
  if (token) start();
  else showPairing();
});
