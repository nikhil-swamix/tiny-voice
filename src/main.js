import './style.css';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { createIdleFade } from './idle.js';
import { createVoiceMeter } from './meter.js';
import { createMicrophone } from './microphone.js';
import { renderMarkdown } from './render.js';
import waveformUrl from './assets/waveform.svg';

const $ = id => document.getElementById(id);
const win = getCurrentWindow(), idle = createIdleFade(document.body);
for (const event of ['pointermove', 'pointerdown', 'keydown', 'input', 'wheel', 'focusin']) document.addEventListener(event, idle.wake, { passive: true });
let state = 'idle', stream, recorder, peer, sessionId, writes = Promise.resolve(), writeError, timer, cancelled = false, saveTask, storageTask, quitting = false, live = new Map(), processing = 0, pendingPaste = [], delivery = Promise.resolve(), mini = false, miniDim = false, miniHeight = 108, miniWidth = 180, resizeQueued = false, viewChange = Promise.resolve();
let meterStop, liveFrame;
let outputMode = 'quick';
try { if (localStorage.getItem('outputMode') === 'pro') outputMode = 'pro'; } catch {}
const inflight = new Map(), targets = new Map();
const microphone = createMicrophone({
  getMedia: () => navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true }, video: false }),
  onChange: () => updateEngine(),
  onLost: () => { if (state === 'recording') { $('status').textContent = 'Microphone unavailable · saving captured audio'; void stop(false); } else if (state === 'starting') cancelled = true; }
});
$('waveicon').src = waveformUrl;
function setView(compact, dim = false, height = 108, width = 180) {
  mini = compact; miniDim = compact && dim; miniHeight = Math.max(108, Math.floor(height)); miniWidth = Math.max(180, Math.floor(width));
  document.body.classList.toggle('mini', compact); document.body.classList.toggle('dim', miniDim);
  viewChange = viewChange.then(() => win.setSize(new LogicalSize(compact ? miniWidth : 300, compact ? miniHeight : 440))).catch(() => {});
  if (compact) schedulePopoverResize();
  return viewChange;
}
function schedulePopoverResize() {
  if (!mini || resizeQueued) return;
  resizeQueued = true;
  queueMicrotask(() => {
    resizeQueued = false;
    if (!mini) return;
    const max = Math.max(108, Math.floor((window.screen?.availHeight || 800) * .85));
    const height = Math.min(max, Math.max(108, $('popover').scrollHeight + 58));
    const width = $('popover').dataset.hasContent === 'true' ? 240 : 180;
    if (Math.abs(height - miniHeight) > 10 || width !== miniWidth) void setView(true, miniDim, height, width);
  });
}
function renderText(text, markdown = false) {
  for (const node of [$('text'), $('popover')]) {
    if (markdown) node.innerHTML = renderMarkdown(text);
    else node.textContent = text;
    node.classList.toggle('plain', !markdown); node.classList.toggle('empty', !text);
    node.scrollTop = markdown ? 0 : node.scrollHeight;
  }
  $('popover').dataset.hasContent = text ? 'true' : 'false';
  schedulePopoverResize();
}
function updateOutputControls() {
  $('turbo').disabled = !sessionId || inflight.has(sessionId) || ['recording', 'starting', 'stopping'].includes(state);
  $('turbo').setAttribute('aria-busy', String(inflight.has(sessionId)));
  for (const button of $('modes').querySelectorAll('button')) button.setAttribute('aria-pressed', String(button.dataset.mode === outputMode));
  updateEngine();
}
function updateEngine() {
  const activity = ({ starting: 'Starting microphone', recording: 'Recording your task', stopping: 'Saving audio', error: 'Waiting for a new recording' })[state]
    || (inflight.size ? 'Refining output · high reasoning' : 'Idle · no task assigned');
  const micState = microphone.ready ? 'mic ready' : 'mic off';
  $('engine').textContent = `${activity} · ${micState}`;
  $('meter').title = `${activity} · ${micState}`; $('meter').dataset.mic = microphone.ready ? 'ready' : 'off';
  $('mic').disabled = !microphone.ready || ['starting', 'stopping'].includes(state);
  $('mic').textContent = microphone.ready ? 'Release mic' : 'Mic off';
  $('mic').title = state === 'recording' ? 'Stop recording and release the microphone' : 'Release the microphone now; it otherwise stays ready for up to five minutes';
}
const status = (message, next = state) => {
  state = next; $('status').textContent = message; $('orb').dataset.state = next; $('orb').title = next === 'recording' ? 'Stop recording' : 'Start recording';
  $('orb').setAttribute('aria-label', next === 'recording' ? 'Stop recording' : 'Start recording');
  $('record').dataset.state = next; $('record').textContent = ({recording:'Stop recording',starting:'Cancel',stopping:'Saving…',processing:'Processing…'})[next] || 'Record';
  $('record').disabled = $('orb').disabled = next === 'stopping';
  updateOutputControls();
  void invoke('set_tray_state', { recording: next === 'recording' }).catch(() => {});
};
const release = (keepMic = false) => { clearTimeout(timer); cancelAnimationFrame(liveFrame); liveFrame = undefined; meterStop?.(); meterStop = undefined; peer?.close(); peer = undefined; if (keepMic) microphone.idle(); else microphone.release(); stream = undefined; };
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
      if (!liveFrame) liveFrame = requestAnimationFrame(() => { liveFrame = undefined; renderText([...live.values()].join('\n')); });
    } catch { /* Ignore unrelated or malformed events. */ }
  };
  pc.onconnectionstatechange = () => { if (['failed', 'disconnected'].includes(pc.connectionState) && state === 'recording') status('Recording locally · stream disconnected'); };
  const offer = await pc.createOffer(); await pc.setLocalDescription(offer);
  const answer = await invoke('connect_realtime', { sdp: offer.sdp });
  if (sessionId !== id || state !== 'recording') { pc.close(); return; }
  await pc.setRemoteDescription({ type: 'answer', sdp: answer });
}

