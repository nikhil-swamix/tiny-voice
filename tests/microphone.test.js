import test from 'node:test';
import assert from 'node:assert/strict';
import { createMicrophone } from '../src/microphone.js';

function fixture() {
  let stopped = 0, requests = 0, timer, delay;
  const track = { readyState: 'live', stop() { stopped++; this.readyState = 'ended'; } };
  const media = { getAudioTracks: () => [track], getTracks: () => [track] };
  const changes = [], mic = createMicrophone({ getMedia: async () => { requests++; return media; }, onChange: (...args) => changes.push(args),
    schedule: (fn, ms) => { timer = fn; delay = ms; return 1; }, cancel: () => { timer = undefined; } });
  return { mic, changes, track, get stopped() { return stopped; }, get requests() { return requests; }, get timer() { return timer; }, get delay() { return delay; } };
}
test('warm microphone releases at five minutes and does not record or send audio', async () => {
  const f = fixture(); await f.mic.get(); f.mic.idle();
  assert.equal(f.delay, 300000); assert.equal(f.stopped, 0); assert.equal(f.mic.ready, true);
  f.timer(); assert.equal(f.stopped, 1); assert.equal(f.mic.ready, false);
  assert.deepEqual(f.changes.at(-1), [false, 'timeout']);
});
test('resuming clears expiry and reuses the same live stream', async () => {
  const f = fixture(); const first = await f.mic.get(); f.mic.idle();
  assert.equal(await f.mic.get(), first); assert.equal(f.timer, undefined); assert.equal(f.requests, 1);
  f.mic.release();
});
test('mute and ended release resources exactly once and signal a lost device', async () => {
  for (const event of ['onmute', 'onended']) {
    const f = fixture(); await f.mic.get(); f.mic.idle(); const lost = f.track[event];
    lost(); lost(); assert.equal(f.stopped, 1); assert.equal(f.mic.ready, false); assert.equal(f.timer, undefined);
  }
});
test('releasing during pending permission closes the stream when permission arrives', async () => {
  let grant; const permission = new Promise(resolve => { grant = resolve; }); let stopped = 0, requests = 0;
  const mic = createMicrophone({ getMedia: () => { requests++; return permission; } });
  const first = mic.get(), second = mic.get();
  mic.release(); grant({ getTracks: () => [{ stop() { stopped++; } }] });
  await Promise.all([assert.rejects(first, /cancelled/), assert.rejects(second, /cancelled/)]);
  assert.equal(requests, 1); assert.equal(stopped, 1); assert.equal(mic.ready, false);
});
