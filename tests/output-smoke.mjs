import { chromium } from 'playwright-core';
import assert from 'node:assert/strict';

// Inject the native bridge before startup; Tauri's real bridge is immutable.
// This UI check uses synthetic media and never records or pastes user data.
const browser = await chromium.launch({ executablePath: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe', headless: true });
const page = await browser.newPage({ viewport: { width: 300, height: 440 } });
await page.exposeFunction('resizeWidget', size => page.setViewportSize({ width: size.width, height: size.height }));
const errors = []; page.on('pageerror', error => errors.push(error.message));
await page.addInitScript(() => {
  window.smoke = { calls: [] };
  Object.defineProperty(window.screen, 'availHeight', { value: 1000 });
  const microphone = { getAudioTracks: () => [{ stop() {} }], getTracks: () => [{ stop() {} }] };
  Object.defineProperty(navigator, 'mediaDevices', { value: { getUserMedia: async () => microphone } });
  window.MediaRecorder = class {
    static isTypeSupported() { return true; }
    constructor() { this.state = 'inactive'; }
    start() { this.state = 'recording'; }
    stop() { this.state = 'inactive'; this.ondataavailable({ data: new Blob(['synthetic audio']) }); queueMicrotask(() => this.onstop()); }
  };
  window.RTCPeerConnection = class {
    addTrack() {} createDataChannel() { return {}; } async createOffer() { return { sdp: 'synthetic offer' }; }
    async setLocalDescription() {} async setRemoteDescription() {} close() {}
  };
  const quick = '## Launch documentation\n\t\t- Publish **Monday**, under 200 words.\n\t\t- <img src=x onerror="window.unsafe=true">Keep the stated deadline.\n\n## Hints\n\t\t- [hint: Check the final draft against the deadline and every stated constraint before sharing it.]';
  const pro = '## Launch documentation\n\t\t- Publish **Monday**, under 200 words.\n\n\t\t### Suggested approach\n\t\t\t\t- Draft the main goal, then remove repetition.\n\t\t\t\t- Ask a reader to check clarity.\n\n\t\t### Suggested checks\n\t\t\t\t- Count words and verify the deadline.\n\n## Hints\n\t\t- [hint: Write the main point first, then use the remaining words for essential supporting details.]\n\t\t- [hint: Ask someone unfamiliar with the task to read the draft and flag unclear instructions.]\n\t\t- [hint: Check the final draft against the deadline and every stated constraint before sharing it.]';
  window.__TAURI_INTERNALS__ = { metadata: { currentWindow: { label: 'main' } }, transformCallback: () => 1, invoke: async (name, args) => {
    smoke.calls.push([name, args]);
    if (name === 'startup_status') return 'Ready · Ctrl+Shift+Space';
    if (name === 'plugin:event|listen') return 1;
    if (name === 'plugin:window|set_size') return resizeWidget(args.value.size);
    if (name === 'begin_recording') return 'renderer-smoke';
    if (name === 'connect_realtime') return 'synthetic answer';
    if (name === 'finish_recording') { await new Promise(resolve => setTimeout(resolve, 30)); return { text: args.mode === 'pro' ? pro : quick, intent: 'request', model: 'gpt-6.1-sol', reasoning: args.mode === 'pro' ? 'high' : 'low' }; }
    if (name === 'capture_paste_target') return 321;
    if (name === 'paste_result') return true;
    if (['append_audio', 'copy_result', 'set_tray_state', 'recording_beep'].includes(name)) return;
  } };
});
await page.goto('http://127.0.0.1:1420');
await page.waitForFunction(() => document.getElementById('orb')?.dataset.state === 'idle');
try {
  await page.locator('#meter').click();
  await page.waitForFunction(() => !document.body.classList.contains('mini'));
  await page.locator('[data-mode=quick]').click();
  await page.locator('#record').click();
  await page.waitForFunction(() => document.getElementById('orb').dataset.state === 'recording');
  await page.locator('#record').click();
  await page.waitForFunction(() => document.getElementById('status').textContent.includes('Quick · copied and pasted'));
  assert.ok(await page.locator('#popover h2').count());
  assert.equal(await page.locator('#popover img, #popover script, #popover [onerror], #popover pre').count(), 0);
  assert.equal(await page.evaluate(() => !!window.unsafe), false);
  assert.equal(await page.locator('#modes').isVisible(), false);
  await page.locator('#turbo').click();
  await page.waitForFunction(() => document.getElementById('status').textContent.includes('Pro · copied and pasted'));
  assert.equal(await page.locator('body.mini').count(), 1, 'Turbo must not expand the mini bar');
  assert.equal(await page.locator('#popover h3').count(), 2);
  await page.screenshot({ path: 'widget-output-mini.png', omitBackground: true });
  await page.locator('#meter').click();
  assert.equal(await page.locator('body.mini').count(), 0);
  await page.waitForFunction(() => innerWidth >= 300);
  assert.equal(await page.locator('#modes').isVisible(), true);
  await page.screenshot({ path: 'widget-output-expanded.png', omitBackground: true });
  const result = await page.evaluate(() => ({
    requests: smoke.calls.filter(([name]) => name === 'finish_recording').map(([, args]) => [args.id, args.mode]),
    copied: smoke.calls.filter(([name]) => name === 'copy_result').length,
    pasted: smoke.calls.filter(([name]) => name === 'paste_result').map(([, args]) => args.target),
    heading: document.querySelector('#text h2').textContent,
    hints: document.getElementById('text').textContent.match(/\[hint:/g)?.length,
    metadata: document.getElementById('model').textContent,
    black: getComputedStyle(document.getElementById('panel')).backgroundColor,
    headerFits: document.getElementById('dragbar').scrollWidth <= document.getElementById('dragbar').clientWidth
  }));
  assert.deepEqual(result.requests, [['renderer-smoke', 'quick'], ['renderer-smoke', 'pro']]);
  assert.equal(result.copied, 2); assert.deepEqual(result.pasted, [321, 321]); assert.equal(result.hints, 3);
  assert.equal(result.metadata, 'gpt-6.1-sol · high reasoning');
  assert.equal(result.black, 'rgb(0, 0, 0)'); assert.equal(result.headerFits, true); assert.deepEqual(errors, []);
  console.log(JSON.stringify(result));
} finally {
  await browser.close();
}
