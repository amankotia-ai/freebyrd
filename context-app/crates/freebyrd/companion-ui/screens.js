// Freebyrd for iPad: the screens. Each is `SCREENS[name](root, arg, alive)`:
// it fills `root`, and checks `alive()` after waiting on the Mac, since the
// person may have moved on. Everything from the Mac is set as text.

"use strict";

function head(title, sub, ...actions) {
  const text = el("div", "", el("h1", "", title), sub ? el("p", "", sub) : null);
  const row = actions.filter(Boolean);
  return el("header", "head", text, row.length ? el("div", "actions", ...row) : null);
}

const label = (text, className = "") => el("h2", className ? `label ${className}` : "label", text);
const empty = (text) => el("p", "empty", text);

function tile(path, state = "") {
  return el("span", state ? `tile ${state}` : "tile", icon(path));
}

// One line: a tile, a title over a quiet line, and something on the right.
function line({ tile: t, title, sub, end, onClick }) {
  const node = el(onClick ? "button" : "div", "line", t, el("span", "line-text", el("b", "", title), sub ? el("small", "", sub) : null));
  if (end) node.append(typeof end === "string" ? el("span", "row-end", end) : end);
  if (onClick) {
    node.type = "button";
    node.addEventListener("click", onClick);
  }
  return node;
}

// Time-of-day words for the head.
function partOfDay(hour) {
  if (hour < 4) return "Late night";
  if (hour < 12) return "Morning";
  if (hour < 17) return "Afternoon";
  if (hour < 21) return "Evening";
  return "Night";
}

const NIGHT_ICON = { preview: P.film, final: P.film, list: P.list };

function nightRow(job) {
  const state = job.state === "failed" ? "error" : job.needs ? "warn" : job.state === "running" ? "night" : "";
  return line({
    tile: tile(NIGHT_ICON[job.kind] ?? P.list, state),
    title: job.title,
    sub: job.sub,
    end: job.end,
    onClick: () => go(`#night/${job.id}`),
  });
}

// Marks cards seen once shown, as opening the notch does; the island's
// glow then stops.
function markSeen(cards) {
  const ids = cards.filter((card) => !card.seenAt).map((card) => card.id);
  if (ids.length) call("seen", { ids }).then(glance).catch(() => {});
}

// ---------- Today ----------

SCREENS.today = async (root, _arg, alive) => {
  const { data: t, stale } = await load("today");
  if (!alive()) return;
  const bits = [];
  const waiting = t.cards.filter((card) => !card.seenAt).length;
  if (waiting) bits.push(waiting === 1 ? "1 card waiting" : `${waiting} cards waiting`);
  if (t.badge?.count) bits.push(t.badge.count === 1 ? "1 to-do due" : `${t.badge.count} to-dos due`);
  if (t.night?.waiting?.length) bits.push(`night shift left ${t.night.waiting.length}`);
  root.append(head(t.date, `${partOfDay(t.hour)}. ${bits.length ? bits.join(" · ") : "Nothing is waiting on you."}`, button("Ask", "primary", () => go("#ask"))));
  if (stale) root.append(staleBanner(stale));
  if (t.pausedUntil) {
    root.append(
      el("div", "banner", el("span", "dot"), `Your Mac isn't watching the screen until ${clock(new Date(t.pausedUntil))}.`, button("Resume", "small", async () => {
        await call("resume").catch((err) => toast(err.message));
        glance();
        refresh();
      })),
    );
  }

  const columns = el("div", "columns");
  const left = el("div", "stack");
  const right = el("div", "stack");
  columns.append(left, right);
  root.append(columns);

  if (t.upNext) left.append(label("Up next"), upNextCard(t.upNext));
  const night = t.night;
  const jobs = [...(night?.now ?? []), ...(night?.waiting ?? [])];
  if (jobs.length) {
    left.append(label(night.needs ? "While you were away · needs you" : "While you were away", night.needs ? "warn" : ""), el("div", "list", ...jobs.slice(0, 5).map(nightRow)));
  }
  if (t.cards.length) {
    left.append(label("Cards"), ...t.cards.slice(0, 6).map((card) => cardElement(card)));
    if (t.cards.length > 6) left.append(button(`All ${t.cards.length} cards`, "quietly", () => go("#cards")));
    markSeen(t.cards.slice(0, 6));
  }
  if (!t.upNext && !jobs.length && !t.cards.length) left.append(label("Waiting"), empty("Nothing needs you right now. New cards, night shift results and what's due show up here."));

  right.append(label("Today"));
  if (t.blocks.length) {
    right.append(dayList(t.blocks, t.planLine), button("Open the day", "quietly", () => go("#day")));
  } else {
    right.append(empty("No day plan yet. Freebyrd can lay out the rest of your day inside your usual hours, with breaks and time off."), button("Plan my day", "", () => go("#day/plan")));
  }
  if (t.recap) {
    const recap = el("article", "card", el("header", "", kindLabel(KINDS.recap, `Recap · ${t.recap.day}`)), el("div", "card-body", markdown(t.recap.markdown)));
    right.append(label("Last recap"), recap);
  }
};

// Up next: the one promise, task or event that most needs the person.
function upNextCard(item) {
  const kind = kindOf(item.kind === "event" ? "event" : "due");
  const header = el("header", "", kindLabel(kind, "Up next", true), el("span", item.overdue ? "overdue" : "", item.when));
  if (item.more) header.append(el("span", "faint", `${item.more} more`));
  const node = el("article", "card", header, el("p", "", el("strong", "", item.title)), item.place ? el("p", "quiet small", item.place) : null, el("p", "quiet small", `From ${item.source}`));
  node.querySelector("p").style.margin = "6px 0 0";
  const row = el("div", "actions");
  const note = status();
  item.actions.forEach((action, index) => {
    row.append(
      // Only an ask leads: an event with nothing to ask has Hide alone.
      button(action.label, index === 0 && action.id === "ask" ? "primary" : "", async () => {
        if (action.id === "ask") {
          askAbout(item.question, `${action.label}: ${item.title}`, `plan:${item.id}`);
          return;
        }
        try {
          const next = await call("up_next_action", { id: item.id, action: action.id });
          node.replaceWith(next ? upNextCard(next) : empty("Nothing else is up next."));
          glance();
        } catch (err) {
          showError(note, err);
        }
      }),
    );
  });
  row.append(note);
  node.append(row);
  return node;
}

// A day plan's blocks as lines: the time, what for, and whether it's on
// the calendar.
function dayList(blocks, planLine) {
  const box = el("div", "list");
  if (planLine) box.append(el("p", "quiet small", planLine));
  const now = new Date();
  for (const block of blocks) {
    const data = block.data ?? {};
    const kind = cardKind(block);
    const added = !block.actions?.some((action) => action.id === "add");
    const start = data.time ? new Date(`${data.date}T${data.time}`) : null;
    const end = start && data.minutes ? new Date(start.getTime() + data.minutes * 60000) : null;
    const isNow = start && end && start <= now && now < end;
    const t = tile(kind.path);
    t.querySelector(".icon").style.fill = kind.color;
    box.append(
      line({
        tile: t,
        title: data.title ?? block.title,
        sub: `${block.title}${added ? " · on your calendar" : ""}${isNow ? " · now" : ""}`,
      }),
    );
  }
  return box;
}

// ---------- Cards ----------

SCREENS.cards = async (root, _arg, alive) => {
  const { data: cards, stale } = await load("cards");
  if (!alive()) return;
  const waiting = cards.filter((card) => !card.seenAt).length;
  root.classList.add("wide");
  root.append(head("Cards", cards.length ? `${cards.length} from the last few days${waiting ? ` · ${waiting} new` : ""}` : "Nothing waiting."));
  if (stale) root.append(staleBanner(stale));
  if (!cards.length) {
    root.append(empty("When Freebyrd catches a promise, drafts something before it's due, finishes a night shift job or has a question, it shows up here, as it does in the notch."));
    return;
  }
  const left = cards.filter((card) => kindOf(card.kind).side === "left");
  const right = cards.filter((card) => kindOf(card.kind).side !== "left");
  const columns = el("div", "columns");
  columns.append(
    el("div", "stack", label("Needs you"), ...(right.length ? right.map((card) => cardElement(card)) : [empty("Nothing needs you.")])),
    el("div", "stack", label("What happened"), ...(left.length ? left.map((card) => cardElement(card)) : [empty("Nothing new happened.")])),
  );
  root.append(columns);
  markSeen(cards);
};

// ---------- To-do ----------

let todoTab = "mine";