async function start() {
  cancelled = false; sessionId = undefined; storageTask = undefined; writes = Promise.resolve(); writeError = undefined;
  status('Starting microphone…', 'starting'); live.clear(); renderText(''); if (mini) void setView(true, false);
  $('model').hidden = true; $('popover').title = '';
  const targetTask = invoke('capture_paste_target').catch(() => 0);
  try {
    stream = await microphone.get();
    if (cancelled) { release(); status('Ready', 'idle'); return; }
    const mimeType = ['audio/webm;codecs=opus', 'audio/webm'].find(t => MediaRecorder.isTypeSupported(t));
    if (!mimeType) throw Error('This WebView does not support WebM audio recording.');
    recorder = new MediaRecorder(stream, { mimeType, audioBitsPerSecond: 64000 }); writes = Promise.resolve(); writeError = undefined;
    const queuedAudio = [];
    recorder.ondataavailable = ({ data }) => {
      if (!data.size) return;
      const encoded = data.arrayBuffer().then(buffer => Array.from(new Uint8Array(buffer)));
      writes = writes.then(async () => {
        const bytes = await encoded;
        if (!sessionId) { queuedAudio.push(bytes); return; }
        await invoke('append_audio', { id: sessionId, bytes });
      }).catch(error => { writeError = error; if (state === 'recording') void stop(); });
    };
    recorder.onerror = event => { writeError = event.error || Error('Microphone recorder failed.'); if (state === 'recording') void stop(); };
    recorder.start(250); meterStop = createVoiceMeter(stream, $('meter')); renderText('Listening…'); status('Recording · Ctrl+Shift+Space to stop', 'recording'); beep(true);
    timer = setTimeout(() => void stop(), 10 * 60 * 1000);
    storageTask = (async () => {
      const id = await invoke('begin_recording');
      sessionId = id; targets.set(id, targetTask); updateOutputControls();
      writes = writes.then(async () => { for (const bytes of queuedAudio.splice(0)) await invoke('append_audio', { id, bytes }); });
      await writes;
      if (state === 'recording') void connectRealtime(id, stream).catch(error => { if (sessionId !== id) return; peer?.close(); peer = undefined; if (state === 'recording') status(`Recording locally · ${error.message || error}`); });
    })();
    await storageTask;
  } catch (error) { if (cancelled) { release(); status('Recording cancelled', 'idle'); } else await fail(error); }
}

