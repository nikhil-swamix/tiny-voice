export function createMicrophone({ getMedia, onChange = () => {}, onLost = () => {}, schedule = setTimeout, cancel = clearTimeout, timeout = 300000 }) {
  let media, pending, pendingGeneration, timer, generation = 0;
  const ready = () => !!media?.getAudioTracks().some(track => track.readyState !== 'ended' && !track.muted);
  function release(reason = 'released') {
    generation++; cancel(timer); timer = undefined;
    const old = media; media = undefined;
    for (const track of old?.getTracks() || []) { track.onmute = track.onended = null; track.stop(); }
    onChange(false, reason);
  }
  async function get() {
    cancel(timer); timer = undefined;
    if (ready()) return media;
    if (pending && pendingGeneration === generation) return pending;
    if (media) release('unavailable');
    const epoch = generation;
    const request = getMedia().then(source => {
      if (epoch !== generation) { source.getTracks().forEach(track => track.stop()); throw Error('Microphone request cancelled'); }
      media = source;
      const lost = () => { if (media !== source) return; release('unavailable'); onLost(); };
      for (const track of source.getAudioTracks()) { track.onmute = track.onended = lost; }
      onChange(true, 'ready'); return source;
    });
    pending = request; pendingGeneration = epoch;
    try { return await request; } finally { if (pending === request) pending = undefined; }
  }
  function idle() {
    cancel(timer); timer = undefined;
    if (!ready()) return;
    timer = schedule(() => release('timeout'), timeout);
    onChange(true, 'warm');
  }
  return { get, idle, release, get ready() { return ready(); } };
}