SCREENS.todo = async (root, _arg, alive) => {
  const { data, stale } = await load("todos");
  if (!alive()) return;
  let list = data;
  let night = null;
  const body = el("div", "");
  const counts = () => ({ mine: list.badge.open, waiting: list.waiting.length, done: list.done.length });
  const tabs = el("div", "tabs");
  const paint = () => {
    const c = counts();
    tabs.replaceChildren(
      ...[["mine", "To do"], ["waiting", "Waiting on"], ["done", "Done"]].map(([id, text]) => {
        const tab = el("button", "tab", text, c[id] ? el("span", id === "mine" && list.badge.overdue ? "count warn" : "count", String(c[id])) : null);
        tab.type = "button";
        tab.setAttribute("aria-selected", String(todoTab === id));
        tab.addEventListener("click", () => {
          todoTab = id;
          paint();
        });
        return tab;
      }),
    );
    body.replaceChildren(...todoBody(list, night, onList));
  };
  const onList = (next) => {
    list = next;
    keep.set("freebyrd.cache.todos", JSON.stringify({ at: Date.now(), data: list }));
    paint();
  };
  root.append(head("To-do", `${list.badge.open} open${list.badge.count ? ` · ${list.badge.count} due` : ""}`, tabs));
  if (stale) root.append(staleBanner(stale));
  root.append(body);
  paint();
  // What Freebyrd can do for these now (night shift's picks), above the list.
  try {
    night = await call("night");
    if (alive()) paint();
  } catch {}
};

function todoBody(list, night, onList) {
  const parts = [];
  if (todoTab === "mine") {
    const stack = smartStack(night);
    if (stack) parts.push(stack);
    parts.push(addTodoForm(onList));
    for (const group of list.mine) {
      parts.push(label(group.label, group.label === "Overdue" ? "warn" : ""), el("div", "list", ...group.items.map((item) => todoRow(item, { onList }))));
    }
    if (!list.mine.length) parts.push(empty("Nothing on your list. Type one above; Freebyrd also adds your promises, Notion tasks and meeting action items as it finds them."));
  } else if (todoTab === "waiting") {
    parts.push(el("div", "list", ...list.waiting.map((item) => todoRow(item, { onList }))));
    if (!list.waiting.length) parts.push(empty("No one owes you anything Freebyrd knows of."));
  } else {
    parts.push(el("div", "list", ...list.done.map((item) => todoRow(item, { onList }))));
    if (!list.done.length) parts.push(empty("Nothing finished in the last two weeks."));
  }
  return parts;
}

function addTodoForm(onList) {
  const input = el("input", "field");
  input.placeholder = "Add a to-do. A day in it sets the date";
  input.setAttribute("aria-label", "Add a to-do");
  input.enterKeyHint = "done";
  const note = status();
  const form = el("form", "", input, note);
  form.style.margin = "4px 0 6px";
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    input.disabled = true;
    try {
      onList(await call("add_todo", { text }));
      toast("Added");
      glance();
    } catch (err) {
      input.disabled = false;
      showError(note, err);
    }
  });
  return form;
}

// Jobs started now, and tonight's picks: Run in Claude Code (a session on
// the Mac), Tonight, Not now.
function smartStack(night) {
  const items = [...(night?.now ?? []), ...(night?.suggested ?? [])];
  if (!items.length) return null;
  const box = el("section", "card", el("header", "", kindLabel({ path: P.spark, color: "#a5a3ff" }, "Freebyrd can do now", true)));
  for (const job of items.slice(0, 3)) {
    const note = status();
    const row = el("div", "actions");
    if (job.state === "suggested") {
      row.append(
        button("Run in Claude Code", "primary small", async () => {
          note.textContent = "Opening Claude Code on your Mac…";
          try {
            await call("night_action", { id: job.id, action: "code", value: null });
            note.textContent = "Started on your Mac. Watch and steer it there, or from the Claude app.";
          } catch (err) {
            showError(note, err);
          }
        }),
        button("Tonight", "small", async () => {
          try {
            await call("night_action", { id: job.id, action: "add", value: null });
            note.textContent = "Queued for tonight";
          } catch (err) {
            showError(note, err);
          }
        }),
        button("Not now", "small", async () => {
          try {
            await call("night_action", { id: job.id, action: "skip", value: null });
            wrap.remove();
          } catch (err) {
            showError(note, err);
          }
        }),
      );
    } else if (job.state === "queued" || job.state === "running") {
      note.textContent = job.state === "running" ? `Running · ${job.end}` : "Starting…";
    } else {
      row.append(button("Open", "primary small", () => go(`#night/${job.id}`)));
    }
    row.append(note);
    const wrap = el("div", "", nightRow(job), row);
    wrap.style.marginTop = "6px";
    box.append(wrap);
  }
  return box;
}

// ---------- People ----------

SCREENS.people = async (root, arg, alive) => {
  if (arg) return personScreen(root, Number(arg), alive);
  const { data: list, stale } = await load("todos");
  if (!alive()) return;
  const people = list.people ?? [];
  root.append(head("People", "Joined from WhatsApp, Gmail and your meetings. Those who need you first."));
  if (stale) root.append(staleBanner(stale));
  if (!people.length) {
    root.append(empty("No one yet. Freebyrd builds a page for each person from WhatsApp, Gmail and Fireflies as it reads them."));
    return;
  }
  root.append(
    el(
      "div",
      "list",
      ...people.map((person) => {
        const node = el("button", "line", el("span", "line-text", el("b", "", person.name)), el("span", person.needsYou ? "row-end overdue" : "row-end", person.detail));
        node.type = "button";
        node.addEventListener("click", () => go(`#people/${person.id}`));
        return node;
      }),
    ),
  );
};

async function personScreen(root, id, alive) {
  let page = await call("person_page", { id });
  if (!alive()) return;
  const paint = () => {
    root.replaceChildren(...personPage(page, async (action, value, note) => {
      try {
        page = await call("person_action", { id: page.id, action, value });
        if (page.id !== id) go(`#people/${page.id}`);
        else paint();
      } catch (err) {
        if (note) showError(note, err);
        else toast(err.message);
      }
    }));
  };
  paint();
}

function personPage(page, act) {
  const parts = [];
  const work = page.work === true ? "work" : page.work === false ? "personal" : null;
  parts.push(
    button([icon(P.back), el("span", "", "People")], "quietly small", () => go("#people")),
    head(page.name, [page.org, work, ...page.places].filter(Boolean).join(" · "), button("Brief me", "primary", () => askAbout(page.question, `Brief me: ${page.name}`, `person:${page.id}`))),
  );
  parts[1].style.marginTop = "14px";
  if (page.last) parts.push(el("p", "quiet", page.last));
  const note = el("textarea", "area");
  note.placeholder = "Your note about them. It goes with questions about them.";
  note.value = page.note ?? "";
  note.style.minHeight = "70px";
  note.addEventListener("change", () => act("note", note.value));
  parts.push(note);
  const section = (title, rows) => {
    if (rows.length) parts.push(label(title), el("div", "list", ...rows));
  };
  section("Waiting on you", page.waiting.map((w) => line({ tile: tile(P.chat, "warn"), title: w.place, sub: `${w.when} · ${w.text}` })));
  section("You owe them", page.owe.map((item) => todoRow(item, { without: page.id })));
  section("They owe you", page.owed.map((item) => todoRow(item, { without: page.id })));
  section("Lately", page.recent.map((r) => line({ tile: tile(P.calendar), title: r.what, sub: r.when })));
  const note2 = status();
  if (page.aliases.length > 1) {
    parts.push(
      label("Also seen as"),
      el(
        "div",
        "chips",
        ...page.aliases.map((alias) => {
          const chip = el("button", "chip", alias.shown, icon(P.dismiss));
          chip.type = "button";
          chip.title = "Not the same person: make this a page of its own";
          chip.querySelector(".icon").style.cssText = "width:14px;height:14px;fill:var(--text-3)";
          chip.addEventListener("click", () => act("split", alias.alias, note2));
          return chip;
        }),
      ),
    );
  }
  section(
    "Maybe also",
    page.maybe.map((maybe) => {
      const row = line({ tile: tile(P.people), title: maybe.shown, sub: maybe.place });
      row.append(button("Same person", "small", () => act("claim", maybe.alias, note2)));
      return row;
    }),
  );
  const row = el("div", "actions");
  if (page.others.length) {
    const merge = el("select", "btn");
    merge.append(new Option("Same person as…", ""));
    for (const other of page.others) merge.append(new Option(other.name, String(other.id)));
    merge.addEventListener("change", () => merge.value && act("merge", merge.value, note2));
    row.append(merge);
  }
  row.append(button(page.hidden ? "Is a person" : "Not a person", "", () => act(page.hidden ? "show" : "hide", null, note2)), note2);
  parts.push(row);
  return parts;
}

// ---------- Day plan ----------

