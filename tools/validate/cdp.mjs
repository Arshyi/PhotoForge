// A small Chrome DevTools Protocol client for the WebView2 of the packaged PhotoForge window.
// Local only: the app was launched by this validation with --remote-debugging-port on 127.0.0.1.
// Input is dispatched into the page (Input.dispatchMouseEvent), never to the desktop, so it cannot
// land in another program.
//
//   node cdp.mjs install                     intercept the native file dialogs, log commands
//   node cdp.mjs eval "<js expression>"      evaluate in the page, print the JSON value
//   node cdp.mjs shot out.png                capture the page
//   node cdp.mjs text                        the visible text of the page
//   node cdp.mjs buttons                     visible buttons: label and centre
//   node cdp.mjs click x y                   a real pointer click at page coordinates
//   node cdp.mjs clickText "Open region"     click the visible button/label with this text
//   node cdp.mjs drag x1 y1 x2 y2            a real pointer drag
//   node cdp.mjs log                         the IPC log recorded since install (command, status, error)
//   node cdp.mjs type x y <text>             click a field, select what is there, type
//   node cdp.mjs key Enter|Tab|Escape        a key into the page's focused element
//   node cdp.mjs evalfile <file.js>          evaluate the contents of a file in the page
//
// `install` wraps window.fetch (Tauri's invoke property cannot be reassigned): the native file
// dialogs (plugin:dialog|open and |save) are answered with window.__pf.nextOpen / nextSave, and every
// other IPC call is passed through and logged with its outcome. The port is PF_CDP_PORT (9333).
import fs from 'node:fs';

const PORT = Number(process.env.PF_CDP_PORT || 9333);

async function connect() {
  const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
  const page = targets.find((target) => target.type === 'page');
  if (!page) throw new Error('no page target: ' + JSON.stringify(targets));
  const socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = reject;
  });
  let next = 1;
  const pending = new Map();
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = next++;
      pending.set(id, { resolve, reject });
      socket.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const result = await send('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true
    });
    if (result.exceptionDetails) {
      throw new Error(result.exceptionDetails.exception?.description || JSON.stringify(result.exceptionDetails));
    }
    return result.result.value;
  };
  return { send, evaluate, close: () => socket.close() };
}

const mouse = (send, type, x, y, extra = {}) =>
  send('Input.dispatchMouseEvent', { type, x, y, button: 'left', clickCount: 1, ...extra });

async function click(send, x, y) {
  await mouse(send, 'mouseMoved', x, y, { button: 'none', clickCount: 0 });
  await mouse(send, 'mousePressed', x, y);
  await mouse(send, 'mouseReleased', x, y);
}

async function drag(send, x1, y1, x2, y2, steps = 14) {
  await mouse(send, 'mouseMoved', x1, y1, { button: 'none', clickCount: 0 });
  await mouse(send, 'mousePressed', x1, y1);
  for (let step = 1; step <= steps; step++) {
    const x = x1 + ((x2 - x1) * step) / steps;
    const y = y1 + ((y2 - y1) * step) / steps;
    await mouse(send, 'mouseMoved', x, y, { buttons: 1 });
  }
  await mouse(send, 'mouseReleased', x2, y2);
}

const INSTALL = `(() => {
  if (window.__pf) return 'already installed';
  // Tauri's own invoke property cannot be reassigned, so the native file dialogs are answered
  // where the call leaves the page: the IPC fetch. Everything else goes through untouched, and
  // is logged with the outcome (an ACL refusal comes back as an error response).
  const originalFetch = window.fetch.bind(window);
  window.__pf = { nextOpen: null, nextSave: null, log: [] };
  window.fetch = async (input, init) => {
    const url = typeof input === 'string' ? input : input.url;
    if (!url.includes('ipc.localhost')) return originalFetch(input, init);
    const command = decodeURIComponent(new URL(url).pathname.slice(1));
    const entry = { command, at: Date.now() };
    window.__pf.log.push(entry);
    const answer = (value) =>
      new Response(JSON.stringify(value), {
        status: 200,
        headers: { 'Content-Type': 'application/json', 'Tauri-Response': 'ok' }
      });
    if (command === 'plugin:dialog|open') {
      entry.stubbed = window.__pf.nextOpen;
      return answer(window.__pf.nextOpen);
    }
    if (command === 'plugin:dialog|save') {
      entry.stubbed = window.__pf.nextSave;
      return answer(window.__pf.nextSave);
    }
    const response = await originalFetch(input, init);
    entry.status = response.status;
    entry.tauri = response.headers.get('Tauri-Response');
    if (entry.tauri === 'error') {
      response.clone().text().then((text) => { entry.error = text.slice(0, 300); });
    } else if (command === 'open_raw_image' || command === 'develop_raw_layer') {
      response.clone().json().then((body) => {
        const source = body.source || null;
        entry.summary = {
          width: body.metadata ? body.metadata.width : body.width,
          height: body.metadata ? body.metadata.height : body.height,
          view: source ? source.view || null : null,
          crop: source ? source.crop || null : null,
          orientationApplied: source ? source.orientationApplied || null : null,
          reference: source ? [source.reference.width, source.reference.height] : null,
          sourceMetadata: body.metadata ? body.metadata.sourceMetadata || null : null
        };
      });
    }
    return response;
  };
  return 'installed';
})()`;

