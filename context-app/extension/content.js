// Runs in every http(s) page. Extracts visible text only when the service
// worker asks, and tells it when a single-page app changes its content.

const MAX_TEXT_CHARS = 200_000;
// What a window shows at once; the desktop side cuts it to less.
const MAX_VIEW_CHARS = 20_000;
const MIN_MAIN_CHARS = 500;
const CHANGE_DEBOUNCE_MS = 2_000;

// Never content: non-rendered elements, form choices, hidden decorations,
// and UI containers (menus, toolbars, tab strips).
const SKIP_SELECTOR = [
  "script", "style", "noscript", "template", "select", "[aria-hidden='true']",
  "[role='menu']", "[role='menubar']", "[role='toolbar']", "[role='tablist']", "[role='tooltip']",
].join(", ");

// Controls. A small one ("Add icon", "Save draft") is a label and is skipped;
// a large one is a clickable container around content (Notion makes whole
// database rows `role="button"`) and is kept.
const CONTROL_SELECTOR = [
  "button", "[role='button']", "[role='menuitem']", "[role='tab']",
  "[role='radio']", "[role='checkbox']", "[role='switch']", "[role='option']",
].join(", ");
const MAX_LABEL_CHILDREN = 3;
const MAX_LABEL_CHARS = 60;

/**
 * Visible text under `root`, one line per block element, with inline pieces
 * (links, bold spans) joined into their sentence. Skips controls, hidden and
 * fully transparent elements (hover-only buttons), and form field values,
 * which are not text nodes. Given an array, `inView` also gets the lines
 * with some text inside the browser's viewport.
 */
function visibleText(root, inView = null) {
  const displays = new Map();
  const visibility = new Map();
  const display = (el) => {
    if (!displays.has(el)) displays.set(el, getComputedStyle(el).display);
    return displays.get(el);
  };
  const isVisible = (el) => {
    if (!visibility.has(el)) {
      visibility.set(el, el.checkVisibility({ opacityProperty: true, visibilityProperty: true }));
    }
    return visibility.get(el);
  };
  const labels = new Map();
  const isLabel = (control) => {
    if (!labels.has(control)) {
      labels.set(
        control,
        control.childElementCount <= MAX_LABEL_CHILDREN &&
          control.textContent.trim().length <= MAX_LABEL_CHARS &&
          // A link inside is content, like a URL property in a database cell.
          !control.querySelector("a[href]"),
      );
    }
    return labels.get(control);
  };
  const insideLabel = (el) => {
    for (let c = el.closest(CONTROL_SELECTOR); c; c = c.parentElement?.closest(CONTROL_SELECTOR)) {
      if (isLabel(c)) return true;
    }
    return false;
  };
  const blockOf = (el) => {
    let e = el;
    while (e !== root && e.parentElement && (display(e).startsWith("inline") || display(e) === "contents")) {
      e = e.parentElement;
    }
    return e;
  };

  const range = inView ? document.createRange() : null;
  const inViewport = (textNode) => {
    range.selectNodeContents(textNode);
    const r = range.getBoundingClientRect();
    return r.width > 0 && r.height > 0 && r.bottom > 0 && r.right > 0 &&
      r.top < innerHeight && r.left < innerWidth;
  };

  const lines = [];
  let block = null;
  let current = "";
  let currentInView = false;
  const flush = () => {
    const line = current.replace(/\s+/g, " ").trim();
    if (line) {
      lines.push(line);
      if (currentInView) inView.push(line);
    }
    current = "";
    currentInView = false;
  };

  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (node.nodeType === Node.ELEMENT_NODE) {
      if (node.tagName === "BR") flush();
      continue;
    }
    const el = node.parentElement;
    if (!el || el.closest(SKIP_SELECTOR) || !isVisible(el) || insideLabel(el)) continue;
    const nodeBlock = blockOf(el);
    if (nodeBlock !== block) {
      flush();
      block = nodeBlock;
    }
    current += node.nodeValue;
    if (inView && !currentInView) currentInView = inViewport(node);
  }
  flush();
  return lines.join("\n");
}

// Gmail: the open thread as a conversation (its subject, and each message's
// sender, their address, who it went to, time and text), so a promise can be
// traced to who made it, the person's own messages told apart, and a forward
// known by who it went to (memory::progress, the person's ask, 2026-10-14). Gmail's markup is obfuscated, but these
// class names have held for years; if they change, the thread is still read
// as plain text below.
function gmailConversation(root = document, host = location.hostname) {
  if (host !== "mail.google.com") return null;
  const subject = root.querySelector("h2.hP")?.innerText.trim();
  if (!subject) return null;
  // Gmail names the account in the tab title: "Subject - me@example.com - Gmail".
  const account = document.title.match(/[\w.+-]+@[\w-]+(?:\.[\w-]+)+/)?.[0]?.toLowerCase() ?? null;
  const messages = [];
  for (const node of root.querySelectorAll("div.adn")) {
    const from = node.querySelector(".gD[email]");
    const email = from?.getAttribute("email")?.toLowerCase() ?? null;
    const sender = from?.getAttribute("name") || from?.textContent.trim() || email || "someone";
    const date = node.querySelector(".g3");
    const time = date?.getAttribute("title") || date?.textContent.trim() || null;
    const body = node.querySelector(".a3s");
    const text = body ? visibleText(body).slice(0, 5_000) : "";
    if (!text) continue;
    // The address goes along so the sender's page joins their name in
    // other apps (people.rs); the person's own is left out.
    const me = Boolean(account && email === account);
    messages.push({ sender, email: me ? null : email, me, to: gmailRecipients(node, from, account), time, text });
  }
  if (!messages.length) return null;
  // The thread's ID ends the address: #inbox/FMfcgz...
  const key = location.hash.split("/").pop() || subject;
  return { app: "gmail", key, title: subject, messages };
}