SCREENS.day = async (root, arg, alive) => {
  const { data, stale } = await load("day");
  if (!alive()) return;
  const note = status();
  const change = el("input", "field");
  change.placeholder = "Change it: lighter, gym at 19:00, more time for the deck…";
  change.enterKeyHint = "go";
  const plan = async (text) => {
    planButton.disabled = true;
    note.classList.remove("error");
    note.textContent = text ? "Changing the plan…" : "Planning your day… This takes about half a minute.";
    try {
      const made = await call("plan_day", { note: text || null });
      if (!alive()) return;
      paint(made.cards, made.balance, made.note);
      form.hidden = false;
      planButton.textContent = "Plan again";
      note.textContent = "";
      change.value = "";
    } catch (err) {
      showError(note, err);
    }
    planButton.disabled = false;
  };
  const planButton = button(data.blocks.length ? "Plan again" : "Plan my day", "primary", () => plan(null));
  root.append(head("Day plan", data.plan?.balance ?? "The rest of your day as blocks, inside your usual hours", planButton));
  if (stale) root.append(staleBanner(stale));
  const form = el("form", "", change);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    if (change.value.trim()) plan(change.value.trim());
  });
  form.hidden = !data.blocks.length;
  root.append(note, form);
  const blocks = el("div", "stack");
  blocks.style.marginTop = "14px";
  root.append(blocks, el("p", "faint small", "Freebyrd can't see your other calendars yet, so check the times against your meetings. It plans at most 6 hours of work, with meals, breaks and time off."));
  const paint = (cards, balance, planNote) => {
    blocks.replaceChildren();
    if (planNote) blocks.append(el("p", "", planNote));
    if (balance) blocks.append(el("p", "quiet small", balance));
    if (!cards.length) blocks.append(empty("No plan for today yet."));
    blocks.append(...cards.map((card) => cardElement(card)));
  };
  paint(data.blocks, null, data.plan?.note);
  if (arg === "plan" && !data.blocks.length) plan(null);
};

// ---------- Ask ----------

// The conversation stays while the app is open, across screens.
const askState = { thread: null, turns: [], busy: false };

const QUICK_ASKS = [
  "What's due this week?",
  "What did I promise people lately?",
  "Catch me up on yesterday",
  "Who is waiting on me?",
];

SCREENS.ask = async (root) => {
  const thread = el("div", "thread");
  const input = el("input", "");
  input.placeholder = "Ask about your work";
  input.setAttribute("aria-label", "Question");
  input.enterKeyHint = "send";
  const send = el("button", "send", icon(P.send));
  send.type = "submit";
  send.setAttribute("aria-label", "Ask");
  const bar = el("form", "ask-bar", input, send);
  const quick = el(
    "div",
    "chips",
    ...QUICK_ASKS.map((text) => {
      const chip = el("button", "chip", text);
      chip.type = "button";
      chip.addEventListener("click", () => ask(text, text, null));
      return chip;
    }),
  );
  const composer = el("div", "composer", quick, bar);
  const sync = () => {
    send.disabled = askState.busy || !input.value.trim();
    quick.hidden = askState.turns.length > 0;
  };
  input.addEventListener("input", sync);
  bar.addEventListener("submit", (event) => {
    event.preventDefault();
    const text = input.value.trim();
    if (!text || askState.busy) return;
    input.value = "";
    ask(text, text, null);
  });
  root.append(
    head(
      "Ask",
      "Answers from your Mac's memory: your work, meetings, people and what's due. It can't see your Mac's screen.",
      button("New conversation", "quietly", () => {
        if (askState.busy) return;
        askState.thread = null;
        askState.turns = [];
        thread.replaceChildren();
        sync();
      }),
    ),
    thread,
    composer,
  );
  for (const turn of askState.turns) thread.append(turnNode(turn));
  sync();

  async function ask(question, shown, focus) {
    if (askState.busy) return;
    askState.busy = true;
    const turn = { shown, text: "", notes: [], error: null, done: false };
    askState.turns.push(turn);
    const node = turnNode(turn);
    thread.append(node);
    sync();
    node.scrollIntoView({ block: "start", behavior: "smooth" });
    const repaint = () => {
      const fresh = turnNode(turn);
      node.replaceChildren(...fresh.childNodes);
    };
    try {
      const started = await call("ask", { question, focus, thread: askState.thread });
      askState.thread = started.thread;
      let from = 0;
      let misses = 0;
      while (true) {
        let answer;
        try {
          answer = await call("ask_poll", { id: started.id, from });
          misses = 0;
        } catch (err) {
          if (++misses > 3) throw err;
          await new Promise((resolve) => setTimeout(resolve, 1500));
          continue;
        }
        for (const event of answer.events) {
          if (event.kind === "text") turn.text += event.text;
          else if (event.kind === "note") turn.notes.push(event.message);
          else if (event.kind === "error") turn.error = event.message;
          else if (event.kind === "refused") turn.error = "Claude declined to answer that.";
        }
        from = answer.next;
        if (answer.done) break;
        repaint();
      }
    } catch (err) {
      turn.error = err.message;
    }
    turn.done = true;
    askState.busy = false;
    repaint();
    sync();
  }

  function turnNode(turn) {
    const reply = el("div", turn.done ? "reply" : "reply streaming");
    if (turn.text) {
      reply.append(markdown(turn.text, true, turn.done ? (rework) => ask(rework, rework, null) : null));
    } else if (!turn.done) {
      reply.append(el("p", "pending", turn.notes.length ? turn.notes[turn.notes.length - 1] : "Thinking…"));
    }
    for (const message of turn.notes) if (turn.text || turn.done) reply.append(el("p", "note-line", message));
    if (turn.error) reply.append(el("p", "error", turn.error));
    return el("div", "turn", el("p", "asked", turn.shown), reply);
  }

  if (pendingAsk) {
    const { question, shown, focus } = pendingAsk;
    pendingAsk = null;
    if (askState.busy) toast("Wait for this answer, then ask again.");
    else {
      askState.thread = null;
      askState.turns = [];
      thread.replaceChildren();
      ask(question, shown, focus);
    }
  } else {
    setTimeout(() => input.focus({ preventScroll: true }), 100);
  }
};

// ---------- Capture ----------

SCREENS.capture = async (root) => {
  const text = el("textarea", "area");
  text.placeholder = "Type it, tap the microphone on the keyboard to say it, or write it with Apple Pencil.";
  text.setAttribute("aria-label", "What to capture");
  const note = status();
  const take = async (what) => {
    const value = text.value.trim();
    if (!value) {
      text.focus();
      return;
    }
    note.classList.remove("error");
    try {
      if (what === "todo") {
        await call("add_todo", { text: value });
        toast("On your to-do list");
        glance();
      } else if (what === "night") {
        note.textContent = "Planning it from your work…";
        await call("night_add", { text: value });
        toast("Queued for tonight's night shift");
        note.textContent = "";
      }
      text.value = "";
    } catch (err) {
      showError(note, err);
    }
  };
  root.append(
    head("Capture", "Say it, write it, or photograph it. Freebyrd puts it where it belongs."),
    text,
    el(
      "div",
      "actions",
      button("Add to to-do", "primary", () => take("todo")),
      button("Ask Freebyrd", "", () => text.value.trim() && askAbout(text.value.trim())),
      button("Run tonight", "", () => take("night")),
      button("Send to Claude Code", "", () => {
        if (!text.value.trim()) {
          text.focus();
          return;
        }
        pendingCodeTask = text.value.trim();
        go("#work");
      }),
      note,
    ),
    el("p", "faint small", "A day in a to-do sets its date (“by Friday”), and a name Freebyrd knows becomes who it's for. Run tonight hands it to night shift, which plans it from your work."),
  );

  // A photo of notes, read into lines to add.
  const file = el("input", "");
  file.type = "file";
  file.accept = "image/*";
  file.hidden = true;
  const preview = el("img", "card-thumb");
  preview.hidden = true;
  preview.alt = "";
  preview.style.maxHeight = "260px";
  preview.style.objectFit = "contain";
  const found = el("div", "list");
  const photoNote = status();
  file.addEventListener("change", async () => {
    const picked = file.files?.[0];
    file.value = "";
    if (!picked) return;
    found.replaceChildren();
    photoNote.classList.remove("error");
    photoNote.textContent = "Reading it…";
    try {
      const jpeg = await shrink(picked, 1600);
      preview.src = jpeg;
      preview.hidden = false;
      const read = await call("capture_photo", { image: jpeg });
      photoNote.textContent = read.items.length ? "Pick what to add." : "No notes could be read in that photo.";
      const boxes = read.items.map((item) => {
        const box = el("input", "");
        box.type = "checkbox";
        box.checked = true;
        const text = el("input", "field");
        text.value = item;
        text.style.height = "38px";
        found.append(el("label", "pick", box, text));
        return [box, text];
      });
      if (boxes.length) {
        found.append(
          el(
            "div",
            "actions",
            button("Add to to-do", "primary", async () => {
              let added = 0;
              for (const [box, text] of boxes) {
                if (!box.checked || !text.value.trim()) continue;
                try {
                  await call("add_todo", { text: text.value.trim() });
                  added++;
                  box.checked = false;
                } catch (err) {
                  showError(photoNote, err);
                  return;
                }
              }
              toast(added === 1 ? "Added 1 to-do" : `Added ${added} to-dos`);
              glance();
            }),
          ),
        );
      }
    } catch (err) {
      showError(photoNote, err);
    }
  });
  root.append(
    label("From a photo"),
    el("p", "quiet small", "A whiteboard, a notebook page, a sticky note. The photo goes to your Mac, then to Claude to read it; it isn't kept."),
    file,
    el("div", "actions", button([icon(P.camera), el("span", "", "Take or choose a photo")], "", () => file.click()), photoNote),
    preview,
    found,
  );
};

