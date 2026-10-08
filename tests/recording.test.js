import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const tick = () => new Promise(resolve => setImmediate(resolve));
async function harness(options = {}) {
  const calls = [], nodes = new Map(), track = { stop() { calls.push(['mic-stopped']); } };
  const media = { getAudioTracks: () => [track], getTracks: () => [track] };
  const get = id => { if (!nodes.has(id)) nodes.set(id, { dataset: {}, value: '', textContent: '', scrollHeight: 0, classList: { toggle() {} }, setAttribute() {}, addEventListener() {} }); return nodes.get(id); };
  class Recorder {
    static isTypeSupported() { return true; }
    constructor() { this.state = 'inactive'; }
    start() { this.state = 'recording'; calls.push(['media-start']); }
    stop() { this.state = 'inactive'; this.ondataavailable({ data: new Blob(['final audio']) }); queueMicrotask(() => this.onstop()); }
  }
  class Peer {
    addTrack() {} createDataChannel() { return {}; } createOffer() { return Promise.resolve({ sdp: 'offer' }); }
    async setLocalDescription() {} async setRemoteDescription() {} close() { calls.push(['peer-closed']); }
  }
  const invoke = async (name, args) => {
    calls.push([name, args]);
    if (name === 'startup_status') return 'Ready';
    if (name === 'begin_recording') { if (options.beginGate) await options.beginGate; return 'session'; }
    if (name === 'connect_realtime') { if (options.streamFail) throw Error('stream offline'); return 'answer'; }
    if (name === 'append_audio') { await tick(); if (options.diskFail) throw Error('disk full'); calls.push(['disk-written']); }
    if (name === 'finish_recording') { if (options.apiFail) throw Error('API offline'); return { text: 'Clean transcript', intent: 'request' }; }
    if (name === 'paste_result') return true;
  };
  const source = (await readFile(new URL('../src/main.js', import.meta.url), 'utf8')).replace(/^import .*;\r?\n/gm, '');
  const app = await vm.runInNewContext(`(async()=>{${source}\nreturn { toggle, start, stop, quit, processLast, get state(){return state} };})()`, {
    invoke, listen: async () => {}, markUrl: 'mark.svg', waveformUrl: 'wave.svg', getCurrentWindow: () => ({ setSize: async () => {} }), LogicalSize: class {}, createIdleFade: () => ({ wake() {}, dispose() {} }),
    document: { getElementById: get, body: { classList: { toggle() {} } }, addEventListener() {} }, window: { screen: { availHeight: 800 }, addEventListener() {} },
    navigator: { mediaDevices: { getUserMedia: options.getMedia || (async () => media) }, clipboard: {} },
    MediaRecorder: Recorder, RTCPeerConnection: Peer, Blob, Uint8Array, setTimeout, clearTimeout, queueMicrotask, console
  });
  return { app, calls, nodes, media, options };
}
const settle = async () => { for (let i = 0; i < 5; i++) await tick(); };

test('stop waits for the last audio chunk before transcription and releases microphone', async () => {
  const { app, calls, nodes } = await harness(); await app.start(); await app.stop(); await settle();
  assert.equal(app.state, 'idle'); assert.equal(nodes.get('text').value, 'Clean transcript');
  assert.ok(calls.findIndex(([name]) => name === 'finish_recording') > calls.findIndex(([name]) => name === 'disk-written'));
  assert.ok(calls.some(([name]) => name === 'mic-stopped'));
  assert.deepEqual(calls.filter(([name]) => name === 'recording_beep').map(([, args]) => args.started), [true, false]);
  assert.ok(calls.findIndex(([name, args]) => name === 'recording_beep' && !args.started) > calls.findIndex(([name]) => name === 'mic-stopped'));
  assert.ok(calls.some(([name]) => name === 'copy_result')); assert.ok(calls.some(([name]) => name === 'paste_result'));
});
test('same hotkey cancels a pending microphone start', async () => {
  let grant; const permission = new Promise(resolve => { grant = resolve; });
  const { app, calls, media } = await harness({ getMedia: () => permission });
  const starting = app.start(); app.toggle(); grant(media); await starting;
  assert.equal(app.state, 'idle'); assert.ok(!calls.some(([name]) => name === 'begin_recording'));
  assert.ok(calls.some(([name]) => name === 'mic-stopped'));
  assert.ok(!calls.some(([name]) => name === 'recording_beep'));
});
test('audio capture starts before the storage session request finishes', async () => {
  let begin;
  const beginGate = new Promise(resolve => { begin = resolve; });
  const { app, calls } = await harness({ beginGate });
  const starting = app.start();
  await settle();
  assert.equal(app.state, 'recording');
  assert.ok(calls.findIndex(([name]) => name === 'media-start') < calls.findIndex(([name]) => name === 'begin_recording'));
  begin();
  await starting;
  await app.stop();
});
test('stream failure keeps local recording and allows final transcription', async () => {
  const { app, calls } = await harness({ streamFail: true }); await app.start(); await tick();
  assert.equal(app.state, 'recording'); await app.stop();
  assert.ok(calls.some(([name]) => name === 'disk-written')); assert.equal(app.state, 'idle');
});
test('API failure retains saved audio and leaves recording controls available', async () => {
  const { app, calls, nodes } = await harness({ apiFail: true }); await app.start(); await app.stop(); await settle();
  assert.equal(app.state, 'idle'); assert.ok(calls.some(([name]) => name === 'disk-written'));
  assert.equal(nodes.get('status').textContent, 'API offline');
});
test('disk failure stops the microphone and skips transcription', async () => {
  const { app, calls } = await harness({ diskFail: true }); await app.start(); await app.stop();
  assert.equal(app.state, 'error'); assert.ok(calls.some(([name]) => name === 'mic-stopped'));
  assert.ok(!calls.some(([name]) => name === 'finish_recording'));
});
test('Tray quit saves the final audio without waiting for network transcription', async () => {
  const { app, calls } = await harness({ apiFail: true }); await app.start(); await app.quit();
  assert.ok(calls.findIndex(([name]) => name === 'quit_app') > calls.findIndex(([name]) => name === 'disk-written'));
  assert.ok(!calls.some(([name]) => name === 'finish_recording'));
});
