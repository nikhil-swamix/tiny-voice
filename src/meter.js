export function createVoiceMeter(media, node) {
  let context, frame;
  try {
    context = new AudioContext();
    const source = context.createMediaStreamSource(media), analyser = context.createAnalyser();
    analyser.fftSize = 256; source.connect(analyser); void context.resume().catch(() => {});
    const data = new Float32Array(analyser.fftSize);
    let level = 0, last = 0;
    const update = now => {
      frame = requestAnimationFrame(update);
      if (now - last < 33) return;
      last = now; analyser.getFloatTimeDomainData(data);
      const rms = Math.sqrt(data.reduce((sum, sample) => sum + sample * sample, 0) / data.length);
      level = level * .65 + Math.max(0, Math.min(1, (20 * Math.log10(rms || 1e-8) + 60) / 42)) * .35;
      node.style.setProperty('--voice', level.toFixed(3));
    };
    frame = requestAnimationFrame(update);
  } catch { /* Meter support must never block capture. */ }
  return () => { cancelAnimationFrame(frame); void context?.close().catch(() => {}); node.style.removeProperty('--voice'); };
}
