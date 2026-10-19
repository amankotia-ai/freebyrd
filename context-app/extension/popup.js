const connection = document.getElementById("connection");
const paused = document.getElementById("paused");
const last = document.getElementById("last");
const skip = document.getElementById("skip");

function render(status = {}) {
  if (status.connected) {
    connection.textContent = "Connected";
    connection.className = "ok";
  } else {
    connection.textContent = status.hostError ?? "Not connected";
    connection.className = "bad";
  }
  if (status.lastCapture) {
    const { title, reason, at } = status.lastCapture;
    last.textContent = `${title || "(untitled)"} · ${reason} · ${new Date(at).toLocaleTimeString()}`;
  }
  skip.textContent = status.lastSkip
    ? `Skipped ${status.lastSkip.host} (${status.lastSkip.reason})`
    : "";
}

chrome.storage.session.get("status").then(({ status }) => render(status));
chrome.storage.onChanged.addListener((changes, area) => {
  if (area === "session" && changes.status) render(changes.status.newValue);
});

chrome.storage.local.get("paused").then(({ paused: value = false }) => {
  paused.checked = value;
});
paused.addEventListener("change", () => chrome.storage.local.set({ paused: paused.checked }));