// A photo as a JPEG no wider or taller than `max`.
async function shrink(file, max) {
  const bitmap = await createImageBitmap(file);
  const scale = Math.min(1, max / Math.max(bitmap.width, bitmap.height));
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(bitmap.width * scale);
  canvas.height = Math.round(bitmap.height * scale);
  canvas.getContext("2d").drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  return canvas.toDataURL("image/jpeg", 0.82);
}

// ---------- Sketch ----------

SCREENS.sketch = async (root, _arg, alive) => {
  const canvas = el("canvas", "");
  canvas.setAttribute("aria-label", "Sketch pad. Draw with Apple Pencil or a finger.");
  const pad = el("div", "pad", canvas);
  const strokes = [];
  let ink = "#ffffff";
  let width = 3;
  let penSeen = false;
  let drawing = null;
  const ctx = canvas.getContext("2d");
  const size = () => {
    const ratio = window.devicePixelRatio || 1;
    const box = canvas.getBoundingClientRect();
    canvas.width = Math.round(box.width * ratio);
    canvas.height = Math.round(box.height * ratio);
    ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
    redraw();
  };
  const segment = (stroke, a, b) => {
    ctx.strokeStyle = stroke.ink;
    ctx.lineCap = "round";
    ctx.lineJoin = "round";
    ctx.lineWidth = stroke.width * (stroke.pen ? 0.35 + b.p * 1.5 : 1) * (stroke.ink === "erase" ? 6 : 1);
    if (stroke.ink === "erase") ctx.strokeStyle = "#0d0d0e";
    ctx.beginPath();
    ctx.moveTo(a.x, a.y);
    ctx.lineTo(b.x, b.y);
    ctx.stroke();
  };
  const redraw = () => {
    ctx.fillStyle = "#0d0d0e";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    for (const stroke of strokes) for (let i = 1; i < stroke.points.length; i++) segment(stroke, stroke.points[i - 1], stroke.points[i]);
  };
  const point = (event) => {
    const box = canvas.getBoundingClientRect();
    return { x: event.clientX - box.left, y: event.clientY - box.top, p: event.pressure || 0.5 };
  };
  canvas.addEventListener("pointerdown", (event) => {
    if (event.pointerType === "pen") penSeen = true;
    // With the Pencil in use, a resting palm draws nothing.
    if (penSeen && event.pointerType === "touch") return;
    event.preventDefault();
    canvas.setPointerCapture(event.pointerId);
    drawing = { ink, width, pen: event.pointerType === "pen", points: [point(event)] };
    strokes.push(drawing);
  });
  canvas.addEventListener("pointermove", (event) => {
    if (!drawing) return;
    const events = event.getCoalescedEvents ? event.getCoalescedEvents() : [event];
    for (const one of events) {
      const next = point(one);
      segment(drawing, drawing.points[drawing.points.length - 1], next);
      drawing.points.push(next);
    }
  });
  const stop = () => (drawing = null);
  canvas.addEventListener("pointerup", stop);
  canvas.addEventListener("pointercancel", stop);

  const inks = [["#ffffff", "White"], ["#8e8e93", "Grey"], ["erase", "Eraser"]].map(([value, name]) => {
    const swatch = el("button", "ink");
    swatch.type = "button";
    swatch.style.setProperty("--ink", value === "erase" ? "repeating-linear-gradient(45deg,#3a3a3c 0 4px,#1c1c1e 4px 8px)" : value);
    swatch.setAttribute("aria-label", name);
    swatch.setAttribute("aria-pressed", String(value === ink));
    swatch.addEventListener("click", () => {
      ink = value;
      for (const other of inks) other.setAttribute("aria-pressed", String(other === swatch));
    });
    return swatch;
  });
  const widths = [[2, "Fine"], [3, "Medium"], [6, "Bold"]].map(([value, name]) => {
    const tab = el("button", "tab", name);
    tab.type = "button";
    tab.setAttribute("aria-selected", String(value === width));
    tab.addEventListener("click", () => {
      width = value;
      for (const other of widths) other.setAttribute("aria-selected", String(other === tab));
    });
    return tab;
  });
  const note = status();
  const gallery = el("div", "sketches");
  const save = button("Save to Mac", "primary", async () => {
    if (!strokes.length) return;
    save.disabled = true;
    try {
      await call("sketch_save", { image: canvas.toDataURL("image/png") });
      toast("Saved on your Mac");
      strokes.length = 0;
      redraw();
      showGallery();
    } catch (err) {
      showError(note, err);
    }
    save.disabled = false;
  });
  root.append(
    head("Sketch", "Draw with Apple Pencil. Sketches are kept on your Mac, to open there or use as a reference."),
    pad,
    el(
      "div",
      "pad-tools",
      ...inks,
      el("span", "", " "),
      el("div", "tabs", ...widths),
      button([icon(P.undo), el("span", "", "Undo")], "small", () => {
        strokes.pop();
        redraw();
      }),
      button("Clear", "small", () => {
        strokes.length = 0;
        redraw();
      }),
      el("span", "", note),
      save,
    ),
    label("Saved"),
    gallery,
  );
  requestAnimationFrame(size);
  const observer = new ResizeObserver(() => size());
  observer.observe(canvas);

  async function showGallery() {
    try {
      const list = await call("sketches");
      if (!alive()) return;
      gallery.replaceChildren(...(list.length ? [] : [empty("Nothing saved yet.")]));
      for (const sketch of list.slice(0, 24)) {
        const img = el("img", "");
        img.alt = "";
        mediaUrl(sketch.url).then((url) => (img.src = url)).catch(() => {});
        const when = new Date(sketch.at);
        gallery.append(
          el(
            "div",
            "sketch",
            img,
            el("span", "faint small", `${when.toLocaleDateString(undefined, { day: "numeric", month: "short" })}, ${clock(when)}`),
            el(
              "div",
              "chips",
              button("Show on Mac", "small", () => call("sketch_open", { name: sketch.name }).then(() => toast("Opened on your Mac")).catch((err) => toast(err.message))),
              button("Delete", "small quietly", async () => {
                await call("sketch_delete", { name: sketch.name }).catch((err) => toast(err.message));
                showGallery();
              }),
            ),
          ),
        );
      }
    } catch (err) {
      gallery.replaceChildren(empty(err.message));
    }
  }
  showGallery();
};

// ---------- Reading ----------

// Links shared from the iPad. X bookmarks were here too until the X
// suggestions were removed (2026-10-16).
let readingFilter = "unread";

SCREENS.reading = async (root, _arg, alive) => {
  const { data: shelf, stale } = await load("reading");
  if (!alive()) return;
  const items = shelf.items;
  const filters = [
    ["unread", "To read", (item) => !item.read],
    ["done", "Done", (item) => item.read],
  ];
  const tabs = el("div", "tabs");
  const grid = el("div", "shelf");
  let shown = 40;
  const paint = () => {
    tabs.replaceChildren(
      ...filters.map(([id, text, test]) => {
        const count = items.filter(test).length;
        const tab = el("button", "tab", text, count ? el("span", "count", String(count)) : null);
        tab.type = "button";
        tab.setAttribute("aria-selected", String(readingFilter === id));
        tab.addEventListener("click", () => {
          readingFilter = id;
          shown = 40;
          paint();
        });
        return tab;
      }),
    );
    const test = filters.find(([id]) => id === readingFilter)[2];
    const picked = items.filter(test);
    grid.replaceChildren(...picked.slice(0, shown).map((item) => book(item, paint)));
    if (!picked.length) grid.append(empty(readingFilter === "unread" ? "Share a page from any app with your Send to Freebyrd shortcut (Settings shows how), and it lands here." : "Nothing here."));
    more.hidden = picked.length <= shown;
  };
  const more = button("Show more", "quietly", () => {
    shown += 40;
    paint();
  });
  root.classList.add("wide");
  root.append(head("Reading", "What you shared from here", tabs));
  if (stale) root.append(staleBanner(stale));
  root.append(grid, more);
  paint();
};

