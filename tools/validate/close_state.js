(() => JSON.stringify({
  closeSeen: window.__closeSeen || null,
  dialogs: [...document.querySelectorAll('[role=dialog],[role=alertdialog],.modal,.overlay')].map((d) => d.textContent.trim().slice(0, 80)),
  hidden: document.hidden,
  visibility: document.visibilityState,
  focus: document.hasFocus()
}))()
