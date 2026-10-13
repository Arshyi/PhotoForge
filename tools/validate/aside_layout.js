(() => {
  const aside = document.querySelector('aside[aria-label="Editing controls"]');
  const cs = getComputedStyle(aside);
  const rect = (e) => { const r = e.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; };
  return JSON.stringify({
    aside: rect(aside),
    rows: cs.gridTemplateRows,
    overflow: cs.overflow,
    display: cs.display,
    children: [...aside.children].map((c) => ({
      tag: c.tagName.toLowerCase(),
      cls: c.className && c.className.baseVal === undefined ? c.className : '',
      text: (c.querySelector('summary, legend, h3, strong, span') || c).textContent.trim().slice(0, 40),
      rect: rect(c),
      display: getComputedStyle(c).display,
      gridRow: getComputedStyle(c).gridRowStart
    }))
  });
})()
