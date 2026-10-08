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
  Object.defineProperty(window.screen, 'availWidth', { value: 1600 });
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
  const quick = '## Launch documentation\n  - Publish **Monday**, under 200 words.\n  - <img src=x onerror="window.unsafe=true">Keep the stated deadline.\n\n## Hints\n  - Check the final draft against the deadline and every stated constraint before sharing it.';
  const pro = '## Launch documentation\n  - Publish **Monday**, under 200 words.\n\n  ### Suggested approach\n  - Draft the main goal, then remove repetition.\n  - Ask a reader to check clarity.\n\n  ### Suggested checks\n  - Count words and verify the deadline.\n\n## Hints\n  - Write the main point first, then use the remaining words for essential supporting details.\n  - Ask someone unfamiliar with the task to read the draft and flag unclear instructions.\n  - Check the final draft against the deadline and every stated constraint before sharing it.';
  window.__TAURI_INTERNALS__ = { metadata: { currentWindow: { label: 'main' } }, transformCallback: () => 1, invoke: async (name, args) => {
    smoke.calls.push([name, args]);
    if (name === 'startup_status') return 'Ready · Ctrl+Shift+Space';
    if (name === 'plugin:event|listen') return 1;
    if (name === 'plugin:window|set_size') return resizeWidget(args.value.size);
    if (name === 'begin_recording') return 'renderer-smoke';
    if (name === 'connect_realtime') return 'synthetic answer';
    if (name === 'finish_recording') { await new Promise(resolve => setTimeout(resolve, 30)); return { text: smoke.output || (args.mode === 'pro' ? pro : quick), intent: 'request', model: 'gpt-6.1-sol', reasoning: 'medium' }; }
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
  assert.equal(await page.locator('[data-mode=pro]').getAttribute('aria-pressed'), 'true', 'Turbo is the default');
  await page.locator('[data-mode=quick]').click();
  await page.locator('#record').click();
  await page.waitForFunction(() => document.getElementById('orb').dataset.state === 'recording');
  await page.locator('#record').click();
  await page.waitForFunction(() => document.getElementById('status').textContent.includes('Quick · copied and pasted'));
  assert.match(await page.locator('#engine').textContent(), /Idle · no task assigned · mic ready/);
  assert.ok(await page.locator('#popover h2').count());
  assert.equal(await page.locator('#popover img, #popover script, #popover [onerror], #popover pre').count(), 0);
  assert.equal(await page.evaluate(() => !!window.unsafe), false);
  assert.equal(await page.locator('#modes').isVisible(), false);
  await page.locator('#turbo').click();
  await page.waitForFunction(() => document.getElementById('status').textContent.includes('Turbo · copied and pasted'));
  assert.equal(await page.locator('body.mini').count(), 1, 'Turbo must not expand the mini bar');
  assert.equal(await page.locator('#popover h3').count(), 2);
  await page.waitForFunction(() => innerWidth === 240);
  const seam = await page.evaluate(() => {
    const bar = document.getElementById('panel').getBoundingClientRect(), paper = document.getElementById('popover').getBoundingClientRect();
    const style = getComputedStyle(document.getElementById('popover'));
    return { barWidth: bar.width, paperWidth: paper.width, gap: paper.top - bar.bottom, left: paper.left - bar.left, topRadius: style.borderTopLeftRadius, opacity: style.opacity };
  });
  assert.equal(seam.barWidth, seam.paperWidth); assert.equal(seam.gap, 0); assert.equal(seam.left, 0);
  assert.equal(seam.topRadius, '0px'); assert.equal(seam.opacity, '1', 'Fading applies once to the joined widget');
  await page.evaluate(() => document.getElementById('minimize').click());
  await page.waitForFunction(() => innerWidth === 240 && !document.body.classList.contains('dim'));
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
    hints: Array.from(document.querySelectorAll('#text h2')).find(node => node.textContent === 'Hints')?.nextElementSibling.querySelectorAll('li').length,
    metadata: document.getElementById('model').textContent,
    black: getComputedStyle(document.getElementById('panel')).backgroundColor,
    headerFits: document.getElementById('dragbar').scrollWidth <= document.getElementById('dragbar').clientWidth
  }));
  assert.deepEqual(result.requests, [['renderer-smoke', 'quick'], ['renderer-smoke', 'pro']]);
  assert.equal(result.copied, 2); assert.deepEqual(result.pasted, [321, 321]); assert.equal(result.hints, 3);
  assert.equal(result.metadata, 'gpt-6.1-sol · medium reasoning');
  assert.equal(result.black, 'rgb(0, 0, 0)'); assert.equal(result.headerFits, true); assert.deepEqual(errors, []);
  await page.locator('#mic').click();
  assert.match(await page.locator('#engine').textContent(), /Idle · no task assigned · mic off/);
  await page.evaluate(() => { smoke.output = '## Large task\n' + Array.from({length: 100}, (_, index) => `  - Step ${index}: verify this part of the recorded task.`).join('\n'); });
  await page.locator('#turbo').click();
  await page.waitForFunction(() => innerHeight === 850);
  assert.ok(await page.locator('#popover').evaluate(node => node.scrollHeight > node.clientHeight), 'Long output scrolls within the screen limit');
  await page.evaluate(() => { smoke.output = '## Note\n  - Brief task.'; });
  await page.locator('#turbo').click();
  await page.waitForFunction(() => innerHeight === 108);
  console.log(JSON.stringify({...result, seam, adaptiveHeight: '108–850'}));
} finally {
  await browser.close();
}
