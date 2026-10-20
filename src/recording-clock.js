export function createRecordingClock(node, { now = () => performance.now(), schedule = setInterval, cancel = clearInterval, progress = node.ownerDocument?.getElementById('timer-progress') } = {}) {
  let timer, started;
  const update = () => {
    const elapsed = Math.max(0, now() - started), seconds = Math.floor(elapsed / 1000);
    node.textContent = `${String(Math.floor(seconds / 60)).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
    node.dataset.warning = String(elapsed >= 60000);
    node.title = elapsed >= 60000 ? 'Over one minute · recording continues' : 'One-minute guide · recording does not stop at 01:00';
    node.setAttribute('aria-label', `${node.textContent} elapsed · recording continues past one minute`);
    if (progress) {
      progress.style.setProperty('--timer-progress', String(Math.min(1, elapsed / 60000)));
      progress.setAttribute('aria-valuenow', String(Math.min(60, seconds)));
      progress.setAttribute('aria-valuetext', `${Math.min(60, seconds)} of 60 seconds · ${node.textContent} elapsed`);
    }
  };
  return {
    start() { if (timer !== undefined) cancel(timer); started = now(); node.hidden = false; if (progress) progress.hidden = false; update(); timer = schedule(update, 500); },
    stop() { if (timer !== undefined) { cancel(timer); timer = undefined; } node.hidden = true; if (progress) progress.hidden = true; }
  };
}
