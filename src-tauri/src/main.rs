#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use std::{collections::{HashMap, HashSet}, path::PathBuf, sync::{atomic::{AtomicBool, Ordering}}, time::{Duration, SystemTime, UNIX_EPOCH}};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{Emitter, Manager, State};
use tauri::{menu::{Menu, MenuItem}, tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState}};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tokio::{io::AsyncWriteExt, sync::Mutex};
mod processing;
mod transcription;
mod screen;
mod paste;
mod preferences;
mod vocabulary;
mod context;
mod local_speech;

// The history lock also tracks unfinished commits that need recovery from transcript files.
struct Backend { client: reqwest::Client, key: Option<String>, local_speech: std::sync::Arc<local_speech::LocalSpeech>, summary_lock: Mutex<()>, screens: screen::Screens, recordings: Recorder, dir: PathBuf, ui_ready: AtomicBool, history_lock: Mutex<bool>, preferences: Mutex<preferences::Preferences>, vocabulary: Mutex<Vec<String>> }
// Keeps one open file handle per active recording so audio chunks are appended
// without re-opening the file, and never blocks the live transcription path on
// disk I/O.
#[derive(Default)]
struct Recorder(Mutex<HashMap<String, Recording>>);
struct Recording { file: tokio::fs::File, bytes: u64 }
impl Recorder {
    async fn open(&self, id: &str, path: PathBuf) -> Result<(), String> {
        let file = tokio::fs::File::create(&path).await.map_err(err)?;
        self.0.lock().await.insert(id.to_owned(), Recording { file, bytes: 0 });
        Ok(())
    }
    async fn append(&self, id: &str, path: PathBuf, bytes: &[u8]) -> Result<(), String> {
        if bytes.len() > 1_000_000 { return Err("Audio chunk too large".into()); }
        let mut recordings = self.0.lock().await;
        if !recordings.contains_key(id) {
            // A trailing chunk can outlive its recording; keep the audio instead of dropping it.
            let file = tokio::fs::OpenOptions::new().append(true).create(true).open(&path).await.map_err(err)?;
            recordings.insert(id.to_owned(), Recording { file, bytes: 0 });
        }
        let recording = recordings.get_mut(id).expect("recording was just inserted");
        if recording.bytes + bytes.len() as u64 > 20_000_000 { return Err("Recording limit reached; audio saved.".into()); }
        recording.file.write_all(bytes).await.map_err(err)?;
        recording.bytes += bytes.len() as u64;
        Ok(())
    }
    async fn close(&self, id: &str) { self.0.lock().await.remove(id); }
}

