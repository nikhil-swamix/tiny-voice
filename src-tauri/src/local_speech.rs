use std::{
    collections::HashMap,
    io::{self, Read, Write},
    path::PathBuf,
    process::{Child, ChildStderr, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use parking_lot::Mutex;
use serde_json::json;
use tauri::Emitter;
use transcribe_cpp::{
    Backend, CommitPolicy, Model, ModelOptions, RunOptions, SessionOptions, Stream, StreamOptions,
};

const MODEL_REPO: &str = "models--handy-computer--nemotron-speech-streaming-en-0.6b-gguf";
const MODEL_REVISION: &str = "7d9b719206789e4068d87c6398262ab4dfd4e45d";
const MODEL_FILE: &str = "nemotron-speech-streaming-en-0.6b-Q8_0.gguf";
const SAMPLE_RATE: i32 = 16_000;
const SAMPLES_PER_CHUNK: usize = 1_600; // 100 ms at 16 kHz
const CHUNK_BYTES: usize = SAMPLES_PER_CHUNK * 4;
const AUDIO_QUEUE_CHUNKS: usize = 64; // ~6.4 s of bounded backpressure
const FINISH_TIMEOUT: Duration = Duration::from_secs(60);

/// Resolve the already-downloaded Handy Nemotron streaming model without ever
/// touching the network. `HUGGINGFACE_HUB_CACHE` is the hub cache root;
/// `HF_HOME` keeps the hub under `hub/`.
pub fn model_path() -> Result<PathBuf, String> {
    let cache = std::env::var_os("HUGGINGFACE_HUB_CACHE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HF_HOME").map(|p| PathBuf::from(p).join("hub")))
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|p| PathBuf::from(p).join(".cache/huggingface/hub"))
        })
        .ok_or("No home directory is available to locate the Hugging Face cache")?;
    let path = cache
        .join(MODEL_REPO)
        .join("snapshots")
        .join(MODEL_REVISION)
        .join(MODEL_FILE);
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => {
            path.canonicalize().map_err(|error| format!("Resolving {}: {error}", path.display()))
        }
        _ => Err(format!(
            "Handy Nemotron Streaming English 0.6B Q8_0 is not cached at {} · no weights were downloaded",
            path.display()
        )),
    }
}

fn inference_threads() -> i32 {
    std::thread::available_parallelism()
        .map_or(4, |threads| threads.get().min(8)) as i32
}

fn load_model(store: &Mutex<Option<Model>>) -> Result<Model, String> {
    let mut slot = store.lock();
    if let Some(model) = slot.as_ref() {
        return Ok(model.clone());
    }
    let path = model_path()?;
    let model = Model::load_with(&path, &ModelOptions { backend: Backend::Cpu, device: None })
        .map_err(|error| format!("Loading Handy Nemotron on strict CPU: {error}"))?;
    if !model.backend().eq_ignore_ascii_case("cpu") {
        return Err(format!("Expected strict CPU inference, got {}", model.backend()));
    }
    let capabilities = model
        .capabilities()
        .map_err(|error| format!("Reading Nemotron capabilities: {error}"))?;
    if !capabilities.supports_streaming {
        return Err("The cached Nemotron model does not support streaming".into());
    }
    if capabilities.native_sample_rate != SAMPLE_RATE {
        return Err(format!(
            "Nemotron expects {} Hz audio, but live captions decode to {SAMPLE_RATE} Hz",
            capabilities.native_sample_rate
        ));
    }
    *slot = Some(model.clone());
    Ok(model)
}

/// State shared by the decoder writer, inference loop, and cancellation.
struct PipelineControl {
    cancel: AtomicBool,
    input_done: AtomicBool,
    child: Mutex<Option<Child>>,
}

