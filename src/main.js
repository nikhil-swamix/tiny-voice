import './style.css';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, LogicalPosition, LogicalSize } from '@tauri-apps/api/window';
import { createVoiceMeter } from './meter.js';
import { createMicrophone } from './microphone.js';
import { renderMarkdown } from './render.js';
import { createRecordingClock } from './recording-clock.js';
import { createOutputLife } from './output-life.js';

const $ = id => document.getElementById(id);
const iconPaths = {
  mic: '<rect x="9" y="2" width="6" height="12" rx="3"/><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3m-4 0h8"/>',
  'mic-off': '<path d="M9 9v3a3 3 0 0 0 5.1 2.1M15 9V5a3 3 0 0 0-5.8-1M5 10v2a7 7 0 0 0 11.7 5.2M19 10v2a7 7 0 0 1-.4 2.3M12 19v3m-4 0h8M3 3l18 18"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="2"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/>',
  history: '<path d="M3 12a9 9 0 1 0 2.6-6.4L3 8"/><path d="M3 3v5h5m4-1v5l3 2"/>',
  expand: '<path d="M15 3h6v6m-7 1 7-7M9 21H3v-6m7-1-7 7"/>',
  collapse: '<path d="M8 3v5H3m13-5v5h5M8 21v-5H3m13 5v-5h5"/>',
  close: '<path d="m18 6-12 12M6 6l12 12"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M3 10h18"/>',
  save: '<path d="M20 7v12a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h10z"/><path d="M8 3v6h8V3m-8 18v-7h8v7"/>',
  normal: '<path d="M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01"/>',
  settings: '<path d="M4 7h16M4 17h16"/><circle cx="9" cy="7" r="3" fill="currentColor"/><circle cx="15" cy="17" r="3" fill="currentColor"/>',
  refresh: '<path d="M20 7V3l-3 3a8 8 0 0 0-13 6m0 5v4l3-3a8 8 0 0 0 13-6M20 7h-4M4 17h4"/>'
};
function iconButton(button, name, hint) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24'); svg.setAttribute('aria-hidden', 'true'); svg.setAttribute('focusable', 'false');
  svg.setAttribute('fill', 'none'); svg.setAttribute('stroke', 'currentColor'); svg.setAttribute('stroke-width', '2');
  svg.setAttribute('stroke-linecap', 'round'); svg.setAttribute('stroke-linejoin', 'round'); svg.innerHTML = iconPaths[name];
  button.classList.add('icon-button'); button.replaceChildren(svg); button.title = hint; button.setAttribute('aria-label', hint);
}
iconButton($('record'), 'mic', 'Start recording');
iconButton($('copy'), 'copy', 'Copy final output'); iconButton($('history'), 'history', 'Browse history');
iconButton($('minimize'), 'expand', 'Expand view'); iconButton($('hide'), 'close', 'Hide to tray');
iconButton($('mic'), 'mic', 'Microphone off'); iconButton($('folder'), 'folder', 'Open audio folder');
iconButton($('settings'), 'settings', 'Processing settings'); iconButton($('refresh-vocabulary'), 'refresh', 'Scan saved transcripts for pronunciation hints');
iconButton($('normal-mode'), 'normal', 'Faithful dictation · minimal formatting');
const win = getCurrentWindow();
const clock = createRecordingClock($('clock'));
let finalOutput = '', finalOutputFormat = '';
const widgetWidth = Math.min(360, Math.max(1, Math.floor((window.screen?.availWidth || 392) - 32)));
const compactHeight = 176;
const expandedHeight = Math.min(520, Math.max(1, Math.floor((window.screen?.availHeight || 800) * .85)));
function anchorRecordingBar() {
  const screenWidth = window.screen?.availWidth || 800, screenHeight = window.screen?.availHeight || 600, height = mini ? compactHeight : expandedHeight;
  return win.setPosition(new LogicalPosition(Math.max(0, (screenWidth - widgetWidth) / 2), Math.max(0, screenHeight - height - 16)));
}
let state = 'idle', stream, recorder, sessionId, streamConnectedId = '', writes = Promise.resolve(), writeError, timer, cancelled = false, saveTask, storageTask, quitting = false, openedByTray = false, live = new Map(), processing = 0, pendingPaste = [], delivery = Promise.resolve(), mini = true, viewChange = Promise.resolve();
let meterStop, liveFrame;
const outputLife = createOutputLife(() => {
  if (processing || ['recording', 'starting', 'stopping'].includes(state)) return;
  document.body.classList.add('dormant'); $('history-viewer').hidden = true;
  $('history-list').replaceChildren(); $('history').setAttribute('aria-expanded', 'false');
  finalOutput = ''; finalOutputFormat = ''; renderText(''); void win.hide().catch(() => {});
});
let preferences = {
  model: 'gpt-6.1-sol', reasoning: 'low', sendImages: false, language: 'en-IN',
  postProcessingProfile: 'local-sglang', localEndpoint: 'http://127.0.0.1:30000/v1',
  localModel: 'minicpm5-2b-fp8', bypassGptTranscribe: true
};
let settingsWrite = Promise.resolve();
try { localStorage.removeItem('outputMode'); localStorage.removeItem('outputDefaults'); } catch {}
const inflight = new Map(), targets = new Map();
const microphone = createMicrophone({
  getMedia: () => navigator.mediaDevices.getUserMedia({ audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true }, video: false }),
  onChange: () => updateEngine(),
  onLost: () => { if (state === 'recording') { $('status').textContent = 'Microphone unavailable · saving captured audio'; void stop(false); } else if (state === 'starting') cancelled = true; }
});
let viewSerial = 0, appliedSize = '', dragStart;
function setView(compact) {
  const changed = mini !== compact;
  mini = compact;
  document.body.classList.toggle('mini', compact);
  displayTranscript();
  $('minimize').setAttribute('aria-expanded', String(!compact));
  iconButton($('minimize'), compact ? 'expand' : 'collapse', compact ? 'Expand view' : 'Collapse view');
  const height = compact ? compactHeight : expandedHeight, signature = `${widgetWidth}x${height}`;
  if (!changed && signature === appliedSize) return viewChange;
  const serial = ++viewSerial;
  viewChange = viewChange.then(async () => {
    if (serial !== viewSerial || signature === appliedSize) return;
    await win.setMinSize?.(new LogicalSize(widgetWidth, height));
    await win.setMaxSize?.(new LogicalSize(widgetWidth, Math.max(compactHeight, expandedHeight)));
    await win.setSize(new LogicalSize(widgetWidth, height)); appliedSize = signature;
    if (serial !== viewSerial) return;
    // Cold microphone permission owns focus until setup finishes.
    if (!compact || state !== 'starting' || microphone.ready) await win.setFocusable?.(!compact);
    await anchorRecordingBar();
  }).catch(() => {});
  return viewChange;
}
let renderedText = '', renderedMarkdown = '', renderedPreview = '', renderedAsMarkdown = false;
const previewSource = document.createElement('div');
$('popover').setAttribute('role', 'button'); $('popover').tabIndex = 0;
function markdownPreview(markup) {
  previewSource.innerHTML = markup;
  for (const boundary of previewSource.querySelectorAll('br, p, h1, h2, h3, h4, h5, h6, li, pre, blockquote, hr, tr, th, td')) boundary.after(document.createTextNode(' '));
  const preview = previewSource.textContent.replace(/\s+/g, ' ').trim();
  previewSource.replaceChildren();
  return preview;
}
function field(value, ...names) {
  for (const name of names) if (value?.[name] !== undefined && value?.[name] !== null) return value[name];
}
function durationLabel(value) {
  const ms = Number(value);
  if (!Number.isFinite(ms) || ms < 0) return '';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10000 ? 1 : 0)} s`;
}
function pipelineText(result, runPreferences = preferences) {
  const profile = String(field(result, 'postProcessingProfile', 'post_processing_profile') || runPreferences.postProcessingProfile || '');
  const transcriptionModel = field(result, 'transcriptionModel', 'transcription_model');
  const formattingModel = field(result, 'model');
  const reasoning = field(result, 'reasoning');
  const transcriptionMs = field(result, 'transcriptionMs', 'transcription_ms');
  const formattingMs = field(result, 'formattingMs', 'formatting_ms');
  const hasMetadata = transcriptionModel !== undefined || field(result, 'postProcessingProfile', 'post_processing_profile') !== undefined
    || formattingModel !== undefined || transcriptionMs !== undefined || formattingMs !== undefined;
  if (result.warning && !hasMetadata) return '';
  const bypassed = transcriptionModel ? /nemotron/i.test(String(transcriptionModel)) : !!runPreferences.bypassGptTranscribe;
  const transcription = bypassed
    ? (transcriptionModel ? `Live transcript · ${transcriptionModel}` : 'Nemotron live transcript')
    : (transcriptionModel ? `GPT Transcribe · ${transcriptionModel}` : 'GPT Transcribe');
  let formatting = result.warning && !formattingModel ? 'raw transcript · formatting unavailable'
    : profile === 'openai' ? 'OpenAI' : profile === 'local-sglang' ? 'local SGLang' : 'post-processing';
  if (!result.warning || formattingModel) {
    if (formattingModel) formatting += ` (${formattingModel})`;
    if (reasoning) formatting += ` · ${reasoning} reasoning`;
  }
  const transcribeTime = durationLabel(transcriptionMs), formatTime = durationLabel(formattingMs);
  const timing = [transcribeTime && `transcription ${transcribeTime}`, formatTime && `formatting ${formatTime}`].filter(Boolean);
  const total = durationLabel(Number(transcriptionMs) + Number(formattingMs));
  if (transcribeTime && formatTime && total) timing.push(`stage sum ${total}`);
  return `${transcription} → ${formatting}${timing.length ? ` · ${timing.join(' · ')}` : ''}`;
}
function renderOutput(text, format, legacyMarkdown = true) {
  const kind = String(format || '').toLowerCase();
  if (kind === 'yaml' || kind === 'yml') {
    const longestFence = Math.max(0, ...Array.from(text.matchAll(/`+/g), match => match[0].length));
    const fence = '`'.repeat(Math.max(3, longestFence + 1));
    renderText(`${fence}yaml\n${text}\n${fence}`, true);
  } else if (kind === 'text' || kind === 'plain') renderText(text, false);
  else renderText(text, legacyMarkdown);
  return kind === 'yml' ? 'yaml' : kind;
}
function setCopyLabel(format) {
  const label = format === 'yaml' ? 'Copy YAML output' : 'Copy final output';
  $('copy').title = label; $('copy').setAttribute('aria-label', label);
}
function displayTranscript() {
  const preview = $('popover'), output = $('text'), hasContent = !!renderedPreview;
  const hint = ({ starting: 'Starting microphone…', recording: 'Listening… Your words will appear here.', stopping: 'Saving your recording…' })[state]
    || (processing ? 'Refining your recording…' : 'Ready · start recording');
  const text = renderedPreview || hint;
  if (preview.textContent !== text) preview.textContent = text;
  preview.classList.add('plain'); preview.classList.toggle('empty', !hasContent);
  preview.dataset.hasContent = String(hasContent);
  preview.setAttribute('aria-disabled', String(!hasContent));
  preview.setAttribute('aria-expanded', String(!mini));
  preview.setAttribute('aria-label', hasContent ? (mini ? 'Expand transcript preview' : 'Transcript preview, expanded') : hint);
  if (!hasContent) {
    if (output.textContent !== hint) output.textContent = hint;
    output.classList.add('plain', 'empty');
  }
}
function renderText(text, markdown = false) {
  text = text || '';
  if (text !== renderedText || markdown !== renderedAsMarkdown) {
    const output = $('text'), markup = markdown ? renderMarkdown(text) : '';
    if (markdown) {
      if (markup !== renderedMarkdown || !renderedAsMarkdown) output.innerHTML = markup;
    } else if (output.textContent !== text || renderedAsMarkdown) output.textContent = text;
    renderedText = text; renderedMarkdown = markup; renderedAsMarkdown = markdown;
    renderedPreview = markdown ? markdownPreview(markup) : text.replace(/\s+/g, ' ').trim();
    output.classList.toggle('plain', !markdown); output.classList.toggle('empty', !renderedPreview);
    output.scrollTop = markdown ? 0 : output.scrollHeight;
  }
  displayTranscript();
}
function updateOutputControls() {
  updateEngine();
}
function updateReadyOutline() {
  const ready = state === 'recording' && microphone.ready && streamConnectedId === sessionId;
  document.body.classList.toggle('stream-ready', ready);
  $('panel').title = ready ? 'Microphone ready · Nemotron CPU stream connected' : '';
}
function updateEngine() {
  const processingActivity = preferences.bypassGptTranscribe ? 'Refining output · Nemotron transcript'
    : preferences.postProcessingProfile === 'openai' ? `Refining output · OpenAI · ${preferences.reasoning} reasoning` : 'Refining output · local SGLang';
  const activity = ({ starting: 'Starting microphone', recording: 'Recording your task', stopping: 'Saving audio', error: 'Waiting for a new recording' })[state]
    || (processing ? processingActivity : 'Idle · no task assigned');
  const micState = microphone.ready ? 'mic ready' : 'mic off';
  $('engine').textContent = `${activity} · ${micState}`;
  $('meter').title = `${activity} · ${micState}`; $('meter').dataset.mic = microphone.ready ? 'ready' : 'off';
  $('mic').disabled = !microphone.ready || ['starting', 'stopping'].includes(state);
  const micHint = state === 'recording' ? 'Stop recording and release the microphone' : microphone.ready ? 'Release the microphone · stays ready for up to five minutes' : 'Microphone off';
  iconButton($('mic'), microphone.ready ? 'mic-off' : 'mic', micHint);
  updateReadyOutline();
  const phase = state === 'error' ? 'error' : ['starting', 'stopping'].includes(state) ? 'connecting' : state === 'recording' ? (streamConnectedId === sessionId ? 'recording' : 'connecting') : processing ? 'processing' : 'ready';
  $('activity').dataset.phase = phase; $('activity').setAttribute('aria-label', activity); $('activity').title = `${activity} · ${micState}`;
  displayTranscript();
}
const status = (message, next = state) => {
  state = next; $('status').textContent = message; $('orb').dataset.state = next;
  document.body.classList.toggle('capturing', next === 'recording');
  $('record').dataset.state = next;
  const recordHint = ({recording:'Stop recording',starting:'Cancel recording start',stopping:'Saving recording',processing:'Processing recording'})[next] || 'Start recording';
  iconButton($('record'), next === 'recording' || next === 'stopping' ? 'stop' : 'mic', recordHint);
  $('record').disabled = next === 'stopping';
  $('record').setAttribute('aria-pressed', String(['starting', 'recording', 'stopping'].includes(next)));
  updateOutputControls();
  void invoke('set_tray_state', { recording: next === 'recording' }).catch(() => {});
};
const release = (keepMic = false) => { clock.stop(); clearTimeout(timer); cancelAnimationFrame(liveFrame); liveFrame = undefined; meterStop?.(); meterStop = undefined; if (keepMic) microphone.idle(); else microphone.release(); stream = undefined; };
const beep = started => { void invoke('recording_beep', { started }).catch(() => {}); };
const fail = async error => {
  if (sessionId) { void invoke('abort_live', {id: sessionId}).catch(() => {}); targets.delete(sessionId); }
  release(); status(String(error?.message || error), 'error');
  document.body.classList.remove('dormant'); void win.show().catch(() => {}); void setView(false);
  outputLife.start();
};

