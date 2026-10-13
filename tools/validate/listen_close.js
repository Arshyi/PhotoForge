(async () => {
  const internals = window.__TAURI_INTERNALS__;
  window.__closeSeen = [];
  const handler = internals.transformCallback((event) => {
    window.__closeSeen.push({ at: Date.now(), id: event && event.id });
  });
  const unlisten = await internals.invoke('plugin:event|listen', {
    event: 'tauri://close-requested',
    target: { kind: 'Any' },
    handler
  });
  return 'listening ' + unlisten;
})()