impl PipelineControl {
    fn new() -> Self {
        Self {
            cancel: AtomicBool::new(false),
            input_done: AtomicBool::new(false),
            child: Mutex::new(None),
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }
}

/// Per-recording control that can also report a provisional-caption failure.
struct LiveControl {
    id: String,
    app: tauri::AppHandle,
    pipeline: Arc<PipelineControl>,
    error_reported: AtomicBool,
}

struct ActiveSession {
    audio: SyncSender<Audio>,
    control: Arc<LiveControl>,
    result: Receiver<Result<String, String>>,
}

enum Audio {
    Bytes(Vec<u8>),
    Stop,
}

/// One resident CPU model; each recording gets its own ffmpeg decoder and
/// native streaming session. The model-store lock serializes complete streams:
/// the native API rejects, rather than waits for, overlapping streams.
#[derive(Default)]
pub struct LocalSpeech {
    model: Arc<Mutex<Option<Model>>>,
    sessions: Mutex<HashMap<String, ActiveSession>>,
}

impl LocalSpeech {
    /// Load and validate the model once. Safe to call from a blocking startup
    /// task; repeated calls return the resident handle.
    pub fn warm(&self) -> Result<(), String> {
        load_model(&self.model).map(|_| ())
    }

    /// Start progressive captions for one recording. The worker loads the
    /// model if startup warming has not finished yet, so this never blocks the
    /// async runtime on native inference or process waits.
    pub async fn start(&self, id: String, app: tauri::AppHandle) -> Result<(), String> {
        // Fail fast on a cold start with a missing cache; a resident model does
        // not need the original cache file to keep serving.
        if self.model.try_lock().is_some_and(|slot| slot.is_none()) {
            model_path()?;
        }
        let (audio_tx, audio_rx) = mpsc::sync_channel(AUDIO_QUEUE_CHUNKS);
        let (result_tx, result_rx) = mpsc::channel();
        let control = Arc::new(LiveControl {
            id: id.clone(),
            app: app.clone(),
            pipeline: Arc::new(PipelineControl::new()),
            error_reported: AtomicBool::new(false),
        });
        {
            let mut sessions = self.sessions.lock();
            if sessions.contains_key(&id) {
                return Err("Live captions are already running for this recording".into());
            }
            sessions.insert(
                id.clone(),
                ActiveSession { audio: audio_tx, control: control.clone(), result: result_rx },
            );
        }

        let input = ChannelReader { audio: audio_rx, control: control.clone(), current: Vec::new(), offset: 0 };
        let model_store = self.model.clone();
        let ready_control = control.clone();
        let text_control = control.clone();
        let worker_control = control.clone();
        let spawned = thread::Builder::new().name(format!("nemotron-live-{id}")).spawn(move || {
            let mut ready = move || {
                let _ = ready_control.app.emit(
                    "transcription-stream",
                    json!({"id": ready_control.id, "connected": true}),
                );
            };
            let mut on_text = move |text: &str| {
                let _ = text_control.app.emit(
                    "live-transcript",
                    json!({"id": text_control.id, "text": text}),
                );
            };
            let outcome = run_webm_pipeline(
                &model_store,
                input,
                &worker_control.pipeline,
                &mut ready,
                &mut on_text,
            );
            match &outcome {
                Ok(_) => {
                    let _ = worker_control.app.emit(
                        "transcription-stream",
                        json!({"id": worker_control.id, "connected": false}),
                    );
                }
                Err(error) => report_preview_error(&worker_control, error),
            }
            let _ = result_tx.send(outcome);
        });
        if let Err(error) = spawned {
            self.sessions.lock().remove(&id);
            return Err(format!("Starting the live caption worker: {error}"));
        }
        Ok(())
    }

