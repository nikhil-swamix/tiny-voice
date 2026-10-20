export function createOutputLife(collapse, { schedule = setTimeout, cancel = clearTimeout } = {}) {
  let timer, generation = 0;
  const stop = () => { generation++; if (timer !== undefined) cancel(timer); timer = undefined; };
  return {
    start() { stop(); const current = generation; timer = schedule(() => { if (current !== generation) return; timer = undefined; collapse(); }, 10000); },
    stop
  };
}
