// Capture the packaged window, not the dev server.
//
// Launch the release binary with WebView2's DevTools port open:
//
//   WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 \
//     src-tauri/target/release/crossup.exe
//
// then run `node scripts/ui-shot.mjs`. Each picture is a `Page.captureScreenshot`
// of the app's own webview, written to docs/ui-pass.
import fs from "node:fs";
import path from "node:path";

const PORT = Number(process.env.CROSSUP_CDP_PORT || 9222);
const OUT = path.resolve("docs/ui-pass");
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function target() {
  for (let i = 0; i < 60; i += 1) {
    try {
      const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = list.find((t) => t.type === "page");
      if (page?.webSocketDebuggerUrl) return page.webSocketDebuggerUrl;
    } catch {
      // The webview is not up yet.
    }
    await wait(500);
  }
  throw new Error(`no DevTools page on ${PORT}`);
}

const socket = new WebSocket(await target());
let nextId = 0;
const pending = new Map();

socket.onmessage = (event) => {
  const message = JSON.parse(event.data);
  const resolve = pending.get(message.id);
  if (resolve) {
    pending.delete(message.id);
    resolve(message);
  }
};

const send = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, resolve);
    socket.send(JSON.stringify({ id, method, params }));
    setTimeout(() => reject(new Error(`${method} timed out`)), 20000);
  });

const evaluate = async (expression) => {
  const reply = await send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  const failure = reply.result?.exceptionDetails;
  if (failure) throw new Error(failure.exception?.description || failure.text);
  return reply.result?.result?.value;
};

const shoot = async (name) => {
  const reply = await send("Page.captureScreenshot", { format: "png" });
  const file = path.join(OUT, `${name}.png`);
  fs.writeFileSync(file, Buffer.from(reply.result.data, "base64"));
  const { width, height } = await evaluate("({ width: innerWidth, height: innerHeight })");
  console.log(`${name}.png  ${width}x${height}`);
};

await new Promise((resolve, reject) => {
  socket.onopen = resolve;
  socket.onerror = () => reject(new Error("devtools socket failed"));
});
await send("Page.enable");
await send("Runtime.enable");

for (let i = 0; i < 40; i += 1) {
  if (await evaluate("!!document.querySelector('.hb__page')")) break;
  await wait(250);
}
await wait(800);

// The previous run may have left the app in light mode; every shot but the last
// is meant to be the default dark preset.
const pickMode = async (label) => {
  await evaluate(`(() => { const b = document.querySelector('button[aria-label="Open settings"]'); if (b) b.click(); })()`);
  await wait(450);
  await evaluate(`(() => { const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim().startsWith('Appearance')); if (b) b.click(); })()`);
  await wait(450);
  await evaluate(`(() => { const r = [...document.querySelectorAll('[role=radio]')].find((x) => x.textContent.trim() === ${JSON.stringify(label)}); if (r) r.click(); })()`);
  await wait(500);
  await evaluate(`(() => { const d = [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Done'); if (d) d.click(); })()`);
  await wait(350);
};
await pickMode("Dark");

// The tab strip renders its label twice for assistive technology, so the text
// is not a reliable handle. Order is fixed: Remapper, Auto clicker, Typer,
// Macros, Replayer.
const openTab = async (index) => {
  const labels = await evaluate(`(() => {
    const tabs = [...document.querySelectorAll('[role=tab]')];
    if (!tabs[${index}]) throw new Error('no tab at ${index}');
    tabs[${index}].click();
    return tabs.map((t) => t.getAttribute('aria-label') || t.id || 'tab');
  })()`);
  await wait(650);
  return labels;
};

const closeDialogs = async () => {
  await evaluate(`(() => {
    const done = [...document.querySelectorAll('button')].find((b) => b.textContent.trim() === 'Done');
    if (done) done.click();
  })()`);
  await wait(350);
};

await shoot("packaged-remapper");

for (const [index, name] of [
  [1, "packaged-clicker"],
  [3, "packaged-macros"],
  [4, "packaged-replayer"],
]) {
  await openTab(index);
  await shoot(name);
}

// The macro builder scrolled down: the palette has to stay whole under the
// sticky title row instead of sliding behind it. An empty macro has nothing to
// scroll, so add a few blocks first and take them away again afterwards; the
// app writes what it shows, so the shot must not leave six blocks behind.
await openTab(3);
await evaluate(`(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const chips = [...document.querySelectorAll('.pal__item')];
  for (const i of [2, 5, 6, 2, 5, 6]) {
    const chip = [...document.querySelectorAll('.pal__item')][i];
    if (!chip) throw new Error('no palette chip at ' + i);
    chip.click();
    await sleep(140);
  }
})()`);
await wait(500);
const scrolled = await evaluate(`(() => {
  const page = document.querySelector('.hb__page');
  page.scrollTop = 420;
  return { scrollTop: page.scrollTop, max: page.scrollHeight - page.clientHeight };
})()`);
console.log("scrolled", JSON.stringify(scrolled));
await wait(500);
await shoot("packaged-palette");

const cleaned = await evaluate(`(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  for (let i = 0; i < 12; i += 1) {
    const remove = [...document.querySelectorAll('button[aria-label]')].find((b) => b.getAttribute('aria-label').startsWith('Remove'));
    if (!remove) break;
    remove.click();
    await sleep(150);
  }
  const page = document.querySelector('.hb__page');
  page.scrollTop = 0;
  return document.querySelector('.hb__sub')?.textContent ?? '';
})()`);
console.log("after cleanup:", cleaned);

await openTab(0);
await evaluate(`(() => {
  const b = document.querySelector('button[aria-label="Open settings"]');
  if (!b) throw new Error('no settings button');
  b.click();
})()`);
await wait(500);
await evaluate(`(() => {
  const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim().startsWith('Appearance'));
  if (!b) throw new Error('no appearance row');
  b.click();
})()`);
await wait(500);
await evaluate(`(() => {
  const light = [...document.querySelectorAll('[role=radio]')].find((r) => r.textContent.trim() === 'Light');
  if (!light) throw new Error('no light control');
  light.click();
})()`);
await wait(800);
await closeDialogs();
await wait(600);
await shoot("packaged-light");

await evaluate(`(() => {
  const dark = [...document.querySelectorAll('[role=radio]')].find((r) => r.textContent.trim() === 'Dark');
  if (dark) dark.click();
})()`);
await wait(400);
await closeDialogs();

socket.close();
