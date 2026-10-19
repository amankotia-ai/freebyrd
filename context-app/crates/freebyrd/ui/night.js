// Night shift (memory::night): work the person queues before they stop,
// run while they are away, waiting here when they are back. Behind the moon
// at the top right (⌘N). Tonight lists the queue; While you were away lists
// what ran, what needs the person first, and each job's page: a preview's
// reel and what was found in it, a final's frames, a list's items. Every
// button acts only when pressed; nothing reaches the person's project
// until they press Keep. Everything shown is set as text. The top input
// is the panel's too: on Tonight it adds a job, after Redo with a note it
// takes the note. It shares app.js's globals (el, icon, element, invoke,
// copyText, question, bar).

const nightPanel = el("night");
const nightToggle = el("night-toggle");

// Solid icons, inner shapes cut out (`icon`).
const NT = {
  film: "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Z M6 6v2h2V6H6Z M6 11v2h2v-2H6Z M6 16v2h2v-2H6Z M16 6v2h2V6h-2Z M16 11v2h2v-2h-2Z M16 16v2h2v-2h-2Z",
  list: "M3.5 5.5a1.5 1.5 0 1 1 3 0a1.5 1.5 0 0 1-3 0Z M9 4.5h11a1 1 0 1 1 0 2H9a1 1 0 0 1 0-2Z M3.5 12a1.5 1.5 0 1 1 3 0a1.5 1.5 0 0 1-3 0Z M9 11h11a1 1 0 1 1 0 2H9a1 1 0 1 1 0-2Z M3.5 18.5a1.5 1.5 0 1 1 3 0a1.5 1.5 0 0 1-3 0Z M9 17.5h11a1 1 0 1 1 0 2H9a1 1 0 1 1 0-2Z",
  play: "M8 5.6v12.8a1 1 0 0 0 1.53.85l10.2-6.4a1 1 0 0 0 0-1.7L9.53 4.75A1 1 0 0 0 8 5.6Z",
  wait: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M9 8a1 1 0 0 0-1 1v6a1 1 0 1 0 2 0V9a1 1 0 0 0-1-1Z M15 8a1 1 0 0 0-1 1v6a1 1 0 1 0 2 0V9a1 1 0 0 0-1-1Z",
  error: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M11 6.5v7h2v-7h-2Z M11 15.5v2h2v-2h-2Z",
  check: CHECK_PATH,
  chevron: "M9.3 6.3a1 1 0 0 1 1.4 0l5 5a1 1 0 0 1 0 1.4l-5 5a1 1 0 0 1-1.4-1.4L13.6 12 9.3 7.7a1 1 0 0 1 0-1.4Z",
  back: "M14.7 6.3a1 1 0 0 1 0 1.4L10.4 12l4.3 4.3a1 1 0 0 1-1.4 1.4l-5-5a1 1 0 0 1 0-1.4l5-5a1 1 0 0 1 1.4 0Z",
  close: DISMISS_PATH,
  spark: "M11 2.5l2.3 6.2 6.2 2.3-6.2 2.3L11 19.5l-2.3-6.2L2.5 11l6.2-2.3L11 2.5Z M18.5 14.5l1 2.5 2.5 1-2.5 1-1 2.5-1-2.5-2.5-1 2.5-1 1-2.5Z",
};

const NT_KIND_ICON = { preview: "film", final: "film", list: "list" };

let ntTab = "tonight";
let ntView = null;
// The job whose page is open, and that page.
let ntJob = null;
let ntPage = null;
// What was typed, while Claude turns it into a job.
let ntAdding = null;
let ntError = "";
// The preview whose redo note the top input takes.
let ntRedo = null;
const NT_ADD_PLACEHOLDER = "What should run tonight? A film to preview, or a list of tasks";
const NT_REDO_PLACEHOLDER = "What should change? Return queues it again for tonight";
// Tonight's work being picked from context, once an opening.
let ntPicking = false;
let ntPickTried = false;

function ntNode(tag, className, ...children) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  node.append(...children.filter((child) => child !== null && child !== undefined));
  return node;
}

// An icon in a tile, tinted when it needs the person.
function ntTile(name, state = "") {
  return ntNode("span", state ? `nt-tile ${state}` : "nt-tile", icon(NT[name]));
}

