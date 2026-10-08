export function createIdleFade(element, delay = 300000, clock = { now: () => Date.now(), setTimeout: (fn, ms) => setTimeout(fn, ms), clearTimeout: id => clearTimeout(id) }) {
  let last = clock.now(), timer, faded = false;
  function check() {
    const remaining = delay - (clock.now() - last);
    if (remaining > 0) timer = clock.setTimeout(check, remaining);
    else { timer = undefined; faded = true; element.classList.add('idle'); }
  }
  function wake() {
    last = clock.now();
    if (faded) { faded = false; element.classList.remove('idle'); }
    if (timer === undefined) timer = clock.setTimeout(check, delay);
  }
  wake();
  return { wake, dispose: () => clock.clearTimeout(timer) };
}