function book(item, paint) {
  const node = el("article", item.read ? "book read" : "book");
  const body = el("div", "body");
  body.append(el("h3", "", item.title));
  body.append(el("span", "by", ["Shared from this iPad", ago(item.at)].join(" · ")));
  if (item.text) body.append(el("p", "what", item.text));
  const row = el("div", "actions");
  if (item.url) {
    const open = el("a", "btn small", "Open");
    open.href = item.url;
    open.target = "_blank";
    open.rel = "noopener noreferrer";
    row.append(open);
  }
  row.append(
    button(item.read ? "Back to read" : "Done", "small quietly", async () => {
      try {
        await call("reading_done", { id: item.id, undo: item.read });
        item.read = !item.read;
        paint();
      } catch (err) {
        toast(err.message);
      }
    }),
  );
  body.append(row);
  node.append(body);
  return node;
}

// ---------- Meetings ----------

const MEETING_QUESTION =
  "Go through the meeting under “Meeting” at the top of the memory: what was decided, what is still open, and who owes what, me first.";

SCREENS.meetings = async (root, _arg, alive) => {
  const { data, stale } = await load("meetings");
  if (!alive()) return;
  root.append(head("Meetings", "What's coming up, and what was said in the last ones"));
  if (stale) root.append(staleBanner(stale));
  const people = (chips) =>
    chips.length
      ? el(
          "div",
          "chips",
          ...chips.map((chip) => {
            const node = el("button", "chip", chip.name);
            node.type = "button";
            node.addEventListener("click", () => go(`#people/${chip.id}`));
            return node;
          }),
        )
      : null;
  root.append(label("Coming up"));
  if (!data.upcoming.length) root.append(empty("No events in the next week that Freebyrd knows of. It sees what lands on its own calendar and what it catches on screen."));
  const coming = el("div", "stack");
  root.append(coming);
  for (const event of data.upcoming) {
    const card = el(
      "article",
      "card",
      el("header", "", kindLabel(KINDS.event, event.onCalendar ? "On your calendar" : "Seen, not added"), el("span", "", event.when)),
      el("p", "", el("strong", "", event.title)),
      people(event.people),
    );
    card.children[1].style.margin = "6px 0 8px";
    if (event.agenda) {
      const agenda = el("p", "", event.agenda);
      agenda.style.whiteSpace = "pre-wrap";
      card.append(el("div", "card-body", ...clamped(agenda, 5)));
    }
    // The button that fits the event (plans::event_ask): Prep me for a
    // meeting, Trip details for travel, none for a birthday or a dinner.
    const ask = event.ask ? button(event.ask.label, "primary small", () => askAbout(event.ask.question, `${event.ask.label}: ${event.title}`, event.focus)) : null;
    card.append(el("div", "actions", ask, el("span", "faint small", `From ${event.source}`)));
    coming.append(card);
  }
  root.append(label("Recent meetings"));
  if (!data.recent.length) root.append(empty("No meetings from Fireflies yet."));
  const list = el("div", "stack");
  for (const meeting of data.recent) {
    const summary = el("p", "", meeting.summary);
    summary.style.whiteSpace = "pre-wrap";
    const counts = meeting.items ? `${meeting.items} action items${meeting.mine ? `, ${meeting.mine} yours` : ""}` : "";
    const card = el(
      "article",
      "card",
      el("header", "", kindLabel({ path: P.video, color: "#ffffff" }, meeting.when), counts ? el("span", meeting.mine ? "overdue" : "", counts) : null),
      el("p", "", el("strong", "", meeting.title)),
      people(meeting.people),
      el("div", "card-body", ...clamped(summary, 3)),
      el("div", "actions", button("What was decided?", "primary small", () => askAbout(MEETING_QUESTION, `What was decided: ${meeting.title}`, meeting.focus))),
    );
    card.children[1].style.margin = "6px 0 8px";
    list.append(card);
  }
  root.append(list);
};

// ---------- Work board ----------

SCREENS.work = async (root, _arg, alive) => {
  const { data, stale } = await load("work");
  if (!alive()) return;
  root.classList.add("wide");
  const sessions = data.sessions;
  const waiting = sessions.filter((s) => s.state === "waiting" || s.state === "blocked");
  const running = sessions.filter((s) => s.state === "running");
  const done = sessions.filter((s) => s.state === "done");
  root.append(head("Work board", `Claude Code since ${data.since}: ${sessions.length} sessions`, button("Refresh", "quietly", refresh)));
  if (stale) root.append(staleBanner(stale));
  const composer = await codeComposer(alive);
  if (!alive()) return;
  if (composer) root.append(composer);
  for (const overlap of data.overlaps) {
    root.append(el("div", "banner", el("span", "dot"), el("span", "", el("strong", "", `${overlap.sessions.length} sessions changed ${overlap.file} in the last two hours: `), overlap.sessions.join("; "), ". Let one finish before the other goes on.")));
  }
  const lane = (title, list, text) => el("section", "lane", el("h2", "", el("span", "", title), el("span", "", String(list.length))), ...(list.length ? list.map(sessionCard) : [empty(text)]));
  root.append(
    el(
      "div",
      "lanes",
      lane("Needs you", waiting, "No session needs you."),
      lane("Running", running, "Nothing running right now."),
      lane("Done today", done, "Nothing finished yet today."),
    ),
    el("p", "faint small", "To steer a session from the iPad, open it in the Claude app with Remote Control. Copy resume gives the command to pick it up in a terminal on the Mac."),
  );
  const timer = setInterval(() => {
    if (!alive()) clearInterval(timer);
    else if (document.visibilityState === "visible" && !typing(root)) refresh();
  }, 45000);
};

// Whether the person is typing on this screen, so it isn't re-read under them.
function typing(root) {
  const field = root.querySelector("textarea");
  return Boolean(field && (field.value.trim() || document.activeElement === field));
}

// A task for Claude Code: typed here, started on the Mac in the Claude
// app's Code tab, in a project the person trusts (`companion::code`).
let pendingCodeTask = null;

async function codeComposer(alive) {
  let info;
  try {
    info = await call("code_projects");
  } catch {
    return null;
  }
  if (!alive()) return null;
  const box = el("section", "card", el("header", "", kindLabel({ path: P.term, color: "#ffffff" }, "New task for Claude Code")));
  box.style.marginBottom = "18px";
  if (!info.allowed) {
    box.append(el("p", "quiet small", "Off. To start Claude Code sessions from the iPad, turn on Start Claude Code sessions on your Mac: Freebyrd's menu-bar icon, Pair an iPad."));
    return box;
  }
  if (!info.projects.length) {
    box.append(el("p", "quiet small", "No project to start in yet. Open a project once in the Claude app's Code tab on your Mac, and it shows up here."));
    return box;
  }
  let folder = keep.get("freebyrd.codeFolder");
  if (!info.projects.some((project) => project.folder === folder)) folder = info.projects[0].folder;
  const tabs = el("div", "tabs");
  tabs.style.marginTop = "6px";
  const paintTabs = () =>
    tabs.replaceChildren(
      ...info.projects.map((project) => {
        const tab = el("button", "tab", project.name);
        tab.type = "button";
        tab.title = project.folder;
        tab.setAttribute("aria-selected", String(project.folder === folder));
        tab.addEventListener("click", () => {
          folder = project.folder;
          keep.set("freebyrd.codeFolder", folder);
          paintTabs();
        });
        return tab;
      }),
    );
  paintTabs();
  const text = el("textarea", "area");
  text.placeholder = "What should Claude Code do? It starts on your Mac, in the Code tab, where you can watch and steer it.";
  text.setAttribute("aria-label", "Task for Claude Code");
  text.style.minHeight = "96px";
  text.style.marginTop = "10px";
  if (pendingCodeTask) {
    text.value = pendingCodeTask;
    pendingCodeTask = null;
  }
  const note = status();
  const list = el("div", "list");
  const paintRecent = (recent) =>
    list.replaceChildren(
      ...recent.slice(0, 5).map((handoff) => {
        const state = handoff.state === "started" ? "ok" : handoff.state === "waiting" ? "warn" : "";
        const path = handoff.state === "started" ? P.circleCheck : handoff.state === "waiting" ? P.wait : P.term;
        return line({ tile: tile(path, state), title: handoff.task, sub: `${handoff.project} \u00b7 ${handoff.note}`, end: ago(handoff.at) });
      }),
    );
  paintRecent(info.recent);
  // Follows what was sent until the Mac says whether it started.
  const follow = () => {
    let tries = 0;
    const timer = setInterval(async () => {
      if (!alive() || ++tries > 25) {
        clearInterval(timer);
        return;
      }
      try {
        const now = await call("code_recent");
        paintRecent(now.recent);
        if (!now.recent.some((handoff) => handoff.state === "opening")) clearInterval(timer);
      } catch {}
    }, 2000);
  };
  const send = button("Send to Claude Code", "primary", async () => {
    const task = text.value.trim();
    if (!task) {
      text.focus();
      return;
    }
    send.disabled = true;
    note.classList.remove("error");
    note.textContent = "Sending to your Mac…";
    try {
      const sent = await call("code_send", { folder, task });
      text.value = "";
      note.textContent = "";
      paintRecent(sent.recent);
      follow();
    } catch (err) {
      showError(note, err);
    }
    send.disabled = false;
  });
  if (info.recent.some((handoff) => handoff.state === "opening")) follow();
  box.append(tabs, text, el("div", "actions", send, note), list);
  return box;
}