function ntText(title, sub) {
  const text = ntNode("span", "nt-text", ntNode("b", "", title));
  if (sub) text.append(ntNode("small", "", sub));
  return text;
}

// The one row: tile, title over a quiet line, then whatever goes right.
function ntRow({ tile, title, sub, end = [], tag = "div" }) {
  const row = ntNode(tag, tag === "div" ? "nt-row hover" : "nt-row", tile, ntText(title, sub), ...end);
  if (tag === "button") {
    row.type = "button";
    row.addEventListener("mousedown", (event) => event.preventDefault());
  }
  return row;
}

function ntButton(label, className, onClick) {
  const button = element("button", label);
  button.type = "button";
  button.className = className;
  button.addEventListener("mousedown", (event) => event.preventDefault());
  button.addEventListener("click", onClick);
  return button;
}

function ntGroup(label, className = "") {
  const group = element("h4", label);
  group.className = className ? `todo-group ${className}` : "todo-group";
  return group;
}

function ntHead(name, title, sub) {
  return ntNode("div", "nt-head", ntTile(name, "night"), ntText(title, sub));
}

function ntNotes(notes) {
  return ntNode(
    "div",
    "nt-notes",
    ...(notes ?? []).map(([state, text]) => ntNode("div", `nt-note ${state}`, text)),
  );
}

// The foot that stays in view: a summary or status on the left, buttons on
// the right. `buttons` are [label, onClick(status), primary].
function ntFoot(summary, buttons) {
  const status = ntNode("span", ntError ? "card-status error" : "card-status", ntError);
  const left = summary
    ? ntNode("div", "nt-summary", summary[0], summary[1] ? ntNode("small", "", summary[1]) : null)
    : null;
  const foot = ntNode("div", "nt-foot", left, status);
  for (const [label, onClick, primary] of buttons) {
    foot.append(ntButton(label, primary ? "card-action primary" : "card-action", () => onClick(status)));
  }
  return foot;
}

// A picture of a frame, or a dark tile while there is none.
function ntPicture(url, className = "") {
  const tile = ntNode("div", className ? `nt-ph ${className}` : "nt-ph");
  if (url) {
    const img = document.createElement("img");
    img.src = url;
    img.alt = "";
    tile.append(img);
  }
  return tile;
}

// Night shift's panel, and the stack on the to-do list, as they are now.
function ntRenderAll() {
  renderNight();
  if (!todos.hidden && todoTab === "mine") renderTodos();
}

async function ntRefresh() {
  try {
    ntView = await invoke("night");
  } catch (err) {
    ntError = String(err);
  }
  ntRenderAll();
  // Not picked yet today: pick tonight's work from what is owed and what
  // the person was in the middle of (a few seconds with Claude).
  if (ntView && !ntView.picksFresh && !ntPickTried) ntPick();
}

async function ntPick() {
  ntPickTried = true;
  ntPicking = true;
  ntRenderAll();
  try {
    ntView = await invoke("night_action", { id: 0, action: "suggest", value: null });
  } catch (err) {
    ntError = String(err);
  }
  ntPicking = false;
  ntRenderAll();
}

// A button's action; the panel as it is afterwards comes back.
async function ntAct(id, action, value, status) {
  if (status) {
    status.classList.remove("error");
    status.textContent = { keep: "Merging…", approve: "Queuing the final…", start: "Starting…" }[action] ?? "";
  }
  try {
    ntView = await invoke("night_action", { id, action, value: value ?? null });
    ntError = "";
    if (status) status.textContent = "";
    return true;
  } catch (err) {
    if (status) {
      status.textContent = String(err);
      status.classList.add("error");
    }
    return false;
  }
}

async function ntOpenJob(id) {
  ntJob = id;
  ntPage = null;
  renderNight();
  try {
    ntPage = await invoke("night_job", { id });
  } catch (err) {
    ntError = String(err);
  }
  renderNight();
}