window.addEventListener('error', event => {
  const message = event.error?.message || event.message || 'Unexpected interface error';
  $('status').textContent = `Widget error · ${message}`;
  document.body.classList.remove('dormant'); void win.show().catch(() => {}); void setView(false);
});
window.addEventListener('unhandledrejection', event => {
  const message = event.reason?.message || String(event.reason || 'Unknown startup error');
  $('status').textContent = `Widget error · ${message}`;
  document.body.classList.remove('dormant'); void win.show().catch(() => {}); void setView(false);
});


async function start() {
  const targetTask = invoke('capture_paste_target').catch(() => null);
  outputLife.stop();
  $('settings-view').hidden = true; $('settings').setAttribute('aria-expanded', 'false');
  document.body.classList.remove('dormant'); $('history-viewer').hidden = true;
  $('history-list').replaceChildren(); $('history').setAttribute('aria-expanded', 'false');
  cancelled = false; sessionId = undefined; streamConnectedId = ''; updateReadyOutline(); storageTask = undefined; writes = Promise.resolve(); writeError = undefined;
  finalOutput = ''; finalOutputFormat = ''; setCopyLabel(''); $('copy').disabled = true;
  status('Starting microphone…', 'starting'); live.clear(); renderText('');
  $('model').hidden = true; $('popover').title = '';
  try {
    // Window geometry is independent of capture; only cold permission needs focus.
    const shown = win.show();
    void setView(true).then(anchorRecordingBar).catch(() => {});
    const warm = microphone.ready;
    if (!warm) { await shown; await win.setFocusable?.(true); }
    else void shown.catch(() => {});
    try { stream = await microphone.get(); }
    finally { if (!warm) await win.setFocusable?.(false).catch(() => {}); }
    if (cancelled) { release(); status('Ready', 'idle'); outputLife.start(); return; }
    const mimeType = ['audio/webm;codecs=opus', 'audio/webm'].find(t => MediaRecorder.isTypeSupported(t));
    if (!mimeType) throw Error('This WebView does not support WebM audio recording.');
    recorder = new MediaRecorder(stream, { mimeType, audioBitsPerSecond: 64000 }); writes = Promise.resolve(); writeError = undefined;
    const recordingTask = invoke('begin_recording').then(async id => {
      sessionId = id; targets.set(id, targetTask); updateOutputControls();
      if (state !== 'recording') await invoke('stop_screen_context', { id });
      return id;
    });
    storageTask = recordingTask;
    recorder.ondataavailable = ({ data }) => {
      if (!data.size) return;
      const encoded = data.arrayBuffer().then(buffer => new Uint8Array(buffer));
      // One FIFO covers early and late chunks; setup cannot let later audio overtake the WebM header.
      writes = writes.then(async () => {
        const id = await recordingTask, bytes = await encoded;
        await invoke('append_audio', bytes, { headers: { 'x-session-id': id } });
      }).catch(error => { writeError = error; if (state === 'recording') void stop(); });
    };
    recorder.onerror = event => { writeError = event.error || Error('Microphone recorder failed.'); if (state === 'recording') void stop(); };
    recorder.start(100); meterStop = createVoiceMeter(stream, $('meter')); renderText(''); status('Recording', 'recording'); beep(true);
    clock.start(); void viewChange.then(anchorRecordingBar).catch(() => {});
    timer = setTimeout(() => void stop(), 10 * 60 * 1000);
    await recordingTask;
  } catch (error) { if (cancelled) { release(); status('Recording cancelled', 'idle'); outputLife.start(); } else await fail(error); }
}