async function processLast(id = sessionId, mode = outputMode, targetTask = targets.get(id)) {
  if (!id || ['recording', 'starting', 'stopping'].includes(state)) return;
  if (inflight.has(id)) return inflight.get(id);
  processing++; $('status').textContent = `${mode === 'pro' ? 'Pro' : 'Quick'} · processing…`;
  const task = (async () => {
    try {
      const result = await invoke('finish_recording', { id, mode });
      if (id === sessionId) {
        renderText(result.text, !result.warning);
        $('model').textContent = `${result.model || 'Model unverified'} · ${result.reasoning || 'unverified'} reasoning`;
        $('model').hidden = false; $('popover').title = $('model').textContent;
        void setView(true, true);
      }
      if (!result.text.trim()) { $('status').textContent = result.warning || 'No speech detected'; return; }
      const target = await (targetTask || 0);
      delivery = delivery.catch(() => {}).then(async () => {
        await invoke('copy_result', { text: result.text });
        if (state !== 'idle') { pendingPaste.push({ text: result.text, target }); return false; }
        return invoke('paste_result', { target });
      });
      const pasted = await delivery;
      if (id === sessionId && state !== 'recording' && state !== 'starting') {
        $('status').textContent = result.warning || `${mode === 'pro' ? 'Pro' : 'Quick'} · ${pasted ? 'copied and pasted' : state === 'idle' ? 'copied · paste target unavailable' : 'copied · paste queued'}`;
      }
    } catch (error) {
      if (id === sessionId && state !== 'recording' && state !== 'starting') $('status').textContent = String(error?.message || error);
    } finally {
      processing = Math.max(0, processing - 1); inflight.delete(id); updateOutputControls();
    }
  })();
  inflight.set(id, task); updateOutputControls(); return task;
}
function flushPendingPaste() {
  delivery = delivery.catch(() => {}).then(async () => {
    while (state === 'idle' && pendingPaste.length) {
      const { text, target } = pendingPaste[0];
      await invoke('copy_result', { text });
      const pasted = await invoke('paste_result', { target }); pendingPaste.shift();
      $('status').textContent = pasted ? 'Copied and pasted' : 'Copied · paste target unavailable';
    }
  }).catch(error => { $('status').textContent = String(error?.message || error); });
  return delivery;
}

async function stop(keepMic = true) {
  if (state !== 'recording') return;
  status('Saving audio…', 'stopping'); clearTimeout(timer);
  saveTask = (async () => {
    const active = recorder;
    if (active?.state !== 'inactive') await new Promise(resolve => { active.onstop = resolve; active.stop(); });
    release(keepMic && !quitting); beep(false); await storageTask; await writes; recorder = undefined;
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
$('mic').onclick = () => { if (state === 'recording') void stop(false); else microphone.release(); };
$('turbo').onclick = () => { idle.wake(); void processLast(sessionId, 'pro', invoke('capture_paste_target').catch(() => 0)); };
for (const button of $('modes').querySelectorAll('button')) button.onclick = () => {
  outputMode = button.dataset.mode; try { localStorage.setItem('outputMode', outputMode); } catch {} updateOutputControls();
};
$('folder').onclick = () => void invoke('open_audio_folder').catch(fail);
$('dragbar').onpointerdown = event => { if (event.button === 0 && !event.target.closest('button')) void win.startDragging().catch(fail); };
$('resize').onpointerdown = event => { if (event.button === 0) void win.startResizeDragging('SouthEast').catch(fail); };
$('minimize').onclick = () => void setView(true, false);
$('hide').onclick = () => { if (state !== 'recording' && state !== 'stopping') { cancelled = state === 'starting'; microphone.release(); } void win.hide().catch(fail); };
$('panel').addEventListener('click', event => { if (mini && !event.target.closest('button')) void setView(false); });
window.addEventListener('blur', () => { void setView(true, true); });
window.addEventListener('pagehide', () => microphone.release());
await listen('quit-requested', () => void quit());
await listen('widget-shown', () => { idle.wake(); void setView(false); });
await listen('toggle-recording', toggle);
status(await invoke('startup_status'));
void microphone.get().then(() => { if (state === 'idle' && !quitting) microphone.idle(); }).catch(() => { if (state === 'idle') updateEngine(); });