function renderNight() {
  if (nightPanel.hidden) return;
  ntPlaceholder();
  const top = nightPanel.scrollTop;
  const parts = ntTab === "tonight" ? ntTonight() : ntJob ? ntDetail() : ntWaiting();
  const body = ntNode("div", "todo-body", ...parts);
  nightPanel.replaceChildren(ntTabs(), body);
  nightPanel.scrollTop = ntJob ? 0 : top;
}

function ntTabs() {
  const tabs = ntNode("nav", "todo-tabs nt-tabs");
  tabs.setAttribute("role", "tablist");
  const waiting = ntView?.waiting.length ?? 0;
  for (const [id, label, count, warn] of [
    ["tonight", "Tonight", ntView?.tonight.length ?? 0, false],
    ["waiting", "While you were away", ntView?.needs || waiting, Boolean(ntView?.needs)],
  ]) {
    const tab = element("button", label);
    tab.type = "button";
    tab.className = "todo-tab";
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", String(ntTab === id));
    if (count) tab.append(ntNode("span", warn ? "todo-tab-count warn" : "todo-tab-count", String(count)));
    tab.addEventListener("mousedown", (event) => event.preventDefault());
    tab.addEventListener("click", () => {
      ntTab = id;
      ntJob = null;
      ntError = "";
      nightPanel.scrollTop = 0;
      renderNight();
    });
    tabs.append(tab);
  }
  return tabs;
}

function ntTonight() {
  const rows = (ntView?.tonight ?? []).map((job) => {
    const end = [ntNode("span", "nt-end", job.end)];
    if (job.state === "queued") {
      const remove = ntButton("", "nt-remove", async () => {
        await ntAct(job.id, "remove");
        renderNight();
      });
      remove.append(icon(NT.close));
      remove.setAttribute("aria-label", "Take it off tonight");
      end.push(remove);
    }
    const tile = ntTile(NT_KIND_ICON[job.kind] ?? "list", job.state === "running" ? "night" : "");
    return ntRow({ tile, title: job.title, sub: job.sub, end });
  });
  if (ntAdding) {
    rows.push(ntRow({ tile: ntTile("list"), title: ntAdding, sub: "Planning it from your work…" }));
  }
  // Picked from what is owed and what the person was in the middle of.
  const picked = (ntView?.suggested ?? []).map((job) => {
    const add = ntButton("Add", "nt-small", async () => {
      await ntAct(job.id, "add");
      renderNight();
    });
    const skip = ntButton("", "nt-remove", async () => {
      await ntAct(job.id, "skip");
      renderNight();
    });
    skip.append(icon(NT.close));
    skip.setAttribute("aria-label", "Not tonight");
    const tile = ntTile(NT_KIND_ICON[job.kind] ?? "list", "night");
    return ntRow({ tile, title: job.title, sub: job.sub, end: [ntNode("span", "nt-end", job.end), add, skip] });
  });
  if (ntPicking) {
    picked.push(ntRow({ tile: ntTile("list"), title: "Picking tonight's work", sub: "From what you owe and what you were in the middle of…" }));
  } else if (!picked.length && ntView?.picksNote) {
    // Why nothing was picked, or that picking failed; never an empty evening.
    const again = ntButton("Pick again", "nt-small", () => ntPick());
    picked.push(ntRow({ tile: ntTile("list"), title: "Nothing picked for tonight", sub: ntView.picksNote, end: [again] }));
  }
  const pickedPart = picked.length ? [ntGroup("Picked from your work"), ...picked] : [];
  if (pickedPart.length && rows.length) pickedPart.push(ntGroup("Tonight"));
  const empty = rows.length || ntView?.suggested?.length || ntPicking
    ? null
    : ntNode(
        "p",
        "todo-empty",
        "Nothing queued. Type above what should run tonight: a preview of a film (every 4th frame) or a list of tasks to work through.",
      );
  const count = ntView?.tonight.length ?? 0;
  const summary = ntView?.running
    ? ["Running now", "What finishes waits under While you were away"]
    : [count === 1 ? "1 job" : `${count} jobs`, ntView?.starts ?? ""];
  const buttons =
    count && !ntView?.running
      ? [
          [
            "Start now",
            async (status) => {
              await ntAct(0, "start", null, status);
              setTimeout(ntRefresh, 1500);
            },
            true,
          ],
        ]
      : [];
  return [...pickedPart, ...rows, empty, ntFoot(summary, buttons)];
}

