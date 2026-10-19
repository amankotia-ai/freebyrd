// Decides when to read the active tab and forwards page text to the
// Freebyrd native host over Chrome native messaging.

import { exclusionReason } from "./exclusions.js";

const HOST_NAME = "com.freebyrd.browser_host";
// Single-page apps can change constantly; re-read a changed tab at most this often.
const MIN_CHANGE_INTERVAL_MS = 10_000;

let port = null;
const lastCaptureAt = new Map();
const pendingChange = new Map();

// Which window had focus, and from when, oldest first. Chrome reports
// WINDOW_ID_NONE when the browser loses focus, and also when focus moves to
// a window this extension cannot see: an incognito window it is not allowed
// in, or another profile's window. "The last focused window" skips those, so
// an on-demand read looks here for the window in front at the moment the
// desktop app saw the browser in front.
const FOCUS_LOG_LENGTH = 20;
let focusLog = [];

function noteFocus(windowId) {
  focusLog = [...focusLog, { windowId, at: Date.now() }].slice(-FOCUS_LOG_LENGTH);
  chrome.storage.session.set({ focusLog }).catch(() => {});
}

// Chrome can stop the worker and start it again; the log survives that in
// session storage. A reload empties it, so it starts with the focus now: a
// window this extension cannot see reads as unfocused.
const focusReady = (async () => {
  const { focusLog: saved = [] } = await chrome.storage.session.get("focusLog").catch(() => ({}));
  focusLog = [...saved, ...focusLog].slice(-FOCUS_LOG_LENGTH);
  if (saved.length > 0) return;
  const window = await chrome.windows.getLastFocused().catch(() => null);
  noteFocus(window?.focused ? window.id : chrome.windows.WINDOW_ID_NONE);
})();

// The window that had focus at `atMs`: its ID, WINDOW_ID_NONE, or undefined
// when the log does not reach back that far.
function focusAt(atMs) {
  let windowId;
  for (const entry of focusLog) {
    if (entry.at > atMs) break;
    windowId = entry.windowId;
  }
  return windowId;
}

async function setStatus(patch) {
  const { status = {} } = await chrome.storage.session.get("status");
  await chrome.storage.session.set({ status: { ...status, ...patch } });
}

function connect() {
  if (port) return port;
  try {
    port = chrome.runtime.connectNative(HOST_NAME);
  } catch (error) {
    setStatus({ connected: false, hostError: String(error) });
    return null;
  }
  port.onMessage.addListener(onHostMessage);
  port.onDisconnect.addListener(() => {
    const reason = chrome.runtime.lastError?.message ?? "disconnected";
    port = null;
    setStatus({ connected: false, hostError: reason });
  });
  port.postMessage({ type: "hello", version: chrome.runtime.getManifest().version });
  return port;
}

function onHostMessage(message) {
  switch (message?.type) {
    case "welcome":
      setStatus({ connected: true, hostError: null, hostVersion: message.hostVersion });
      break;
    case "ack":
      setStatus({ lastStored: message.stored });
      break;
    case "error":
      setStatus({ hostError: message.message });
      break;
    case "readActivePage":
      answerReadRequest(message.requestId, message.frontAtMs);
      break;
    case "reloadExtension":
      // Picks up a new unpacked build without a trip to the extensions page.
      chrome.runtime.reload();
      break;
  }
}

