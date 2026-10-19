// Overlay UI. Everything from the screen or from Claude is inserted as text,
// never as HTML, because it comes from untrusted pages.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const el = (id) => document.getElementById(id);
const island = el("island");
const content = el("content");
const source = el("source");
const status = el("status");
const bar = el("bar");
const question = el("question");
const read = el("read");
const thread = el("thread");
const cards = el("cards");
const quick = el("quick");
const upnext = el("upnext");
const todos = el("todos");
const todoToggle = el("todo-toggle");

let session = 0;
let screen = null;
let pendingQuestion = null;
let asking = false;
let reading = "Reading the screen…";
// The newest turn's answer element, its text so far, and the notes and
// errors shown under it.
let reply = null;
let answerText = "";
let extras = [];
let renderQueued = false;
// Where the thread was last scrolled to while following the newest turn;
// null once the person scrolls it themselves.
let pinned = null;
// What the next question is about, sent with it once: `plan:12` (Up next,
// a draft card), `listed:12` or `todo:3` (the to-do list), `person:4` (a
// page's Brief me).
let nextFocus = null;
// The newest turn is a day plan: what is typed next changes it.
let planShown = false;

const PLACEHOLDER = "Ask about this screen or your work";
const FOLLOW_UP = "Ask a follow-up";

const SOURCE_LABELS = {
  browserExtension: "browser extension",
  accessibility: "accessibility",
};

function reset(appName) {
  screen = null;
  pendingQuestion = null;
  asking = false;
  reading = appName ? `Reading ${appName}…` : "Reading the screen…";
  question.value = "";
  question.placeholder = PLACEHOLDER;
  el("app").textContent = appName ?? "Screen";
  source.dataset.state = "reading";
  describeRead(reading);
  el("read-text").textContent = "";
  showView(null);
  showRecalled("");
  setStatus(true);
  toggleRead(false);
  toggleTodos(false);
  expanded = null;
  showCards([]);
  showQuick([]);
  showUpNext(null);
  nextFocus = null;
  planShown = false;
  clearThread();
  updateSend();
  question.focus({ preventScroll: true });
}

// The level meter while reading or answering, beside the to-do list's icon.
// What was read (how much, or why nothing) is in the drawer under the app's
// name.
function setStatus(busy) {
  status.toggleAttribute("data-busy", busy);
}

// One line about what was read: the drawer's heading and the chip's tooltip.
function describeRead(text) {
  el("read-meta").textContent = text;
  source.title = text;
}

function toggleRead(open = read.hidden) {
  // One drawer at a time: what was read, or the to-do list.
  if (open && !todos.hidden) toggleTodos(false);
  read.hidden = !open;
  source.setAttribute("aria-expanded", String(open));
}

function showContext(next) {
  screen = next;
  el("app").textContent = next.appName ?? "Screen";
  if (next.withheld) {
    source.dataset.state = "withheld";
    describeRead(`Nothing was read: ${next.withheld}. Claude only sees your question.`);
    // Say so now, not after they ask.
    toggleRead(true);
  } else {
    source.dataset.state = "ready";
    const title =
      next.windowTitle && next.windowTitle !== next.appName ? `${next.windowTitle} · ` : "";
    const chars = next.text.length.toLocaleString();
    const cut = next.truncated ? " (cut short)" : "";
    describeRead(
      `${title}${chars} characters via ${SOURCE_LABELS[next.source]} in ${next.elapsedMs} ms${cut}`,
    );
    el("read-text").textContent = next.text || "(no text found)";
  }
  showView(next.withheld ? null : next);
  setStatus(false);
  if (pendingQuestion) {
    const queued = pendingQuestion;
    pendingQuestion = null;
    send(queued);
  }
}

// What of the window was in view, and the picture of it: both go with each
// question, so "this" means what the person was looking at.
function showView(next) {
  const view = next?.inView;
  const lines = [];
  if (view && view === next.text) lines.push("All of it was in view.");
  else if (view) lines.push(`In view: ${view.length.toLocaleString()} characters.`);
  if (next?.picture) {
    lines.push(`Also sent: a picture of the window, ${next.picture.width} × ${next.picture.height}.`);
  } else if (next?.noPicture) {
    lines.push(`No picture: ${next.noPicture}.`);
  }
  el("view-meta").textContent = lines.join(" ");
  el("view-text").textContent = view && view !== next.text ? view : "";
  const picture = el("read-picture");
  if (next?.picture) picture.src = next.picture.dataUrl;
  else picture.removeAttribute("src");
  picture.hidden = !next?.picture;
}

// The memory sent with the last question, under what was read from the screen.
function showRecalled(text) {
  el("memory-meta").textContent = text
    ? `Also sent from Freebyrd's memory on this Mac: ${text.length.toLocaleString()} characters`
    : "";
  el("memory-text").textContent = text;
}

// Each kind of card has a side of the notch and a solid icon (icons in
// Freebyrd are always solid). Left: what happened, the recaps and scheduled
// jobs' results. Right: what is coming up or needs you: due promises,
// prepared drafts, suggested skills and scheduled jobs.
const KINDS = {
  recap: {
    label: "Recap",
    side: "left",
    color: "#64d2ff",
    // A page with lines of text.
    path: "M7 2h7.59a2 2 0 0 1 1.41.59l3.41 3.41A2 2 0 0 1 20 7.41V20a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2Z M8.75 7a1 1 0 0 0 0 2h3.5a1 1 0 1 0 0-2h-3.5Z M8.75 11a1 1 0 1 0 0 2h7.5a1 1 0 1 0 0-2h-7.5Z M8.75 15a1 1 0 1 0 0 2h7.5a1 1 0 1 0 0-2h-7.5Z",
  },
  // Night shift's morning card: what ran while you were away (night.js).
  night: {
    label: "Night shift",
    side: "left",
    color: "#5e5ce6",
    // A crescent moon.
    path: "M20.6 14.2A8.6 8.6 0 0 1 9.8 3.4a.7.7 0 0 0-.9-.86A9.6 9.6 0 1 0 21.46 15.1a.7.7 0 0 0-.86-.9Z",
  },
  // The easy stop: near the usual stop, what is open handed to the night
  // shift with one press (night/stop.rs).
  stop: {
    label: "Stop for tonight",
    side: "right",
    color: "#5e5ce6",
    path: "M20.6 14.2A8.6 8.6 0 0 1 9.8 3.4a.7.7 0 0 0-.9-.86A9.6 9.6 0 1 0 21.46 15.1a.7.7 0 0 0-.86-.9Z",
  },
  // Night shift's evening card: work picked for tonight, waiting for Add.
  nightplan: {
    label: "Tonight",
    side: "right",
    color: "#5e5ce6",
    path: "M20.6 14.2A8.6 8.6 0 0 1 9.8 3.4a.7.7 0 0 0-.9-.86A9.6 9.6 0 1 0 21.46 15.1a.7.7 0 0 0-.86-.9Z",
  },
  // A scheduled job's result: something that happened while you were away.
  job: {
    label: "Job",
    side: "left",
    color: "#5e5ce6",
    // A clock face, its hands cut out.
    path: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M12 6.5a1 1 0 0 0-1 1v4.91a1 1 0 0 0 .29.7l3 3a1 1 0 0 0 1.42-1.41L13 12v-4.5a1 1 0 0 0-1-1Z",
  },
  due: {
    label: "Due",
    side: "right",
    color: "#ff9f0a",
    // A calendar.
    path: "M8 2a1 1 0 0 1 1 1v1h6V3a1 1 0 1 1 2 0v1h1a3 3 0 0 1 3 3v11a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3h1V3a1 1 0 0 1 1-1Z M5 9.5h14V11H5Z M7.5 13.25h3v3h-3Z",
  },
  draft: {
    label: "Draft",
    side: "right",
    color: "#30d158",
    // A pencil.
    path: "M16.59 3.41a2 2 0 0 1 2.82 0l1.18 1.18a2 2 0 0 1 0 2.82L9.4 18.6a2 2 0 0 1-.88.51l-4.27 1.22a.8.8 0 0 1-.99-.99l1.22-4.27a2 2 0 0 1 .51-.88L16.59 3.41Z",
  },
  suggestion: {
    label: "Suggestion",
    side: "right",
    color: "#bf5af2",
    // A four-point sparkle.
    path: "M12 2.5l2.04 6.12a2 2 0 0 0 1.26 1.26L21.5 12l-6.2 2.12a2 2 0 0 0-1.26 1.26L12 21.5l-2.04-6.12a2 2 0 0 0-1.26-1.26L2.5 12l6.2-2.12a2 2 0 0 0 1.26-1.26L12 2.5Z",
  },
  // Something calendar-able seen on screen, filled in as an event to add.
  event: {
    label: "Event",
    side: "right",
    color: "#ff375f",
    // A calendar with a plus.
    path: "M8 2a1 1 0 0 1 1 1v1h6V3a1 1 0 1 1 2 0v1h1a3 3 0 0 1 3 3v11a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3h1V3a1 1 0 0 1 1-1Z M12 10.5a1 1 0 0 0-1 1V13H9.5a1 1 0 1 0 0 2H11v1.5a1 1 0 1 0 2 0V15h1.5a1 1 0 1 0 0-2H13v-1.5a1 1 0 0 0-1-1Z",
  },
  // A block of a day plan (day.rs); BLOCKS colors it by what it is for.
  block: {
    label: "Plan",
    side: "right",
    color: "#0a84ff",
    // A briefcase.
    path: "M9 3h6a2 2 0 0 1 2 2v1h2a3 3 0 0 1 3 3v9a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3V9a3 3 0 0 1 3-3h2V5a2 2 0 0 1 2-2Z M9 5v1h6V5H9Z",
  },
  // Freebyrd needs an answer, such as whether a chat is about work.
  question: {
    label: "Question",
    side: "right",
    color: "#ffd60a",
    // A question mark in a circle.
    path: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M12 6.5a3.5 3.5 0 0 0-3.5 3.5 1 1 0 1 0 2 0 1.5 1.5 0 1 1 2.12 1.37c-.79.35-1.62 1.05-1.62 2.13v.5a1 1 0 1 0 2 0v-.37c0-.08.07-.2.44-.37A3.5 3.5 0 0 0 12 6.5Z M12 15.75a1.25 1.25 0 1 0 0 2.5 1.25 1.25 0 0 0 0-2.5Z",
  },
  // A tool asks to read a project's memory (the context connector): Allow,
  // Allow once, Not this project.
  access: {
    label: "Access",
    side: "right",
    color: "#ffd60a",
    // A key, its bow's hole cut out.
    path: "M15 2a7 7 0 0 0-6.64 9.2L2.3 17.29A1 1 0 0 0 2 18v3a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1.5h1.5a1 1 0 0 0 1-1V17h1.5a1 1 0 0 0 .71-.29l1.1-1.1A7 7 0 1 0 15 2Z M16.5 5.5a2 2 0 1 1 0 4a2 2 0 0 1 0-4Z",
  },
  // Something that helps with what the person just started (moments).
  idea: {
    label: "Idea",
    side: "right",
    color: "#ac8e68",
    // A light bulb.
    path: "M12 2a7 7 0 0 0-4 12.74V17a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1v-2.26A7 7 0 0 0 12 2Z M9 19.5h6a.5.5 0 0 1 .5.5 2 2 0 0 1-2 2h-3a2 2 0 0 1-2-2 .5.5 0 0 1 .5-.5Z",
  },
};
// A kind this page does not know yet still shows, on the right, as a dot.
const kindOf = (name) =>
  KINDS[name] ?? { label: name, side: "right", color: "#ffffff", path: "M12 6a6 6 0 1 1 0 12a6 6 0 0 1 0-12Z" };