function ntWaiting() {
  const jobs = ntView?.waiting ?? [];
  const row = (job) => {
    const state = job.state === "failed" ? "error" : job.needs ? "warn" : "";
    const chevron = icon(NT.chevron);
    chevron.classList.add("nt-chev");
    const node = ntRow({
      tile: ntTile(NT_KIND_ICON[job.kind] ?? "list", state),
      title: job.title,
      sub: job.sub,
      end: [ntNode("span", "nt-end", job.end), chevron],
      tag: "button",
    });
    node.addEventListener("click", () => ntOpenJob(job.id));
    return node;
  };
  const needs = jobs.filter((job) => job.needs);
  const ready = jobs.filter((job) => !job.needs);
  const parts = [];
  if (needs.length) parts.push(ntGroup("Needs you", "warn"), ...needs.map(row));
  if (ready.length) parts.push(ntGroup("Ready to look at"), ...ready.map(row));
  if (!jobs.length) {
    parts.push(ntNode("p", "todo-empty", "Nothing from last night. What runs tonight waits here when you're back."));
  } else {
    const count = jobs.length === 1 ? "1 job" : `${jobs.length} jobs`;
    const sub = needs.length ? `${count} · ${needs.length} need you` : count;
    parts.push(ntFoot([ntView?.ran ?? "Last night", sub], []));
  }
  return parts;
}

function ntDetail() {
  const back = ntButton("", "nt-back", () => {
    ntJob = null;
    ntPage = null;
    ntError = "";
    renderNight();
  });
  back.append(icon(NT.back), "While you were away");
  if (!ntPage) return [back, ntNode("p", "todo-empty", ntError || "Opening…")];
  const parts = { preview: ntPreviewPage, final: ntFinalPage, list: ntListPage }[ntPage.kind];
  return [back, ...(parts ? parts(ntPage) : [])];
}

// After a button that puts the job away: back to the list.
async function ntDone(id, action, value, status) {
  if (await ntAct(id, action, value, status)) {
    ntJob = null;
    ntPage = null;
    renderNight();
  }
}

function ntPreviewPage(page) {
  const result = page.result ?? {};
  const shots = result.shots ?? [];
  const head = ntHead(
    "film",
    page.title,
    [`Preview, every ${result.step ?? 4}th frame`, page.took, result.frames ? `${result.frames} frames` : null]
      .filter(Boolean)
      .join(" · "),
  );
  const buttons = [
    ["Folder", () => ntAct(page.id, "folder")],
    [
      "Redo with a note",
      (status) => {
        ntRedo = page.id;
        ntPlaceholder();
        question.focus({ preventScroll: true });
        status.textContent = "Type what should change above";
      },
    ],
  ];
  if (page.state === "done") buttons.push(["Approve, final tonight", (status) => ntDone(page.id, "approve", null, status), true]);
  else buttons.push(["Put away", (status) => ntDone(page.id, "dismiss", null, status), true]);
  const foot = ntFoot(null, buttons);
  if (!shots.length) return [head, ntNotes(result.notes), foot];

  // The reel opens in QuickTime; the timeline is the shots in order,
  // coloured by what was found.
  const first = shots.find((shot) => shot.state !== "ok") ?? shots[0];
  const reel = ntPicture(first?.thumb, "big clip");
  reel.append(ntNode("i", "nt-play", icon(NT.play)));
  reel.append(ntNode("span", "", result.reel ? "Play at real speed" : "Open the frames"));
  reel.addEventListener("click", () => ntAct(page.id, "open"));
  const track = ntNode("div", "nt-track");
  for (const shot of shots) {
    const state = shot.state === "error" ? "error" : shot.state === "warn" ? "warn" : "";
    const segment = ntNode("span", state, ntNode("small", "", shot.name));
    segment.title = shot.note || shot.name;
    track.append(segment);
  }
  const reelBlock = ntNode("div", "nt-reel", ntNode("div", "nt-reel-main", reel, track), ntNotes(result.notes));
  return [head, reelBlock, foot];
}

