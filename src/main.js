import './style.css';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { createIdleFade } from './idle.js';

const $ = id => document.getElementById(id);
const win = getCurrentWindow(), idle = createIdleFade(document.body);
for (const event of ['pointermove', 'pointerdown', 'keydown', 'input', 'wheel', 'focusin']) document.addEventListener(event, idle.wake, { passive: true });
let state = 'idle', stream, recorder, peer, sessionId, writes = Promise.resolve(), writeError, timer, cancelled = false, saveTask, quitting = false, live = new Map(), processing = 0, pendingPaste = [], delivery = Promise.resolve(), mini = false, viewChange = Promise.resolve();
function setView(compact, dim = false) {
  mini = compact; document.body.classList.toggle('mini', compact); document.body.classList.toggle('dim', compact && dim);
  viewChange = viewChange.then(() => win.setSize(new LogicalSize(compact ? 240 : 300, compact ? 144 : 440))).catch(() => {});
  return viewChange;
}
function renderText(text) {
  $('text').value = text; $('popover').textContent = text;
  $('popover').dataset.hasContent = text ? 'true' : 'false';
}
const status = (message, next = state) => {
  state = next; $('status').textContent = message; $('orb').dataset.state = next; $('orb').title = message;
  $('orb').setAttribute('aria-label', next === 'recording' ? 'Stop recording' : 'Start recording');
  $('record').dataset.state = next; $('record').textContent = ({recording:'Stop recording',starting:'Cancel',stopping:'Saving…',processing:'Processing…'})[next] || 'Record';
  $('record').disabled = $('orb').disabled = next === 'stopping';
  void invoke('set_tray_state', { recording: next === 'recording' }).catch(() => {});
};
const release = () => { clearTimeout(timer); peer?.close(); peer = undefined; stream?.getTracks().forEach(t => t.stop()); stream = undefined; };
const beep = started => { void invoke('recording_beep', { started }).catch(() => {}); };
const fail = async error => { release(); status(String(error?.message || error), 'error'); };

async function connectRealtime(id, media) {
  const pc = new RTCPeerConnection(); peer = pc;
  for (const track of media.getAudioTracks()) pc.addTrack(track, media);
  const channel = pc.createDataChannel('oai-events');
  channel.onmessage = ({ data }) => {
    if (sessionId !== id || !['recording', 'stopping'].includes(state)) return;
    try {
      const event = JSON.parse(data);
      if (event.type === 'error') { status(`Recording locally · ${event.error?.message || 'stream error'}`); return; }
      if (event.type?.endsWith('input_audio_transcription.delta')) live.set(event.item_id, (live.get(event.item_id) || '') + event.delta);
      if (event.type?.endsWith('input_audio_transcription.completed')) live.set(event.item_id, event.transcript);
      renderText([...live.values()].join('\n'));
    } catch { /* Ignore unrelated or malformed events. */ }
  };
  pc.onconnectionstatechange = () => { if (['failed', 'disconnected'].includes(pc.connectionState) && state === 'recording') status('Recording locally · stream disconnected'); };
  const offer = await pc.createOffer(); await pc.setLocalDescription(offer);
  const answer = await invoke('connect_realtime', { sdp: offer.sdp });
  if (sessionId !== id || state !== 'recording') { pc.close(); return; }
  await pc.setRemoteDescription({ type: 'answer', sdp: answer });
}

async function start() {
  cancelled = false; status('Starting microphone…', 'starting'); live.clear(); renderText(''); if (mini) document.body.classList.remove('dim');
  try {
    stream = await navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true }, video: false });
    if (cancelled) { release(); status('Ready', 'idle'); return; }
    sessionId = await invoke('begin_recording');
    if (cancelled) { release(); status('Ready', 'idle'); return; }
    const mimeType = ['audio/webm;codecs=opus', 'audio/webm'].find(t => MediaRecorder.isTypeSupported(t));
    if (!mimeType) throw Error('This WebView does not support WebM audio recording.');
    recorder = new MediaRecorder(stream, { mimeType, audioBitsPerSecond: 64000 }); writes = Promise.resolve(); writeError = undefined;
    const id = sessionId;
    recorder.ondataavailable = ({ data }) => {
      if (!data.size) return;
      writes = writes.then(async () => { await invoke('append_audio', { id, bytes: Array.from(new Uint8Array(await data.arrayBuffer())) }); }).catch(error => { writeError = error; if (state === 'recording') void stop(); });
    };
    recorder.onerror = event => { writeError = event.error || Error('Microphone recorder failed.'); if (state === 'recording') void stop(); };
    stream.getAudioTracks()[0].onended = () => { if (state === 'recording') void stop(); };
    recorder.start(1000); renderText('Listening…'); status('Recording · Ctrl+Shift+Space to stop', 'recording'); beep(true);
    timer = setTimeout(() => void stop(), 10 * 60 * 1000);
    void connectRealtime(id, stream).catch(error => { if (sessionId !== id) return; peer?.close(); peer = undefined; if (state === 'recording') status(`Recording locally · ${error.message || error}`); });
  } catch (error) { await fail(error); }
}