// A day plan's blocks, by what the time is for: work, a break (a meal, a
// pause), or life (moving, people, rest, the end of the work day).
const BLOCKS = {
  work: { label: "Work", color: "#0a84ff", path: KINDS.block.path },
  break: {
    label: "Break",
    color: "#63e6e2",
    // A pause sign in a circle.
    path: "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M9 8a1 1 0 0 0-1 1v6a1 1 0 1 0 2 0V9a1 1 0 0 0-1-1Z M15 8a1 1 0 0 0-1 1v6a1 1 0 1 0 2 0V9a1 1 0 0 0-1-1Z",
  },
  life: {
    label: "Life",
    color: "#ff7a59",
    // A heart.
    path: "M12 21s-7.5-4.6-9.6-9.4C.9 8.2 2.9 4.5 6.6 4.5c2.1 0 3.6 1.1 5.4 3.2 1.8-2.1 3.3-3.2 5.4-3.2 3.7 0 5.7 3.7 4.2 7.1C19.5 16.4 12 21 12 21Z",
  },
};
const cardKind = (card) =>
  (card.kind === "block" && BLOCKS[card.data?.category]) ||
  (card.kind === "suggestion" && ANSWERED[card.data?.state]) ||
  kindOf(card.kind);
// A filled circle with an x cut out.
const DISMISS_PATH =
  "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M8.7 7.3a1 1 0 0 0-1.4 1.4l3.3 3.3-3.3 3.3a1 1 0 1 0 1.4 1.4l3.3-3.3 3.3 3.3a1 1 0 0 0 1.4-1.4L13.4 12l3.3-3.3a1 1 0 0 0-1.4-1.4L12 10.6 8.7 7.3Z";
// A filled circle with a check cut out: dim until done, green when done.
const CHECK_PATH =
  "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M16.7 8.3a1 1 0 0 0-1.4 0l-4.8 4.8-1.8-1.8a1 1 0 1 0-1.4 1.4l2.5 2.5a1 1 0 0 0 1.4 0l5.5-5.5a1 1 0 0 0 0-1.4Z";
// A suggestion once answered (`state` in its data, jobs.rs `card_state`):
// the header says where the fix stands instead of "Suggestion". Set up is a
// green check that stays lit, as a done to-do's; the rest are quiet.
const ANSWERED = {
  added: { label: "Skill added", color: "#30d158", path: CHECK_PATH, lit: true },
  scheduled: { label: "Scheduled", color: "#30d158", path: CHECK_PATH, lit: true },
  paused: { label: "Paused", color: "#ffffff", path: BLOCKS.break.path },
  removed: { label: "Removed", color: "#ffffff", path: DISMISS_PATH },
  declined: { label: "Declined", color: "#ffffff", path: DISMISS_PATH },
};
// Buttons that undo or stop something are never the white one, even first:
// Pause and Remove on a skill that was just added.
const QUIET_ACTIONS = new Set(["pause", "remove"]);

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

// Cards from Freebyrd (a recap, for now), shown when the island opens and
// put away once a question is asked.
function showCards(list) {
  cards.replaceChildren(...list.map(cardElement));
  cards.hidden = list.length === 0;
}

function cardElement(card) {
  const node = document.createElement("article");
  node.className = "card";
  node.classList.toggle("new", !card.seenAt);
  node.classList.toggle("block", card.kind === "block");
  const kind = cardKind(card);
  node.classList.toggle("lit", Boolean(kind.lit));
  // A block not yet added: a new plan puts it away.
  node.classList.toggle(
    "waiting",
    card.kind === "block" && Boolean(card.actions?.some((action) => action.id === "add")),
  );

  const head = document.createElement("header");
  const label = document.createElement("span");
  label.className = "card-kind";
  label.style.setProperty("--kind", kind.color);
  label.append(icon(kind.path), kind.label);
  const dismiss = document.createElement("button");
  dismiss.append(icon(DISMISS_PATH));
  dismiss.type = "button";
  dismiss.className = "card-dismiss";
  dismiss.setAttribute("aria-label", "Dismiss");
  // Keep the question focused.
  dismiss.addEventListener("mousedown", (event) => event.preventDefault());
  dismiss.addEventListener("click", () => {
    invoke("dismiss_card", { id: card.id });
    // A plan's cards sit in the thread, not in the card list.
    const list = node.parentElement;
    node.remove();
    if (list === cards) cards.hidden = !cards.hasChildNodes();
  });
  const when = element("strong", card.title);
  head.append(label, when, dismiss);

  const body = document.createElement("div");
  body.className = "card-body";
  // A nudge's event or a plan's block, ready to change before adding.
  const editable = card.kind === "event" || card.kind === "block";
  const form = editable && card.data?.date ? eventForm(card.data) : null;
  // A plan's block shows what to do; its fields open with Change, so a day
  // of blocks stays short.
  const what = form && card.kind === "block" ? element("p", card.data.title ?? "") : null;
  if (what) {
    what.className = "block-title";
    form.node.hidden = true;
    body.append(what);
  }
  if (form) body.append(form.node);
  body.append(markdown(card.body));
  node.append(head, body);
  // A suggestion's draft (the skill's SKILL.md, or the job's times and
  // prompt), shown on request below the text, in a box of its own. Drafted
  // from untrusted data: text only. Once set up it is the skill or the job.
  if (card.kind === "suggestion" && typeof card.data?.draft === "string") {
    const what = !card.data.state ? "draft" : card.data.fix === "schedule" ? "job" : "skill";
    node.append(...draftToggle(card.data.draft, what));
  }
  // A draft prepared before something is due (drafts.rs): the text to
  // send, with Copy. Drafted from untrusted data: text only.
  if (card.kind === "draft" && typeof card.data?.draft === "string") {
    node.append(preparedDraft(card.data));
    if (card.data.done?.length) node.append(...doneToggle(card.data.done));
  }

  if (card.actions?.length) {
    const row = document.createElement("div");
    row.className = "card-actions";
    const status = element("span", "");
    status.className = "card-status";
    card.actions.forEach((action, index) => {
      const button = element("button", action.label);
      button.type = "button";
      button.className =
        index === 0 && !QUIET_ACTIONS.has(action.id) ? "card-action primary" : "card-action";
      // Keep working asks in the thread, and Open shows night shift's panel:
      // the question keeps the keyboard.
      if ((card.kind === "draft" && action.id === "keep") || card.kind.startsWith("night")) {
        button.addEventListener("mousedown", (event) => event.preventDefault());
      }
      button.addEventListener("click", async () => {
        // Night shift's cards open its panel (night.js) and are put away:
        // the morning's on what ran, the evening's on tonight.
        if (card.kind.startsWith("night")) {
          invoke("dismiss_card", { id: card.id });
          node.remove();
          cards.hidden = !cards.children.length;
          openNight?.(card.kind === "nightplan" ? "tonight" : "waiting");
          return;
        }
        // Keep working on a prepared draft: its question (drafts.rs) asked in
        // the thread, with the plan and the draft sent along as memory.
        if (card.kind === "draft" && action.id === "keep") {
          if (!card.data?.plan || !card.data?.question) return;
          nextFocus = `plan:${card.data.plan}`;
          askNow(card.data.question, `Keep working: ${card.data.title ?? "the draft"}`);
          return;
        }
        for (const other of row.querySelectorAll("button")) other.disabled = true;
        // A work block's agenda is written before it goes on the calendar.
        const agenda = card.kind === "block" && card.data?.category === "work";
        status.textContent = action.id === "add" && agenda ? "Writing its agenda…" : WORKING[action.id] ?? "";
        try {
          const updated = await invoke("card_action", {
            id: card.id,
            action: action.id,
            template: form && action.id === "add" ? form.read() : null,
          });
          node.replaceWith(cardElement({ ...updated, seenAt: updated.seenAt ?? Date.now() }));
          // An undone promise leaves the to-do list.
          refreshTodos();
        } catch (err) {
          status.textContent = String(err);
          status.classList.add("error");
          for (const other of row.querySelectorAll("button")) other.disabled = false;
        }
      });
      row.append(button);
    });
    if (what) row.append(changeButton(form, what, when));
    row.append(status);
    node.append(row);
  }
  return node;
}

// Opens a block's fields, and on closing shows what was changed.
function changeButton(form, what, when) {
  const button = element("button", "Change");
  button.type = "button";
  button.className = "card-action";
  button.setAttribute("aria-expanded", "false");
  button.addEventListener("click", () => {
    const opening = form.node.hidden;
    form.node.hidden = !opening;
    what.hidden = opening;
    button.textContent = opening ? "Close" : "Change";
    button.setAttribute("aria-expanded", String(opening));
    if (opening) {
      form.node.querySelector(".ev-title")?.focus();
      return;
    }
    const event = form.read();
    what.textContent = event.title;
    when.textContent = event.time ? `${event.time}\u2013${clockAfter(event.time, event.minutes)}` : "All day";
  });
  return button;
}

// "16:30" plus 45 minutes: "17:15".
function clockAfter(time, minutes) {
  const [hours, mins] = time.split(":").map(Number);
  const end = (hours * 60 + mins + minutes) % (24 * 60);
  const pad = (n) => String(n).padStart(2, "0");
  return `${pad(Math.floor(end / 60))}:${pad(end % 60)}`;
}

// What a card's button is doing while its answer comes.
const WORKING = {
  add: "Adding…",
  yes: "Setting it up…",
  run: "Starting…",
  pause: "Pausing…",
  resume: "Resuming…",
  remove: "Removing…",
  open: "Opening…",
  copy: "Copying…",
  redo: "Gathering your work…",
  done: "Marking it done…",
  handoff: "Handing it to the night…",
};