function sessionCard(session) {
  const minutes = Math.round((Date.now() - session.lastAt) / 60000);
  const node = el("article", "session", el("span", "project", session.project), el("span", "title", session.title || session.firstAsk || "Untitled session"));
  if (session.lastAsk && session.lastAsk !== session.title) node.append(el("span", "ask", `Last: ${session.lastAsk}`));
  // What the session is doing, in words (`sessions.rs`): "Asked you a
  // question", "Waiting on 3 agents", "Done 4 min ago".
  if (session.label) node.append(el("span", session.state === "blocked" ? "state-line warn" : "state-line", session.label));
  else if (session.state === "blocked") node.append(el("span", "state-line warn", `Stopped for ${minutes} min: a question, a permission prompt, or a long command`));
  else if (session.state === "waiting") node.append(el("span", "state-line", `Claude finished ${ago(session.lastAt)}; your turn`));
  if (session.state === "running") node.append(el("div", "live", el("i", "")));
  const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
  node.append(el("span", "meta", [ago(session.lastAt), session.asks ? `${plural(session.asks, "ask", "asks")} today` : null, session.files.length ? `${plural(session.files.length, "file", "files")} changed` : null].filter(Boolean).join(" · ")));
  if (session.files.length) node.append(...revealer("Files", "Hide files", el("pre", "box", session.files.join("\n"))));
  if (session.resume) node.append(el("div", "chips", copyButton(() => session.resume, "Copy resume")));
  return node;
}

// ---------- Night shift ----------

SCREENS.night = async (root, arg, alive) => {
  if (arg && arg !== "tonight") return nightJob(root, Number(arg), alive);
  const { data: view, stale } = await load("night");
  if (!alive()) return;
  let tab = arg === "tonight" ? "tonight" : view.waiting.length || view.now.length ? "waiting" : "tonight";
  const note = status();
  const start = view.tonight.length && !view.running
    ? button("Start now", "primary", async () => {
        try {
          await call("night_action", { id: 0, action: "start", value: null });
          toast("Night shift started on your Mac");
          setTimeout(refresh, 1500);
        } catch (err) {
          showError(note, err);
        }
      })
    : null;
  root.append(head("Night shift", view.running ? "Running now on your Mac" : view.starts, start));
  if (stale) root.append(staleBanner(stale));
  root.append(note);
  const tabs = el("div", "tabs");
  const body = el("div", "");
  const paint = () => {
    tabs.replaceChildren(
      ...[["waiting", "While you were away", view.needs || view.waiting.length + view.now.length], ["tonight", "Tonight", view.tonight.length]].map(([id, text, count]) => {
        const node = el("button", "tab", text, count ? el("span", id === "waiting" && view.needs ? "count warn" : "count", String(count)) : null);
        node.type = "button";
        node.setAttribute("aria-selected", String(tab === id));
        node.addEventListener("click", () => {
          tab = id;
          paint();
        });
        return node;
      }),
    );
    body.replaceChildren(...(tab === "waiting" ? nightWaiting(view) : nightTonight(view, (next) => {
      Object.assign(view, next);
      paint();
    })));
  };
  root.append(tabs, body);
  paint();
};

function nightWaiting(view) {
  const jobs = [...view.now, ...view.waiting];
  if (!jobs.length) return [empty("Nothing from last night. What runs tonight waits here when you're back.")];
  const needs = jobs.filter((job) => job.needs);
  const ready = jobs.filter((job) => !job.needs);
  const parts = [];
  if (view.ran) parts.push(el("p", "quiet small", view.ran));
  if (needs.length) parts.push(label("Needs you", "warn"), el("div", "list", ...needs.map(nightRow)));
  if (ready.length) parts.push(label("Ready to look at"), el("div", "list", ...ready.map(nightRow)));
  return parts;
}

function nightTonight(view, onView) {
  const parts = [];
  const input = el("input", "field");
  input.placeholder = "What should run tonight? A film to preview, or a list of tasks";
  input.enterKeyHint = "go";
  const note = status();
  const form = el("form", "", input, note);
  form.style.margin = "12px 0";
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    const text = input.value.trim();
    if (!text) return;
    input.disabled = true;
    note.textContent = "Planning it from your work…";
    try {
      onView(await call("night_add", { text }));
      toast("Queued for tonight");
    } catch (err) {
      input.disabled = false;
      showError(note, err);
    }
  });
  parts.push(form);
  if (view.suggested.length) {
    parts.push(
      label("Picked from your work"),
      el(
        "div",
        "list",
        ...view.suggested.map((job) => {
          const row = nightRow(job);
          const actions = el(
            "div",
            "actions",
            button("Add", "primary small", async () => onView(await call("night_action", { id: job.id, action: "add", value: null }).catch((err) => (toast(err.message), view)))),
            button("Not tonight", "small", async () => onView(await call("night_action", { id: job.id, action: "skip", value: null }).catch((err) => (toast(err.message), view)))),
          );
          actions.style.margin = "0 0 8px 56px";
          return el("div", "", row, actions);
        }),
      ),
    );
  }
  parts.push(label("Tonight"));
  if (!view.tonight.length) parts.push(empty("Nothing queued. Type above what should run tonight: a preview of a film (every 4th frame) or a list of tasks to work through."));
  for (const job of view.tonight) {
    const row = nightRow(job);
    if (job.state === "queued") {
      const remove = button("Take off", "small quietly", async () => onView(await call("night_action", { id: job.id, action: "remove", value: null }).catch((err) => (toast(err.message), view))));
      row.append(remove);
    }
    parts.push(row);
  }
  return parts;
}

async function nightJob(root, id, alive) {
  const page = await call("night_job", { id });
  if (!alive()) return;
  const result = page.result ?? {};
  const note = status();
  const done = async (action, value = null) => {
    note.classList.remove("error");
    note.textContent = { keep: "Merging…", approve: "Queuing the final…" }[action] ?? "";
    try {
      await call("night_action", { id: page.id, action, value });
      toast({ approve: "The final renders tonight", keep: "Kept: merged into your branch", discard: "Discarded", dismiss: "Put away", redo: "Queued again for tonight" }[action] ?? "Done");
      go("#night");
      glance();
    } catch (err) {
      showError(note, err);
    }
  };
  const sub = [page.kind === "preview" ? `Preview, every ${result.step ?? 4}th frame` : page.kind === "final" ? "Final render" : `Keep going · ${result.branch ?? "its own branch"}`, page.took, result.frames ? `${result.frames} frames` : null].filter(Boolean).join(" · ");
  root.append(button([icon(P.back), el("span", "", "Night shift")], "quietly small", () => go("#night")), head(page.title, sub));
  root.children[1].style.marginTop = "14px";

  if (page.kind === "preview" || page.kind === "final") {
    if (page.reelUrl) {
      const video = el("video", "reel");
      video.controls = true;
      video.playsInline = true;
      video.preload = "metadata";
      const shots = result.shots ?? [];
      if (shots[0]?.thumb) video.poster = shots[0].thumb;
      root.append(video, el("p", "faint small", "Loading the reel from your Mac…"));
      mediaUrl(page.reelUrl)
        .then((url) => {
          video.src = url;
          video.nextElementSibling?.remove();
        })
        .catch((err) => (video.nextElementSibling.textContent = err.message));
    }
    const frames = el("div", "frames");
    for (const shot of result.shots ?? []) {
      const state = shot.state === "error" ? "error" : shot.state === "warn" ? "warn" : "";
      const frame = el("div", state ? `frame ${state}` : "frame");
      if (shot.thumb) {
        const img = el("img", "");
        img.alt = "";
        img.src = shot.thumb;
        frame.append(img);
      }
      frame.append(el("small", "", shot.name));
      frame.title = shot.note || shot.name;
      frames.append(frame);
    }
    if (frames.childElementCount) root.append(frames);
  } else if (page.kind === "list") {
    const items = result.items ?? [];
    const row = (item) => {
      const state = item.state === "ok" ? "ok" : item.state === "warn" ? "warn" : "error";
      const path = { ok: P.circleCheck, warn: P.wait, error: P.error }[state];
      return line({ tile: tile(path, state), title: item.text, sub: item.note, end: item.commit ? (item.files === 1 ? "1 file" : `${item.files} files`) : null });
    };
    const needs = items.filter((item) => item.state !== "ok");
    const ok = items.filter((item) => item.state === "ok");
    if (needs.length) root.append(label("Needs you", "warn"), el("div", "list", ...needs.map(row)));
    if (ok.length) root.append(label("Done"), el("div", "list", ...ok.map(row)));
    for (const text of result.not_reached ?? []) root.append(line({ tile: tile(P.wait), title: text, sub: "Not reached" }));
  }
  const notes = el("div", "notes", ...(result.notes ?? []).map(([state, text]) => el("div", `note ${state}`, text)));
  if (notes.childElementCount) root.append(notes);

  const actions = el("div", "actions");
  if (page.kind === "preview") {
    const redo = el("input", "field");
    redo.placeholder = "What should change? It queues again for tonight";
    redo.style.flex = "1 1 280px";
    if (page.state === "done") actions.append(button("Approve, final tonight", "primary", () => done("approve")));
    actions.append(
      redo,
      button("Redo with this note", "", () => redo.value.trim() && done("redo", redo.value.trim())),
      button("Put away", "quietly", () => done("dismiss")),
    );
  } else if (page.kind === "list") {
    const ok = (result.items ?? []).some((item) => item.state === "ok");
    if (ok) actions.append(button("Keep", "primary", () => done("keep")));
    if (result.branch) actions.append(button("Discard", "", () => done("discard")));
    if (page.session && result.tree) actions.append(copyButton(() => `cd "${result.tree}" && claude --resume ${page.session}`, "Copy resume"));
    if (!ok) actions.append(button("Put away", "quietly", () => done("dismiss")));
  } else {
    actions.append(button("Put away", "primary", () => done("dismiss")));
  }
  actions.append(note);
  root.append(actions);
}