function ntFinalPage(page) {
  const result = page.result ?? {};
  const strip = ntNode("div", "nt-strip three");
  for (const shot of (result.shots ?? []).slice(0, 6)) {
    const picture = ntPicture(shot.thumb, "clip");
    picture.append(ntNode("i", "nt-play", icon(NT.play)));
    picture.addEventListener("click", () => ntAct(page.id, "open"));
    strip.append(ntNode("div", "", picture, ntNode("div", "nt-cap", shot.name)));
  }
  return [
    ntHead("film", page.title, [page.took, result.frames ? `${result.frames} frames` : null].filter(Boolean).join(" · ")),
    strip,
    ntNotes(result.notes),
    ntFoot(null, [
      ["Put away", (status) => ntDone(page.id, "dismiss", null, status)],
      ["Folder", () => ntAct(page.id, "folder")],
      ["Play", () => ntAct(page.id, "open"), true],
    ]),
  ];
}

function ntListPage(page) {
  const result = page.result ?? {};
  const items = result.items ?? [];
  const row = (item) => {
    const state = item.state === "ok" ? "ok" : item.state === "warn" ? "warn" : "error";
    const name = { ok: "check", warn: "wait", error: "error" }[state];
    const end = item.commit ? [ntNode("span", "nt-end", item.files === 1 ? "1 file" : `${item.files} files`)] : [];
    return ntRow({ tile: ntTile(name, state), title: item.text, sub: item.note, end });
  };
  const needs = items.filter((item) => item.state !== "ok");
  const done = items.filter((item) => item.state === "ok");
  const parts = [ntHead("list", page.title, `Keep going · ${result.branch ?? result.place ?? "its own branch"}`)];
  if (needs.length) parts.push(ntGroup("Needs you", "warn"), ...needs.map(row));
  if (done.length) parts.push(ntGroup("Done"), ...done.map(row));
  for (const text of result.not_reached ?? []) {
    parts.push(ntRow({ tile: ntTile("wait"), title: text, sub: "Not reached" }));
  }
  parts.push(ntNotes(result.notes));
  const resume = page.session && result.tree ? `cd "${result.tree}" && claude --resume ${page.session}` : null;
  const buttons = [];
  if (result.branch || result.source === "copy") buttons.push(["Discard", (status) => ntDone(page.id, "discard", null, status)]);
  if (resume) {
    buttons.push([
      "Copy resume",
      async (status) => {
        await copyText(resume);
        status.textContent = "Copied; paste it in a terminal";
      },
    ]);
  }
  if (done.length) buttons.push(["Keep", (status) => ntDone(page.id, "keep", null, status), true]);
  else buttons.push(["Put away", (status) => ntDone(page.id, "dismiss", null, status), true]);
  parts.push(ntFoot(null, buttons));
  return parts;
}

// The smart stack on the to-do list: what Freebyrd can do for the person's
// work right now, one card at a time. Jobs run now come first (running,
// then done and not yet looked at), then tonight's picks with Run in
// Claude Code (a session in the Code tab), Tonight and Not now. Arrows, the dots or a sideways swipe move through it.
let ntStackIndex = 0;
let ntSwipe = 0;
let ntLoading = false;