async function processLast(id = sessionId, targetTask = targets.get(id)) {
  if (!id || ['recording', 'starting', 'stopping'].includes(state)) return;
  if (inflight.has(id)) return inflight.get(id);
  processing++;
  const runPreferences = { ...preferences };
  $('status').textContent = runPreferences.bypassGptTranscribe
    ? 'Finalizing Nemotron transcript · no audio upload…'
    : `GPT Transcribe · refining audio, then ${runPreferences.postProcessingProfile === 'openai' ? 'OpenAI' : 'local SGLang'} formatting…`;
  const task = (async () => {
    try {
      const result = await invoke('finish_recording', { id });
      if (!result.text?.trim()) {
        if (id === sessionId) {
          finalOutput = ''; finalOutputFormat = ''; renderText(''); setCopyLabel('');
          $('copy').hidden = true; $('copy').disabled = true; $('model').hidden = true; $('popover').title = '';
          $('status').textContent = result.warning || 'No speech detected'; outputLife.start();
        }
        return;
      }
      if (id === sessionId) {
        finalOutput = result.text; $('copy').hidden = false; $('copy').disabled = false;
        finalOutputFormat = renderOutput(result.text, field(result, 'outputFormat', 'output_format'), !result.warning);
        setCopyLabel(finalOutputFormat);
        const pipeline = pipelineText(result, runPreferences);
        $('model').textContent = pipeline; $('model').hidden = !pipeline;
        $('popover').title = pipeline;
      }
      const target = await (targetTask || null);
      delivery = delivery.catch(() => {}).then(async () => {
        await invoke('copy_result', { text: result.text });
        if (state !== 'idle') { pendingPaste.push({ text: result.text, target }); return false; }
        return invoke('paste_result', { target });
      });
      const pasted = await delivery;
      if (id === sessionId && state !== 'recording' && state !== 'starting') {
        $('status').textContent = result.warning || `Dictation · ${pasted ? 'copied and pasted' : state === 'idle' ? 'copied · paste skipped' : 'copied · paste queued'}`;
        if (state === 'idle') outputLife.start();
      }
    } catch (error) {
      if (id === sessionId && state !== 'recording' && state !== 'starting') {
        if (!live.get('local')?.trim()) {
          finalOutput = ''; finalOutputFormat = ''; renderText(''); setCopyLabel('');
          $('copy').hidden = true; $('copy').disabled = true; $('model').hidden = true; $('popover').title = '';
        }
        else outputLife.start();
        $('status').textContent = String(error?.message || error);
      }
    } finally {
      processing = Math.max(0, processing - 1); inflight.delete(id); targets.delete(id); live.clear(); updateOutputControls();
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
      $('status').textContent = pasted ? 'Copied and pasted' : 'Copied · paste skipped';
    }
    if (state === 'idle' && pendingPaste.length === 0) outputLife.start();
  }).catch(error => { $('status').textContent = String(error?.message || error); });
  return delivery;
}