// "What's done": the work a prepared draft found toward it, hidden until
// pressed so the card fits the notch. Text only.
function doneToggle(lines) {
  const list = document.createElement("ul");
  list.className = "card-done";
  list.hidden = true;
  list.append(...lines.map((line) => element("li", line)));
  const toggle = element("button", "What\u2019s done");
  toggle.type = "button";
  toggle.className = "card-draft-toggle";
  toggle.setAttribute("aria-expanded", "false");
  // Keep the question focused.
  toggle.addEventListener("mousedown", (event) => event.preventDefault());
  toggle.addEventListener("click", () => {
    list.hidden = !list.hidden;
    toggle.textContent = list.hidden ? "What\u2019s done" : "Hide what\u2019s done";
    toggle.setAttribute("aria-expanded", String(!list.hidden));
  });
  return [toggle, list];
}

// A draft prepared before something is due, inside its card: what it is
// and who it is for, then the text alone, with Copy.
function preparedDraft(data) {
  const node = document.createElement("div");
  node.className = "answer-draft prepared-draft";
  const head = document.createElement("header");
  const label = document.createElement("span");
  label.className = "answer-draft-label";
  const what = [
    `${data.kind === "update" ? "Update" : "Draft"}${data.to ? ` to ${data.to}` : ""}`,
    data.channel,
  ].filter(Boolean);
  label.append(icon(KINDS.draft.path), what.join(" \u00b7 "));
  const copied = element("span", "Copy");
  const copy = draftButton("answer-draft-copy", async () => {
    copied.textContent = (await copyText(data.draft)) ? "Copied" : "Could not copy";
    setTimeout(() => {
      copied.textContent = "Copy";
    }, 1500);
  });
  copy.append(icon(COPY_PATH), copied);
  head.append(label, copy);
  const text = element("p", data.draft);
  text.className = "answer-draft-text";
  node.append(head, text);
  return node;
}

// "Show the draft" (or the skill, or the job) and the draft itself, hidden
// until pressed.
function draftToggle(draft, what = "draft") {
  const text = element("pre", draft);
  text.className = "card-draft";
  text.hidden = true;
  const toggle = element("button", `Show the ${what}`);
  toggle.type = "button";
  toggle.className = "card-draft-toggle";
  toggle.setAttribute("aria-expanded", "false");
  // Keep the question focused.
  toggle.addEventListener("mousedown", (event) => event.preventDefault());
  toggle.addEventListener("click", () => {
    text.hidden = !text.hidden;
    toggle.textContent = `${text.hidden ? "Show" : "Hide"} the ${what}`;
    toggle.setAttribute("aria-expanded", String(!text.hidden));
  });
  return [toggle, text];
}

const LENGTHS = [15, 30, 45, 60, 90, 120];