function nightStack() {
  if (!ntView) {
    if (!ntLoading) {
      ntLoading = true;
      ntRefresh().finally(() => (ntLoading = false));
    }
    return null;
  }
  const items = [...(ntView.now ?? []), ...(ntView.suggested ?? [])];
  if (!items.length && !ntPicking) return null;
  ntStackIndex = Math.min(ntStackIndex, Math.max(items.length - 1, 0));
  const stack = ntNode("section", items.length > 1 ? "nt-stack many" : "nt-stack");
  stack.setAttribute("aria-label", "What Freebyrd can do now");

  const label = ntNode("span", "nt-stack-label", icon(NT.spark), "Freebyrd can do now");
  const head = ntNode("header", "", label);
  if (items.length > 1) {
    const dots = ntNode("span", "nt-dots");
    items.forEach((_, index) => {
      const dot = ntButton("", index === ntStackIndex ? "nt-dot on" : "nt-dot", () => ntStackGo(index));
      dot.setAttribute("aria-label", `Card ${index + 1} of ${items.length}`);
      dots.append(dot);
    });
    const prev = ntButton("", "nt-step", () => ntStackGo(ntStackIndex - 1));
    prev.append(icon(NT.back));
    prev.setAttribute("aria-label", "Previous");
    const next = ntButton("", "nt-step", () => ntStackGo(ntStackIndex + 1));
    next.append(icon(NT.chevron));
    next.setAttribute("aria-label", "Next");
    head.append(prev, dots, next);
  }
  stack.append(head);

  if (!items.length) {
    stack.append(ntRow({ tile: ntTile("list", "night"), title: "Looking at your work", sub: "What you owe and what you were in the middle of…" }));
    return stack;
  }
  const job = items[ntStackIndex];
  const status = ntNode("span", "card-status");
  const actions = ntNode("div", "nt-stack-actions", status);
  const tileState = job.state === "failed" ? "error" : job.needs ? "warn" : "night";
  let end = job.end;
  if (job.state === "suggested") {
    actions.append(
      ntButton("Not now", "card-action", async () => {
        await ntAct(job.id, "skip", null, status);
        ntRenderAll();
      }),
      ntButton("Tonight", "card-action", async () => {
        await ntAct(job.id, "add", null, status);
        ntRenderAll();
      }),
      // A session in the person's Code tab, in the project, with the job as
      // its prompt: they watch and steer it there.
      ntButton("Run in Claude Code", "card-action primary", async () => {
        status.textContent = "Opening Claude Code…";
        if (await ntAct(job.id, "code", null, status)) ntRenderAll();
      }),
    );
  } else if (job.state === "queued" || job.state === "running") {
    end = "";
    status.textContent = job.state === "running" ? `Running · ${job.end}` : "Starting…";
    status.classList.add("nt-live");
  } else {
    actions.append(
      ntButton("Put away", "card-action", async () => {
        await ntAct(job.id, "dismiss", null, status);
        ntRenderAll();
      }),
      ntButton("Open", "card-action primary", () => {
        openNight("waiting");
        ntOpenJob(job.id);
      }),
    );
  }
  const endNode = end ? [ntNode("span", "nt-end", end)] : [];
  stack.append(ntRow({ tile: ntTile(NT_KIND_ICON[job.kind] ?? "list", tileState), title: job.title, sub: job.sub, end: endNode }), actions);

  // A sideways swipe on the trackpad moves through the stack.
  stack.addEventListener(
    "wheel",
    (event) => {
      if (Math.abs(event.deltaX) <= Math.abs(event.deltaY)) return;
      event.preventDefault();
      ntSwipe += event.deltaX;
      if (Math.abs(ntSwipe) > 60) {
        ntStackGo(ntStackIndex + Math.sign(ntSwipe));
        ntSwipe = -Math.sign(ntSwipe) * 400;
        setTimeout(() => (ntSwipe = 0), 350);
      }
    },
    { passive: false },
  );
  return stack;
}

function ntStackGo(index) {
  const count = (ntView?.now?.length ?? 0) + (ntView?.suggested?.length ?? 0);
  if (!count) return;
  ntStackIndex = (index + count) % count;
  if (!todos.hidden) renderTodos();
}

// The stack's data, each time the to-do list opens; picks made if stale.
function ntForTodos() {
  if (todos.hidden) return;
  ntRefresh().then(() => {
    if (ntView && !ntView.picksFresh && !ntPickTried) ntPick();
  });
}

// Where the person last was in the to-do list or night shift: the panel,
// its tab, a job's page and how far down. A double tap of Option always
// opens the normal notch (the person's rule); ⌘⌥ within `NT_KEEP_MS` comes
// back here (`ntRestoreTasks`, from app.js). Openings without a panel in
// between leave it as it was.
const NT_KEEP_MS = 15 * 60 * 1000;
let ntSaved = null;

listen("fold", () => {
  const panel = !nightPanel.hidden ? "night" : !todos.hidden ? "todos" : null;
  if (!panel) return;
  ntSaved = {
    panel,
    todoTab,
    ntTab,
    ntJob,
    scroll: panel === "night" ? nightPanel.scrollTop : todos.scrollTop,
    at: Date.now(),
  };
});