// The desktop app asks for the page in front of the person right now, for
// example when they open the assistant. Same exclusions and pause as above.
// `frontAtMs` is when it saw the browser in front, before its own window
// took focus: the page is the active tab of the window focused then.
async function answerReadRequest(requestId, frontAtMs) {
  const reply = (fields) =>
    port?.postMessage({ type: "pageResponse", requestId, page: null, error: null, ...fields });

  let query = { active: true, lastFocusedWindow: true };
  if (frontAtMs) {
    await focusReady;
    const windowId = focusAt(frontAtMs);
    if (windowId === chrome.windows.WINDOW_ID_NONE) return reply({ error: "hidden-window" });
    if (windowId !== undefined) query = { active: true, windowId };
  }
  const [tab] = await chrome.tabs.query(query);
  if (!tab?.url) return reply({ error: "no-active-tab" });
  if (tab.incognito) return reply({ error: "incognito" });

  const { paused = false } = await chrome.storage.local.get("paused");
  if (paused) return reply({ error: "paused" });

  const excluded = exclusionReason(tab.url);
  if (excluded) return reply({ error: `excluded:${excluded}` });

  // What is in view tells the desktop side what "this" means.
  const page = await requestExtraction(tab.id, { view: true }).catch(() => null);
  if (!page) return reply({ error: "unreadable" });
  if (exclusionReason(page.url)) return reply({ error: "excluded:navigated" });

  const now = Date.now();
  lastCaptureAt.set(tab.id, now);
  reply({ page: { tabId: tab.id, reason: "requested", capturedAtMs: now, ...page } });
  await setStatus({ lastCapture: { title: page.title, url: page.url, reason: "requested", at: now } });
}

async function requestExtraction(tabId, options = {}) {
  const message = { type: "extract", ...options };
  try {
    return await chrome.tabs.sendMessage(tabId, message);
  } catch {
    // Tabs opened before the extension loaded have no content script yet.
    await chrome.scripting.executeScript({ target: { tabId }, files: ["content.js"] });
    return chrome.tabs.sendMessage(tabId, message);
  }
}

async function capture(tabId, reason) {
  const tab = await chrome.tabs.get(tabId).catch(() => null);
  if (!tab?.active || tab.incognito || tab.status !== "complete" || !tab.url) return;

  const { paused = false } = await chrome.storage.local.get("paused");
  if (paused) return;

  const excluded = exclusionReason(tab.url);
  if (excluded) {
    await setStatus({ lastSkip: { host: new URL(tab.url).hostname, reason: excluded, at: Date.now() } });
    return;
  }

  const now = Date.now();
  if (reason === "changed") {
    // Defer rather than drop, so the last change in a burst is still read.
    const wait = MIN_CHANGE_INTERVAL_MS - (now - (lastCaptureAt.get(tabId) ?? 0));
    if (wait > 0) {
      if (!pendingChange.has(tabId)) {
        pendingChange.set(tabId, setTimeout(() => {
          pendingChange.delete(tabId);
          capture(tabId, "changed");
        }, wait));
      }
      return;
    }
  }
  lastCaptureAt.set(tabId, now);

  const page = await requestExtraction(tabId).catch(() => null);
  // The page may have navigated between the check above and extraction.
  if (!page || exclusionReason(page.url)) return;

  const host = connect();
  if (!host) return;
  host.postMessage({ type: "page", tabId, reason, capturedAtMs: now, ...page });
  await setStatus({ lastCapture: { title: page.title, url: page.url, reason, at: now } });
}

async function captureActiveTab(windowId, reason) {
  if (windowId === chrome.windows.WINDOW_ID_NONE) return;
  const [tab] = await chrome.tabs.query({ active: true, windowId });
  if (tab) await capture(tab.id, reason);
}

chrome.tabs.onActivated.addListener(({ tabId }) => capture(tabId, "activated"));

chrome.tabs.onUpdated.addListener((tabId, changeInfo, tab) => {
  if (changeInfo.status === "complete" && tab.active) capture(tabId, "loaded");
});

chrome.windows.onFocusChanged.addListener((windowId) => {
  noteFocus(windowId);
  captureActiveTab(windowId, "focused");
});

chrome.runtime.onMessage.addListener((message, sender) => {
  if (message?.type === "pageChanged" && sender.tab?.id !== undefined) capture(sender.tab.id, "changed");
});

// On-demand reads need the host running, so connect whenever the worker
// starts. The alarm restarts the worker, and so the connection, if Chrome
// stops it.
chrome.alarms.create("bridge-keepalive", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "bridge-keepalive") connect();
});
connect();