    /// Forward one recorded WebM chunk. This never fails the recording path:
    /// when the preview falls behind it stops, reports through
    /// `live-transcript`, and leaves durable audio/GPT transcription alone.
    pub async fn audio(&self, id: &str, bytes: &[u8]) {
        let (sender, control) = {
            let sessions = self.sessions.lock();
            match sessions.get(id) {
                Some(session) => (session.audio.clone(), session.control.clone()),
                None => return,
            }
        };
        match sender.try_send(Audio::Bytes(bytes.to_vec())) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                control.pipeline.cancel.store(true, Ordering::SeqCst);
                stop_process(&control.pipeline);
                report_preview_error(
                    &control,
                    "CPU live captions fell behind and stopped · audio is still being saved for GPT transcription",
                );
            }
        }
    }

    /// Stop accepting audio, drain the decoder and inference, and return the
    /// final live preview. An absent session is a successful empty preview.
    pub async fn finish(&self, id: &str) -> Result<String, String> {
        let Some(session) = self.sessions.lock().remove(id) else {
            return Ok(String::new());
        };
        let ActiveSession { audio, control, result } = session;
        tauri::async_runtime::spawn_blocking(move || finish_session(audio, control, result))
            .await
            .map_err(|error| format!("Joining the live caption worker: {error}"))?
    }

    /// Cancel immediately, kill ffmpeg, and let the worker unwind without
    /// publishing a preview. The saved recording and cloud path are untouched.
    pub async fn abort(&self, id: &str) {
        if let Some(session) = self.sessions.lock().remove(id) {
            session.control.pipeline.cancel.store(true, Ordering::SeqCst);
            stop_process(&session.control.pipeline);
        }
    }

}

fn finish_session(
    audio: SyncSender<Audio>,
    control: Arc<LiveControl>,
    result: Receiver<Result<String, String>>,
) -> Result<String, String> {
    // Never block an async thread on a full queue; Stop is the last item and
    // the worker drains the chunks queued before it.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match audio.try_send(Audio::Stop) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => break,
            Err(TrySendError::Full(_)) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(TrySendError::Full(_)) => {
                control.pipeline.cancel.store(true, Ordering::SeqCst);
                stop_process(&control.pipeline);
                break;
            }
        }
    }
    match result.recv_timeout(FINISH_TIMEOUT) {
        Ok(outcome) => outcome,
        Err(_) => {
            control.pipeline.cancel.store(true, Ordering::SeqCst);
            stop_process(&control.pipeline);
            Err("CPU live captions did not stop cleanly; final transcription is unaffected".into())
        }
    }
}

fn report_preview_error(control: &LiveControl, message: &str) {
    if control.error_reported.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = control.app.emit(
        "live-transcript",
        json!({"id": control.id, "error": message}),
    );
    let _ = control.app.emit(
        "transcription-stream",
        json!({"id": control.id, "connected": false}),
    );
}

