// The pairing page, on the Mac only (`companion::pair`): the code, where the
// iPad can reach this Mac, what it may see, and the paired iPads. Device
// names come from the iPad: set as text.

"use strict";

const $ = (id) => document.getElementById(id);
let state = null;

function el(tag, className, ...children) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  for (const child of children) if (child !== null && child !== undefined) node.append(child);
  return node;
}

async function load() {
  const response = await fetch("/pair/state");
  if (!response.ok) throw new Error((await response.json().catch(() => ({}))).error || "The pairing page could not load.");
  state = await response.json();
  paint();
}

async function change(action, value) {
  $("status").textContent = "";
  try {
    const response = await fetch("/pair/settings", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Freebyrd": "pair" },
      body: JSON.stringify({ action, value }),
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(body.error || "That did not work.");
    state = body;
    paint();
  } catch (err) {
    $("status").textContent = err.message;
    $("status").classList.add("error");
  }
}

function copyable(text) {
  const code = el("code", "", text);
  const copy = el("button", "btn small", "Copy");
  copy.type = "button";
  copy.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(text);
      copy.textContent = "Copied";
    } catch {
      copy.textContent = "Select it and copy";
    }
    setTimeout(() => (copy.textContent = "Copy"), 1500);
  });
  return el("div", "address", code, copy);
}

function row(title, text, end) {
  return el("div", "setting", el("div", "", el("b", "", title), text ? el("small", "", text) : null), end ?? null);
}

function toggle(checked, onChange, label) {
  const box = el("input", "switch");
  box.type = "checkbox";
  box.setAttribute("aria-label", label);
  box.checked = checked;
  box.addEventListener("change", () => onChange(box.checked));
  return box;
}

function paint() {
  $("code").textContent = state.code;
  const minutes = Math.max(0, Math.round((state.expiresAt - Date.now()) / 60000));
  $("expires").textContent = minutes ? `Expires in ${minutes} min.` : "Expired. Make a new one.";

  // Where to go on the iPad.
  const ts = state.tailscale ?? {};
  const addresses = [];
  if (ts.url && ts.serving) addresses.push(el("p", "quiet small", "Through Tailscale (encrypted, works anywhere):"), copyable(ts.url));
  if (state.settings.lan) {
    addresses.push(el("p", "quiet small", "On this Wi-Fi:"), ...state.lan.map(copyable));
  }
  if (!addresses.length) addresses.push(el("p", "quiet small", "Turn on one of the ways below first."));
  $("addresses").replaceChildren(...addresses);

  // Tailscale and the Wi-Fi.
  const routes = [];
  if (!ts.installed) {
    const link = el("a", "", "tailscale.com/download");
    link.href = "https://tailscale.com/download";
    link.target = "_blank";
    routes.push(row("Tailscale: not installed", "Recommended. Free for personal use. Install it on this Mac and on the iPad, sign in to the same account on both, then reload this page. It gives the iPad an encrypted HTTPS address that works away from home, and lets it show notifications.", null), el("p", "small", link));
  } else if (!ts.serving) {
    routes.push(row("Tailscale: installed", "One step left: let Tailscale pass HTTPS on to Freebyrd. Run this once in Terminal, then reload this page:"), copyable(ts.command));
  } else {
    routes.push(row("Tailscale: on", `The iPad can use ${ts.url} from anywhere. Notifications work there.`));
  }
  routes.push(
    row(
      "This Wi-Fi",
      "Plain HTTP on this network: quick to start, but anyone on the same Wi-Fi could read what travels, and the iPad can't show notifications. Fine at home.",
      toggle(state.settings.lan, (on) => change("lan", on), "Answer on this Wi-Fi"),
    ),
  );
  $("routes").replaceChildren(...routes);

  // What the iPad may see.
  const shown = state.settings.shown;
  const names = [
    ["ask", "Ask", "Answers from your memory, as the notch gives them"],
    ["people", "People", "A page per person, with what you owe each other"],
    ["meetings", "Meetings", "Coming events, agendas, and Fireflies summaries"],
    ["work", "Work board", "Today's Claude Code sessions, with your first and last asks"],
    ["reading", "Reading shelf", "Links you shared from the iPad"],
  ];
  $("shown").replaceChildren(
    ...names.map(([key, title, text]) =>
      row(title, text, toggle(shown[key] !== false, (on) => change("shown", { ...shown, [key]: on }), `Show ${title} on the iPad`)),
    ),
  );

  // What the iPad may do.
  $("may-do").replaceChildren(
    row(
      "Start Claude Code sessions",
      "A task typed on the iPad starts a session in the Claude app's Code tab on this Mac, in a project you trust, where you see and steer it. Whoever holds the iPad's key could do the same; over the Wi-Fi that key travels unencrypted, so this is safest with Tailscale.",
      toggle(Boolean(state.settings.claudeCode), (on) => change("claude_code", on), "Let the iPad start Claude Code sessions"),
    ),
  );

  // Paired iPads.
  const devices = state.devices.map((device) => {
    const remove = el("button", "btn small", "Unpair");
    remove.type = "button";
    remove.addEventListener("click", () => change("remove", device.id));
    const seen = new Date(device.lastSeenAt);
    return row(device.name, `Paired ${new Date(device.pairedAt).toLocaleDateString()}, last seen ${seen.toLocaleString()}${device.notifications ? " · notifications on" : ""}`, remove);
  });
  $("devices").replaceChildren(...(devices.length ? devices : [row("None yet", "Pair one with the code above.")]));
}

$("new-code").addEventListener("click", () => change("code"));
load().catch((err) => {
  $("status").textContent = err.message;
  $("status").classList.add("error");
});
// The code runs out; keep what's shown current.
setInterval(() => load().catch(() => {}), 30000);
