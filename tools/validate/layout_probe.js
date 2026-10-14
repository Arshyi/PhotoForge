// What a window of this size shows, as numbers. Evaluated in the page by `cdp.mjs layout`.
// Returns a JSON string: the sizes, and the problems found (an empty list is a clean window).
(() => {
  const problems = [];
  const round = (value) => Math.round(value * 10) / 10;
  const visible = (element) => {
    const rect = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
  };
  const label = (element) =>
    (element.getAttribute('aria-label') || element.textContent || element.value || element.tagName)
      .trim()
      .replace(/\s+/g, ' ')
      .slice(0, 36);
  const width = window.innerWidth;
  const root = document.documentElement;
  if (root.scrollWidth > root.clientWidth + 1) {
    problems.push(`the page scrolls sideways: ${root.scrollWidth} wide in ${root.clientWidth}`);
  }

  // The header: nothing past the window's right edge.
  const header = document.querySelector('header, .topbar');
  const headerButtons = header ? [...header.querySelectorAll('button')].filter(visible) : [];
  for (const button of headerButtons) {
    const rect = button.getBoundingClientRect();
    if (rect.right > width + 0.5) problems.push(`header button "${label(button)}" ends at ${round(rect.right)} of ${width}`);
  }

  // The editing panel: controls inside it, in the app's own look, and at one scale.
  const aside = document.querySelector('aside[aria-label="Editing controls"]');
  const sizes = new Map();
  let controls = 0;
  if (aside && visible(aside)) {
    const box = aside.getBoundingClientRect();
    if (aside.scrollWidth > aside.clientWidth + 1) {
      problems.push(`the editing panel scrolls sideways: ${aside.scrollWidth} wide in ${aside.clientWidth}`);
    }
    for (const control of aside.querySelectorAll('button, select, input:not([type=range]):not([type=checkbox]):not([type=radio])')) {
      if (!visible(control)) continue;
      controls += 1;
      const rect = control.getBoundingClientRect();
      const style = getComputedStyle(control);
      const name = `${control.tagName.toLowerCase()} "${label(control)}"`;
      if (rect.right > box.right + 1 || rect.left < box.left - 1) {
        problems.push(`${name} is outside the panel (${round(rect.left)} to ${round(rect.right)}, panel ${round(box.left)} to ${round(box.right)})`);
      }
      // The browser's own look: a light grey or white control on the dark panel.
      const background = style.backgroundColor;
      if (/^rgb\(2[34]\d, 2[34]\d, 2[34]\d\)$/.test(background) || background === 'rgb(255, 255, 255)') {
        problems.push(`${name} is drawn with the browser's own look (${background})`);
      }
      if (control.tagName !== 'BUTTON' || label(control).length > 3) {
        sizes.set(style.fontSize, (sizes.get(style.fontSize) || 0) + 1);
      }
      if (control.scrollWidth > control.clientWidth + 1 && style.overflow !== 'visible' && control.tagName === 'BUTTON') {
        problems.push(`${name} clips its text (${control.scrollWidth} in ${control.clientWidth})`);
      }
    }
  }
  const sizeList = [...sizes.entries()].sort((a, b) => parseFloat(a[0]) - parseFloat(b[0]));
  return JSON.stringify({
    width,
    height: window.innerHeight,
    devicePixelRatio: window.devicePixelRatio,
    asideVisible: Boolean(aside && visible(aside)),
    controls,
    controlFontSizes: sizeList.map(([size, count]) => `${size} x${count}`),
    problems
  });
})()