/// Bounded audio channel exposed as a blocking `Read` so ffmpeg stdin can be
/// filled without putting any blocking I/O on the async runtime.
struct ChannelReader {
    audio: Receiver<Audio>,
    control: Arc<LiveControl>,
    current: Vec<u8>,
    offset: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if self.control.pipeline.cancelled() {
                return Ok(0);
            }
            if self.offset < self.current.len() {
                let count = output.len().min(self.current.len() - self.offset);
                output[..count]
                    .copy_from_slice(&self.current[self.offset..self.offset + count]);
                self.offset += count;
                return Ok(count);
            }
            match self.audio.recv_timeout(Duration::from_millis(100)) {
                Ok(Audio::Bytes(bytes)) => {
                    self.current = bytes;
                    self.offset = 0;
                }
                Ok(Audio::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(0),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

fn spawn_decoder() -> Result<Child, String> {
    let mut command = Command::new("ffmpeg");
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-probesize",
            "32",
            "-analyzeduration",
            "1",
            "-f",
            "webm",
            "-i",
            "pipe:0",
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-flush_packets",
            "1",
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
        .spawn()
        .map_err(|error| format!("Live captions require ffmpeg on PATH: {error}"))
}

fn run_webm_pipeline<R>(
    model_store: &Mutex<Option<Model>>,
    input: R,
    control: &Arc<PipelineControl>,
    ready: &mut dyn FnMut(),
    on_text: &mut dyn FnMut(&str),
) -> Result<String, String>
where
    R: Read + Send + 'static,
{
    if control.cancelled() {
        return Err("CPU live captions were cancelled".into());
    }
    let model = load_model(model_store)?;
    let _inference = model_store.lock();
    if control.cancelled() {
        return Err("CPU live captions were cancelled".into());
    }

    let mut child = spawn_decoder()?;
    let Some(stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("The ffmpeg decoder did not expose stdin".into());
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("The ffmpeg decoder did not expose stdout".into());
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("The ffmpeg decoder did not expose stderr".into());
    };
    *control.child.lock() = Some(child);

    let stderr_task = match thread::Builder::new()
        .name("nemotron-ffmpeg-stderr".into())
        .spawn(move || drain_stderr(stderr))
    {
        Ok(task) => task,
        Err(error) => {
            stop_process(control);
            return Err(format!("Starting the ffmpeg stderr reader: {error}"));
        }
    };

    let mut session = match model.session_with(&SessionOptions {
        n_threads: inference_threads(),
        ..Default::default()
    }) {
        Ok(session) => session,
        Err(error) => {
            stop_process(control);
            let _ = stderr_task.join();
            return Err(format!("Creating the CPU live caption session: {error}"));
        }
    };
    let mut stream = match session.stream(
        &RunOptions::default(),
        &StreamOptions { commit_policy: CommitPolicy::Auto, ..Default::default() },
    ) {
        Ok(stream) => stream,
        Err(error) => {
            stop_process(control);
            let _ = stderr_task.join();
            return Err(format!("Starting Nemotron streaming: {error}"));
        }
    };

    let writer_control = control.clone();
    let writer = match thread::Builder::new()
        .name("nemotron-webm-input".into())
        .spawn(move || write_webm(input, stdin, &writer_control))
    {
        Ok(writer) => writer,
        Err(error) => {
            drop(stream);
            stop_process(control);
            let _ = stderr_task.join();
            return Err(format!("Starting the WebM writer: {error}"));
        }
    };

    ready();
    let mut last = String::new();
    let fed = feed_pcm(stdout, &mut stream, control, &mut *on_text, &mut last);
    let result = match fed {
        Err(error) => Err(error),
        Ok(0) => Ok(String::new()),
        Ok(_) => {
            if control.cancelled() {
                Err("CPU live captions were cancelled".into())
            } else {
                match stream.finalize() {
                    Err(error) => Err(format!("Finishing CPU live captions: {error}")),
                    Ok(_) => {
                        let text = current_text(&stream);
                        if !text.is_empty() && text != last {
                            on_text(&text);
                        }
                        Ok(text)
                    }
                }
            }
        }
    };
    drop(stream);
    drop(session);

    if result.is_err() {
        control.cancel.store(true, Ordering::SeqCst);
    }
    if !control.input_done.load(Ordering::SeqCst) {
        // ffmpeg closed stdout before the input ended: do not leave the writer
        // blocked on a dead pipe.
        control.cancel.store(true, Ordering::SeqCst);
    }
    if control.cancelled() {
        stop_process(control);
    }
    let _ = writer.join();
    let status = wait_process(control);
    let stderr = stderr_task.join().unwrap_or_default();
    if !stderr.is_empty() && !status.as_ref().is_some_and(ExitStatus::success) {
        return Err(format!("Live audio decoding failed: {stderr}"));
    }

    let text = result?;
    if control.cancelled() {
        return Err("CPU live captions were cancelled; audio is still saved for final transcription".into());
    }
    match status {
        Some(status) if !status.success() => Err(decoder_error(status, &stderr)),
        None => Err("Live audio decoding did not exit cleanly; final transcription is unaffected".into()),
        Some(_) => Ok(text),
    }
}

fn feed_pcm<R>(
    mut pcm: R,
    stream: &mut Stream<'_>,
    control: &PipelineControl,
    on_text: &mut dyn FnMut(&str),
    last: &mut String,
) -> Result<usize, String>
where
    R: Read,
{
    let mut raw = [0u8; 32 * 1024];
    let mut pending: Vec<u8> = Vec::with_capacity(raw.len() + CHUNK_BYTES);
    let mut fed = 0usize;
    let mut samples = [0.0f32; SAMPLES_PER_CHUNK];
    loop {
        if control.cancelled() {
            return Err("CPU live captions were cancelled".into());
        }
        let count = match pcm.read(&mut raw) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Reading decoded audio: {error}")),
        };
        pending.extend_from_slice(&raw[..count]);
        let mut start = 0;
        while pending.len() - start >= CHUNK_BYTES {
            for (sample, bytes) in samples.iter_mut().zip(pending[start..start + CHUNK_BYTES].chunks_exact(4)) {
                *sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            }
            feed_chunk(stream, &samples, control, &mut *on_text, last)?;
            fed += samples.len();
            start += CHUNK_BYTES;
        }
        if start > 0 {
            pending.drain(..start);
        }
    }
    if pending.len() % 4 != 0 {
        return Err("The audio decoder returned an incomplete PCM sample".into());
    }
    if !pending.is_empty() {
        let count = pending.len() / 4;
        for (sample, bytes) in samples.iter_mut().zip(pending.chunks_exact(4)) {
            *sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        feed_chunk(stream, &samples[..count], control, &mut *on_text, last)?;
        fed += count;
    }
    Ok(fed)
}

fn feed_chunk(
    stream: &mut Stream<'_>,
    samples: &[f32],
    control: &PipelineControl,
    on_text: &mut dyn FnMut(&str),
    last: &mut String,
) -> Result<(), String> {
    if control.cancelled() {
        return Err("CPU live captions were cancelled".into());
    }
    let update = stream.feed(samples)
        .map_err(|error| format!("CPU live caption inference failed: {error}"))?;
    if !update.result_changed { return Ok(()); }
    let text = current_text(stream);
    if !text.is_empty() && text != *last {
        on_text(&text);
        *last = text;
    }
    Ok(())
}

fn current_text(stream: &Stream<'_>) -> String {
    stream.text().display().trim().to_owned()
}

fn write_webm<R: Read>(mut input: R, mut stdin: ChildStdin, control: &PipelineControl) {
    let mut buffer = [0u8; 32 * 1024];
    loop {
        if control.cancelled() {
            break;
        }
        match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if stdin.write_all(&buffer[..count]).is_err() {
                    control.cancel.store(true, Ordering::SeqCst);
                    stop_process(control);
                    break;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                control.cancel.store(true, Ordering::SeqCst);
                stop_process(control);
                break;
            }
        }
    }
    control.input_done.store(true, Ordering::SeqCst);
    drop(stdin);
}

fn drain_stderr(mut stderr: ChildStderr) -> String {
    let mut output = Vec::new();
    let mut buffer = [0u8; 2 * 1024];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if output.len() < 8 * 1024 {
                    let take = count.min(8 * 1024 - output.len());
                    output.extend_from_slice(&buffer[..take]);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&output).trim().to_owned()
}

fn stop_process(control: &PipelineControl) {
    let mut slot = control.child.lock();
    if let Some(mut child) = slot.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn wait_process(control: &PipelineControl) -> Option<ExitStatus> {
    let child = { control.child.lock().take() };
    let Some(mut child) = child else { return None; };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
        if control.cancelled() || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn decoder_error(status: ExitStatus, stderr: &str) -> String {
    if stderr.is_empty() {
        format!("Live audio decoding failed (ffmpeg {status}); final transcription is unaffected")
    } else {
        format!("Live audio decoding failed (ffmpeg {status}: {stderr}); final transcription is unaffected")
    }
}