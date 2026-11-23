// Maestro's remote page, while it is closed: a message from the computer becomes a notification, and a tap on it opens the session.
self.addEventListener('install', () => self.skipWaiting());
self.addEventListener('activate', (e) => e.waitUntil(self.clients.claim()));
self.addEventListener('push', (e) => {
  let d = {}; try { d = e.data ? e.data.json() : {}; } catch (_) {}
  e.waitUntil(self.registration.showNotification(d.title || 'Maestro', { body: d.body || 'A session is waiting for you', tag: d.name || 'maestro', data: { name: d.name || '' }, icon: 'icon.png', badge: 'icon.png' }));
});
self.addEventListener('notificationclick', (e) => {
  const name = (e.notification.data && e.notification.data.name) || ''; e.notification.close();
  e.waitUntil(self.clients.matchAll({ type: 'window', includeUncontrolled: true }).then((all) => {
    for (const c of all) { c.postMessage({ open: name }); if (c.focus) return c.focus(); }
    return self.clients.openWindow(name ? `./?s=${encodeURIComponent(name)}` : './');
  }));
});