async function stop(keepMic = true) {
  if (state !== 'recording') return;
  status('Saving audio…', 'stopping'); clearTimeout(timer);
  clock.stop();
  if (sessionId) void invoke('stop_screen_context', { id: sessionId }).catch(() => {});
  saveTask = (async () => {
    const active = recorder;
    if (active?.state !== 'inactive') await new Promise(resolve => { active.onstop = resolve; active.stop(); });
    release(keepMic && !quitting); beep(false); await storageTask; await writes; recorder = undefined;
    if (writeError) throw writeError;
    status('Audio saved', 'idle'); void flushPendingPaste();
  })();
  try {
    await saveTask;
    if (!quitting) void processLast(sessionId);
  } catch (error) { await fail(error); }
}
async function quit() {
  outputLife.stop();
  quitting = true; cancelled = true;
  if (state === 'recording') await stop();
  else if (state === 'stopping') await saveTask.catch(() => {});
  if (sessionId) void invoke('abort_live', {id: sessionId}).catch(() => {});
  release(); await invoke('quit_app');
}
const toggle = () => { if (state === 'starting') cancelled = true; else if (state === 'recording') void stop(); else if (state === 'idle' || state === 'error') void start(); };
// Bind activation before the remaining startup IPC so the first global press is not dropped.
await listen('toggle-recording', toggle);
$('record').onclick = toggle;
$('copy').onclick = async () => {
  if (!finalOutput) return;
  try {
    await invoke('copy_result', { text: finalOutput });
    $('copy').title = finalOutputFormat === 'yaml' ? 'YAML output copied' : 'Final output copied';
    outputLife.start();
  }
  catch (error) { $('status').textContent = `Copy failed: ${String(error?.message || error)}`; }
};
$('mic').onclick = () => { if (state === 'recording') void stop(false); else microphone.release(); };
$('folder').onclick = () => void invoke('open_audio_folder').catch(fail);
$('history').onclick = async () => {
  outputLife.stop(); document.body.classList.remove('dormant');
  $('settings-view').hidden = true; $('settings').setAttribute('aria-expanded', 'false');
  await win.setFocusable?.(true);
  try {
    await win.show(); await setView(false);
    $('history-viewer').hidden = false; $('history').setAttribute('aria-expanded', 'true');
    $('history-list').replaceChildren();
    const entries = await invoke('history_list');
    if (!entries.length) { const empty = document.createElement('small'); empty.textContent = 'No saved recordings yet'; $('history-list').append(empty); }
    let cursor = 0;
    const appendBatch = () => {
      const end = Math.min(cursor + 50, entries.length);
      while (cursor < end) {
        const entry = entries[cursor++], button = document.createElement('button'), title = document.createElement('span'), time = document.createElement('time');
        button.type = 'button'; button.className = 'history-item'; title.className = 'history-title'; title.textContent = entry.title;
        time.dateTime = new Date(Number(entry.timestampMs)).toISOString(); time.textContent = new Date(Number(entry.timestampMs)).toLocaleString();
        button.append(title,time); button.onclick = async () => {
          try {
            outputLife.stop();
            const stored = await invoke('history_entry', {id: entry.id});
            const source = typeof stored === 'string' ? { text: stored, format: '' }
              : { text: String(stored?.text ?? ''), format: field(stored, 'outputFormat', 'output_format') || '' };
            finalOutput = source.text;
            $('copy').hidden = $('copy').disabled = !finalOutput.trim(); $('model').hidden = true;
            finalOutputFormat = renderOutput(finalOutput, source.format, true);
            setCopyLabel(finalOutputFormat); await setView(false);
            $('status').textContent = `History preview · ${entry.title}`;
            for (const item of $('history-list').children) item.removeAttribute('aria-current');
            button.setAttribute('aria-current','true');
          } catch (error) { $('status').textContent = String(error?.message || error); }
        };
        $('history-list').append(button);
      }
      if (cursor < entries.length) {
        const more = document.createElement('button'); more.className = 'history-more'; more.textContent = 'Load 50 older recordings';
        more.onclick = () => { more.remove(); appendBatch(); }; $('history-list').append(more);
      }
    };
    appendBatch();
  } catch (error) { $('status').textContent = String(error?.message || error); }
};
$('dragbar').onpointerdown = event => { if (event.button === 0 && !event.target.closest('button')) dragStart = [event.clientX, event.clientY]; };
window.addEventListener('pointermove', event => {
  if (dragStart && Math.hypot(event.clientX - dragStart[0], event.clientY - dragStart[1]) > 6) {
    dragStart = undefined; void win.startDragging().catch(fail);
  }
});
window.addEventListener('pointerup', () => { dragStart = undefined; });
$('minimize').onclick = () => { void setView(!mini); };
const expandPreview = () => {
  if (mini && renderedPreview) void setView(false).then(() => { if (!mini) $('text').focus({ preventScroll: true }); });
};
$('popover').onclick = expandPreview;
$('popover').onkeydown = event => {
  if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); expandPreview(); }
};
$('hide').onclick = () => {
  if (state !== 'recording' && state !== 'stopping') {
    cancelled = state === 'starting'; microphone.release(); outputLife.stop(); finalOutput = ''; finalOutputFormat = ''; setCopyLabel(''); renderText('');
    document.body.classList.add('dormant'); $('history-viewer').hidden = true; $('history-list').replaceChildren();
  }
  void win.hide().catch(fail);
};
window.addEventListener('pagehide', () => microphone.release());
function showVocabulary(terms) {
  $('vocabulary-info').textContent = `${terms.length} observed pronunciation hints`;
  $('vocabulary-info').title = terms.join(', ') || 'Recurring vocabulary appears after saved recordings';
}
function syncProfileFields() {
  const local = $('setting-profile').value === 'local-sglang', bypass = !$('setting-gpt-transcribe').checked;
  $('local-endpoint-setting').hidden = $('setting-local-endpoint').disabled = !local;
  $('local-model-setting').hidden = $('setting-local-model').disabled = !local;
  $('cloud-model-setting').hidden = $('setting-model').disabled = local;
  $('cloud-reasoning-setting').hidden = $('setting-reasoning').disabled = local;
  $('image-setting').hidden = $('setting-images').disabled = local;
  if (local) $('setting-images').checked = false;
  $('transcription-language-setting').classList.toggle('is-disabled', bypass);
  $('setting-language').disabled = bypass;
  $('bypass-note').dataset.active = String(!bypass);
}
function fillPreferences() {
  $('setting-profile').value = preferences.postProcessingProfile || 'local-sglang';
  $('setting-local-endpoint').value = preferences.localEndpoint || 'http://127.0.0.1:30000/v1';
  $('setting-local-model').value = preferences.localModel || 'minicpm5-2b-fp8';
  $('setting-model').value = preferences.model; $('setting-reasoning').value = preferences.reasoning;
  $('setting-images').checked = preferences.sendImages; $('setting-language').value = preferences.language;
  $('setting-gpt-transcribe').checked = !preferences.bypassGptTranscribe;
  syncProfileFields();
}
function formPreferences() {
  const local = $('setting-profile').value === 'local-sglang';
  return {
    model: $('setting-model').value, reasoning: $('setting-reasoning').value,
    sendImages: local ? false : $('setting-images').checked, language: $('setting-language').value,
    postProcessingProfile: $('setting-profile').value,
    localEndpoint: $('setting-local-endpoint').value.trim(), localModel: $('setting-local-model').value.trim(),
    bypassGptTranscribe: !$('setting-gpt-transcribe').checked
  };
}
function describePreferences(next) {
  const local = next.postProcessingProfile === 'local-sglang';
  const profile = local ? `Local SGLang · ${next.localModel}` : `OpenAI · ${next.model} · ${next.reasoning} reasoning`;
  const transcription = next.bypassGptTranscribe
    ? (local ? 'GPT Transcribe off · no audio upload · finalized Nemotron text'
      : 'GPT Transcribe off · no audio upload · transcript text sent to OpenAI')
    : (local ? 'GPT Transcribe on · audio sent to OpenAI · formatting stays local'
      : 'GPT Transcribe on · audio and transcript text sent to OpenAI');
  return `${profile} · ${transcription}${local ? ' · text only' : ''}`;
}
function showFormattingPrompt(result) {
  $('setting-prompt').value = result.prompt;
  $('prompt-note').dataset.error = String(!!result.error);
  $('prompt-note').textContent = result.error
    ? `Saved prompt ignored: ${result.error}`
    : result.custom
      ? 'Custom prompt saved · applies to the next recording. The YAML schema is built in.'
      : 'Default prompt · edits are saved only when you press Save prompt. The YAML schema is built in.';
}
async function loadPreferences() {
  const [result, prompt] = await Promise.all([invoke('get_preferences'), invoke('get_formatting_prompt')]);
  preferences = { ...preferences, ...result.preferences };
  fillPreferences(); showVocabulary(result.vocabulary); updateEngine();
  showFormattingPrompt(prompt);
  $('settings-note').textContent = describePreferences(preferences);
}
$('setting-prompt').oninput = () => {
  $('prompt-note').dataset.error = 'false';
  $('prompt-note').textContent = 'Unsaved prompt changes · press Save prompt to apply them.';
};
$('save-prompt').onclick = async () => {
  const button = $('save-prompt');
  button.disabled = true;
  $('prompt-note').dataset.error = 'false';
  $('prompt-note').textContent = 'Saving prompt…';
  try {
    await invoke('save_formatting_prompt', { prompt: $('setting-prompt').value });
    $('prompt-note').textContent = 'Prompt saved · applies to the next recording. The YAML schema is built in.';
  } catch (error) {
    $('prompt-note').dataset.error = 'true';
    $('prompt-note').textContent = `Save failed: ${String(error?.message || error)}`;
  } finally { button.disabled = false; }
};
$('reset-prompt').onclick = async () => {
  const button = $('reset-prompt');
  button.disabled = true;
  try {
    $('setting-prompt').value = await invoke('reset_formatting_prompt');
    $('prompt-note').dataset.error = 'false';
    $('prompt-note').textContent = 'Default prompt restored · applies to the next recording.';
  } catch (error) {
    $('prompt-note').dataset.error = 'true';
    $('prompt-note').textContent = `Restore failed: ${String(error?.message || error)}`;
  } finally { button.disabled = false; }
};
$('settings').onclick = async () => {
  outputLife.stop(); await setView(false); const opening = $('settings-view').hidden;
  $('settings-view').hidden = !opening; $('settings').setAttribute('aria-expanded', String(opening));
  if (opening) { $('history-viewer').hidden = true; $('history-list').replaceChildren(); $('history').setAttribute('aria-expanded', 'false'); }
  await win.setFocusable?.(!mini);
  if (opening) await loadPreferences().catch(error => { fillPreferences(); $('settings-note').textContent = String(error); });
};
for (const id of ['setting-profile', 'setting-local-endpoint', 'setting-local-model', 'setting-model', 'setting-reasoning', 'setting-images', 'setting-language', 'setting-gpt-transcribe']) $(id).onchange = () => {
  syncProfileFields();
  const next = formPreferences();
  settingsWrite = settingsWrite.catch(() => {}).then(async () => {
    await invoke('save_preferences', { preferences: next }); preferences = next; updateEngine();
    $('settings-note').textContent = `Saved · ${describePreferences(next)}`;
  }).catch(error => {
    $('settings-note').textContent = `Save failed: ${String(error?.message || error)}`;
    fillPreferences();
  });
};
$('refresh-vocabulary').onclick = async () => {
  $('refresh-vocabulary').disabled = true;
  try { showVocabulary(await invoke('refresh_vocabulary')); } catch (error) { $('settings-note').textContent = String(error); }
  finally { $('refresh-vocabulary').disabled = false; }
};
await loadPreferences().catch(error => { fillPreferences(); $('settings-note').textContent = String(error); });
await listen('transcription-stream', ({payload}) => {
  if (payload.connected) streamConnectedId = payload.id;
  else if (streamConnectedId === payload.id) streamConnectedId = '';
  updateEngine();
});
await listen('live-transcript', ({payload}) => {
  if (payload.id !== sessionId || !['recording', 'stopping'].includes(state)) return;
  if (payload.error) { $('status').textContent = payload.error; return; }
  live.set('local', payload.text);
  if (!liveFrame) liveFrame = requestAnimationFrame(() => { liveFrame = undefined; renderText(live.get('local') || ''); });
});
await listen('screen-context', ({payload}) => {
  if (payload.id !== sessionId || state !== 'recording') return;
  $('clock').setAttribute('aria-label', payload.error || `Recording · ${payload.count} desktop screenshots captured`);
  if (payload.error) $('status').textContent = payload.error;
});
await listen('quit-requested', () => void quit());
await listen('widget-shown', () => { openedByTray = true; document.body.classList.remove('dormant'); void setView(false); });
status(await invoke('startup_status'));
await setView(mini);
if (!openedByTray) await win.hide();
