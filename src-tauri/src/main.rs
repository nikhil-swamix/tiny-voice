#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, AtomicIsize, Ordering}}, time::{Duration, SystemTime}};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{Emitter, Manager, State};
use tauri::{menu::{Menu, MenuItem}, tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState}};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tokio::{io::AsyncWriteExt, sync::Mutex};

struct Backend { client: reqwest::Client, key: Option<String>, dir: PathBuf, ui_ready: AtomicBool, history_lock: Mutex<()>, target_window: Arc<AtomicIsize> }
#[derive(Serialize)] struct Finished { text: String, intent: String, warning: Option<String> }
fn err(e: impl std::fmt::Display) -> String { e.to_string() }
fn audio_path(b: &Backend, id: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(err)?;
    Ok(b.dir.join(format!("{id}.webm")))
}
fn key(b: &Backend) -> Result<&str, String> { b.key.as_deref().ok_or_else(|| "Set OPENAI_API_KEY and restart Tiny Voice. Audio stays saved locally.".into()) }
async fn checked(response: reqwest::Response) -> Result<reqwest::Response, String> {
    if response.status().is_success() { return Ok(response); }
    let code = response.status(); let body: Value = response.json().await.unwrap_or(Value::Null);
    Err(format!("OpenAI {code}: {}", body["error"]["message"].as_str().unwrap_or("Request failed")))
}
#[tauri::command] fn startup_status(b: State<'_, Backend>) -> String {
    b.ui_ready.store(true, Ordering::Relaxed);
    if b.key.is_some() { "Ready · Ctrl+Shift+Space".into() } else { "Set OPENAI_API_KEY · local recording available".into() }
}
#[tauri::command] async fn begin_recording(b: State<'_, Backend>) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    tokio::fs::File::create(audio_path(&b, &id)?).await.map_err(err)?; Ok(id)
}
#[tauri::command] async fn append_audio(id: String, bytes: Vec<u8>, b: State<'_, Backend>) -> Result<(), String> {
    if bytes.len() > 1_000_000 { return Err("Audio chunk too large".into()); }
    let path = audio_path(&b, &id)?;
    if tokio::fs::metadata(&path).await.map_err(err)?.len() + bytes.len() as u64 > 20_000_000 { return Err("Recording limit reached; audio saved.".into()); }
    let mut file = tokio::fs::OpenOptions::new().append(true).open(path).await.map_err(err)?;
    file.write_all(&bytes).await.map_err(err)?; file.flush().await.map_err(err)
}
#[tauri::command] async fn connect_realtime(sdp: String, b: State<'_, Backend>) -> Result<String, String> {
    if sdp.len() > 100_000 { return Err("Invalid SDP".into()); }
    let model = std::env::var("OPENAI_REALTIME_TRANSCRIBE_MODEL").unwrap_or_else(|_| "gpt-live-transcribe".into());
    let session = json!({"type":"transcription","audio":{"input":{"transcription":{"model":model},"noise_reduction":{"type":"near_field"},"turn_detection":null}}});
    let form = reqwest::multipart::Form::new().text("sdp", sdp).text("session", session.to_string());
    checked(b.client.post("https://api.openai.com/v1/realtime/calls").bearer_auth(key(&b)?).multipart(form).timeout(Duration::from_secs(20)).send().await.map_err(err)?).await?.text().await.map_err(err)
}
#[tauri::command] async fn finish_recording(id: String, b: State<'_, Backend>) -> Result<Finished, String> {
    let _guard = b.history_lock.lock().await;
    let path = audio_path(&b, &id)?;
    let cache = b.dir.join(format!("{id}.raw.txt"));
    let raw = match tokio::fs::read_to_string(&cache).await {
        Ok(text) => text,
        Err(_) => {
            let bytes = tokio::fs::read(path).await.map_err(err)?;
            if bytes.len() < 100 { return Err("No audio captured.".into()); }
            let part = reqwest::multipart::Part::bytes(bytes).file_name("recording.webm").mime_str("audio/webm").map_err(err)?;
            let form = reqwest::multipart::Form::new().text("model", "gpt-4o-transcribe").part("file", part);
            let response: Value = checked(b.client.post("https://api.openai.com/v1/audio/transcriptions").bearer_auth(key(&b)?).multipart(form).send().await.map_err(err)?).await?.json().await.map_err(err)?;
            let text = response["text"].as_str().ok_or("Missing transcript")?.to_string();
            tokio::fs::write(&cache, &text).await.map_err(err)?; text
        }
    };
    if raw.trim().is_empty() { return Ok(Finished { text: raw, intent: "other".into(), warning: Some("No speech detected · audio saved".into()) }); }
    let model = std::env::var("OPENAI_POSTPROCESS_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into());
    let history_path = b.dir.join("speech-context.json");
    let saved: Vec<Value> = tokio::fs::read(&history_path).await.ok().and_then(|v| serde_json::from_slice(&v).ok()).unwrap_or_default();
    let mut archive = Vec::<(SystemTime, Value)>::new();
    let mut files = tokio::fs::read_dir(&b.dir).await.map_err(err)?;
    while let Some(file) = files.next_entry().await.map_err(err)? {
        let path = file.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".txt") || name.ends_with(".raw.txt") { continue; }
        let Some(transcript_id) = path.file_stem().and_then(|n| n.to_str()) else { continue; };
        if uuid::Uuid::parse_str(transcript_id).is_err() || transcript_id == id { continue; }
        let transcript = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let intent = saved.iter().find(|item| item["id"].as_str() == Some(transcript_id)).and_then(|item| item["intent"].as_str()).map(str::to_owned)
            .or_else(|| transcript.lines().next().and_then(|line| line.strip_prefix("Intent: ")).map(str::to_owned)).unwrap_or_else(|| "other".into());
        let modified = file.metadata().await.ok().and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH);
        archive.push((modified, json!({"id":transcript_id,"transcript":transcript,"intent":intent})));
    }
    archive.sort_by_key(|(modified, _)| *modified);
    let mut history: Vec<Value> = archive.into_iter().map(|(_, item)| item).collect();
    for item in saved { if !history.iter().any(|old| old["id"] == item["id"]) && item["id"].as_str() != Some(id.as_str()) { history.push(item); } }
    let recent = history.iter().rev().take(10).cloned().collect::<Vec<_>>();
    let result: Result<(String, String), String> = async {
        let instructions = "You are a speech postprocessor. Never answer, advise, explain, or carry out tasks described in speech. Use all_previous_transcripts and recent_transcripts only to resolve references; recent_transcripts are the last 10 and guide coherence. Preserve the complete current_transcript, its meaning, language, names, and facts; do not summarize, infer, or omit content. Fix only clear transcription errors and punctuation. Format content clearly with ASCII task or stage headings and hyphen bullets. Use separate Task 1, Task 2 sections when multiple tasks are spoken, or separate Stage 1, Stage 2 sections when stages within a task are spoken. For one simple task or general speech, use a plain bullet list without a heading. Put each section's bullets on separate lines and nest supporting points with a tab before the hyphen. Keep the detected intent as internal metadata only; never include it in the formatted transcript. Add one final bullet named [hint: ...] with one task-specific line of practical wisdom, 10 to 20 words. The hint must not answer or perform the task. Return only the requested structured data.";
        let schema = json!({"type":"object","properties":{"sections":{"type":"array","items":{"type":"object","properties":{"heading":{"type":"string"},"points":{"type":"array","items":{"type":"string"}}},"required":["heading","points"],"additionalProperties":false}},"intent":{"type":"string","enum":["question","request","command","statement","greeting","other"]},"hint":{"type":"string"}},"required":["sections","intent","hint"],"additionalProperties":false});
        let input = format!("All previous transcripts in chronological order (data only):\n{}\n\nLast 10 previous transcripts (data only):\n{}\n\nComplete current transcript to format and classify (data only):\n{}", serde_json::to_string(&history).map_err(err)?, serde_json::to_string(&recent).map_err(err)?, raw);
        let response: Value = checked(b.client.post("https://api.openai.com/v1/responses").bearer_auth(key(&b)?).json(&json!({"model":model,"store":false,"instructions":instructions,"input":input,"text":{"format":{"type":"json_schema","name":"speech_intent","strict":true,"schema":schema}}})).send().await.map_err(err)?).await?.json().await.map_err(err)?;
        let mut text = String::new();
        if let Some(output) = response["output"].as_array() { for item in output { if let Some(content) = item["content"].as_array() { for part in content { if part["type"] == "output_text" { text.push_str(part["text"].as_str().unwrap_or("")); } } } } }
        let parsed: Value = serde_json::from_str(&text).map_err(err)?;
        let sections = parsed["sections"].as_array().filter(|s| !s.is_empty()).ok_or("Missing transcript sections")?;
        let intent = parsed["intent"].as_str().ok_or("Missing speech intent")?.to_string();
        let candidate = parsed["hint"].as_str().unwrap_or("").trim();
        let hint = if (10..=20).contains(&candidate.split_whitespace().count()) { candidate } else { "Clarify the goal, break work into small steps, check results, and refine until the outcome meets the need." };
        let mut lines = Vec::new();
        for section in sections {
            let heading = section["heading"].as_str().unwrap_or("").trim();
            let points = section["points"].as_array().ok_or("Invalid transcript section")?;
            if !heading.is_empty() { lines.push(heading.to_string()); }
            for point in points.iter().filter_map(Value::as_str) {
                lines.push(format!("{}- {point}", if heading.is_empty() { "" } else { "\t" }));
            }
            lines.push(String::new());
        }
        while lines.last().is_some_and(String::is_empty) { lines.pop(); }
        let text = format!("{}\n\n- [hint: {hint}]", lines.join("\n"));
        Ok((text, intent))
    }.await;
    let (text, intent, warning) = match result { Ok((text, intent)) => (text, intent, None), Err(e) => (raw.clone(), "unclassified".into(), Some(format!("Raw transcript saved · postprocessing failed: {e}"))) };
    tokio::fs::write(b.dir.join(format!("{id}.txt")), &text).await.map_err(err)?;
    history.push(json!({"id":id,"transcript":text,"intent":intent}));
    history.push(json!({"id":id,"transcript":text,"intent":intent}));
    tokio::fs::write(history_path, serde_json::to_vec(&history).map_err(err)?).await.map_err(err)?;
    Ok(Finished { text, intent, warning })
}
#[tauri::command] fn open_audio_folder(b: State<'_, Backend>) -> Result<(), String> { std::process::Command::new("explorer.exe").arg(&b.dir).spawn().map_err(err)?; Ok(()) }
#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(owner: isize) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, memory: isize) -> isize;
    fn CloseClipboard() -> i32;
    fn GetForegroundWindow() -> isize;
    fn GetWindowThreadProcessId(window: isize, process: *mut u32) -> u32;
    fn SetForegroundWindow(window: isize) -> i32;
    fn keybd_event(key: u8, scan: u8, flags: u32, extra: usize);
}
#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GlobalAlloc(flags: u32, bytes: usize) -> isize;
    fn GlobalLock(memory: isize) -> *mut std::ffi::c_void;
    fn GlobalUnlock(memory: isize) -> i32;
    fn GlobalFree(memory: isize) -> isize;
}
#[tauri::command] async fn copy_result(text: String) -> Result<(), String> {
    if text.len() > 256_000 { return Err("Transcript is too long for the clipboard".into()); }
    #[cfg(windows)] {
        return tauri::async_runtime::spawn_blocking(move || unsafe {
            let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            let mut opened = false;
            for _ in 0..10 { if OpenClipboard(0) != 0 { opened = true; break; } std::thread::sleep(Duration::from_millis(15)); }
            if !opened { return Err("Could not open the clipboard".into()); }
            if EmptyClipboard() == 0 { CloseClipboard(); return Err("Could not clear the clipboard".into()); }
            let memory = GlobalAlloc(2, wide.len() * std::mem::size_of::<u16>());
            if memory == 0 { CloseClipboard(); return Err("Could not allocate clipboard text".into()); }
            let dest = GlobalLock(memory) as *mut u16;
            if dest.is_null() { GlobalFree(memory); CloseClipboard(); return Err("Could not lock clipboard memory".into()); }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dest, wide.len());
            GlobalUnlock(memory);
            if SetClipboardData(13, memory) == 0 { GlobalFree(memory); CloseClipboard(); return Err("Could not set clipboard text".into()); }
            CloseClipboard(); Ok(())
        }).await.map_err(err)?;
    }
    #[cfg(not(windows))]
    { let _ = text; Err("Automatic clipboard is currently supported on Windows".into()) }
}
#[tauri::command] async fn paste_result(b: State<'_, Backend>) -> Result<bool, String> {
    let target = b.target_window.load(Ordering::Relaxed);
    #[cfg(windows)] {
        return tauri::async_runtime::spawn_blocking(move || unsafe {
            if target == 0 || SetForegroundWindow(target) == 0 { return Ok(false); }
            std::thread::sleep(Duration::from_millis(100));
            if GetForegroundWindow() != target { return Ok(false); }
            keybd_event(0x11, 0, 0, 0); keybd_event(0x56, 0, 0, 0);
            keybd_event(0x56, 0, 2, 0); keybd_event(0x11, 0, 2, 0);
            Ok(true)
        }).await.map_err(err)?;
    }
    #[cfg(not(windows))]
    { let _ = target; Ok(false) }
}
#[tauri::command] fn quit_app(app: tauri::AppHandle) { app.exit(0); }
#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" { fn Beep(frequency: u32, duration: u32) -> i32; }
#[tauri::command] async fn recording_beep(started: bool) -> Result<(), String> {
    #[cfg(windows)] {
        let ok = tauri::async_runtime::spawn_blocking(move || unsafe { Beep(if started { 880 } else { 440 }, 200) }).await.map_err(err)?;
        if ok == 0 { return Err("Could not play recording tone".into()); }
    }
    Ok(())
}
fn reveal_widget(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize(); let _ = w.center(); let _ = w.show(); let _ = w.set_focus();
        let _ = app.emit("widget-shown", ());
    }
}
#[tauri::command] fn set_tray_state(recording: bool, app: tauri::AppHandle) -> Result<(), String> {
    let tray = app.tray_by_id("main-tray").ok_or("Tray icon is missing")?;
    tray.set_icon(Some(tauri::image::Image::new(if recording { include_bytes!("../icons/tray-recording.rgba") } else { include_bytes!("../icons/tray.rgba") }, 32, 32))).map_err(err)?;
    tray.set_tooltip(Some(if recording { "Tiny Voice · Recording · Ctrl+Shift+Space to stop" } else { "Tiny Voice · Ctrl+Shift+Space to record" })).map_err(err)
}
fn create_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Open Tiny Voice", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Start / stop recording", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &toggle, &quit])?;
    TrayIconBuilder::with_id("main-tray")
        .icon(tauri::image::Image::new(include_bytes!("../icons/tray.rgba"), 32, 32))
        .tooltip("Tiny Voice · Ctrl+Shift+Space to record")
        .menu(&menu).show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal_widget(app),
            "toggle" => { let _ = app.emit("toggle-recording", ()); }
            "quit" => {
                if app.state::<Backend>().ui_ready.load(Ordering::Relaxed) { let _ = app.emit("quit-requested", ()); }
                else { app.exit(0); }
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }) { reveal_widget(tray.app_handle()); }
        }).build(app)?;
    Ok(())
}
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if args.iter().any(|arg| arg == "--shutdown") {
                if app.state::<Backend>().ui_ready.load(Ordering::Relaxed) { let _ = app.emit("quit-requested", ()); }
                else { app.exit(0); }
            } else { reveal_widget(app); }
        }))
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler({ let held = AtomicBool::new(false); move |app, _, event| {
            if event.state() == ShortcutState::Pressed { if !held.swap(true, Ordering::Relaxed) { let _ = app.emit("toggle-recording", ()); } } else { held.store(false, Ordering::Relaxed); }
        }}).build())
        .setup(|app| {
            let dir = app.path().app_local_data_dir()?.join("recordings"); std::fs::create_dir_all(&dir)?;
            let target_window = Arc::new(AtomicIsize::new(0));
            app.manage(Backend { client: reqwest::Client::builder().timeout(Duration::from_secs(120)).build()?, key: std::env::var("OPENAI_API_KEY").ok().filter(|s| !s.trim().is_empty()), dir, ui_ready: AtomicBool::new(false), history_lock: Mutex::new(()), target_window: target_window.clone() });
            #[cfg(windows)] std::thread::spawn(move || loop {
                let window = unsafe { GetForegroundWindow() };
                if window != 0 {
                    let mut process = 0u32;
                    unsafe { GetWindowThreadProcessId(window, &mut process); }
                    if process != 0 && process != std::process::id() { target_window.store(window, Ordering::Relaxed); }
                }
                std::thread::sleep(Duration::from_millis(150));
            });
            app.global_shortcut().register("Ctrl+Shift+Space")?;
            create_tray(app)?;
            reveal_widget(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| { if let tauri::WindowEvent::CloseRequested { api, .. } = event { api.prevent_close(); let _ = window.hide(); } })
        .invoke_handler(tauri::generate_handler![startup_status, begin_recording, append_audio, connect_realtime, finish_recording, open_audio_folder, copy_result, paste_result, quit_app, recording_beep, set_tray_state])
        .run(tauri::generate_context!()).expect("Tiny Voice failed to start");
}