// Who a message went to: the "to" and "cc" names in its header (Gmail marks
// each with its address), leaving out the sender and the person themselves.
const MAX_RECIPIENTS = 10;

function gmailRecipients(node, from, account) {
  const seen = new Set();
  const to = [];
  const chips = node.querySelectorAll(".hb [email], .g2[email]");
  for (const chip of chips) {
    if (chip === from) continue;
    const email = chip.getAttribute("email")?.toLowerCase();
    if (!email || email === account || seen.has(email)) continue;
    seen.add(email);
    const name = chip.getAttribute("name") || chip.textContent.trim() || null;
    to.push({ name: name && name !== email ? name : null, email });
    if (to.length === MAX_RECIPIENTS) break;
  }
  return to;
}

// AI chats: the person's own last message, so Freebyrd can bring back what
// they saved at the moment it helps (memory::moments). Only their message,
// never the assistant's or anyone else's; on reads the desktop side asks for.
// ChatGPT (2026-10-13): the person's live conversation page marks each of
// their messages with data-user-message-bubble; its bundle also knows
// data-message-role, and older pages data-message-author-role.
const CHAT_ASK_SELECTORS = {
  "chatgpt.com": '[data-user-message-bubble], [data-message-role="user"], [data-message-author-role="user"]',
  "claude.ai": '[data-testid="user-message"]',
  "gemini.google.com": "user-query .query-text, user-query",
};
// The person's last few messages (their choice, 2026-10-13: the last one
// alone missed what they had asked for earlier in the chat), newest first,
// so a reader that cuts the text short keeps the latest, joined by an ASCII
// record separator so a message's own blank lines stay inside it. Then a
// group separator and how many they have sent in the chat, and when that is
// more than the last few, another and their first message: Freebyrd weighs
// a chat at its start, by where the first message says it is headed (their
// idea, 2026-10-13), and knows the chat by it.
const ASK_MESSAGES = 3;
const MAX_ASK_CHARS = 600;
const ASK_SEPARATOR = "\u001e";
const FIRST_SEPARATOR = "\u001d";

function chatAsk(root = document, host = location.hostname) {
  const site = Object.keys(CHAT_ASK_SELECTORS).find((name) => host === name || host.endsWith(`.${name}`));
  if (!site) return null;
  // One message can match twice (a turn and its bubble): keep the innermost.
  const matched = [...root.querySelectorAll(CHAT_ASK_SELECTORS[site])];
  const mine = matched.filter((el) => !matched.some((other) => other !== el && el.contains(other)));
  const text = (el) =>
    visibleText(el).trim().replaceAll(ASK_SEPARATOR, " ").replaceAll(FIRST_SEPARATOR, " ").slice(0, MAX_ASK_CHARS);
  const texts = mine.slice(-ASK_MESSAGES).reverse().map(text).filter(Boolean);
  if (!texts.length) return null;
  let ask = texts.join(ASK_SEPARATOR) + FIRST_SEPARATOR + mine.length;
  if (mine.length > ASK_MESSAGES) ask += FIRST_SEPARATOR + text(mine[0]);
  return ask;
}

// `view`: also say which lines are in the viewport, for the desktop side's
// question about what the person sees.
function extractPage({ view = false } = {}) {
  // Prefer the page's main region when it holds real content; nav bars and
  // footers are noise for a context assistant.
  const main = document.querySelector("main, [role='main'], article");
  let inView = view ? [] : null;
  let text = main ? visibleText(main, inView) : "";
  if (text.length < MIN_MAIN_CHARS) {
    inView = view ? [] : null;
    text = visibleText(document.body, inView);
  }

  const headings = [...document.querySelectorAll("h1, h2, h3")]
    .map((h) => ({ level: Number(h.tagName[1]), text: h.innerText.trim() }))
    .filter((h) => h.text)
    .slice(0, 100);

  return {
    url: location.href,
    title: document.title,
    lang: document.documentElement.lang || null,
    text: text.slice(0, MAX_TEXT_CHARS),
    truncated: text.length > MAX_TEXT_CHARS,
    headings,
    selection: String(getSelection() ?? "").trim().slice(0, 5_000),
    conversation: gmailConversation(),
    ...(inView ? { inView: inView.join("\n").slice(0, MAX_VIEW_CHARS) } : {}),
    ...(view ? { ask: chatAsk() } : {}),
  };
}

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message?.type === "extract") sendResponse(extractPage({ view: Boolean(message.view) }));
});

function notifyChanged() {
  try {
    chrome.runtime.sendMessage({ type: "pageChanged" }).catch(() => {});
  } catch {
    // The extension was reloaded or removed; this copy of the script is orphaned.
  }
}

// Changes in a hidden page (background window, covered window) are held
// until the person can see the page again.
let changeTimer = null;
let changedWhileHidden = false;
new MutationObserver(() => {
  clearTimeout(changeTimer);
  changeTimer = setTimeout(() => {
    if (document.visibilityState === "visible") notifyChanged();
    else changedWhileHidden = true;
  }, CHANGE_DEBOUNCE_MS);
}).observe(document.documentElement, { childList: true, subtree: true, characterData: true });

document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible" && changedWhileHidden) {
    changedWhileHidden = false;
    notifyChanged();
  }
});