// Fields for a nudge's event: title; date, time (empty for all day) and
// length; place. `read` returns them as the event to add.
function eventForm(event) {
  const field = (tag, className, attributes = {}) => {
    const input = document.createElement(tag);
    input.className = className;
    for (const [key, value] of Object.entries(attributes)) input.setAttribute(key, value);
    return input;
  };
  const title = field("input", "ev-title", { type: "text", "aria-label": "Title", spellcheck: "false" });
  title.value = event.title ?? "";
  const date = field("input", "ev-date", { type: "date", "aria-label": "Date" });
  date.value = event.date ?? "";
  const time = field("input", "ev-time", { type: "time", "aria-label": "Time (empty for all day)" });
  time.value = event.time ?? "";
  const length = field("select", "ev-length", { "aria-label": "Length" });
  const minutes = event.minutes ?? 30;
  for (const value of [...new Set([...LENGTHS, minutes])].sort((a, b) => a - b)) {
    const option = element("option", lengthLabel(value));
    option.value = String(value);
    option.selected = value === minutes;
    length.append(option);
  }
  const place = field("input", "ev-place", { type: "text", placeholder: "Where", "aria-label": "Where", spellcheck: "false" });
  place.value = event.location ?? "";
  // When to be reminded, for an event an answer offered (a flight's bag
  // drop); a nudge's event keeps the usual half hour.
  const alert = "alertMinutes" in event ? field("select", "ev-alert", { "aria-label": "Alert" }) : null;
  if (alert) {
    const before = event.alertMinutes ?? 30;
    for (const value of [...new Set([...ALERTS, before])].sort((a, b) => a - b)) {
      const option = element("option", value ? `Alert ${lengthLabel(value)} before` : "Alert at start");
      option.value = String(value);
      option.selected = value === before;
      alert.append(option);
    }
  }
  // All day: no length, and the alert is the morning's.
  const sync = () => {
    length.disabled = !time.value;
    if (alert) alert.disabled = !time.value;
  };
  time.addEventListener("input", sync);
  sync();

  const row = document.createElement("div");
  row.className = "ev-row";
  row.append(date, time, length);
  const node = document.createElement("div");
  node.className = "event-form";
  if (alert) {
    const where = document.createElement("div");
    where.className = "ev-row ev-where";
    where.append(place, alert);
    node.append(title, row, where);
  } else {
    node.append(title, row, place);
  }
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

// "45 min", "2 h", "1 h 45 min".
function lengthLabel(minutes) {
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  if (!hours) return `${rest} min`;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}

function element(tag, text) {
  const node = document.createElement(tag);
  node.textContent = text;
  return node;
}

function message(kind, text) {
  const node = document.createElement("p");
  node.className = kind;
  inline(node, text);
  return node;
}

function clearThread() {
  reply = null;
  answerText = "";
  extras = [];
  pinned = null;
  thread.replaceChildren();
  thread.hidden = true;
}

// A question moves from the input into the thread, above the space for its
// answer, and the input is free for a follow-up.
function addTurn(text) {
  answerText = "";
  extras = [];
  offeredNodes = new Map();
  const turn = document.createElement("div");
  turn.className = "turn";
  const asked = element("p", text);
  asked.className = "asked";
  reply = document.createElement("div");
  reply.className = "reply streaming";
  turn.append(asked, reply);
  thread.append(turn);
  thread.hidden = false;
  toggleTodos(false);
  cards.hidden = true;
  quick.hidden = true;
  upnext.hidden = true;
  question.value = "";
  question.placeholder = FOLLOW_UP;
  pinned = thread.scrollTop;
}

// A shimmering placeholder ("Thinking…") until the answer starts.
function showPending(text) {
  reply.replaceChildren(message("pending", text));
  follow();
}

function render() {
  renderQueued = false;
  if (!reply) return;
  const following = pinned !== null && Math.abs(thread.scrollTop - pinned) < 2;
  reply.replaceChildren(markdown(answerText, true), ...extras);
  if (following) follow();
  else pinned = null;
  updateFades();
}

// Keeps the newest question in view as its answer grows: the thread scrolls
// with the answer until the question reaches the top, then stays there, so
// the answer reads from its start. Below the top fade, so it stays sharp.
const PIN_GAP = 24;
function follow() {
  if (pinned === null) return;
  thread.scrollTop = reply.parentElement.offsetTop - PIN_GAP;
  pinned = thread.scrollTop;
  updateFades();
}

// Fades the thread's edges where there is more to scroll to.
function updateFades() {
  const { scrollTop, scrollHeight, clientHeight } = thread;
  thread.classList.toggle("more-above", scrollTop > 0);
  thread.classList.toggle("more-below", scrollHeight - scrollTop - clientHeight > 1);
}

// Text arrives a few characters at a time; render at most once a frame.
function queueRender() {
  if (renderQueued) return;
  renderQueued = true;
  requestAnimationFrame(render);
}

function finish() {
  asking = false;
  reply?.classList.remove("streaming");
  render();
  setStatus(false);
  updateSend();
  if (!frame) fitWindow(height.value, true);
}

function onAnswer(event) {
  switch (event.kind) {
    case "text":
      answerText += event.text;
      queueRender();
      break;
    case "note":
      extras.push(message("note", event.message));
      queueRender();
      break;
    case "refused":
      extras.push(
        message("error", `Claude declined to answer${event.category ? ` (${event.category})` : ""}.`),
      );
      finish();
      break;
    case "error":
      extras.push(message("error", event.message));
      finish();
      break;
    case "done":
      finish();
      break;
  }
}

async function send(text) {
  asking = true;
  showPending("Thinking…");
  setStatus(true);
  updateSend();
  const focus = nextFocus;
  nextFocus = null;
  try {
    await invoke("ask", { question: text, focus });
  } catch (err) {
    extras.push(message("error", String(err)));
    finish();
  }
}

// Claude answers in Markdown. The common parts (paragraphs, lists, headings,
// quotes, code, bold, italic, links) become elements whose text is set
// directly, so nothing in an answer is ever parsed as HTML.
// In an answer (`drafts`), a ```draft block is a draft to send (draftCard),
// and an ```event block an event to add to the calendar (offeredEvent).
function markdown(text, drafts = false) {
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

  const lines = text.split("\n");
  for (const raw of lines) {
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
      code =
        drafts && /^\s*```draft\s*$/.test(raw)
          ? draftCard(block("div"))
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
      // An indented line continues the last item.
      const item = list.lastElementChild;
      item.append(document.createElement("br"));
      inline(item, line.trim());
    } else {
      if (paragraph) paragraph.append(document.createElement("br"));
      else paragraph = block("p");
      inline(paragraph, line);
    }
  }
  // An event still arriving, or one whose block was never closed.
  if (offered) out.append(asking ? message("pending", "Filling in the event…") : offeredEvent(offered.join("\n")));
  return out;
}

// Events an answer offered, by their JSON, so a render while the answer
// streams keeps the fields being edited, and the card once added. A new
// question starts afresh (addTurn).
let offeredNodes = new Map();

// An event Claude offered because the person asked for something to go on
// their calendar (claude.rs): the event filled in, to change, and Add to
// calendar, which puts it on (add_event) and turns it into the card that
// says so, with Undo. Nothing is added before the press. The JSON comes
// from an answer that read the screen and the memory: its values are only
// ever set as text and field values.
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
    const pre = document.createElement("pre");
    pre.append(element("code", json));
    return pre;
  }
  const count = (value, max) => (Number.isInteger(value) && value >= 0 && value <= max ? value : null);
  const time = /^\d{2}:\d{2}$/.test(event.time ?? "") ? event.time : null;
  const data = {
    title: text(event.title, 160),
    date: event.date,
    time,
    minutes: count(event.minutes, 24 * 60) || 30,
    location: text(event.location, 200) || null,
    alertMinutes: time ? count(event.alertMinutes, 24 * 60) ?? 30 : null,
  };
  const source = text(event.source, 120);
  const notes = text(event.notes, 300);

  const node = document.createElement("article");
  // Its icon in color while it waits for the press, like a fresh nudge.
  node.className = "card offered new";
  const head = document.createElement("header");
  const label = document.createElement("span");
  label.className = "card-kind";
  label.style.setProperty("--kind", KINDS.event.color);
  label.append(icon(KINDS.event.path), KINDS.event.label);
  const when = element("strong", whenLabel(data.date, data.time));
  head.append(label, when);

  const body = document.createElement("div");
  body.className = "card-body";
  const form = eventForm(data);
  form.node.addEventListener("input", () => {
    const { date, time } = form.read();
    when.textContent = whenLabel(date, time);
  });
  body.append(form.node);
  const about = [notes, source && `From ${source}`].filter(Boolean).join(" \u00b7 ");
  if (about) {
    const line = element("p", about);
    line.className = "offered-about";
    body.append(line);
  }

  const row = document.createElement("div");
  row.className = "card-actions";
  const add = element("button", "Add to calendar");
  add.type = "button";
  add.className = "card-action primary";
  const status = element("span", "");
  status.className = "card-status";
  add.addEventListener("click", async () => {
    add.disabled = true;
    status.classList.remove("error");
    status.textContent = WORKING.add;
    try {
      const card = await invoke("add_event", {
        offer: { ...form.read(), source: source || null, notes: notes || null },
      });
      const done = cardElement({ ...card, seenAt: card.seenAt ?? Date.now() });
      done.classList.add("offered");
      offeredNodes.set(key, done);
      node.replaceWith(done);
      refreshTodos();
    } catch (err) {
      status.textContent = String(err);
      status.classList.add("error");
      add.disabled = false;
    }
  });
  row.append(add, status);
  node.append(head, body, row);
  offeredNodes.set(key, node);
  return node;
}

// "Fri 16 Oct, 23:30", or "Fri 16 Oct" for all day, as the cards say it.
function whenLabel(date, time) {
  const day = new Date(`${date}T00:00`);
  if (Number.isNaN(day.getTime())) return time ?? "";
  const label = day.toLocaleDateString("en-GB", { weekday: "short", day: "numeric", month: "short" }).replace(",", "");
  return time ? `${label}, ${time}` : label;
}

// Two sheets, the front one whole.
const COPY_PATH =
  "M9 2h9a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2h-1V8a3 3 0 0 0-3-3H7V4a2 2 0 0 1 2-2Z M6 7h8a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2Z";

// What each rework button asks, as a follow-up in the thread.
const REWORK = [
  ["Shorter", "Make the draft shorter."],
  ["More formal", "Rewrite the draft to be more formal."],
  ["Warmer", "Rewrite the draft to be warmer."],
];

// A draft to send, from a ```draft block in an answer: the draft alone, set
// as text, with Copy, and buttons that ask for it reworked (the newest
// answer's only). Returns the element its lines go into.
function draftCard(node) {
  node.className = "answer-draft";
  const text = document.createElement("p");
  text.className = "answer-draft-text";

  const head = document.createElement("header");
  const label = document.createElement("span");
  label.className = "answer-draft-label";
  label.append(icon(KINDS.draft.path), "Draft");
  const copied = element("span", "Copy");
  const copy = draftButton("answer-draft-copy", async () => {
    copied.textContent = (await copyText(text.textContent)) ? "Copied" : "Could not copy";
    setTimeout(() => {
      copied.textContent = "Copy";
    }, 1500);
  });
  copy.append(icon(COPY_PATH), copied);
  head.append(label, copy);

  const rework = document.createElement("div");
  rework.className = "answer-draft-rework";
  for (const [name, ask] of REWORK) {
    const button = draftButton("quick-ask", () => askNow(ask));
    button.textContent = name;
    rework.append(button);
  }
  const own = draftButton("quick-ask", () => {
    question.value = "Rework the draft: ";
    question.focus({ preventScroll: true });
    question.setSelectionRange(question.value.length, question.value.length);
    updateSend();
  });
  own.textContent = "Rework…";
  rework.append(own);

  node.append(head, text, rework);
  return text;
}

// A button that leaves the question focused and out of the keyboard's way.
function draftButton(className, onClick) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = className;
  button.tabIndex = -1;
  button.addEventListener("mousedown", (event) => event.preventDefault());
  button.addEventListener("click", onClick);
  return button;
}

// Through the Mac's pasteboard (main.rs): WebKit lets a page write the
// clipboard only in some cases.
async function copyText(text) {
  try {
    await invoke("copy_text", { text });
    return true;
  } catch {
    return false;
  }
}

// `code`, **bold**, *italic* or _italic_, and [label](url). Links show their
// label, with the address as a tooltip; the overlay opens nothing.
const INLINE =
  /`([^`]+)`|\*\*(.+?)\*\*|(?<![\w*])\*(?![\s*])(.+?)(?<![\s*])\*(?![\w*])|(?<!\w)_(?![\s_])(.+?)(?<![\s_])_(?!\w)|\[([^\]]+)\]\(([^)\s]+)\)/g;

function inline(parent, text) {
  let at = 0;
  for (const m of text.matchAll(INLINE)) {
    parent.append(text.slice(at, m.index));
    at = m.index + m[0].length;
    const [, code, bold, star, underscore, label, href] = m;
    if (code !== undefined) {
      parent.append(element("code", code));
    } else if (label !== undefined) {
      const link = element("span", label);
      link.className = "link";
      link.title = href;
      parent.append(link);
    } else {
      const node = document.createElement(bold !== undefined ? "strong" : "em");
      inline(node, bold ?? star ?? underscore);
      parent.append(node);
    }
  }
  parent.append(text.slice(at));
}

// A follow-up can be typed while an answer streams, and sent once it is done.
function updateSend() {
  bar.classList.toggle("can-send", !asking && question.value.trim() !== "");
}

// Asks `text`, showing `shown` as the question in the thread.
function askNow(text, shown = text) {
  if (!text || asking) return;
  addTurn(shown);
  if (!screen) {
    // Still reading; ask as soon as the screen arrives.
    asking = true;
    pendingQuestion = text;
    showPending(reading);
    updateSend();
    return;
  }
  send(text);
}

bar.addEventListener("submit", (event) => {
  event.preventDefault();
  const text = question.value.trim();
  if (planShown && text) planDay(text, text);
  else askNow(text);
});

// Plan my day (day.rs): the rest of the working day as blocks, each a card
// to change, add to the calendar or dismiss, under a line on how the time
// splits between work, breaks and life. What is typed next changes the
// plan; `note` is that text.
async function planDay(note, shown) {
  if (asking) return;
  addTurn(shown);
  asking = true;
  showPending(note ? "Changing the plan…" : "Planning your day…");
  setStatus(true);
  updateSend();
  const asked = session;
  try {
    const plan = await invoke("plan_day", { note });
    if (asked !== session) return;
    // The earlier plan's blocks still waiting were put away (day.rs).
    for (const old of thread.querySelectorAll(".day-plan .card.waiting")) old.remove();
    extras = [planElement(plan)];
    planShown = true;
    question.placeholder = "Change the plan: lighter, gym at 19:00…";
  } catch (err) {
    if (asked !== session) return;
    extras = [message("error", String(err))];
  }
  finish();
}

function planElement(plan) {
  const node = document.createElement("div");
  node.className = "day-plan";
  if (plan.note) node.append(message("plan-note", plan.note));
  const balance = element("p", [plan.day, plan.balance].filter(Boolean).join(" · "));
  balance.className = "plan-balance";
  const list = document.createElement("div");
  list.className = "plan-cards";
  list.append(...plan.cards.map(cardElement));
  const caveat = element(
    "p",
    "Freebyrd can't see your other calendars yet, so check the times against your meetings.",
  );
  caveat.className = "plan-caveat";
  node.append(balance, list, caveat);
  return node;
}

// Quick asks: questions picked for the screen in front (quick.rs), asked
// with one press, or with ⌘1 to ⌘3.
function showQuick(asks) {
  quick.replaceChildren(
    ...asks.map((ask, index) => {
      const button = element("button", ask.label);
      button.type = "button";
      button.className = "quick-ask";
      const does = ask.plan ? "Lay out your day as blocks to add to your calendar" : ask.question;
      button.title = `${does} (⌘${index + 1})`;
      // Keep the question focused, so Return only ever sends what was typed.
      button.tabIndex = -1;
      button.addEventListener("mousedown", (event) => event.preventDefault());
      button.addEventListener("click", () =>
        ask.plan ? planDay(null, ask.label) : askNow(ask.question),
      );
      return button;
    }),
  );
  quick.hidden = asks.length === 0;
}

// Up next: the one promise, task or event that most needs the person
// (`up_next` in plans.rs), with its buttons. The first asks about it, with
// the plan sent along as memory; Done and Not today bring up the next one.
function showUpNext(item) {
  upnext.hidden = !item;
  if (!item) {
    upnext.replaceChildren();
    return;
  }
  const kind = kindOf(item.kind === "event" ? "event" : "due");
  const head = document.createElement("header");
  const label = document.createElement("span");
  label.className = "upnext-label";
  label.style.setProperty("--kind", kind.color);
  label.append(icon(kind.path), "Up next");
  const when = element("span", item.when);
  when.className = item.overdue ? "upnext-when overdue" : "upnext-when";
  head.append(label, when);
  if (item.more) {
    const more = element("span", `${item.more} more`);
    more.className = "upnext-more";
    more.title = "Done or Not today brings up the next one";
    head.append(more);
  }
  const title = element("p", item.title);
  title.className = "upnext-title";
  title.title = `${item.title}\nFrom ${item.source}`;
  // Where an event is ("Goa to Bengaluru (T1)"), as text.
  const place = item.place ? element("p", item.place) : null;
  if (place) place.className = "upnext-place";

  const row = document.createElement("div");
  row.className = "card-actions";
  const status = element("span", "");
  status.className = "card-status";
  for (const [index, action] of item.actions.entries()) {
    const button = element("button", action.label);
    button.type = "button";
    // Only an ask leads: an event with nothing to ask has Hide alone.
    button.className = index === 0 && action.id === "ask" ? "card-action primary" : "card-action";
    button.addEventListener("mousedown", (event) => event.preventDefault());
    button.addEventListener("click", async () => {
      if (action.id === "ask") {
        nextFocus = `plan:${item.id}`;
        askNow(item.question, `${action.label}: ${item.title}`);
        return;
      }
      for (const other of row.querySelectorAll("button")) other.disabled = true;
      try {
        showUpNext(await invoke("up_next_action", { id: item.id, action: action.id }));
        // Done and Not today here are the to-do list's too.
        refreshTodos();
      } catch (err) {
        status.textContent = String(err);
        status.classList.add("error");
        for (const other of row.querySelectorAll("button")) other.disabled = false;
      }
    });
    row.append(button);
  }
  row.append(status);
  upnext.replaceChildren(...[head, title, place, row].filter(Boolean));
}

// The to-do list behind the checklist icon at the top right (todos.rs):
// what the person owes, what others owe them, the people they deal with
// (people.rs) and what they finished lately. It opens where the quick
// asks and cards sit, which come back when it closes; the number on the
// icon is what is overdue or due today, orange when anything is overdue.
let todoList = null;
let todoTab = "mine";
// The person's page shown in the list, if any.
let personShown = null;
// The to-do whose details and buttons are open.
let expanded = null;
// After adding a to-do, the Add field keeps the keyboard for the next.
let refocusAdd = false;

const TODO_TABS = [
  ["mine", "To do"],
  ["sessions", "Sessions"],
  ["waiting", "Waiting on"],
  ["people", "People"],
  ["done", "Done"],
];

const TODO_EMPTY = {
  mine: "Nothing on your list. Type one above; Freebyrd also adds your promises, Notion tasks and meeting action items as it finds them.",
  waiting: "No one owes you anything Freebyrd knows of.",
  people: "No one yet. Freebyrd builds a page for each person from WhatsApp, Gmail and Fireflies as it reads them.",
  done: "Nothing finished in the last two weeks.",
};

function showBadge(badge) {
  const count = el("todo-count");
  count.textContent = badge?.count ? String(badge.count) : "";
  count.hidden = !badge?.count;
  todoToggle.classList.toggle("overdue", Boolean(badge?.overdue));
  const due = badge?.count ? `, ${badge.count} overdue or due today` : "";
  todoToggle.title = badge ? `To-do list: ${badge.open} open${due} (⌘T)` : "To-do list (⌘T)";
}

function toggleTodos(open = todos.hidden) {
  todos.hidden = !open;
  todoToggle.setAttribute("aria-expanded", String(open));
  // The quick asks, Up next, cards and the thread step aside meanwhile.
  content.classList.toggle("listing", open);
  refitSoon();
  if (open) {
    toggleRead(false);
    renderTodos();
    refreshTodos();
    refreshSessions();
  } else {
    personShown = null;
    expanded = null;
  }
}

async function refreshTodos() {
  try {
    await setList(await invoke("todos"));
  } catch (err) {
    console.error(err);
  }
}

// The list as it is after a change; a page shown is read again too, since
// what is open between the person and them may have changed.
async function setList(list) {
  todoList = list;
  showBadge(list.badge);
  if (personShown) {
    try {
      personShown = await invoke("person_page", { id: personShown.id });
    } catch {
      personShown = null;
    }
  }
  renderTodos();
}

function renderTodos() {
  if (todos.hidden) return;
  const top = todos.scrollTop;
  if (personShown) {
    todos.replaceChildren(pageElement(personShown));
  } else {
    todos.replaceChildren(todoTabs(), todoBody());
  }
  todos.scrollTop = top;
  if (refocusAdd) {
    refocusAdd = false;
    todos.querySelector(".todo-add input")?.focus({ preventScroll: true });
  }
}

function todoTabs() {
  const tabs = document.createElement("nav");
  tabs.className = "todo-tabs";
  tabs.setAttribute("role", "tablist");
  const counts = {
    mine: todoList?.badge.open,
    sessions: sessionBoard?.needs,
    waiting: todoList?.waiting.length,
  };
  for (const [id, label] of TODO_TABS) {
    const tab = element("button", label);
    tab.type = "button";
    tab.className = "todo-tab";
    tab.setAttribute("role", "tab");
    tab.setAttribute("aria-selected", String(todoTab === id));
    if (counts[id]) {
      const count = element("span", String(counts[id]));
      count.className = "todo-tab-count";
      tab.append(count);
    }
    tab.addEventListener("mousedown", (event) => event.preventDefault());
    tab.addEventListener("click", () => {
      todoTab = id;
      expanded = null;
      todos.scrollTop = 0;
      renderTodos();
      if (id === "sessions") refreshSessions();
    });
    tabs.append(tab);
  }
  return tabs;
}

function todoBody() {
  const body = document.createElement("div");
  body.className = "todo-body";
  // What Freebyrd can do for these right now (night.js), above the list.
  const stack = todoTab === "mine" && typeof nightStack === "function" ? nightStack() : null;
  if (stack) body.append(stack);
  if (todoTab === "sessions") {
    body.append(...sessionsBody());
    return body;
  }
  if (todoTab === "mine") body.append(addForm());
  if (!todoList) {
    body.append(todoEmpty("Reading your list…"));
    return body;
  }
  if (todoTab === "mine") {
    for (const group of todoList.mine) {
      const label = element("h4", group.label);
      label.className = group.label === "Overdue" ? "todo-group overdue" : "todo-group";
      body.append(label, ...group.items.map((item) => todoRow(item)));
    }
    if (!todoList.mine.length) body.append(todoEmpty(TODO_EMPTY.mine));
  } else if (todoTab === "waiting") {
    body.append(...todoList.waiting.map((item) => todoRow(item)));
    if (!todoList.waiting.length) body.append(todoEmpty(TODO_EMPTY.waiting));
  } else if (todoTab === "people") {
    body.append(...todoList.people.map(personRow));
    if (!todoList.people.length) body.append(todoEmpty(TODO_EMPTY.people));
  } else {
    body.append(...todoList.done.map((item) => todoRow(item)));
    if (!todoList.done.length) body.append(todoEmpty(TODO_EMPTY.done));
  }
  return body;
}

// The Sessions tab: the person's Claude Code sessions of today (sessions.rs),
// under what they need. A row opens its session in the Claude app; Push
// hands a finished session's files to a new session that commits and
// pushes them. Read again every few seconds while the tab shows.
let sessionBoard = null;
let connectorReads = [];
let sessionTimer = null;
const SESSION_GROUPS = [
  ["working", "Working"],
  ["needs", "Needs you"],
  ["push", "Not pushed"],
  ["earlier", "Earlier"],
];
// A solid speech bubble: a session waits for the person.
const SESSION_PATH =
  "M12 3C6.48 3 2 6.58 2 11c0 2.3 1.22 4.37 3.17 5.83L4.5 21l4.37-2.43c1 .28 2.05.43 3.13.43 5.52 0 10-3.58 10-8s-4.48-8-10-8Z";
// A filled circle with an exclamation cut out: a session is blocked.
const BLOCKED_PATH =
  "M12 2a10 10 0 1 1 0 20a10 10 0 0 1 0-20Z M12 6.5a1.1 1.1 0 0 0-1.1 1.1v5.3a1.1 1.1 0 0 0 2.2 0V7.6A1.1 1.1 0 0 0 12 6.5Z M12 15.4a1.3 1.3 0 1 0 0 2.6 1.3 1.3 0 0 0 0-2.6Z";

// Claude at work: three dots that brighten in turn, as a reply being typed.
function workingDots() {
  const dots = document.createElement("i");
  dots.className = "dots";
  dots.append(...Array.from({ length: 3 }, () => document.createElement("span")));
  return dots;
}

// The board is kept fresh in the background (at start, every 45 s while the
// list is closed, and as the pointer comes over the closed island), so the
// tab opens on the whole list at once rather than growing out of a
// "Reading…" line. One read at a time.
let sessionsLoading = null;
function loadSessions() {
  sessionsLoading ??= Promise.all([invoke("sessions_board"), invoke("connector_reads").catch(() => [])])
    .then(([board, reads]) => {
      sessionBoard = board;
      connectorReads = reads;
      if (!todos.hidden && !personShown) renderTodos();
    })
    .catch((err) => console.error(err))
    .finally(() => (sessionsLoading = null));
  return sessionsLoading;
}

// Read on opening the list (for the tab's count) and every few seconds
// while the tab shows.
async function refreshSessions() {
  clearTimeout(sessionTimer);
  if (todos.hidden) return;
  await loadSessions();
  if (!todos.hidden && todoTab === "sessions") sessionTimer = setTimeout(refreshSessions, 5000);
}

loadSessions();
setInterval(() => {
  if (todos.hidden) loadSessions();
}, 45_000);

function sessionsBody() {
  if (!sessionBoard) return [todoEmpty("Reading your sessions…")];
  const nodes = [];
  for (const [group, label] of SESSION_GROUPS) {
    const rows = sessionBoard.rows.filter((row) => row.group === group);
    if (!rows.length) continue;
    const heading = element("h4", label);
    heading.className = "todo-group";
    nodes.push(heading, ...rows.map(sessionRow));
  }
  if (connectorReads.length) {
    const heading = element("h4", "Read by Claude Code today");
    heading.className = "todo-group";
    nodes.push(heading, ...connectorReads.map(readRow));
  }
  if (!nodes.length) return [todoEmpty("No Claude Code sessions today.")];
  return nodes;
}

// Who read what (the context connector, connector.rs): each session that
// read the memory today, what it asked for and how many items, counts
// only; Useful and Not useful are the person's word for the two weeks'
// measure (`connectorReads`, read with the board).
// A solid eye: something read the memory.
const READ_PATH =
  "M12 4.5C6.9 4.5 2.73 7.6 1 12c1.73 4.4 5.9 7.5 11 7.5s9.27-3.1 11-7.5c-1.73-4.4-5.9-7.5-11-7.5Z M12 8a4 4 0 1 1 0 8a4 4 0 0 1 0-8Z M12 10a2 2 0 1 0 0 4a2 2 0 0 0 0-4Z";

function readRow(reads) {
  const node = document.createElement("div");
  node.className = "todo-row session-row read-row";
  const mark = document.createElement("span");
  mark.className = "session-mark";
  mark.append(icon(READ_PATH));
  const main = document.createElement("div");
  main.className = "todo-main";
  const title = element("span", reads.title || reads.project);
  title.className = "todo-title";
  const meta = document.createElement("span");
  meta.className = "todo-meta";
  const items = `${reads.items} ${reads.items === 1 ? "item" : "items"}`;
  const when = new Date(reads.lastAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
  meta.append(element("span", reads.project), element("span", reads.asks), element("span", items), element("span", when));
  main.append(title, meta);
  const verdicts = document.createElement("div");
  verdicts.className = "read-verdicts";
  for (const [value, label] of [
    ["useful", "Useful"],
    ["not-useful", "Not useful"],
  ]) {
    const button = element("button", label);
    button.type = "button";
    button.className = `card-action${reads.verdict === value ? " primary" : ""}`;
    button.setAttribute("aria-pressed", String(reads.verdict === value));
    button.addEventListener("mousedown", (event) => event.preventDefault());
    button.addEventListener("click", async () => {
      try {
        await invoke("connector_verdict", { session: reads.sessionId, verdict: value });
        reads.verdict = value;
        renderTodos();
      } catch (err) {
        console.error(err);
      }
    });
    verdicts.append(button);
  }
  node.append(mark, main, verdicts);
  return node;
}

function sessionRow(row) {
  const node = document.createElement("div");
  node.className = `todo-row session-row ${row.state}${row.push ? " unpushed" : ""}`;

  // The mark (its speech bubble, pie or check) and the title both open the
  // session in the Claude app.
  const mark = document.createElement("button");
  mark.type = "button";
  mark.className = "session-mark";
  mark.title = "Open in Claude";
  mark.setAttribute("aria-label", `Open ${row.title} in Claude`);
  if (row.state === "running") {
    mark.append(workingDots());
  } else {
    mark.append(icon(row.state === "blocked" ? BLOCKED_PATH : row.state === "waiting" ? SESSION_PATH : CHECK_PATH));
  }

  const main = document.createElement("button");
  main.type = "button";
  main.className = "todo-main";
  main.title = "Open in Claude";
  const title = element("span", row.title);
  title.className = "todo-title";
  const meta = document.createElement("span");
  meta.className = "todo-meta";
  meta.append(element("span", row.folder));
  const state = element("span", row.label);
  if (row.state === "blocked") state.className = "todo-when overdue";
  meta.append(state);
  if (row.push) meta.append(element("span", row.push.label));
  main.append(title, meta);
  const status = element("span", "");
  status.className = "card-status";
  const open = async () => {
    try {
      await invoke("session_open", { id: row.id });
    } catch (err) {
      status.textContent = String(err);
      status.classList.add("error");
    }
  };
  for (const target of [mark, main]) {
    target.addEventListener("mousedown", (event) => event.preventDefault());
    target.addEventListener("click", open);
  }
  node.append(mark, main);

  if (row.push) {
    const push = element("button", "Push");
    push.type = "button";
    push.className = "card-action primary session-push";
    push.title = `Commit and push in a new Claude Code session: ${row.push.label}`;
    push.addEventListener("mousedown", (event) => event.preventDefault());
    push.addEventListener("click", async () => {
      push.disabled = true;
      status.classList.remove("error");
      status.textContent = "Opening a session to push it…";
      try {
        await invoke("session_push", { id: row.id });
      } catch (err) {
        push.disabled = false;
        status.textContent = String(err);
        status.classList.add("error");
      }
    });
    node.append(push);
  }
  node.append(status);
  return node;
}

function todoEmpty(text) {
  const node = element("p", text);
  node.className = "todo-empty";
  return node;
}

// Typed to-dos: a day in the text ("by Thursday", "tomorrow", "15 Oct")
// sets the date, and a name Freebyrd knows becomes who it is for.
function addForm() {
  const form = document.createElement("form");
  form.className = "todo-add";
  form.autocomplete = "off";
  const input = document.createElement("input");
  input.type = "text";
  input.placeholder = "Add a to-do. A day in it sets the date";
  input.spellcheck = false;
  input.setAttribute("aria-label", "Add a to-do");
  const status = element("span", "");
  status.className = "card-status";
  form.append(input, status);
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    input.disabled = true;
    try {
      refocusAdd = true;
      await setList(await invoke("add_todo", { text }));
    } catch (err) {
      refocusAdd = false;
      input.disabled = false;
      status.textContent = String(err);
      status.classList.add("error");
    }
  });
  return form;
}

// A to-do as a row. On a person's page (`without` is theirs), their own
// name is left off.
function todoRow(item, without = null) {
  const row = document.createElement("div");
  row.className = "todo-row";
  const done = item.actions.find((action) => action.id === "done" || action.id === "undo");
  row.classList.toggle("finished", done?.id === "undo");

  const check = document.createElement("button");
  check.type = "button";
  check.className = "todo-check";
  check.setAttribute("aria-label", done?.label ?? "Done");
  check.title = done?.id === "undo" ? "Undo: back on the list" : (done?.label ?? "");
  check.append(icon(CHECK_PATH));
  check.addEventListener("mousedown", (event) => event.preventDefault());
  check.addEventListener("click", () => {
    if (done) todoAct(item, done.id, null, null);
  });

  const main = document.createElement("button");
  main.type = "button";
  main.className = "todo-main";
  main.setAttribute("aria-expanded", String(expanded === item.id));
  const title = element("span", item.title);
  title.className = "todo-title";
  main.append(title);
  const meta = document.createElement("span");
  meta.className = "todo-meta";
  if (item.person && item.person.id !== without) meta.append(personChip(item.person));
  else if (item.who) meta.append(element("span", item.who));
  if (item.when) {
    const when = element("span", item.when);
    when.className = item.overdue ? "todo-when overdue" : "todo-when";
    meta.append(when);
  }
  if (meta.hasChildNodes()) main.append(meta);
  // What last moved it, read from the work (progress.rs); opened, the
  // details say it in full.
  if (item.latest && expanded !== item.id) {
    const latest = element("span", `${item.latest.when} \u00b7 ${item.latest.note}`);
    latest.className = "todo-latest";
    main.append(latest);
  }
  main.addEventListener("mousedown", (event) => event.preventDefault());
  main.addEventListener("click", () => {
    expanded = expanded === item.id ? null : item.id;
    renderTodos();
  });

  row.append(check, main);
  if (expanded === item.id) row.append(todoDetails(item));
  return row;
}

// Under an opened to-do: where it came from, and its buttons. Ask closes the
// list and asks in the thread, with the to-do sent along as memory.
function todoDetails(item) {
  const box = document.createElement("div");
  box.className = "todo-details";
  const from = element("p", `From ${item.source}`);
  from.className = "todo-source";
  const latest = item.latest
    ? element("p", `Latest, ${item.latest.when} in ${item.latest.from}: ${item.latest.note}${item.latest.next ? `. Next: ${item.latest.next}` : ""}`)
    : null;
  if (latest) latest.className = "todo-source";
  const row = document.createElement("div");
  row.className = "card-actions";
  const status = element("span", "");
  status.className = "card-status";
  for (const action of item.actions) {
    if (action.id === "done" || action.id === "undo") continue;
    const button = element("button", action.label);
    button.type = "button";
    button.className = action.id === "ask" ? "card-action primary" : "card-action";
    button.addEventListener("mousedown", (event) => event.preventDefault());
    button.addEventListener("click", () => {
      if (action.id === "ask") {
        if (asking) return;
        nextFocus = item.focus;
        toggleTodos(false);
        askNow(item.question, `${action.label}: ${item.title}`);
        return;
      }
      if (action.id === "date") {
        row.replaceWith(datePicker(item));
        return;
      }
      for (const other of row.querySelectorAll("button")) other.disabled = true;
      status.textContent = action.id === "calendar" ? "Adding it, with an agenda…" : "";
      todoAct(item, action.id, null, status).finally(() => {
        for (const other of row.querySelectorAll("button")) other.disabled = false;
      });
    });
    row.append(button);
  }
  row.append(status);
  box.append(from);
  if (latest) box.append(latest);
  box.append(row);
  return box;
}

function datePicker(item) {
  const row = document.createElement("div");
  row.className = "card-actions todo-date";
  const input = document.createElement("input");
  input.type = "date";
  const today = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  input.min = `${today.getFullYear()}-${pad(today.getMonth() + 1)}-${pad(today.getDate())}`;
  input.value = input.min;
  const set = element("button", "Set");
  set.type = "button";
  set.className = "card-action primary";
  const cancel = element("button", "Cancel");
  cancel.type = "button";
  cancel.className = "card-action";
  const status = element("span", "");
  status.className = "card-status";
  set.addEventListener("click", () => todoAct(item, "date", input.value, status));
  cancel.addEventListener("click", () => renderTodos());
  row.append(input, set, cancel, status);
  setTimeout(() => input.focus(), 0);
  return row;
}

async function todoAct(item, action, value, status) {
  try {
    await setList(await invoke("todo_action", { id: item.id, action, value }));
  } catch (err) {
    if (status) {
      status.textContent = String(err);
      status.classList.add("error");
    } else {
      console.error(err);
    }
  }
}

function personChip(chip) {
  const button = element("span", chip.name);
  button.className = "person-chip";
  button.setAttribute("role", "button");
  button.title = `${chip.name}: their page`;
  button.addEventListener("mousedown", (event) => event.preventDefault());
  button.addEventListener("click", (event) => {
    event.stopPropagation();
    openPerson(chip.id);
  });
  return button;
}

function personRow(person) {
  const row = document.createElement("button");
  row.type = "button";
  row.className = "person-row";
  const name = element("span", person.name);
  name.className = "person-name";
  const detail = element("span", person.detail);
  detail.className = person.needsYou ? "person-detail needs" : "person-detail";
  row.append(name, detail);
  row.addEventListener("mousedown", (event) => event.preventDefault());
  row.addEventListener("click", () => openPerson(person.id));
  return row;
}

async function openPerson(id) {
  try {
    personShown = await invoke("person_page", { id });
    expanded = null;
    todos.scrollTop = 0;
  } catch (err) {
    console.error(err);
    return;
  }
  if (todos.hidden) toggleTodos(true);
  else renderTodos();
}

async function personAct(page, action, value, status) {
  try {
    personShown = await invoke("person_action", { id: page.id, action, value });
    await refreshTodos();
  } catch (err) {
    if (status) {
      status.textContent = String(err);
      status.classList.add("error");
    }
  }
}

// One person's page: who they are, what waits on the person, what is open
// either way, where they met lately, their other names, and the person's
// corrections. Everything on it came from their messages: text only.
function pageElement(page) {
  const node = document.createElement("div");
  node.className = "person-page";

  const head = document.createElement("header");
  head.className = "page-head";
  const back = element("button", "‹ People");
  back.type = "button";
  back.className = "page-back";
  back.addEventListener("mousedown", (event) => event.preventDefault());
  back.addEventListener("click", () => {
    personShown = null;
    todoTab = "people";
    renderTodos();
  });
  const name = element("strong", page.name);
  const work = page.work === true ? "work" : page.work === false ? "personal" : null;
  const about = element("span", [page.org, work, ...page.places].filter(Boolean).join(" · "));
  about.className = "page-about";
  head.append(back, name, about);
  node.append(head);
  if (page.last) node.append(pageText(page.last, "page-last"));

  const note = document.createElement("textarea");
  note.className = "page-note";
  note.rows = 1;
  note.placeholder = "Your note about them. It goes with questions about them.";
  note.value = page.note ?? "";
  note.addEventListener("change", () => personAct(page, "note", note.value));
  note.addEventListener("keydown", (event) => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      note.blur();
    }
  });
  node.append(note);

  pageSection(
    node,
    "Waiting on you",
    page.waiting.map((waiting) => pageLine(`${waiting.place}, ${waiting.when}`, waiting.text)),
  );
  pageSection(node, "You owe them", page.owe.map((item) => todoRow(item, page.id)));
  pageSection(node, "They owe you", page.owed.map((item) => todoRow(item, page.id)));
  pageSection(
    node,
    "Lately",
    page.recent.map((recent) => pageLine(recent.what, recent.when)),
  );

  const status = element("span", "");
  status.className = "card-status";
  if (page.aliases.length > 1) {
    pageSection(
      node,
      "Also seen as",
      [
        aliasChips(
          page.aliases,
          "Not the same person: make this a page of its own",
          (alias) => personAct(page, "split", alias.alias, status),
        ),
      ],
    );
  }
  pageSection(
    node,
    "Maybe also",
    page.maybe.map((maybe) => {
      const line = pageLine(maybe.shown, maybe.place);
      const same = element("button", "Same person");
      same.type = "button";
      same.className = "card-action";
      same.addEventListener("click", () => personAct(page, "claim", maybe.alias, status));
      line.append(same);
      return line;
    }),
  );

  const row = document.createElement("div");
  row.className = "card-actions";
  const brief = element("button", "Brief me");
  brief.type = "button";
  brief.className = "card-action primary";
  brief.addEventListener("mousedown", (event) => event.preventDefault());
  brief.addEventListener("click", () => {
    if (asking) return;
    nextFocus = `person:${page.id}`;
    toggleTodos(false);
    askNow(page.question, `Brief me: ${page.name}`);
  });
  row.append(brief);
  if (page.others.length) {
    const merge = document.createElement("select");
    merge.className = "page-merge";
    merge.append(new Option("Same person as…", ""));
    for (const other of page.others) merge.append(new Option(other.name, String(other.id)));
    merge.addEventListener("change", () => {
      if (merge.value) personAct(page, "merge", merge.value, status);
    });
    row.append(merge);
  }
  const hide = element("button", page.hidden ? "Is a person" : "Not a person");
  hide.type = "button";
  hide.className = "card-action";
  hide.title = page.hidden ? "Show them again" : "A bot or a shared inbox: leave them out";
  hide.addEventListener("click", () => personAct(page, page.hidden ? "show" : "hide", null, status));
  row.append(hide, status);
  node.append(row);
  return node;
}

function pageSection(node, title, rows) {
  if (!rows.length) return;
  const label = element("h4", title);
  label.className = "todo-group";
  node.append(label, ...rows);
}

function pageText(text, className) {
  const line = element("p", text);
  line.className = className;
  return line;
}

function pageLine(head, text) {
  const line = document.createElement("div");
  line.className = "page-line";
  const strong = element("span", head);
  strong.className = "page-line-head";
  line.append(strong);
  if (text) {
    const rest = element("span", text);
    rest.className = "page-line-text";
    line.append(rest);
  }
  return line;
}

function aliasChips(aliases, splitTitle, onSplit) {
  const box = document.createElement("div");
  box.className = "alias-chips";
  for (const alias of aliases) {
    const chip = element("span", alias.shown);
    chip.className = "alias-chip";
    const split = document.createElement("button");
    split.type = "button";
    split.className = "alias-split";
    split.title = splitTitle;
    split.setAttribute("aria-label", `${alias.shown}: not the same person`);
    split.append(icon(DISMISS_PATH));
    split.addEventListener("click", () => onSplit(alias));
    chip.append(split);
    box.append(chip);
  }
  return box;
}

todoToggle.addEventListener("click", () => toggleTodos());
// Opening the list with the mouse leaves the question focused.
todoToggle.addEventListener("mousedown", (event) => event.preventDefault());

question.addEventListener("input", updateSend);
thread.addEventListener("scroll", updateFades, { passive: true });

source.addEventListener("click", () => toggleRead());
// Opening the drawer with the mouse leaves the question focused.
source.addEventListener("mousedown", (event) => event.preventDefault());

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") invoke("dismiss");
  if (event.metaKey && event.key.toLowerCase() === "t") {
    event.preventDefault();
    toggleTodos();
    return;
  }
  const listing = content.classList.contains("listing");
  const number = event.metaKey && !quick.hidden && !listing && /^[1-3]$/.test(event.key);
  if (number) {
    event.preventDefault();
    quick.children[Number(event.key) - 1]?.click();
  }
});

// The window is a little wider than the island. A click beside the island
// closes it, as a click on another app would.
document.addEventListener("mousedown", (event) => {
  if (!island.contains(event.target)) invoke("dismiss");
});

// The island's size follows springs stepped once per display frame, 120
// times a second on ProMotion (frame_rate.rs). Each step uses the exact
// solution, so the motion is the same at any frame rate, and a new target
// keeps the current velocity, so the island never stalls when an answer grows
// mid-motion.
class Spring {
  constructor(value) {
    this.value = value;
    this.target = value;
    this.velocity = 0;
    this.omega = 0;
    this.zeta = 1;
  }

  to(target, { response, damping }) {
    this.target = target;
    this.omega = (2 * Math.PI) / response;
    this.zeta = damping;
  }

  jump(value) {
    this.value = value;
    this.target = value;
    this.velocity = 0;
  }

  // Advances `dt` seconds; returns whether it is still moving.
  step(dt) {
    const { omega, zeta } = this;
    const x0 = this.value - this.target;
    const v0 = this.velocity;
    let x;
    let v;
    if (zeta < 1) {
      const wd = omega * Math.sqrt(1 - zeta * zeta);
      const decay = Math.exp(-zeta * omega * dt);
      const b = (v0 + zeta * omega * x0) / wd;
      const cos = Math.cos(wd * dt);
      const sin = Math.sin(wd * dt);
      x = decay * (x0 * cos + b * sin);
      v = decay * ((b * wd - zeta * omega * x0) * cos - (x0 * wd + zeta * omega * b) * sin);
    } else {
      const decay = Math.exp(-omega * dt);
      const b = v0 + omega * x0;
      x = decay * (x0 + b * dt);
      v = decay * (b - omega * (x0 + b * dt));
    }
    if (Math.abs(x) < 0.01 && Math.abs(v) < 0.5) {
      this.jump(this.target);
      return false;
    }
    this.value = this.target + x;
    this.velocity = v;
    return true;
  }
}

// Opening overshoots by about 1.5%, like the Dynamic Island. Growing with an
// answer barely does. Folding does not, and is done within FOLD in main.rs.
const OPEN = { response: 0.38, damping: 0.8 };
const GROW = { response: 0.32, damping: 0.92 };
const FOLD = { response: 0.19, damping: 1 };

const rootStyle = getComputedStyle(document.documentElement);
const OPEN_WIDTH = parseFloat(rootStyle.getPropertyValue("--width"));
const RADIUS = parseFloat(rootStyle.getPropertyValue("--radius"));
const NOTCH_RADIUS = 10;

const width = new Spring(island.offsetWidth);
const height = new Spring(island.offsetHeight);
// How far the island's center sits right of the notch's: nonzero when only
// one side widens for waiting cards.
const shift = new Spring(0);
let closed = { width: width.value, height: height.value };
let isOpen = false;
let contentHeight = 0;
// The content's height with just the strip and the question: what opens.
let openingHeight = 0;
let frame = 0;
let lastFrame = 0;

function paint() {
  island.style.width = `${width.value}px`;
  island.style.height = `${height.value}px`;
  island.style.translate = `calc(-50% + ${shift.value}px) 0`;
  // The corners round out from the notch's own as the island opens.
  const opened = (width.value - closed.width) / (OPEN_WIDTH - closed.width);
  const radius = NOTCH_RADIUS + (RADIUS - NOTCH_RADIUS) * Math.min(Math.max(opened, 0), 1);
  island.style.borderRadius = `0 0 ${radius}px ${radius}px`;
}

function tick(now) {
  // After a stall, step at most a frame at 30 fps.
  const dt = lastFrame ? Math.min((now - lastFrame) / 1000, 1 / 30) : 1 / 120;
  lastFrame = now;
  const widthMoving = width.step(dt);
  const heightMoving = height.step(dt);
  const shiftMoving = shift.step(dt);
  paint();
  if (widthMoving || heightMoving || shiftMoving) {
    frame = requestAnimationFrame(tick);
  } else {
    frame = 0;
    lastFrame = 0;
    if (isOpen && !asking) fitWindow(height.value, true);
  }
}

const reduceMotion = matchMedia("(prefers-reduced-motion: reduce)");

function animate(toWidth, toHeight, spring, toShift = 0) {
  width.to(toWidth, spring);
  height.to(toHeight, spring);
  shift.to(toShift, spring);
  fitWindow(Math.max(height.value, toHeight));
  if (reduceMotion.matches) {
    width.jump(toWidth);
    height.jump(toHeight);
    shift.jump(toShift);
    paint();
    if (isOpen && !asking) fitWindow(toHeight, true);
  } else if (!frame) {
    frame = requestAnimationFrame(tick);
  }
}

// The window must hold the island at its biggest, with room for the opening
// overshoot. Resizing a web view can cost a frame, so the window grows ahead
// of the island in steps (an answer arriving line by line resizes it rarely)
// and is trimmed to fit only once the island is still and the answer done.
const OVERSHOOT = 8;
const STEP = 96;
let fitted = 0;
function fitWindow(islandHeight, trim = false) {
  const exact = Math.ceil(islandHeight) + OVERSHOOT;
  let needed = fitted;
  if (exact > fitted) needed = Math.ceil(exact / STEP) * STEP;
  else if (trim) needed = exact;
  if (needed === fitted) return;
  fitted = needed;
  invoke("fit", { height: needed });
}

const measure = () => Math.ceil(content.getBoundingClientRect().height);

function refit() {
  contentHeight = measure();
  if (isOpen) animate(OPEN_WIDTH, contentHeight, GROW);
}

new ResizeObserver(refit).observe(content);

// A panel shown or hidden (the to-do list, night shift) asks for a fit of
// its own. Each opening hides the list (reset) and a click on the island
// shows it again at once, so the content can end up the size it was when
// the notch last closed, and the observer, which reports only a size that
// differs from its last report, stays quiet: the notch then opened at its
// plain height with the list cut off (seen on the real notch, 2026-10-14).
function refitSoon() {
  requestAnimationFrame(refit);
}

// Takes the size of the notch the window now hangs from. Without a notch,
// the island drops from the middle of the top edge.
function fitNotch(notch) {
  const root = document.documentElement;
  root.classList.toggle("flat", notch.height === 0);
  closed = { width: notch.height ? notch.width : 160, height: notch.height };
  root.style.setProperty("--notch-w", `${closed.width}px`);
  root.style.setProperty("--notch-h", `${closed.height}px`);
}

// The closed island. Cards are waiting: it glows and widens beside the
// notch, the way the Dynamic Island's compact view does, with the icon of
// the newest waiting kind on its side (KINDS). Something is live (live.rs):
// it widens without a glow, like a Live Activity, and stays while that is
// true. Right of the notch, how much on the to-do list is overdue or due
// today; left of it, work in progress: a job started with Run now, as a pie
// filling up, and night shift, the moon with how many jobs wait for
// tonight, or a pie of tonight's run. Card icons sit at the outer edges, so
// a card arriving widens the island without moving what is live. Only a side
// with something on it widens. On a screen without a notch, a small black
// tab does the same. Clicks go through the window to the menu bar, except
// over the island (hover.rs): a click there opens the notch, on the to-do
// list for its count or a job run now, on night shift for the moon.
const GLOW_ROOM = 18;
// From a side's outer edge to its first indicator (as .wing in style.css),
// and from its last to the notch.
const WING_OUTER = 12;
const WING_INNER = 10;
const NO_NOTCH = { width: 56, height: 26 };
const TASK_COLOR = "#30d158";
// Night shift's indigo, lighter than its card's so a count reads on black.
const NIGHT_COLOR = "#7d7aff";
// The to-do list's icon, as on its button in index.html.
const TODO_PATH =
  "M6 3h12a3 3 0 0 1 3 3v12a3 3 0 0 1-3 3H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3Z M16.7 8.3a1 1 0 0 0-1.4 0l-4.8 4.8-1.8-1.8a1 1 0 1 0-1.4 1.4l2.5 2.5a1 1 0 0 0 1.4 0l5.5-5.5a1 1 0 0 0 0-1.4Z";

// The indicators by key, kept across updates: a count or a pie changes in
// place, and only one that was not showing pops in.
const indicators = new Map();
// Where a click on an indicator opens the notch; a card's opens it as a
// double tap does, with the cards.
const PANELS = { todo: "todos", task: "todos", night: "night", "sessions-work": "sessions", "sessions-needs": "sessions", reads: "sessions" };

function indicator(key, is, parts) {
  let node = indicators.get(key);
  if (!node) {
    node = document.createElement("span");
    node.className = "chip entering";
    if (PANELS[key]) node.dataset.panel = PANELS[key];
    node.addEventListener("animationend", () => node.classList.remove("entering"), { once: true });
    indicators.set(key, node);
  }
  if (node.dataset.is !== is) {
    node.dataset.is = is;
    node.replaceChildren(...parts());
  }
  return node;
}

function cardIndicator(key, kind) {
  const node = indicator(key, kind.label, () => [icon(kind.path)]);
  node.style.color = kind.color;
  return node;
}

// An icon and a number: the to-do list, or the jobs waiting for tonight.
function countIndicator(key, path, count, color) {
  const node = indicator(key, "count", () => [icon(path), document.createElement("b")]);
  node.lastChild.textContent = String(count);
  node.style.color = color;
  return node;
}

// A pie that fills as a job goes, and where it is ("2/5", "40%"). While
// it starts, a quarter of it turns.
function pieIndicator(key, progress, color) {
  const node = indicator(key, "pie", () => {
    const pie = document.createElement("i");
    pie.className = "pie";
    return [pie, document.createElement("b")];
  });
  const [pie, label] = node.children;
  pie.classList.toggle("starting", progress.done == null);
  pie.style.setProperty("--p", String(Math.round((progress.done ?? 0) * 1000) / 10));
  label.textContent = progress.label ?? "";
  label.hidden = !progress.label;
  node.style.color = color;
  return node;
}

function showWings(kinds, live) {
  const cards = { left: null, right: null };
  for (const name of kinds) {
    const kind = kindOf(name);
    cards[kind.side] ??= kind;
  }
  const left = [];
  if (cards.left) left.push(cardIndicator("card-left", cards.left));
  if (live?.night?.running) left.push(pieIndicator("night", live.night.running, NIGHT_COLOR));
  else if (live?.night) left.push(countIndicator("night", KINDS.night.path, live.night.queued, NIGHT_COLOR));
  if (live?.task) left.push(pieIndicator("task", live.task, TASK_COLOR));
  // Claude Code sessions: the working dots with how many work, nearest the
  // notch; on the right, how many wait for the person (orange if one is
  // blocked).
  if (live?.sessions?.working) {
    const node = indicator("sessions-work", "dots", () => [workingDots(), document.createElement("b")]);
    node.lastChild.textContent = String(live.sessions.working);
    node.style.color = "var(--text)";
    left.push(node);
  }
  // The context connector: Claude Code read the memory in the last few
  // minutes; the eye with how many items today. A click opens Who read what.
  if (live?.reads) {
    const node = countIndicator("reads", READ_PATH, live.reads.count, "var(--text)");
    node.title = live.reads.label;
    node.setAttribute("aria-label", live.reads.label);
    left.push(node);
  }
  const right = [];
  if (live?.todo) {
    const color = live.todo.overdue ? "var(--warn)" : "var(--text)";
    right.push(countIndicator("todo", TODO_PATH, live.todo.count, color));
  }
  if (live?.sessions?.needs) {
    const color = live.sessions.blocked ? "var(--warn)" : "var(--text)";
    right.push(countIndicator("sessions-needs", SESSION_PATH, live.sessions.needs, color));
  }
  if (cards.right) right.push(cardIndicator("card-right", cards.right));
  // Forget what went, so it pops in again when it comes back.
  const kept = new Set([...left, ...right]);
  for (const [key, node] of indicators) if (!kept.has(node)) indicators.delete(key);
  return { left: placeWing("left", left), right: placeWing("right", right) };
}

// Puts a side's indicators in order and returns how far that side widens.
function placeWing(side, nodes) {
  const wing = el(`wing-${side}`);
  const same = nodes.length === wing.children.length && nodes.every((node, i) => wing.children[i] === node);
  if (!same) wing.replaceChildren(...nodes);
  wing.classList.toggle("on", nodes.length > 0);
  return nodes.length ? Math.ceil(wing.offsetWidth) + WING_OUTER + WING_INNER : 0;
}

const glowListening = listen("glow", ({ payload }) => {
  const root = document.documentElement;
  const shown = root.classList.contains("shown");
  fitNotch(payload.notch);
  isOpen = false;
  island.classList.remove("open");
  const base = closed.height ? closed : NO_NOTCH;
  const wings = showWings(payload.kinds, payload.live);
  // Widen out of the notch; an island already showing just adjusts.
  if (!shown) {
    width.jump(base.width);
    height.jump(base.height);
    shift.jump(0);
    paint();
    lastFrame = 0;
  }
  root.classList.add("shown");
  root.classList.toggle("glow", payload.kinds.length > 0);
  const total = base.width + wings.left + wings.right;
  const offset = (wings.right - wings.left) / 2;
  animate(total, base.height, OPEN, offset);
  fitWindow(base.height + GLOW_ROOM, true);
  // Where it ends up in the window, so a click there reaches it.
  const center = innerWidth / 2 + offset;
  invoke("island_shape", { left: center - total / 2, right: center + total / 2, bottom: base.height });
});

// The pointer is over the closed island (hover.rs): it swells a little to
// say it can be clicked. Not CSS :hover, which would stay on once clicks go
// through again.
listen("hover", ({ payload: over }) => {
  island.classList.toggle("hover", over && !isOpen);
  // A click may follow: have the Sessions tab ready.
  if (over && !isOpen) loadSessions();
});

// A click on the closed island: the indicator nearest the click, if it is
// close, says where the notch opens; elsewhere it opens as a double tap does.
// A click reaches the closed page only over the island (hover.rs), and has
// made Freebyrd the active app, so it always opens something.
const CLICK_SLOP = 12;
island.addEventListener("click", (event) => {
  if (isOpen) return;
  let nearest = null;
  let gap = CLICK_SLOP;
  for (const node of island.querySelectorAll(".wing.on .chip")) {
    const box = node.getBoundingClientRect();
    const away = Math.max(box.left - event.clientX, event.clientX - box.right, 0);
    if (away <= gap) {
      gap = away;
      nearest = node;
    }
  }
  island.classList.remove("hover");
  invoke("open_island", { panel: nearest?.dataset.panel ?? null });
});

// Nothing waits or is live and main.rs hides the window: back to the notch,
// so the next showing widens out of it again.
listen("unglow", () => {
  document.documentElement.classList.remove("shown", "glow");
  island.classList.remove("hover");
  width.jump(closed.width);
  height.jump(closed.height);
  shift.jump(0);
  paint();
});

// The window is in place over the notch: grow the island out of it, or out
// of the closed island if it was showing.
listen("open", ({ payload: notch }) => {
  const root = document.documentElement;
  const shown = root.classList.contains("shown");
  root.classList.remove("shown", "glow");
  island.classList.remove("hover");
  fitNotch(notch);
  if (!shown) {
    width.jump(closed.width);
    height.jump(closed.height);
    shift.jump(0);
    paint();
  }
  // A loop left from before the window hid must not take one long step.
  lastFrame = 0;
  isOpen = true;
  island.classList.add("open");
  contentHeight = measure();
  openingHeight = contentHeight;
  animate(OPEN_WIDTH, contentHeight, OPEN);
  question.focus({ preventScroll: true });
});

// Closing: fold back into the notch; main.rs hides the window after FOLD.
listen("fold", () => {
  isOpen = false;
  island.classList.remove("open");
  animate(closed.width, closed.height, FOLD);
  // Once hidden, ready the window for the next opening, so that it opens
  // without a resize. Frames stop while hidden, so this is not left to `tick`.
  setTimeout(() => {
    if (!isOpen) fitWindow(openingHeight, true);
  }, 300);
});

listen("capturing", ({ payload }) => {
  session = payload.session;
  reset(payload.appName);
});

listen("context", ({ payload }) => {
  if (payload.session === session) showContext(payload.screen);
});

listen("cards", ({ payload }) => {
  if (payload.session === session) showCards(payload.cards);
});

listen("upnext", ({ payload }) => {
  if (payload.session === session) showUpNext(payload.item);
});

// ⌘⌥ pressed and let go on their own (tap.rs): the to-do list, from
// anywhere, back where the person was in it (or night shift) within 15
// minutes. Pressed again while either shows, the notch closes.
listen("tasks", ({ payload }) => {
  const listing = !todos.hidden || (typeof nightPanel !== "undefined" && !nightPanel.hidden);
  if (!payload.opened && listing) {
    invoke("dismiss");
    return;
  }
  // Back where the person was in the list or night shift, if recent.
  if (typeof ntRestoreTasks === "function" && ntRestoreTasks()) return;
  if (typeof toggleNight === "function") toggleNight(false);
  todoTab = "mine";
  if (todos.hidden) toggleTodos(true);
  else renderTodos();
  if (typeof ntForTodos === "function") ntForTodos();
});

// The closed island was clicked on an indicator (main.rs `open_island`): the
// to-do list on To do, where a job run now leads the stack, or night shift
// on Tonight.
listen("island", ({ payload }) => {
  if (payload.panel === "night" && typeof openNight === "function") {
    openNight("tonight");
    return;
  }
  if (typeof toggleNight === "function") toggleNight(false);
  todoTab = payload.panel === "sessions" ? "sessions" : "mine";
  if (todos.hidden) toggleTodos(true);
  else renderTodos();
  if (todoTab === "sessions") refreshSessions();
  else if (typeof ntForTodos === "function") ntForTodos();
});

listen("todos", ({ payload }) => {
  if (payload.session !== session || !payload.list) return;
  todoList = payload.list;
  showBadge(todoList.badge);
  renderTodos();
});

listen("quick", ({ payload }) => {
  // Not once a question is asked: the thread has taken over.
  if (payload.session === session && thread.hidden) showQuick(payload.asks);
});

listen("recalled", ({ payload }) => {
  if (payload.session === session) showRecalled(payload.text);
});

listen("answer", ({ payload }) => {
  if (payload.session === session) onAnswer(payload.event);
});

// A glow sent before the page was listening was lost; ask for it again.
glowListening.then(() => invoke("page_ready"));