#[derive(Serialize)] struct Finished { text: String, intent: String, warning: Option<String>, model: Option<String>, reasoning: Option<String>, transcription_model: String, post_processing_profile: String, transcription_ms: u64, formatting_ms: u64, output_format: String }
fn err(e: impl std::fmt::Display) -> String { e.to_string() }
fn audio_path(b: &Backend, id: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(id).map_err(err)?;
    Ok(b.dir.join(format!("{id}.webm")))
}
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().or_else(|| winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey("Environment").ok()?.get_value(name).ok()).filter(|key: &String| !key.trim().is_empty())
}
fn key(b: &Backend) -> Result<&str, String> { b.key.as_deref().ok_or_else(|| "Set OPENAI_API_KEY and restart Tiny Voice. Audio stays saved locally.".into()) }
const FORMATTING_PROMPT_FILE: &str = "formatting-prompt.txt";
async fn load_formatting_prompt(dir: &PathBuf) -> String {
    match tokio::fs::read_to_string(dir.join(FORMATTING_PROMPT_FILE)).await {
        Ok(prompt) if processing::validate_prompt(&prompt).is_ok() => prompt,
        _ => processing::DEFAULT_PROMPT.to_owned(),
    }
}
#[tauri::command] async fn get_formatting_prompt(b: State<'_, Backend>) -> Result<Value, String> {
    let saved = tokio::fs::read_to_string(b.dir.join(FORMATTING_PROMPT_FILE)).await;
    let (prompt, custom, error) = match saved {
        Ok(prompt) => match processing::validate_prompt(&prompt) {
            Ok(()) => (prompt, true, Value::Null),
            Err(error) => (processing::DEFAULT_PROMPT.to_owned(), false, json!(error)),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (processing::DEFAULT_PROMPT.to_owned(), false, Value::Null),
        Err(error) => (processing::DEFAULT_PROMPT.to_owned(), false, json!(format!("Could not read the saved prompt: {error}"))),
    };
    Ok(json!({"prompt":prompt,"defaultPrompt":processing::DEFAULT_PROMPT,"custom":custom,"error":error,"maxBytes":processing::MAX_PROMPT_BYTES}))
}
#[tauri::command] async fn save_formatting_prompt(prompt: String, b: State<'_, Backend>) -> Result<(), String> {
    processing::validate_prompt(&prompt)?;
    tokio::fs::write(b.dir.join(FORMATTING_PROMPT_FILE), prompt).await.map_err(err)
}
#[tauri::command] async fn reset_formatting_prompt(b: State<'_, Backend>) -> Result<String, String> {
    match tokio::fs::remove_file(b.dir.join(FORMATTING_PROMPT_FILE)).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    Ok(processing::DEFAULT_PROMPT.to_owned())
}
#[tauri::command] async fn startup_status(b: State<'_, Backend>) -> Result<String, String> {
    b.ui_ready.store(true, Ordering::Relaxed);
    let preferences = b.preferences.lock().await;
    let speech = if preferences.bypass_gpt_transcribe { "Nemotron CPU only" } else { "Nemotron CPU → GPT Transcribe" };
    let formatter = if preferences.post_processing_profile == "local-sglang" { "local SGLang" } else { "OpenAI" };
    Ok(format!("{speech} → {formatter} · YAML"))
}
#[tauri::command] async fn get_preferences(b: State<'_, Backend>) -> Result<Value, String> { Ok(json!({"preferences": b.preferences.lock().await.clone(), "vocabulary": b.vocabulary.lock().await.clone()})) }
#[tauri::command] async fn save_preferences(preferences: preferences::Preferences, b: State<'_, Backend>) -> Result<(), String> {
    preferences.validate()?;
    let mut current = b.preferences.lock().await;
    tokio::fs::write(b.dir.join("preferences.json"), serde_json::to_vec(&preferences).map_err(err)?).await.map_err(err)?;
    *current = preferences; Ok(())
}
#[tauri::command] async fn refresh_vocabulary(b: State<'_, Backend>) -> Result<Vec<String>, String> {
    let history = { let _guard = b.history_lock.lock().await; load_history(&b, "").await? };
    let corrections = vocabulary::load_corrections(&b.dir)?;
    let terms = vocabulary::scan(&history, &corrections); *b.vocabulary.lock().await = terms.clone(); Ok(terms)
}
#[tauri::command] fn stop_screen_context(id: String, b: State<'_, Backend>) { b.screens.stop(&id); }
#[tauri::command] async fn abort_live(id: String, b: State<'_, Backend>) -> Result<(), String> { b.screens.remove(&id); b.recordings.close(&id).await; b.local_speech.abort(&id).await; Ok(()) }
#[tauri::command] async fn begin_recording(b: State<'_, Backend>, app: tauri::AppHandle) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let path = audio_path(&b, &id)?;
    let preferences = b.preferences.lock().await.clone();
    b.recordings.open(&id, path).await?;
    if let Err(error) = b.local_speech.start(id.clone(), app.clone()).await {
        let next = if preferences.bypass_gpt_transcribe { "GPT Transcribe is bypassed; final recognition requires the local stream" } else { "audio is still being saved for GPT transcription" };
        let _ = app.emit("live-transcript", json!({"id":id,"error":format!("CPU live captions unavailable: {error} · {next}")}));
    }
    if preferences.send_images { b.screens.start(id.clone(), app); }
    Ok(id)
}
#[tauri::command] async fn append_audio(request: tauri::ipc::Request<'_>, b: State<'_, Backend>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else { return Err("Audio must be sent as raw bytes".into()); };
    let id = request.headers().get("x-session-id").and_then(|value| value.to_str().ok()).ok_or("Missing recording id")?.to_owned();
    let path = audio_path(&b, &id)?;
    // Hand the chunk to the live stream before touching the disk so transcription is never gated by local I/O.
    b.local_speech.audio(&id, bytes).await;
    b.recordings.append(&id, path, bytes).await
}
async fn saved_history(b: &Backend) -> Option<Vec<Value>> {
    tokio::fs::read(b.dir.join("speech-context.json")).await.ok().and_then(|v| serde_json::from_slice(&v).ok())
}
async fn load_history(b: &Backend, id: &str) -> Result<Vec<Value>, String> {
    let saved = saved_history(b).await.unwrap_or_default();
    let saved_ids: HashSet<String> = saved.iter().filter_map(|item| item["id"].as_str().map(str::to_owned)).collect();
    let mut archive = Vec::<(SystemTime, Value)>::new();
    let mut files = tokio::fs::read_dir(&b.dir).await.map_err(err)?;
    while let Some(file) = files.next_entry().await.map_err(err)? {
        let path = file.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !name.ends_with(".txt") || name.ends_with(".raw.txt") { continue; }
        let Some(transcript_id) = path.file_stem().and_then(|n| n.to_str()) else { continue; };
        if uuid::Uuid::parse_str(transcript_id).is_err() || transcript_id == id { continue; }
        if saved_ids.contains(transcript_id) { continue; }
        let transcript = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let raw_path = b.dir.join(format!("{transcript_id}.raw.txt"));
        let raw = tokio::fs::read_to_string(&raw_path).await.unwrap_or_default();
        let modified = tokio::fs::metadata(&raw_path).await.ok().and_then(|m| m.modified().ok())
            .or(file.metadata().await.ok().and_then(|m| m.modified().ok())).unwrap_or(SystemTime::UNIX_EPOCH);
        archive.push((modified, json!({"id":transcript_id,"transcript":transcript,"raw":raw,"intent":"other","mode":"quick"})));
    }
    archive.sort_by_key(|(modified, _)| *modified);
    let mut history: Vec<Value> = archive.into_iter().map(|(_, item)| item).collect();
    let mut ids: HashSet<String> = history.iter().filter_map(|item| item["id"].as_str().map(str::to_owned)).collect();
    for item in saved { if item["id"].as_str() != Some(id) && item["id"].as_str().is_some_and(|value| ids.insert(value.to_owned())) { history.push(item); } }
    Ok(history)
}
async fn refresh_history(b: &Backend, id: &str, history: &mut Vec<Value>, recover: bool) -> Result<(), String> {
    // Call under the history lock. Fresh saved entries supersede the per-finish
    // snapshot; unindexed transcript files from that snapshot remain intact.
    let latest = if recover { load_history(b, id).await? }
        else if let Some(saved) = saved_history(b).await { saved }
        else { load_history(b, id).await? };
    let latest_ids: HashSet<&str> = latest.iter().filter_map(|item| item["id"].as_str()).collect();
    history.retain(|item| item["id"].as_str().is_some_and(|value| value != id && !latest_ids.contains(value)));
    drop(latest_ids);
    let mut ids: HashSet<String> = history.iter().filter_map(|item| item["id"].as_str().map(str::to_owned)).collect();
    for item in latest { if item["id"].as_str() != Some(id) && item["id"].as_str().is_some_and(|value| ids.insert(value.to_owned())) { history.push(item); } }
    Ok(())
}
#[derive(Serialize)] #[serde(rename_all = "camelCase")] struct HistoryItem { id: String, title: String, timestamp_ms: u128 }
fn history_id(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".md")?;
    let (id, variant) = stem.split_once('.').unwrap_or((stem, ""));
    if !variant.is_empty() && !["quick", "pro"].contains(&variant) { return None; }
    uuid::Uuid::parse_str(id).ok().map(|_| id)
}
async fn history_document(b: &Backend, id: &str) -> PathBuf {
    let mut latest: Option<(SystemTime, PathBuf)> = None;
    for suffix in ["", ".quick", ".pro"] {
        let path = b.dir.join(format!("{id}{suffix}.md"));
        if let Ok(metadata) = tokio::fs::metadata(&path).await {
            let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
            if latest.as_ref().is_none_or(|(date, _)| modified > *date) { latest = Some((modified, path)); }
        }
    }
    latest.map(|(_, path)| path).unwrap_or_else(|| b.dir.join(format!("{id}.md")))
}
#[tauri::command] async fn history_list(b: State<'_, Backend>) -> Result<Vec<HistoryItem>, String> {
    let mut latest = HashMap::<String, (SystemTime, PathBuf)>::new(); let mut files = tokio::fs::read_dir(&b.dir).await.map_err(err)?;
    while let Some(file) = files.next_entry().await.map_err(err)? {
        let path = file.path(); let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Some(id) = history_id(name) else { continue; };
        let metadata = file.metadata().await.map_err(err)?;
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        if latest.get(id).is_none_or(|(date, _)| modified > *date) { latest.insert(id.to_owned(), (modified, path)); }
    }
    let mut items = Vec::with_capacity(latest.len());
    for (id, (modified, path)) in latest {
        let timestamp_ms = modified.duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
        let markdown = tokio::fs::read_to_string(path).await.unwrap_or_default();
        let output = markdown.rsplit_once("\n==========\n\n## Final Output\n\n").map(|(_, rest)| rest).unwrap_or(&markdown);
        let title = output.lines().filter_map(|line| {
            let heading = line.trim(); let depth = heading.bytes().take_while(|b| *b == b'#').count();
            (depth > 0 && depth <= 6).then(|| heading[depth..].trim()).filter(|text| !text.is_empty() && !["Hints", "Voice Recording", "Raw Transcript", "Final Output"].contains(text))
        }).collect::<Vec<_>>().join(" · ");
        let title = if title.is_empty() { output.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("Voice recording").chars().take(120).collect() } else { title };
        items.push(HistoryItem { id, title, timestamp_ms });
    }
    items.sort_unstable_by(|a,b| b.timestamp_ms.cmp(&a.timestamp_ms)); Ok(items)
}
#[tauri::command] async fn history_entry(id: String, b: State<'_, Backend>) -> Result<Value, String> {
    uuid::Uuid::parse_str(&id).map_err(err)?;
    let markdown = tokio::fs::read_to_string(history_document(&b, &id).await).await.map_err(err)?;
    let text = markdown.rsplit_once("\n==========\n\n## Final Output\n\n").map(|(_, output)| output.trim()).unwrap_or(&markdown);
    let history = load_history(&b, "").await?;
    if let Some(format) = history.iter().find(|item| item["id"].as_str() == Some(&id)).and_then(|item| item["output_format"].as_str()) {
        Ok(json!({"text": text, "output_format": format}))
    } else { Ok(json!(text)) }
}
#[tauri::command] async fn history_context_audit(id: Option<String>, b: State<'_, Backend>) -> Result<Value, String> {
    if let Some(id) = &id { uuid::Uuid::parse_str(id).map_err(err)?; }
    let _guard = b.history_lock.lock().await;
    let history = load_history(&b, id.as_deref().unwrap_or("")).await?;
    let preferences = b.preferences.lock().await.clone(); let terms = b.vocabulary.lock().await.clone();
    let summary = context::cached(&b.dir, &history)?.ok_or("Compact history is not current; finish a recording to refresh it")?;
    let corrections = vocabulary::load_corrections(&b.dir)?;
    Ok(processing::audit(&processing::request_with_preferences("", &summary, &preferences, &terms, &vocabulary::correction_hints(&corrections), processing::DEFAULT_PROMPT)))
}
#[tauri::command] async fn finish_recording(id: String, b: State<'_, Backend>) -> Result<Finished, String> {
    let mode = "normal";
    let path = audio_path(&b, &id)?;
    let mut screenshots = Some(b.screens.take(&id));
    let preferences = b.preferences.lock().await.clone();
    let terms = b.vocabulary.lock().await.clone();
    let formatting_prompt = load_formatting_prompt(&b.dir).await;
    let transcription_model = if preferences.bypass_gpt_transcribe { "nemotron-speech-streaming-en-0.6b-Q8_0" } else { transcription::MODEL }.to_owned();
    let cache = b.dir.join(format!("{id}.gpt-transcript.json"));
    let live_cache = b.dir.join(format!("{id}.live.txt"));
    b.recordings.close(&id).await;
    let drain_started = std::time::Instant::now();
    let (gpt, history, live) = tokio::join!(
        async {
            if preferences.bypass_gpt_transcribe { return Ok::<_, String>((String::new(), 0)); }
            if let Ok(bytes) = tokio::fs::read(&cache).await {
                let transcript: transcription::Transcript = serde_json::from_slice(&bytes).map_err(err)?;
                if transcript.model != transcription::MODEL { return Err("Cached transcript model does not match GPT transcription".into()); }
                Ok((transcript.text, 0))
            } else {
                let started = std::time::Instant::now();
                let transcript = transcription::refine(&b.client, key(&b)?, &path, &terms, preferences.language_code()).await?;
                let elapsed = started.elapsed().as_millis() as u64;
                tokio::fs::write(&cache, serde_json::to_vec(&transcript).map_err(err)?).await.map_err(err)?;
                Ok((transcript.text, elapsed))
            }
        },
        async { let _guard = b.history_lock.lock().await; load_history(&b, &id).await },
        async {
            let text = b.local_speech.finish(&id).await;
            (text, drain_started.elapsed().as_millis() as u64)
        }
    );
    let (live, drain_ms) = live;
    if let Ok(preview) = &live { if !preview.trim().is_empty() { tokio::fs::write(&live_cache, preview).await.map_err(err)?; } }
    let (raw, transcription_ms) = if preferences.bypass_gpt_transcribe {
        let text = match live {
            Ok(text) if !text.trim().is_empty() => text,
            outcome => match tokio::fs::read_to_string(&live_cache).await {
                Ok(cached) => cached,
                Err(_) => outcome?,
            },
        };
        (text, drain_ms)
    } else { gpt? };
    let raw_path = b.dir.join(format!("{id}.raw.txt"));
    if !tokio::fs::try_exists(&raw_path).await.map_err(err)? { tokio::fs::write(&raw_path, &raw).await.map_err(err)?; }
    if raw.trim().is_empty() { b.screens.remove(&id); return Ok(Finished { text: raw, intent: "other".into(), warning: Some("No speech detected · audio saved".into()), model: None, reasoning: None, transcription_model, post_processing_profile: preferences.post_processing_profile, transcription_ms, formatting_ms: 0, output_format: "text".into() }); }
    let mut history = history?;
    {
        let guard = b.history_lock.lock().await;
        // Include outputs completed while this stream was being finalized.
        refresh_history(&b, &id, &mut history, *guard).await?;
    }
    let mut audit_update = None;
    let mut formatting_ms = 0;
    let result: Result<(String, String, Option<String>, Option<String>), String> = async {
        let local = preferences.post_processing_profile == "local-sglang";
        let auth = if local { b.key.as_deref().unwrap_or("") } else { key(&b)? };
        let corrections = vocabulary::load_corrections(&b.dir)?;
        let hints = vocabulary::correction_hints(&corrections);
        let summary = if local { String::new() } else {
            let _guard = b.summary_lock.lock().await;
            context::compact(&b.client, auth, &b.dir, &history, &preferences).await?
        };
        let context = if local { "" } else { summary.as_str() };
        let formatting_hints = if local { "" } else { hints.as_str() };
        let mut body = processing::request_with_preferences(&raw, context, &preferences, &terms, formatting_hints, &formatting_prompt);
        if processing::history_words(&body) >= 500 {
            return Err("Combined compact history and spelling hints exceed the 499-word budget; no formatting request was sent.".into());
        }
        if let Some(screenshots) = screenshots.take() { if preferences.send_images { screen::attach(&mut body, &screenshots); } }
        let mut audit = processing::audit(&body);
        audit["transcription_model"] = json!(transcription_model);
        audit["transcription_backend"] = json!(if preferences.bypass_gpt_transcribe { "cpu" } else { "openai" });
        audit["transcription_ms"] = json!(transcription_ms);
        audit["post_processing_profile"] = json!(preferences.post_processing_profile);
        audit["live_transcription_model"] = json!("nemotron-speech-streaming-en-0.6b-Q8_0");
        audit["live_transcription_backend"] = json!("cpu");
        let audit_path = b.dir.join(format!("{id}.{mode}.request-audit.json"));
        audit["delivery"] = json!("prepared; provider receipt unverified");
        // A prepared audit must still be durable before sending the requested call.
        tokio::fs::write(&audit_path, serde_json::to_vec(&audit).map_err(err)?).await.map_err(err)?;
        let response = processing::complete(&b.client, b.key.as_deref(), &body, &preferences).await?;
        drop(body);
        formatting_ms = response["latency_ms"].as_u64().unwrap_or(0);
        audit["formatting_ms"] = json!(formatting_ms);
        audit["delivery"] = json!("provider response received; semantic use of history unverified");
        audit["provider_status"] = response["status"].clone();
        audit["model_returned"] = response["model"].clone();
        audit["reasoning_returned"] = response["reasoning"]["effort"].clone();
        audit["input_tokens"] = response["usage"]["input_tokens"].clone();
        audit_update = Some((audit_path.clone(), serde_json::to_vec(&audit).map_err(err)?));
        let parsed = processing::response_text(&response)
            .and_then(|text| processing::format(serde_json::from_str(&text).map_err(err)?));
        let (text, intent) = match parsed {
            Ok(result) => result,
            Err(first_error) if preferences.post_processing_profile == "local-sglang" => {
                // A weak local model can degenerate into repeated empty groups.
                // One single-leaf retry preserves YAML instead of returning raw text.
                audit["formatting_retry"] = json!(true);
                let retry_body = processing::request_with_preferences(&raw, context, &preferences, &terms, formatting_hints, processing::RECOVERY_PROMPT);
                let retry = processing::complete(&b.client, b.key.as_deref(), &retry_body, &preferences).await?;
                formatting_ms = formatting_ms.saturating_add(retry["latency_ms"].as_u64().unwrap_or(0));
                audit["formatting_ms"] = json!(formatting_ms);
                audit["retry_status"] = retry["status"].clone();
                audit["retry_model_returned"] = retry["model"].clone();
                audit_update = Some((audit_path, serde_json::to_vec(&audit).map_err(err)?));
                processing::response_text(&retry)
                    .and_then(|text| processing::format(serde_json::from_str(&text).map_err(err)?))
                    .map_err(|retry_error| format!("{first_error}; single-leaf retry failed: {retry_error}"))?
            }
            Err(first_error) => return Err(first_error),
        };
        Ok((text, intent, response["model"].as_str().map(str::to_owned), response["reasoning"]["effort"].as_str().map(str::to_owned)))
    }.await;
    b.screens.remove(&id);
    // Receipt auditing and output persistence are independent. Join even on a
    // formatting/save failure; receipt-write errors remain best-effort as before.
    let ((), saved) = tokio::join!(
        async {
            if let Some((path, bytes)) = audit_update { let _ = tokio::fs::write(path, bytes).await; }
        },
        async {
            let output_path = b.dir.join(format!("{id}.txt"));
            let (text, intent, warning, model, reasoning) = match result {
                Ok((text, intent, model, reasoning)) => (text, intent, None, model, reasoning),
                Err(e) if tokio::fs::try_exists(&output_path).await.unwrap_or(false) => return Err(format!("Processing failed · existing output kept: {e}")),
                Err(e) => (raw.clone(), "unclassified".into(), Some(format!("Raw transcript saved · postprocessing failed: {e}")), None, None)
            };
            let final_section = warning.as_ref().map(|w| format!("_Formatting unavailable._\n\n{w}")).unwrap_or_else(|| text.clone());
            let document = processing::document(&raw, &final_section);
            let mut guard = b.history_lock.lock().await;
            let recover = *guard;
            *guard = true;
            let (output_saved, document_saved, refreshed) = tokio::join!(
                tokio::fs::write(&output_path, &text),
                tokio::fs::write(b.dir.join(format!("{id}.md")), document),
                refresh_history(&b, &id, &mut history, recover)
            );
            output_saved.map_err(err)?;
            document_saved.map_err(err)?;
            refreshed?;
            let output_format = if warning.is_none() { "yaml" } else { "text" }.to_owned();
            history.push(json!({"id":id,"transcript":text,"raw":raw,"intent":intent,"mode":mode,"model":model,"reasoning":reasoning,"output_format":output_format,"post_processing_profile":preferences.post_processing_profile,"transcription_model":transcription_model,"transcription_ms":transcription_ms,"formatting_ms":formatting_ms}));
            let archive = serde_json::to_vec(&history).map_err(err)?;
            let (archived, terms) = tokio::join!(
                tokio::fs::write(b.dir.join("speech-context.json"), archive),
                async { vocabulary::load_corrections(&b.dir).map(|corrections| vocabulary::scan(&history, &corrections)) }
            );
            archived.map_err(err)?;
            *guard = false;
            *b.vocabulary.lock().await = terms?;
            Ok(Finished { text, intent, warning, model, reasoning, transcription_model, post_processing_profile: preferences.post_processing_profile, transcription_ms, formatting_ms, output_format })
        }
    );
    saved
}
#[tauri::command] fn open_audio_folder(b: State<'_, Backend>) -> Result<(), String> { std::process::Command::new("explorer.exe").arg(&b.dir).spawn().map_err(err)?; Ok(()) }
#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn OpenClipboard(owner: isize) -> i32;
    fn EmptyClipboard() -> i32;
    fn SetClipboardData(format: u32, memory: isize) -> isize;
    fn CloseClipboard() -> i32;
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
#[tauri::command] fn capture_paste_target() -> Option<paste::Target> { paste::capture() }
#[tauri::command] async fn paste_result(target: Option<paste::Target>) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || paste::send(target)).await.map_err(err)
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
        let _ = w.unminimize(); let _ = w.center(); let _ = w.show();
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
#[cfg(windows)]
fn allow_widget_microphone(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use webview2_com::Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_PERMISSION_KIND, COREWEBVIEW2_PERMISSION_KIND_MICROPHONE, COREWEBVIEW2_PERMISSION_STATE_ALLOW};
    window.with_webview(|webview| {
        let Ok(core) = (unsafe { webview.controller().CoreWebView2() }) else { return; };
        let handler = webview2_com::PermissionRequestedEventHandler::create(Box::new(|_, args| {
            if let Some(args) = args {
                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                unsafe {
                    args.PermissionKind(&mut kind)?;
                    if kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE { args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?; }
                }
            }
            Ok(())
        }));
        let mut token = 0i64;
        let _ = unsafe { core.add_PermissionRequested(&handler, &mut token) };
    })
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
            if event.state() == ShortcutState::Pressed {
                if !held.swap(true, Ordering::Relaxed) {
                    let app_handle = app.clone();
                    let main_thread_app = app_handle.clone();
                    let _ = app_handle.run_on_main_thread(move || {
                        if let Some(window) = main_thread_app.get_webview_window("main") { let _ = window.unminimize(); let _ = window.show(); }
                        let _ = main_thread_app.emit("toggle-recording", ());
                    });
                }
            } else { held.store(false, Ordering::Relaxed); }
        }}).build())
        .setup(|app| {
            if let Ok(exe) = std::env::current_exe() { if let Some(dir) = exe.parent() { let _ = dotenvy::from_path(dir.join(".env")); } }
            let _ = dotenvy::dotenv();
            let dir = app.path().app_local_data_dir()?.join("recordings"); std::fs::create_dir_all(&dir)?;
            let preferences = preferences::Preferences::load(&dir.join("preferences.json"));
            let local_speech = std::sync::Arc::new(local_speech::LocalSpeech::default());
            let warm = local_speech.clone();
            tauri::async_runtime::spawn_blocking(move || { if let Err(error) = warm.warm() { eprintln!("{error}"); } });
            app.manage(Backend { client: reqwest::Client::builder().timeout(Duration::from_secs(120)).build()?, key: env("OPENAI_API_KEY"), local_speech, summary_lock: Mutex::new(()), screens: screen::Screens::default(), recordings: Recorder::default(), dir, ui_ready: AtomicBool::new(false), history_lock: Mutex::new(false), preferences: Mutex::new(preferences), vocabulary: Mutex::new(Vec::new()) });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let b = handle.state::<Backend>();
                let _ = refresh_vocabulary(handle.state::<Backend>()).await;
                if b.key.is_some() && b.preferences.lock().await.post_processing_profile == "openai" {
                    let history = { let _guard = b.history_lock.lock().await; load_history(&b, "").await };
                    if let Ok(history) = history {
                        let preferences = b.preferences.lock().await.clone();
                        let auth = b.key.as_deref().unwrap_or("");
                        let _guard = b.summary_lock.lock().await;
                        if let Err(error) = context::compact(&b.client, auth, &b.dir, &history, &preferences).await { eprintln!("History compaction: {error}"); }
                    }
                }
            });
            app.global_shortcut().register("Ctrl+Shift+Space")?;
            create_tray(app)?;
            if let Some(window) = app.get_webview_window("main") {
                #[cfg(windows)] allow_widget_microphone(&window)?;
                let _ = window.hide();
            }
            Ok(())
        })
        .on_window_event(|window, event| { if let tauri::WindowEvent::CloseRequested { api, .. } = event { api.prevent_close(); let _ = window.hide(); } })
        .invoke_handler(tauri::generate_handler![get_preferences, save_preferences, get_formatting_prompt, save_formatting_prompt, reset_formatting_prompt, refresh_vocabulary, startup_status, history_context_audit, history_list, history_entry, stop_screen_context, abort_live, begin_recording, append_audio, finish_recording, open_audio_folder, copy_result, capture_paste_target, paste_result, quit_app, recording_beep, set_tray_state])
        .run(tauri::generate_context!()).expect("Tiny Voice failed to start");
}
