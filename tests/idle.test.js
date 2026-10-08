import test from 'node:test';
import assert from 'node:assert/strict';
import { createIdleFade } from '../src/idle.js';

function setup() {
  let now = 0, serial = 0; const timers = new Map(), classes = new Set();
  const clock = { now: () => now, setTimeout: (fn, delay) => { timers.set(++serial, { fn, at: now + delay }); return serial; }, clearTimeout: id => timers.delete(id) };
  const element = { classList: { add: cls => classes.add(cls), remove: cls => classes.delete(cls) } };
  const idle = createIdleFade(element, 300000, clock);
  const advance = ms => {
    const end = now + ms;
    for (;;) {
      const next = [...timers].sort((a,b) => a[1].at-b[1].at)[0];
      if (!next || next[1].at > end) break;
      now = next[1].at; timers.delete(next[0]); next[1].fn();
    }
    now = end;
  };
  return { idle, advance, classes, timers };
}
test('fade starts after exactly five minutes of inactivity', () => {
  const { advance, classes } = setup(); advance(299999); assert.ok(!classes.has('idle'));
  advance(1); assert.ok(classes.has('idle'));
});
test('interaction restores opacity and restarts the full idle delay', () => {
  const { idle, advance, classes, timers } = setup(); advance(300000); idle.wake();
  assert.ok(!classes.has('idle')); assert.equal(timers.size, 1);
  advance(299999); assert.ok(!classes.has('idle')); advance(1); assert.ok(classes.has('idle'));
});
test('frequent activity uses one timer and fading waits for the last interaction', () => {
  const { idle, advance, classes, timers } = setup();
  for (let i=0;i<1000;i++) { idle.wake(); assert.equal(timers.size,1); }
  advance(299000); idle.wake(); advance(1000); assert.ok(!classes.has('idle'));
  advance(299000); assert.ok(classes.has('idle')); idle.dispose(); assert.equal(timers.size,0);
});