async function processLast(id = sessionId) {
  if (!id || ['recording', 'starting', 'stopping'].includes(state)) return;
  processing++; if (state === 'idle') $('status').textContent = `Processing ${processing} speech${processing === 1 ? '' : 'es'} · recording is available`;
  try {
    const result = await invoke('finish_recording', { id });
    if (id === sessionId) { renderText(result.text); void setView(true, true); }
    delivery = delivery.then(async () => {
      await invoke('copy_result', { text: result.text });
      if (state !== 'idle') { pendingPaste.push(result.text); return false; }
      return invoke('paste_result');
    });
    const pasted = await delivery;
    if (id === sessionId && state !== 'recording' && state !== 'starting') {
      $('status').textContent = result.warning || (pasted ? 'Copied and pasted' : state === 'idle' ? 'Copied · paste target unavailable' : 'Copied · paste queued');
    }
  } catch (error) {
    if (id === sessionId && state !== 'recording' && state !== 'starting') $('status').textContent = String(error?.message || error);
  } finally {
    processing = Math.max(0, processing - 1);
    if (state === 'idle' && processing) $('status').textContent = `Processing ${processing} speech${processing === 1 ? '' : 'es'} · recording is available`;
  }
}
function flushPendingPaste() {
  delivery = delivery.then(async () => {
    while (state === 'idle' && pendingPaste.length) {
      await invoke('copy_result', { text: pendingPaste.shift() });
      const pasted = await invoke('paste_result');
      $('status').textContent = pasted ? 'Copied and pasted' : 'Copied · paste target unavailable';
    }
  });
  return delivery;
}

async function stop() {
  if (state !== 'recording') return;
  status('Saving audio…', 'stopping'); clearTimeout(timer);
  saveTask = (async () => {
    const active = recorder;
    if (active?.state !== 'inactive') await new Promise(resolve => { active.onstop = resolve; active.stop(); });
    release(); beep(false); await writes; recorder = undefined;
    if (writeError) throw writeError;
    status('Audio saved', 'idle'); void setView(true, true); await flushPendingPaste();
  })();
  try {
    await saveTask;
    if (!quitting) void processLast(sessionId);
  } catch (error) { await fail(error); }
}
async function quit() {
  quitting = true; cancelled = true;
  if (state === 'recording') await stop();
  else if (state === 'stopping') await saveTask.catch(() => {});
  release(); idle.dispose(); await invoke('quit_app');
}
const toggle = () => { idle.wake(); if (state === 'starting') cancelled = true; else if (state === 'recording') void stop(); else if (state === 'idle' || state === 'error') void start(); };
$('orb').onclick = $('record').onclick = toggle;
$('folder').onclick = () => void invoke('open_audio_folder').catch(fail);
$('dragbar').onpointerdown = event => { if (event.button === 0 && !event.target.closest('button')) void win.startDragging().catch(fail); };
$('resize').onpointerdown = event => { if (event.button === 0) void win.startResizeDragging('SouthEast').catch(fail); };
$('minimize').onclick = () => void setView(true, false);
$('hide').onclick = () => void win.hide().catch(fail);
$('panel').addEventListener('click', event => { if (mini && !event.target.closest('#hide, #minimize')) void setView(false); });
window.addEventListener('blur', () => { void setView(true, true); });
await listen('quit-requested', () => void quit());
await listen('widget-shown', () => { idle.wake(); void setView(false); });
await listen('toggle-recording', toggle);
status(await invoke('startup_status'));