// ⌘⌥: back where the person was, if recent. Returns whether it was.
function ntRestoreTasks() {
  const saved = ntSaved;
  if (!saved || Date.now() - saved.at >= NT_KEEP_MS) return false;
  ntRestore(saved);
  return true;
}

function ntRestore(saved) {
  if (saved.panel === "todos") {
    toggleNight(false);
    todoTab = saved.todoTab;
    toggleTodos(true);
    ntForTodos();
    requestAnimationFrame(() => (todos.scrollTop = saved.scroll));
  } else {
    ntTab = saved.ntTab;
    toggleNight(true);
    if (saved.ntJob) ntOpenJob(saved.ntJob);
    else requestAnimationFrame(() => (nightPanel.scrollTop = saved.scroll));
  }
}

// What the top input does while the panel is open: add a job on Tonight,
// take a redo note on a preview, or ask as usual.
function ntMode() {
  if (nightPanel.hidden) return null;
  if (ntRedo && ntJob === ntRedo) return "redo";
  if (ntTab === "tonight") return "add";
  return null;
}

function ntPlaceholder() {
  const mode = ntMode();
  if (mode === "add") question.placeholder = NT_ADD_PLACEHOLDER;
  else if (mode === "redo") question.placeholder = NT_REDO_PLACEHOLDER;
  else question.placeholder = thread.hidden ? PLACEHOLDER : FOLLOW_UP;
}

async function ntAdd(text) {
  ntAdding = text;
  ntError = "";
  renderNight();
  try {
    ntView = await invoke("night_add", { text });
  } catch (err) {
    ntError = String(err);
  }
  ntAdding = null;
  renderNight();
}

// The top input's Return, caught before app.js asks it as a question.
content.addEventListener(
  "submit",
  (event) => {
    const mode = event.target === bar ? ntMode() : null;
    if (!mode) return;
    event.preventDefault();
    event.stopPropagation();
    const text = question.value.trim();
    if (!text || ntAdding) return;
    question.value = "";
    updateSend();
    if (mode === "add") ntAdd(text);
    else {
      const id = ntRedo;
      ntRedo = null;
      ntDone(id, "redo", text, null);
    }
  },
  true,
);

function toggleNight(open = nightPanel.hidden) {
  if (open && !todos.hidden) toggleTodos(false);
  nightPanel.hidden = !open;
  nightToggle.setAttribute("aria-expanded", String(open));
  content.classList.toggle("listing", open || !todos.hidden);
  refitSoon();
  if (open) {
    toggleRead(false);
    renderNight();
    ntRefresh();
  } else {
    ntJob = null;
    ntPage = null;
    ntError = "";
    ntPickTried = false;
    ntRedo = null;
    ntPlaceholder();
  }
}

// From the night's card (app.js): the panel, on While you were away.
function openNight(tab = "waiting") {
  ntTab = tab;
  ntJob = null;
  toggleNight(true);
}

nightToggle.addEventListener("mousedown", (event) => event.preventDefault());
nightToggle.addEventListener("click", () => toggleNight());
// The list and night shift close each other; asking a question closes both.
todoToggle.addEventListener("click", () => {
  if (!nightPanel.hidden && !todos.hidden) toggleNight(false);
  ntForTodos();
});
bar.addEventListener("submit", () => toggleNight(false));
document.addEventListener("keydown", (event) => {
  if (!event.metaKey) return;
  const key = event.key.toLowerCase();
  if (key === "n") {
    event.preventDefault();
    toggleNight();
  } else if (key === "t") {
    if (!nightPanel.hidden && !todos.hidden) toggleNight(false);
    ntForTodos();
  }
});

// While the panel or the to-do list is open, a running job's progress
// keeps moving.
setInterval(() => {
  const running = ntView?.running || (ntView?.now ?? []).some((job) => job.state === "queued" || job.state === "running");
  if (!running) return;
  if ((!nightPanel.hidden && !ntJob) || (!todos.hidden && todoTab === "mine")) ntRefresh();
}, 5000);

// Every opening starts as the normal notch.
listen("capturing", () => {
  toggleNight(false);
  ntPickTried = false;
});