// ---------- Week ----------

SCREENS.week = async (root, _arg, alive) => {
  const { data: week, stale } = await load("week");
  if (!alive()) return;
  root.append(head("Week", week.label));
  if (stale) root.append(staleBanner(stale));
  const p = week.promises;
  root.append(
    el(
      "div",
      "tiles",
      stat(`${week.hours} h`, "with activity"),
      stat(String(week.sessions), "Claude Code sessions"),
      stat(String(week.asks), "asks to Claude"),
      stat(p.kept + p.slipped.length + p.open ? `${p.kept} of ${p.kept + p.slipped.length + p.open}` : "None", "promises kept"),
    ),
    label("Hours with activity, by day"),
    weekChart(week.days),
    el("p", "faint small", "Ten-minute stretches with something done on the Mac: Claude Code, pages, windows. The dashed line is the 6 hours a day plan allows."),
  );
  if (p.slipped.length) root.append(label("Slipped", "warn"), el("div", "list", ...p.slipped.map((title) => line({ tile: tile(P.calendar, "warn"), title }))));
  const projects = el("div", "stack");
  root.append(label("By project"), projects);
  for (const project of week.projects) {
    const node = el("article", "card", el("header", "", el("strong", "", project.name), el("span", "", `${project.sessions} sessions · ${project.asks} asks · ${project.edits} changes`)));
    if (project.done.length) node.append(el("div", "card-body", el("ul", "", ...project.done.map((done) => el("li", "", done)))));
    projects.append(node);
  }
  if (!week.projects.length) root.append(empty("No Claude Code work this week."));
  const days = week.days.filter((day) => day.headline);
  if (days.length) root.append(label("Recaps"), el("div", "list", ...days.map((day) => line({ tile: tile(KINDS.recap.path), title: day.headline, sub: day.label }))));
};

function stat(value, text) {
  return el("div", "tile-stat", el("b", "", value), el("span", "", text));
}

// Bars by day, to one scale, in white: today brightest.
function weekChart(days) {
  const NS = "http://www.w3.org/2000/svg";
  const width = 640;
  const height = 210;
  const x0 = 44;
  const x1 = width - 8;
  const top = 22;
  const base = height - 30;
  const max = Math.max(8, Math.ceil(Math.max(...days.map((d) => d.hours)) / 2) * 2);
  const y = (hours) => base - (hours / max) * (base - top);
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
  svg.setAttribute("class", "chart");
  svg.setAttribute("role", "img");
  svg.setAttribute("aria-label", `Hours with activity by day: ${days.map((d) => `${d.short} ${d.hours}`).join(", ")}`);
  svg.style.width = "100%";
  svg.style.height = "auto";
  const add = (tag, attributes, text) => {
    const node = document.createElementNS(NS, tag);
    for (const [key, value] of Object.entries(attributes)) node.setAttribute(key, value);
    if (text !== undefined) node.textContent = text;
    svg.append(node);
    return node;
  };
  for (let hours = 0; hours <= max; hours += max > 10 ? 4 : 2) {
    add("line", { x1: x0, x2: x1, y1: y(hours), y2: y(hours), stroke: "rgb(255 255 255 / 0.08)" });
    add("text", { x: x0 - 8, y: y(hours) + 4, "text-anchor": "end", "font-size": 12, fill: "rgb(255 255 255 / 0.4)" }, `${hours} h`);
  }
  add("line", { x1: x0, x2: x1, y1: y(6), y2: y(6), stroke: "rgb(255 255 255 / 0.45)", "stroke-dasharray": "4 5" });
  const slot = (x1 - x0) / days.length;
  const bar = Math.min(44, slot * 0.55);
  days.forEach((day, index) => {
    const x = x0 + index * slot + (slot - bar) / 2;
    const h = Math.max(base - y(day.hours), day.hours ? 2 : 0);
    add("rect", { x, y: base - h, width: bar, height: h, rx: 6, fill: day.today ? "rgb(255 255 255 / 0.92)" : "rgb(255 255 255 / 0.32)" });
    add("text", { x: x + bar / 2, y: base - h - 7, "text-anchor": "middle", "font-size": 12.5, "font-weight": 600, fill: "rgb(255 255 255 / 0.85)" }, day.hours ? day.hours.toFixed(1) : "");
    add("text", { x: x + bar / 2, y: base + 20, "text-anchor": "middle", "font-size": 12.5, fill: day.today ? "rgb(255 255 255 / 0.92)" : "rgb(255 255 255 / 0.5)" }, day.today ? "Today" : day.short);
  });
  return svg;
}

// ---------- Desk mode ----------

SCREENS.desk = async (root, _arg, alive) => {
  root.className = "desk";
  holdWake();
  const island = el("span", "isl desk-isl", el("span", "wing"), el("span", "says"), el("span", "wing"));
  const time = el("div", "clock", clock());
  const date = el("div", "desk-date", "");
  const nowBlock = el("div", "now-block");
  const next = el("div", "desk-next");
  const leave = button("Leave desk mode", "quietly small", () => go("#today"));
  root.append(
    island,
    el("div", "desk-mid", el("div", "", time, date), el("div", "", nowBlock, next)),
    el("div", "desk-foot", el("span", "", "The screen stays on. The island glows when something waits."), leave),
  );
  let data = null;
  const paint = () => {
    const now = new Date();
    time.textContent = clock(now);
    date.textContent = now.toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" });
    if (!data) return;
    const blocks = (data.blocks ?? []).map((block) => {
      const d = block.data ?? {};
      const start = d.time ? new Date(`${d.date}T${d.time}`) : null;
      const end = start && d.minutes ? new Date(start.getTime() + d.minutes * 60000) : null;
      return { title: d.title ?? block.title, start, end, kind: cardKind(block) };
    }).filter((block) => block.start);
    const current = blocks.find((block) => block.start <= now && block.end && now < block.end);
    nowBlock.replaceChildren(
      el("small", "", current ? `Now · until ${clock(current.end)}` : "Now"),
      el("b", "", current ? current.title : data.upNext ? data.upNext.title : "Nothing planned right now"),
    );
    if (!current && data.upNext) nowBlock.firstChild.textContent = `Up next · ${data.upNext.when}`;
    const upcoming = blocks.filter((block) => block.start > now).slice(0, 4);
    next.replaceChildren(...upcoming.map((block) => el("div", "", el("span", "", clock(block.start)), el("span", "", block.title))));
    if (!upcoming.length && !current) next.append(el("div", "", el("span", "", ""), el("span", "faint", "Plan your day from Day plan to see it here.")));
    const count = data.cards?.filter((card) => !card.seenAt).length ?? 0;
    const kinds = data.waiting ?? [];
    island.classList.toggle("glow", count > 0);
    island.style.visibility = count ? "visible" : "hidden";
    const left = kinds.map(kindOf).find((kind) => kind.side === "left");
    const right = kinds.map(kindOf).find((kind) => kind.side === "right");
    island.children[0].replaceChildren(left ? icon(left.path) : "");
    island.children[0].style.color = left?.color ?? "";
    island.children[2].replaceChildren(right ? icon(right.path) : "");
    island.children[2].style.color = right?.color ?? "";
    island.children[1].textContent = count ? `${count} waiting` : "";
  };
  const fetchData = async () => {
    try {
      data = (await load("today")).data;
    } catch {}
    if (alive()) paint();
  };
  paint();
  await fetchData();
  island.addEventListener("click", () => go("#cards"));
  const tick = setInterval(() => (alive() ? paint() : clearInterval(tick)), 10000);
  const reload = setInterval(() => (alive() ? fetchData() : clearInterval(reload)), 120000);
};