const BUTTONS = `(() => {
  const visible = (element) => {
    const box = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return box.width > 0 && box.height > 0 && style.visibility !== 'hidden' && style.display !== 'none';
  };
  return [...document.querySelectorAll('button, [role=button], [role=tab], summary, label')]
    .filter(visible)
    .map((element) => {
      const box = element.getBoundingClientRect();
      return {
        text: (element.getAttribute('aria-label') || element.textContent || '').trim().replace(/\\s+/g, ' ').slice(0, 60),
        tag: element.tagName.toLowerCase(),
        disabled: element.disabled === true,
        x: Math.round(box.x + box.width / 2),
        y: Math.round(box.y + box.height / 2)
      };
    });
})()`;

const main = async () => {
  const [command, ...rest] = process.argv.slice(2);
  const { send, evaluate, close } = await connect();
  try {
    switch (command) {
      case 'install':
        console.log(await evaluate(INSTALL));
        break;
      case 'evalfile':
        console.log(JSON.stringify(await evaluate(fs.readFileSync(rest[0], 'utf8')), null, 2));
        break;
      case 'eval':
        console.log(JSON.stringify(await evaluate(rest.join(' ')), null, 2));
        break;
      case 'shot': {
        const result = await send('Page.captureScreenshot', { format: 'png' });
        fs.writeFileSync(rest[0], Buffer.from(result.data, 'base64'));
        console.log('wrote', rest[0]);
        break;
      }
      case 'text':
        console.log(await evaluate('document.body.innerText'));
        break;
      case 'buttons':
        for (const item of await evaluate(BUTTONS)) {
          console.log(`${item.disabled ? '[off] ' : ''}${item.tag} "${item.text}" @ ${item.x},${item.y}`);
        }
        break;
      case 'click':
        await click(send, Number(rest[0]), Number(rest[1]));
        console.log('clicked', rest[0], rest[1]);
        break;
      case 'clickText': {
        const wanted = rest.join(' ');
        const items = await evaluate(BUTTONS);
        const hit = items.find((item) => item.text === wanted) || items.find((item) => item.text.includes(wanted));
        if (!hit) throw new Error('no visible control with text: ' + wanted);
        await click(send, hit.x, hit.y);
        console.log('clicked', JSON.stringify(hit));
        break;
      }
      case 'drag':
        await drag(send, ...rest.slice(0, 4).map(Number));
        console.log('dragged');
        break;
      case 'type': {
        // type x y text: a real triple-click to select what is there, then the text.
        const [x, y, ...words] = rest;
        await click(send, Number(x), Number(y));
        await mouse(send, 'mousePressed', Number(x), Number(y), { clickCount: 2 });
        await mouse(send, 'mouseReleased', Number(x), Number(y), { clickCount: 2 });
        await mouse(send, 'mousePressed', Number(x), Number(y), { clickCount: 3 });
        await mouse(send, 'mouseReleased', Number(x), Number(y), { clickCount: 3 });
        await send('Input.insertText', { text: words.join(' ') });
        console.log('typed', words.join(' '));
        break;
      }
      case 'key': {
        // key Enter | Tab | Escape ... into the page's focused element.
        const name = rest[0];
        const codes = { Enter: 13, Tab: 9, Escape: 27 };
        const base = { key: name, code: name, windowsVirtualKeyCode: codes[name] || 0 };
        await send('Input.dispatchKeyEvent', { type: 'keyDown', ...base });
        await send('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
        console.log('key', name);
        break;
      }
      case 'log':
        console.log(JSON.stringify(await evaluate('window.__pf ? window.__pf.log : null'), null, 1));
        break;
      default:
        throw new Error('unknown command ' + command);
    }
  } finally {
    close();
  }
};

main().catch((error) => {
  console.error('ERROR', error.message);
  process.exit(1);
});
