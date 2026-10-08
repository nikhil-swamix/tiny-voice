import { chromium } from 'playwright-core';
import assert from 'node:assert/strict';
const browser = await chromium.connectOverCDP('http://127.0.0.1:9237');
let page;
for (let attempt = 0; attempt < 50 && !page; attempt++) {
  page = browser.contexts().flatMap(context => context.pages()).find(page => page.url().startsWith('http://tauri.localhost'));
  if (!page) await new Promise(resolve => setTimeout(resolve, 200));
}
assert.ok(page, 'The executable must load its embedded UI, without a development server');
await page.waitForFunction(() => document.getElementById('orb')?.dataset.state === 'idle' && document.getElementById('status')?.textContent.includes('Ctrl+Shift+Space'));
const state = await page.evaluate(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  await invoke('set_tray_state', { recording: false });
  await invoke('recording_beep', { started: true });
  await invoke('recording_beep', { started: false });
  return {
    visible: await invoke('plugin:window|is_visible', { label: 'main' }),
    size: await invoke('plugin:window|inner_size', { label: 'main' }),
    position: await invoke('plugin:window|outer_position', { label: 'main' }),
    record: document.getElementById('record').textContent,
    opacity: getComputedStyle(document.body).opacity,
    overflow: document.documentElement.scrollHeight > innerHeight,
    secure: isSecureContext,
    audio: !!navigator.mediaDevices?.getUserMedia,
    rtc: typeof RTCPeerConnection === 'function',
    status: document.getElementById('status').textContent,
    engine: document.getElementById('engine').textContent
  };
});
assert.equal(state.visible,true); assert.equal(state.record,'Record'); assert.equal(state.overflow,false);
assert.ok(state.secure && state.audio && state.rtc);
assert.match(state.engine, /Idle · no task assigned · mic (off|ready)/);
await page.screenshot({ path: 'widget-preview.png', omitBackground: true });
console.log(JSON.stringify(state));
await browser.close();
