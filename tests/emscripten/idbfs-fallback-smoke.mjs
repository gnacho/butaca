#!/usr/bin/env node
// With a built app and an isolated page open, reload once with IndexedDB hidden. The Emscripten
// preRun hook must skip IDBFS rather than aborting the Wasm runtime; the app should still create
// its runtime directory and draw. The script removes the hook and reloads normally afterward.
// Modes: "missing" hides the IndexedDB global; "blocked" makes indexedDB.open throw a
// SecurityError, exercising the restore-error callback. Usage:
//   node tests/emscripten/idbfs-fallback-smoke.mjs [cdp-port] [missing|blocked] [screenshot-path]

const cdpPort = Number(process.argv[2] || 9234);
const mode = process.argv[3] || 'missing';
if (!['missing', 'blocked'].includes(mode)) throw new Error(`Unknown storage failure mode: ${mode}`);
const screenshotPath = process.argv[4] || `/tmp/butaca-idbfs-${mode}.png`;
const targets = await (await fetch(`http://127.0.0.1:${cdpPort}/json/list`)).json();
const target = targets.find((item) => item.type === 'page' && item.url.includes('butaca-sim.html'));
if (!target) throw new Error(`Butaca page not found on CDP port ${cdpPort}`);

const ws = new WebSocket(target.webSocketDebuggerUrl);
let nextId = 0;
const pending = new Map();
const events = [];
ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) {
    pending.get(message.id)(message);
    pending.delete(message.id);
  } else if (message.method) {
    events.push(message);
  }
};
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = reject;
});

const send = (method, params = {}) => new Promise((resolve, reject) => {
  const id = ++nextId;
  let timer;
  pending.set(id, (message) => {
    clearTimeout(timer);
    resolve(message);
  });
  ws.send(JSON.stringify({ id, method, params }));
  timer = setTimeout(() => {
    if (pending.has(id)) {
      pending.delete(id);
      reject(new Error(`CDP command timed out: ${method}`));
    }
  }, 10_000);
});
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const evaluate = async (expression) => {
  const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
  return result.result?.result?.value;
};

await send('Runtime.enable');
await send('Log.enable');
await send('Page.enable');
const injectionSource = mode === 'missing'
  ? "Object.defineProperty(window, 'indexedDB', { configurable: true, value: undefined });"
  : "Object.defineProperty(window.indexedDB, 'open', { configurable: true, value: () => { throw new DOMException('Storage blocked by test', 'SecurityError'); } });";
const injection = await send('Page.addScriptToEvaluateOnNewDocument', { source: injectionSource });
const identifier = injection.result.identifier;
try {
  await send('Page.reload', { ignoreCache: true });
  let state;
  let screenshot;
  let visual;
  const deadline = Date.now() + 35_000;
  do {
    await pause(500);
    state = await evaluate(`({
      isolated: crossOriginIsolated,
      sharedArrayBuffer: typeof SharedArrayBuffer,
      indexedDB: typeof indexedDB,
      canvas: (() => { const c = document.getElementById('canvas'); return c && [c.width, c.height]; })()
    })`);
    if (state?.canvas?.[0] === 960 && state?.canvas?.[1] === 540) {
      screenshot = await send('Page.captureScreenshot', { format: 'png' });
      visual = await evaluate(`(async(data) => {
        const bytes = Uint8Array.from(atob(data), (c) => c.charCodeAt(0));
        const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
        const probe = document.createElement('canvas');
        probe.width = bitmap.width;
        probe.height = bitmap.height;
        const context = probe.getContext('2d', { willReadFrequently: true });
        context.drawImage(bitmap, 0, 0);
        const pixels = context.getImageData(0, 0, bitmap.width, bitmap.height).data;
        let brightPixels = 0;
        let maxLuminance = 0;
        for (let i = 0; i < pixels.length; i += 4) {
          const y = 0.2126 * pixels[i] + 0.7152 * pixels[i + 1] + 0.0722 * pixels[i + 2];
          if (y > 180) brightPixels++;
          if (y > maxLuminance) maxLuminance = y;
        }
        return { width: bitmap.width, height: bitmap.height, brightPixels, maxLuminance };
      })(${JSON.stringify(screenshot.result.data)})`);
    }
  } while (!(visual?.brightPixels > 1_000) && Date.now() < deadline);

  const messages = events
    .filter((event) => event.method === 'Runtime.consoleAPICalled')
    .flatMap((event) => event.params.args || [])
    .map((arg) => arg.value || arg.description || '');
  const expectedLog = mode === 'missing' ? 'IndexedDB is unavailable' : 'IndexedDB restore failed';
  const fallbackLogged = messages.some((message) => message.includes(expectedLog));
  const expectedIndexedDbType = mode === 'missing' ? 'undefined' : 'object';
  const aborted = messages.some((message) => message.includes('Aborted('));
  const exceptions = events
    .filter((event) => event.method === 'Runtime.exceptionThrown')
    .map((event) => event.params.exceptionDetails?.text || 'uncaught exception');
  if (
    state?.isolated !== true ||
    state?.sharedArrayBuffer !== 'function' ||
    state?.indexedDB !== expectedIndexedDbType
  ) {
    throw new Error(`Unexpected probe environment: ${JSON.stringify(state)}`);
  }
  if (
    state?.canvas?.[0] !== 960 ||
    state?.canvas?.[1] !== 540 ||
    !(visual?.brightPixels > 1_000) ||
    !fallbackLogged ||
    aborted ||
    exceptions.length
  ) {
    throw new Error(`IDBFS fallback failed: ${JSON.stringify({ mode, state, visual, fallbackLogged, aborted, exceptions, messages })}`);
  }

  const fs = await import('node:fs/promises');
  await fs.writeFile(screenshotPath, Buffer.from(screenshot.result.data, 'base64'));
  console.log(`PASS: app rendered a visible ${state.canvas.join('x')} UI with storage mode=${mode}; screenshot ${screenshotPath}`);
} finally {
  await send('Page.removeScriptToEvaluateOnNewDocument', { identifier });
  await send('Page.reload', { ignoreCache: true });
  await pause(5_000);
  ws.close();
}
