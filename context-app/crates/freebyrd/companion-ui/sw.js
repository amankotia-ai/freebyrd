// Freebyrd for iPad's service worker (HTTPS only, through Tailscale). It
// keeps the app's own files for when the Mac is asleep or away, and shows
// notifications. A push carries nothing; this asks the Mac what to say, with
// the token the page handed it, and says only that something is waiting if
// the Mac cannot be reached.

const SHELL = "freebyrd-shell-v1";
const FILES = ["/", "/app.css", "/app.js", "/screens.js", "/manifest.webmanifest", "/icon-180.png", "/icon-192.png"];
const TOKEN = "/__freebyrd-token";

self.addEventListener("install", (event) => {
  event.waitUntil(caches.open(SHELL).then((cache) => cache.addAll(FILES)).then(() => self.skipWaiting()));
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches.keys()
      .then((names) => Promise.all(names.filter((name) => name !== SHELL && name !== "freebyrd-token").map((name) => caches.delete(name))))
      .then(() => self.clients.claim()),
  );
});

// The app's files: the Mac's copy when it answers, the kept one when not.
// Calls to the Mac are never kept here; the page keeps what it was shown.
self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);
  if (event.request.method !== "GET" || url.origin !== location.origin || url.pathname.startsWith("/api/")) return;
  const path = event.request.mode === "navigate" ? "/" : url.pathname;
  if (!FILES.includes(path)) return;
  event.respondWith(
    fetch(event.request)
      .then((response) => {
        if (response.ok) {
          const copy = response.clone();
          caches.open(SHELL).then((cache) => cache.put(path, copy));
        }
        return response;
      })
      .catch(() => caches.match(path)),
  );
});

self.addEventListener("message", (event) => {
  if (event.data?.type !== "token") return;
  event.waitUntil(
    caches.open("freebyrd-token").then((cache) =>
      event.data.token ? cache.put(TOKEN, new Response(event.data.token)) : cache.delete(TOKEN),
    ),
  );
});

async function token() {
  const cache = await caches.open("freebyrd-token");
  const kept = await cache.match(TOKEN);
  return kept ? kept.text() : null;
}

self.addEventListener("push", (event) => {
  event.waitUntil(
    (async () => {
      let note = { title: "Freebyrd", body: "Something is waiting.", count: 0, url: "/#cards" };
      try {
        const key = await token();
        const response = await fetch("/api/call/notification", {
          method: "POST",
          headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json" },
          body: "{}",
        });
        if (response.ok) note = await response.json();
      } catch {}
      if (self.navigator.setAppBadge) {
        try {
          if (note.count) await self.navigator.setAppBadge(note.count);
          else await self.navigator.clearAppBadge();
        } catch {}
      }
      await self.registration.showNotification(note.title, {
        body: note.body,
        tag: "freebyrd",
        icon: "/icon-192.png",
        data: { url: note.url || "/" },
      });
    })(),
  );
});

self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const url = event.notification.data?.url || "/";
  event.waitUntil(
    self.clients.matchAll({ type: "window", includeUncontrolled: true }).then((windows) => {
      const open = windows.find((client) => new URL(client.url).origin === location.origin);
      if (open) {
        open.postMessage({ type: "go", url });
        return open.focus();
      }
      return self.clients.openWindow(url);
    }),
  );
});