// ---------- Settings ----------

SCREENS.settings = async (root, _arg, alive) => {
  const { data: s, stale } = await load("settings");
  if (!alive()) return;
  root.append(head("Settings", `${s.device?.name ?? "This iPad"} and ${s.mac}`));
  if (stale) root.append(staleBanner(stale));
  if (!navigator.standalone && !matchMedia("(display-mode: standalone)").matches) {
    root.append(el("div", "banner", el("span", "dot"), "To keep Freebyrd on your Home Screen: in Safari, tap Share, then Add to Home Screen."));
  }

  // The Mac.
  const macNote = status();
  const paused = s.pausedUntil && s.pausedUntil > Date.now();
  const pause = (minutes) => async () => {
    try {
      await call("pause", { minutes });
      glance();
      refresh();
    } catch (err) {
      showError(macNote, err);
    }
  };
  root.append(
    label("Your Mac"),
    el(
      "div",
      "panel",
      setting(s.mac, paused ? `Not watching the screen until ${clock(new Date(s.pausedUntil))}. A double tap of Option still works.` : "Answering. Watching the screen for things to put on your calendar."),
      el(
        "div",
        "actions",
        paused ? button("Resume watching", "primary", async () => {
          await call("resume").catch((err) => showError(macNote, err));
          glance();
          refresh();
        }) : button("Pause watching for 1 hour", "", pause(60)),
        paused ? null : button("For 3 hours", "quietly", pause(180)),
        button("Start night shift now", "quietly", async () => {
          try {
            await call("night_action", { id: 0, action: "start", value: null });
            toast("Night shift started on your Mac");
          } catch (err) {
            showError(macNote, err);
          }
        }),
        macNote,
      ),
    ),
  );
  root.lastChild.lastChild.style.margin = "0 0 12px";

  // Notifications.
  root.append(label("Notifications"), await notificationsPanel(s));

  // Share and Siri.
  const inbox = `${location.origin}/api/inbox`;
  const bearer = `Bearer ${token}`;
  root.append(
    label("Share and Siri"),
    el(
      "div",
      "panel",
      el("p", "quiet small", "A web app can't add itself to the share sheet or to Siri, but a shortcut can. Make these once in the Shortcuts app on this iPad."),
      el("p", "", el("strong", "", "Send to Freebyrd"), " (share sheet: links go on the Reading shelf, text becomes a to-do)"),
      el(
        "ol",
        "steps",
        el("li", "", "New shortcut. In its details, turn on Show in Share Sheet."),
        el("li", "", "Add Get Contents of URL. URL: ", el("code", "", inbox), ". Method: POST."),
        el("li", "", "Headers: add Authorization with the value below."),
        el("li", "", "Request Body: JSON, one field named ", el("code", "", "text"), " set to Shortcut Input."),
      ),
      el("p", "", el("strong", "", "Tell Freebyrd"), " (Siri: “Hey Siri, Tell Freebyrd”)"),
      el(
        "ol",
        "steps",
        el("li", "", "New shortcut named Tell Freebyrd. Add Dictate Text."),
        el("li", "", "Add Get Contents of URL, as above, with JSON fields ", el("code", "", "text"), " set to Dictated Text and ", el("code", "", "as"), " set to ", el("code", "", "todo"), "."),
      ),
      el("div", "actions", copyButton(() => inbox, "Copy the address"), copyButton(() => bearer, "Copy the Authorization value")),
      el("p", "faint small", "The Authorization value is this iPad's key to your Mac. Keep it in these shortcuts only; unpairing this iPad turns it off."),
    ),
  );

  // What the iPad sees.
  const shown = s.shown ?? {};
  const seen = [
    ["ask", "Answers from your memory"],
    ["people", "People pages"],
    ["meetings", "Meetings"],
    ["work", "The work board"],
    ["reading", "The reading shelf"],
  ];
  root.append(
    label("Privacy"),
    el(
      "div",
      "panel",
      setting("Address", location.origin.startsWith("https:") ? "Through Tailscale: encrypted, only your devices." : "Over the Wi-Fi, without encryption. Use the Tailscale address when you can (Pair an iPad on the Mac shows it)."),
      setting("This iPad can see", seen.filter(([key]) => shown[key] !== false).map(([, text]) => text).join(", ") || "Cards and to-dos only"),
      setting("Claude Code", s.claudeCode ? "This iPad can start sessions on your Mac, in projects you trust." : "This iPad can't start sessions on your Mac. Turn it on there, in Pair an iPad."),
      setting("Never sent to the iPad", "What's on your Mac's screen, pictures of your windows, and the text of pages you read."),
      el("p", "faint small", "Change what the iPad may see on the Mac: Freebyrd's menu-bar icon, Pair an iPad."),
    ),
  );

  // Unpairing.
  let sure = false;
  const unpair = button("Unpair this iPad", "", async () => {
    if (!sure) {
      sure = true;
      unpair.textContent = "Tap again to unpair";
      return;
    }
    try {
      await call("unpair");
    } catch {}
    forget();
    showPairing();
  });
  root.append(label("This iPad"), el("div", "panel", setting(s.device?.name ?? "iPad", `Paired ${new Date(s.device?.pairedAt ?? Date.now()).toLocaleDateString()}`), el("div", "actions", unpair)));
  root.lastChild.lastChild.style.margin = "0 0 12px";
};

function setting(title, text, end = null) {
  return el("div", "setting", el("div", "", el("b", "", title), text ? el("small", "", text) : null), end);
}

async function notificationsPanel(s) {
  const panel = el("div", "panel");
  const note = status();
  const detail = el("div", "tabs");
  const paintDetail = (value) => {
    detail.replaceChildren(
      ...[["private", "Private"], ["full", "Full details"]].map(([id, text]) => {
        const tab = el("button", "tab", text);
        tab.type = "button";
        tab.setAttribute("aria-selected", String(value === id));
        tab.addEventListener("click", async () => {
          try {
            const next = await call("settings_set", { notify: id });
            paintDetail(next.notify);
          } catch (err) {
            showError(note, err);
          }
        });
        return tab;
      }),
    );
  };
  paintDetail(s.notify);
  const quiet = `None from ${pad2(s.quietFrom)}:00 to ${pad2(s.quietUntil)}:00, while you sleep; what came in arrives after.`;
  if (!window.isSecureContext || !("PushManager" in window)) {
    panel.append(
      setting("Not available on this address", window.isSecureContext ? "Add Freebyrd to your Home Screen and open it from there to turn on notifications." : "Notifications need the Tailscale address (HTTPS). Over the Wi-Fi, the app works but can't notify you."),
    );
    return panel;
  }
  const on = s.subscribed && Notification.permission === "granted";
  panel.append(
    setting(on ? "On" : "Off", on ? "Your Mac tells this iPad when a card arrives." : "Get a notification when a card arrives: a draft ready, a promise caught, night shift done."),
    setting("Lock Screen shows", "Private says how many things wait. Full details name them.", detail),
    el("p", "faint small", quiet),
  );
  const row = el("div", "actions");
  if (on) {
    row.append(
      button("Send a test", "", async () => {
        try {
          await call("push_test");
          note.textContent = "Sent. It should arrive in a few seconds.";
        } catch (err) {
          showError(note, err);
        }
      }),
      button("Turn off", "quietly", async () => {
        try {
          const registration = await navigator.serviceWorker.ready;
          await (await registration.pushManager.getSubscription())?.unsubscribe();
          await call("push_unsubscribe");
          refresh();
        } catch (err) {
          showError(note, err);
        }
      }),
    );
  } else {
    row.append(
      button("Turn on notifications", "primary", async () => {
        try {
          const permission = await Notification.requestPermission();
          if (permission !== "granted") throw new Error("Notifications were not allowed. Turn them on in Settings, Notifications, Freebyrd.");
          const { key } = await call("push_key");
          const registration = await navigator.serviceWorker.ready;
          const subscription = await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: base64Bytes(key) });
          await call("push_subscribe", { endpoint: subscription.endpoint });
          tellWorker();
          toast("Notifications are on");
          refresh();
        } catch (err) {
          showError(note, err);
        }
      }),
    );
  }
  row.append(note);
  panel.append(row);
  row.style.margin = "0 0 12px";
  return panel;
}

function base64Bytes(text) {
  const padded = text.replace(/-/g, "+").replace(/_/g, "/") + "=".repeat((4 - (text.length % 4)) % 4);
  return Uint8Array.from(atob(padded), (c) => c.charCodeAt(0));
}
