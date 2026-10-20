use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashSet, time::{Duration, Instant}};

/// Editable in Settings. The strict output schema stays native so an edited
/// prompt cannot change the response contract or bypass validation.
pub const DEFAULT_PROMPT: &str = include_str!("../prompts/formatting.txt");
pub const MAX_PROMPT_BYTES: usize = 20_000;
pub const RECOVERY_PROMPT: &str = r#"<role>Clean one dictated transcript faithfully. Return exactly one leaf field. Do not nest, repeat, or add commentary.</role>
<output_contract>The response is the strict JSON object required by the schema. fields must contain exactly one leaf: {"key":"message","value":"the complete cleaned text","children":[]}. Preserve names, numbers, negation, uncertainty, and self-corrections. No groups, arrays, Markdown, or JSON text in values. Choose intent from the speech.</output_contract>"#;
const INTENTS: [&str; 6] = ["question", "request", "command", "statement", "greeting", "other"];

pub fn validate_prompt(prompt: &str) -> Result<(), String> {
    if prompt.trim().is_empty() { return Err("The formatting prompt must not be empty".into()); }
    if prompt.len() > MAX_PROMPT_BYTES { return Err(format!("The formatting prompt must be at most {MAX_PROMPT_BYTES} bytes")); }
    if let Some(character) = prompt.chars().find(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t')) {
        return Err(format!("The formatting prompt contains unsupported control character U+{:04X}", character as u32));
    }
    Ok(())
}

fn output_schema() -> Value {
    // Explicit depth levels keep the schema strict-compatible. Root fields are
    // leaves, two-level groups, or three-level groups; deeper trees fail here
    // and in native validation.
    let leaf = json!({"type":"object","properties":{
        "key":{"type":"string","description":"Nonempty key, unique within its mapping"},
        "value":{"type":"string","description":"Nonempty string value"},
        "children":{"type":"array","maxItems":0,"items":{"type":"object"}}
    },"required":["key","value","children"],"additionalProperties":false});
    let group2 = json!({"type":"object","properties":{
        "key":{"type":"string","description":"Nonempty group key, unique within its mapping"},
        "value":{"type":"null"},
        "children":{"type":"array","minItems":2,"items":leaf}
    },"required":["key","value","children"],"additionalProperties":false});
    let group3 = json!({"type":"object","properties":{
        "key":{"type":"string","description":"Nonempty group key, unique within its mapping"},
        "value":{"type":"null"},
        "children":{"type":"array","minItems":2,"items":group2}
    },"required":["key","value","children"],"additionalProperties":false});
    json!({"type":"object","properties":{
        "fields":{"type":"array","minItems":1,"items":{"anyOf":[leaf, group2, group3]}},
        "intent":{"type":"string","enum":INTENTS}
    },"required":["fields","intent"],"additionalProperties":false})
}

/// A recursive node is easier for the small local model than the explicit
/// three-level union. Native validation still enforces the three-level limit.
fn local_output_schema() -> Value {
    json!({"type":"object","$defs":{
        "node":{"type":"object","properties":{
            "key":{"type":"string","description":"Nonempty short key"},
            "value":{"type":["string","null"],"description":"String for a leaf; null for a group"},
            "children":{"type":"array","items":{"$ref":"#/$defs/node"},"description":"Group children; empty for a leaf"}
        },"required":["key","value","children"],"additionalProperties":false}
    },"properties":{
        "fields":{"type":"array","minItems":1,"items":{"$ref":"#/$defs/node"}},
        "intent":{"type":"string","enum":INTENTS}
    },"required":["fields","intent"],"additionalProperties":false})
}

pub fn request_with_preferences(raw: &str, context: &str, preferences: &crate::preferences::Preferences, vocabulary: &[String], correction_hints: &str, instructions: &str) -> Value {
    let local = preferences.post_processing_profile == "local-sglang";
    json!({"model":if local { &preferences.local_model } else { &preferences.model },"store":false,
        "reasoning":{"effort":if local { Value::Null } else { json!(preferences.reasoning) }},"max_output_tokens":16384,
        "instructions":instructions,
        "input":[{"role":"user","content":[{"type":"input_text","text":json!({"compact_history":context,"current_transcript":raw,"observed_vocabulary":vocabulary,"spelling_corrections":correction_hints}).to_string()}]}],
        "text":{"format":{"type":"json_schema","name":"logical_dictation","strict":true,"schema":output_schema()}}
    })
}

struct ByteCount(usize);
impl std::io::Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { self.0 += bytes.len(); Ok(bytes.len()) }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}
fn serialized_len(value: &Value) -> usize { let mut count = ByteCount(0); let _ = serde_json::to_writer(&mut count,value); count.0 }

fn request_input(body: &Value) -> Value {
    body["input"][0]["content"].as_array().and_then(|content| content.iter().find(|part| part["type"] == "input_text"))
        .and_then(|part| part["text"].as_str()).and_then(|text| serde_json::from_str::<Value>(text).ok()).unwrap_or(Value::Null)
}

fn input_history_words(input: &Value) -> usize {
    input["compact_history"].as_str().map_or(0, words)
        + input["observed_vocabulary"].as_array().map_or(0, |terms| terms.iter().filter_map(Value::as_str).map(words).sum::<usize>())
        + input["spelling_corrections"].as_str().map_or(0, words)
}

/// Counts every history-derived word sent with formatting, including multiword
/// vocabulary terms, using the same whitespace rule as compaction.
pub fn history_words(body: &Value) -> usize { input_history_words(&request_input(body)) }

// Counts only: never persist speech, transcript IDs or credentials in request audits.
pub fn audit(body: &Value) -> Value {
    let input = request_input(body);
    json!({"compact_history_words":input["compact_history"].as_str().map_or(0, |text| text.split_whitespace().count()),
        "historical_context_words":input_history_words(&input),
        "api_input_messages":body["input"].as_array().map_or(0, Vec::len),
        "current_transcript_present":input["current_transcript"].is_string(),
        "image_attachments":body["input"][0]["content"].as_array().map_or(0, |c| c.iter().filter(|p| p["type"] == "input_image").count()),
        "image_detail":body["input"][0]["content"].as_array().and_then(|c| c.iter().find(|p| p["type"] == "input_image")).map_or(Value::Null, |p| p["detail"].clone()),
        "vocabulary_terms":input["observed_vocabulary"].as_array().map_or(0, Vec::len),
        "request_bytes":serialized_len(body),"model_requested":body["model"],"reasoning_requested":body["reasoning"]["effort"]})
}

fn words(text: &str) -> usize { text.split_whitespace().count() }
pub fn document(raw: &str, final_text: &str) -> String {
    let fence = "`".repeat(raw.split(|c| c != '`').map(str::len).max().unwrap_or(0).max(2)+1);
    format!("# Voice Recording\n\n## Raw Transcript\n\n{fence}text\n{raw}\n{fence}\n\n==========\n\n## Final Output\n\n{final_text}\n")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalOutput { fields: Vec<LogicalField>, intent: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalField { key: String, value: Option<String>, #[serde(default)] children: Vec<LogicalField> }

/// Renders one self-contained YAML scalar. Every single-line scalar is double
/// quoted and escaped so text such as `true`, `123`, `# comment`, `!tag`, or
/// `null` can never be reinterpreted as another YAML type.
fn yaml_scalar(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            '\u{8}' => quoted.push_str("\\b"),
            '\u{c}' => quoted.push_str("\\f"),
            // YAML treats these as line breaks or a byte-order mark even inside
            // quoted scalars, so escape them to keep the value exactly.
            '\u{85}' => quoted.push_str("\\u0085"),
            '\u{2028}' => quoted.push_str("\\u2028"),
            '\u{2029}' => quoted.push_str("\\u2029"),
            '\u{feff}' => quoted.push_str("\\uFEFF"),
            control if control.is_control() => quoted.push_str(&format!("\\u{:04X}", control as u32)),
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

/// Plain keys stay readable; anything ambiguous is quoted instead.
fn plain_key_safe(key: &str) -> bool {
    let bytes = key.as_bytes();
    if bytes.first().is_none_or(|byte| !byte.is_ascii_alphabetic()) { return false; }
    if !bytes.iter().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_' | b'-')) { return false; }
    if key.ends_with(' ') || key.ends_with(':') { return false; }
    !matches!(key.to_ascii_lowercase().as_str(), "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n")
}

fn yaml_key(key: &str) -> String {
    if plain_key_safe(key) { key.to_owned() } else { yaml_scalar(key) }
}

/// Multi-line LF-only text with no trailing spaces can be a literal block
/// scalar. `|2-` preserves leading indentation exactly and drops the synthetic
/// final line break. Everything else falls back to an escaped quoted scalar.
fn block_safe(value: &str) -> bool {
    if !value.contains('\n') || value.starts_with('\n') || value.ends_with('\n') || value.contains('\r') { return false; }
    if value.contains('\u{85}') || value.contains('\u{2028}') || value.contains('\u{2029}') || value.contains('\u{feff}') { return false; }
    value.lines().all(|line| !line.ends_with(' ') && !line.ends_with('\t') && !line.contains('\t'))
        && !value.chars().any(|character| character.is_control() && character != '\n')
}

fn yaml_value(value: &str, indent: &str) -> String {
    if !block_safe(value) { return yaml_scalar(value); }
    let mut rendered = String::from("|2-\n");
    for (index, line) in value.split('\n').enumerate() {
        if index > 0 { rendered.push('\n'); }
        if !line.is_empty() { rendered.push_str(indent); rendered.push_str(line); }
    }
    rendered
}

fn validate_key(key: &str, scope: &str) -> Result<(), String> {
    if key.trim().is_empty() { return Err(format!("{scope} key must not be empty")); }
    if key.chars().any(char::is_control) { return Err(format!("{scope} key must not contain control characters: {key:?}")); }
    Ok(())
}
fn validate_value(value: &str, scope: &str) -> Result<(), String> {
    if value.trim().is_empty() { return Err(format!("{scope} value must not be empty")); }
    Ok(())
}

const MAX_YAML_DEPTH: usize = 3;

/// A one-child group is redundant. Promote its child/value into the parent so
/// weak local models that emit `message: message: message: "..."` still render
/// clean YAML. This also keeps the schema's `minItems: 2` guideline recoverable.
fn normalize_fields(fields: Vec<LogicalField>) -> Vec<LogicalField> {
    let mut normalized: Vec<LogicalField> = fields.into_iter().map(|mut field| {
        field.children = normalize_fields(field.children);
        if field.value.is_none() {
            while field.children.len() == 1 {
                let child = field.children.remove(0);
                field.value = child.value;
                field.children = child.children;
            }
        }
        field
    }).collect();
    let mut unique: Vec<LogicalField> = Vec::with_capacity(normalized.len());
    for mut field in normalized.drain(..) {
        if let Some(existing) = unique.iter_mut().find(|item| item.key == field.key) {
            if existing.children.is_empty() && field.children.is_empty() {
                if existing.value != field.value {
                    let mut value = existing.value.take().unwrap_or_default();
                    if !value.is_empty() { value.push('\n'); }
                    value.push_str(field.value.as_deref().unwrap_or(""));
                    existing.value = Some(value);
                }
                continue;
            }
            if !existing.children.is_empty() && !field.children.is_empty() {
                let mut children = std::mem::take(&mut existing.children);
                children.extend(std::mem::take(&mut field.children));
                existing.children = normalize_fields(children);
                continue;
            }
            let base = field.key.clone();
            let mut suffix = 2;
            while unique.iter().any(|item| item.key == field.key) {
                field.key = format!("{base}_{suffix}");
                suffix += 1;
            }
        }
        unique.push(field);
    }
    unique
}

fn render_fields(fields: &[LogicalField], indent: &str, depth: usize, parent: Option<&str>, lines: &mut Vec<String>) -> Result<(), String> {
    if fields.is_empty() { return Err("Missing logical fields".into()); }
    let scope = match depth { 1 => "Root", 2 => "Child", _ => "Nested" };
    let mut keys = HashSet::new();
    for field in fields {
        validate_key(&field.key, scope)?;
        if !keys.insert(field.key.as_str()) {
            return Err(match depth {
                1 => format!("Duplicate root key: {:?}", field.key),
                2 => format!("Duplicate child key under {:?}: {:?}", parent.unwrap_or(""), field.key),
                _ => format!("Duplicate nested key under {:?}: {:?}", parent.unwrap_or(""), field.key),
            });
        }
        let key = yaml_key(&field.key);
        if field.children.is_empty() {
            let value = field.value.as_deref().ok_or_else(|| format!("Field {:?} has no value or child fields", field.key))?;
            validate_value(value, &format!("Field {:?}", field.key))?;
            lines.push(format!("{indent}{key}: {}", yaml_value(value, &format!("{indent}  "))));
            continue;
        }
        if depth >= MAX_YAML_DEPTH { return Err("YAML nesting is limited to three mappings".into()); }
        if field.value.is_some() {
            return Err(format!("Field {:?} has both a value and child fields", field.key));
        }
        lines.push(format!("{indent}{key}:"));
        render_fields(&field.children, &format!("{indent}  "), depth + 1, Some(&field.key), lines)?;
    }
    Ok(())
}

fn render_yaml(fields: &[LogicalField]) -> Result<String, String> {
    let mut lines = Vec::new();
    render_fields(fields, "", 1, None, &mut lines)?;
    Ok(lines.join("\n"))
}

pub fn response_text(response: &Value) -> Result<String, String> {
    if response["status"] == "incomplete" { return Err("Processing reached its output limit · audio and raw transcript retained.".into()); }
    let mut text = String::new();
    if let Some(output) = response["output"].as_array() {
        for item in output {
            if let Some(content) = item["content"].as_array() {
                for part in content {
                    if part["type"] == "refusal" { return Err(part["refusal"].as_str().unwrap_or("The model declined this transcript").into()); }
                    if part["type"] == "output_text" { text.push_str(part["text"].as_str().unwrap_or("")); }
                }
            }
        }
    }
    Ok(text)
}

pub fn format(value: Value) -> Result<(String, String), String> {
    let mut output: LogicalOutput = serde_json::from_value(value).map_err(|error| format!("Invalid logical dictation JSON: {error}"))?;
    if !INTENTS.contains(&output.intent.as_str()) { return Err("Unsupported dictation intent".into()); }
    output.fields = normalize_fields(output.fields);
    let yaml = render_yaml(&output.fields)?;
    Ok((yaml, output.intent))
}

fn local_url(endpoint: &str) -> String { format!("{}/chat/completions", endpoint.trim_end_matches('/')) }

/// Converts a Responses-shaped request into an SGLang Chat Completions request.
/// The same instructions, input text, and JSON schema are reused for every
/// local caller, including history compaction. Screenshots are rejected rather
/// than silently dropped because this profile is text-only.
fn local_request(body: &Value, preferences: &crate::preferences::Preferences) -> Result<Value, String> {
    let input = body["input"].as_array().ok_or("Local SGLang request is missing input messages")?;
    let mut messages = Vec::with_capacity(input.len() + 1);
    if let Some(instructions) = body["instructions"].as_str() {
        if !instructions.trim().is_empty() { messages.push(json!({"role":"system","content":instructions})); }
    }
    for (index, message) in input.iter().enumerate() {
        let role = message["role"].as_str().ok_or_else(|| format!("Local SGLang input message {index} has no role"))?;
        if !matches!(role, "user" | "assistant" | "system") { return Err(format!("Local SGLang does not support the input role {role}")); }
        let content = match message.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(parts)) => {
                let mut texts = Vec::with_capacity(parts.len());
                for part in parts {
                    let kind = part["type"].as_str().ok_or_else(|| format!("Local SGLang input message {index} has a content part without a type"))?;
                    match kind {
                        "input_text" | "text" | "output_text" => {
                            texts.push(part["text"].as_str().ok_or_else(|| format!("Local SGLang {kind} content must contain text"))?);
                        }
                        "input_image" | "image_url" | "image" => return Err("Local SGLang profile is text-only; disable desktop screenshots or choose the OpenAI profile.".into()),
                        other => return Err(format!("Local SGLang does not support content type {other}")),
                    }
                }
                if texts.is_empty() { return Err(format!("Local SGLang input message {index} has no text content")); }
                texts.join("\n")
            }
            _ => return Err(format!("Local SGLang input message {index} has no string or content-part array")),
        };
        messages.push(json!({"role":role,"content":content}));
    }
    if messages.is_empty() { return Err("Local SGLang request has no messages".into()); }
    let maximum = body["max_output_tokens"].as_u64().or_else(|| body["max_tokens"].as_u64()).unwrap_or(4096);
    let mut local = json!({"model":preferences.local_model,"messages":messages,"temperature":0,"stream":false,
        "max_tokens":maximum.clamp(1,1024),"chat_template_kwargs":{"enable_thinking":false}});
    if let Some(format) = body["text"]["format"].as_object() {
        let kind = format.get("type").and_then(Value::as_str).ok_or("Local SGLang text format has no type")?;
        match kind {
            "json_schema" => {
                local["response_format"] = json!({"type":"json_schema","json_schema":{
                    "name":format.get("name").and_then(Value::as_str).unwrap_or("structured_output"),
                    "strict":format.get("strict").and_then(Value::as_bool).unwrap_or(true),
                    "schema":local_output_schema()
                }});
            }
            "text" => {}
            other => return Err(format!("Local SGLang does not support the {other} text format")),
        }
    }
    Ok(local)
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn api_error(profile: &str, status: reqwest::StatusCode, response: &Value) -> String {
    let message = response["error"]["message"].as_str().or_else(|| response["message"].as_str()).unwrap_or("Request failed");
    format!("{profile} {status}: {message}")
}

fn normalize_cloud(mut response: Value, requested_model: &str, latency_ms: u64) -> Result<Value, String> {
    if !response.is_object() { return Err("OpenAI response was not a JSON object".into()); }
    if !response["reasoning"].is_object() { response["reasoning"] = json!({"effort":Value::Null}); }
    if !response["usage"].is_object() { response["usage"] = json!({}); }
    if !response["output"].is_array() { response["output"] = json!([]); }
    if !response["status"].is_string() { response["status"] = json!("completed"); }
    if !response["model"].is_string() { response["model"] = json!(requested_model); }
    response["latency_ms"] = json!(latency_ms);
    Ok(response)
}

fn normalize_local(response: Value, preferences: &crate::preferences::Preferences, latency_ms: u64) -> Result<Value, String> {
    let choice = response["choices"].as_array().and_then(|choices| choices.first()).ok_or("Local SGLang response has no choices")?;
    let message = choice.get("message").filter(|message| message.is_object()).ok_or("Local SGLang response has no message")?;
    let finish_reason = choice["finish_reason"].as_str().unwrap_or("");
    let status = if finish_reason == "length" { "incomplete" } else { "completed" };
    let content = if let Some(refusal) = message["refusal"].as_str().filter(|refusal| !refusal.trim().is_empty()) {
        json!([{"type":"refusal","refusal":refusal}])
    } else {
        let text = message["content"].as_str().ok_or_else(|| format!("Local SGLang returned no text content (finish_reason {finish_reason:?})"))?;
        json!([{"type":"output_text","text":text}])
    };
    let input_tokens = response["usage"]["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = response["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    let total_tokens = response["usage"]["total_tokens"].as_u64().unwrap_or_else(|| input_tokens.saturating_add(output_tokens));
    let model = response["model"].as_str().unwrap_or(&preferences.local_model);
    Ok(json!({
        "object":"response","status":status,"model":model,
        "reasoning":{"effort":Value::Null},
        "usage":{"input_tokens":input_tokens,"output_tokens":output_tokens,"total_tokens":total_tokens},
        "output":[{"type":"message","role":"assistant","content":content}],
        "latency_ms":latency_ms
    }))
}

/// Sends one generic Responses-shaped request through the selected profile.
/// Local SGLang has no cloud fallback and no cloud key requirement. The timer
/// covers the network response body plus its complete JSON decode.
pub async fn complete(client: &reqwest::Client, auth: Option<&str>, body: &Value, preferences: &crate::preferences::Preferences) -> Result<Value, String> {
    match preferences.post_processing_profile.as_str() {
        "local-sglang" => {
            let request = local_request(body, preferences)?;
            let started = Instant::now();
            let response = client.post(local_url(&preferences.local_endpoint)).json(&request)
                .timeout(Duration::from_secs(240)).send().await
                .map_err(|error| format!("Local SGLang request failed: {error}"))?;
            let status = response.status();
            let bytes = response.bytes().await.map_err(|error| format!("Local SGLang response body failed: {error}"))?;
            let decoded = serde_json::from_slice::<Value>(&bytes);
            let latency_ms = elapsed_ms(started);
            if !status.is_success() {
                return Err(api_error("Local SGLang", status, decoded.as_ref().unwrap_or(&Value::Null)));
            }
            let response = decoded.map_err(|error| format!("Local SGLang returned invalid JSON: {error}"))?;
            normalize_local(response, preferences, latency_ms)
        }
        "openai" => {
            let token = auth.filter(|token| !token.trim().is_empty()).ok_or("OPENAI_API_KEY is required for the OpenAI post-processing profile")?;
            let started = Instant::now();
            let response = client.post("https://api.openai.com/v1/responses").bearer_auth(token).json(body)
                .timeout(Duration::from_secs(240)).send().await
                .map_err(|error| format!("OpenAI processing request failed: {error}"))?;
            let status = response.status();
            let bytes = response.bytes().await.map_err(|error| format!("OpenAI response body failed: {error}"))?;
            let decoded = serde_json::from_slice::<Value>(&bytes);
            let latency_ms = elapsed_ms(started);
            if !status.is_success() {
                return Err(api_error("OpenAI", status, decoded.as_ref().unwrap_or(&Value::Null)));
            }
            let response = decoded.map_err(|error| format!("OpenAI returned invalid JSON: {error}"))?;
            normalize_cloud(response, body["model"].as_str().unwrap_or("unknown"), latency_ms)
        }
        other => Err(format!("Unsupported post-processing profile: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_budget_counts_summary_vocabulary_and_entire_hint_text() {
        let preferences = crate::preferences::Preferences::default();
        let summary = vec!["summary"; 350].join(" ");
        let vocabulary = vec!["two\u{2003}words".to_owned(); 50];
        let hints = vec!["hint"; 49].join("\n");
        let body = request_with_preferences("Current speech is not historical context.", &summary, &preferences, &vocabulary, &hints, DEFAULT_PROMPT);
        assert_eq!(history_words(&body), 499);
        let metrics = audit(&body);
        assert_eq!(metrics["compact_history_words"], 350);
        assert_eq!(metrics["historical_context_words"], 499);
        let over = request_with_preferences("Current speech is not historical context.", &summary, &preferences, &vocabulary, &format!("{hints} extra"), DEFAULT_PROMPT);
        assert_eq!(history_words(&over), 500);
    }

    #[test]
    fn formatting_preserves_multiline_text_and_code_indentation() {
        let expected = "    keep_first_line_indented();\r\n\r\n```rust\r\nfn main() {\r\n    let x = 1;  \r\n}\r\n```\r\n\r\nLast line.";
        let input = json!({"fields":[{"key":"code","value":expected,"children":[]}],"intent":"request"});
        let (yaml, intent) = format(input).unwrap();
        assert_eq!(intent, "request");
        assert_eq!(yaml, "code: \"    keep_first_line_indented();\\r\\n\\r\\n```rust\\r\\nfn main() {\\r\\n    let x = 1;  \\r\\n}\\r\\n```\\r\\n\\r\\nLast line.\"");
    }

    #[test]
    fn yaml_is_shallow_ordered_and_safely_quoted() {
        let input = json!({"fields":[
            {"key":"action","value":null,"children":[{"key":"command","value":"send the release notes"},{"key":"when","value":"tomorrow"}]},
            {"key":"notes","value":"Don't cancel","children":[]}
        ],"intent":"request"});
        let (yaml, intent) = format(input).unwrap();
        assert_eq!(intent, "request");
        assert_eq!(yaml, "action:\n  command: \"send the release notes\"\n  when: \"tomorrow\"\nnotes: \"Don't cancel\"");
    }

    #[test]
    fn redundant_single_child_groups_are_flattened() {
        let input = json!({"fields":[{"key":"message","value":null,"children":[
            {"key":"message","value":null,"children":[
                {"key":"message","value":"Keep this sentence.","children":[]}
            ]}
        ]}],"intent":"statement"});
        let (yaml, _) = format(input).unwrap();
        assert_eq!(yaml, "message: \"Keep this sentence.\"");
        let different_key = json!({"fields":[{"key":"styling","value":null,"children":[
            {"key":"message","value":"Use a darker border.","children":[]}
        ]}],"intent":"statement"});
        assert_eq!(format(different_key).unwrap().0, "styling: \"Use a darker border.\"");
    }

    #[test]
    fn duplicate_model_keys_are_merged_or_disambiguated() {
        let input = json!({"fields":[
            {"key":"message","value":"First sentence.","children":[]},
            {"key":"message","value":"Second sentence.","children":[]},
            {"key":"details","value":null,"children":[{"key":"a","value":"one","children":[]},{"key":"x","value":"first","children":[]}]},
            {"key":"details","value":null,"children":[{"key":"b","value":"two","children":[]},{"key":"y","value":"second","children":[]}]}
        ],"intent":"statement"});
        let (yaml, _) = format(input).unwrap();
        assert_eq!(yaml, "message: |2-\n  First sentence.\n  Second sentence.\ndetails:\n  a: \"one\"\n  x: \"first\"\n  b: \"two\"\n  \"y\": \"second\"");
    }

    #[test]
    fn custom_instructions_are_sent_unchanged() {
        let preferences = crate::preferences::Preferences::default();
        let body = request_with_preferences("Speech.", "", &preferences, &[], "", "CUSTOM PROMPT\nLine two.");
        assert_eq!(body["instructions"], "CUSTOM PROMPT\nLine two.");
    }

    #[test]
    fn multiline_values_use_readable_yaml_block_scalars() {
        let input = json!({"fields":[
            {"key":"notes","value":"first line\nsecond line\n  indented","children":[]},
            {"key":"details","value":null,"children":[{"key":"steps","value":"one\ntwo"},{"key":"owner","value":"Sam"}]}
        ],"intent":"statement"});
        let (yaml, _) = format(input).unwrap();
        assert_eq!(yaml, "notes: |2-\n  first line\n  second line\n    indented\ndetails:\n  steps: |2-\n    one\n    two\n  owner: \"Sam\"");
    }

    #[test]
    fn prompt_validation_rejects_empty_oversized_and_control_text() {
        assert!(validate_prompt(DEFAULT_PROMPT).is_ok());
        assert!(validate_prompt("  \n").unwrap_err().contains("empty"));
        assert!(validate_prompt(&"x".repeat(MAX_PROMPT_BYTES + 1)).unwrap_err().contains("at most"));
        assert!(validate_prompt("prompt\u{0}").unwrap_err().contains("U+0000"));
        assert!(validate_prompt("prompt\r\nwith\ttabs").is_ok());
    }

    #[test]
    fn ambiguous_scalars_and_keys_stay_strings() {
        let input = json!({"fields":[
            {"key":"true","value":"false","children":[]},
            {"key":"123","value":"42","children":[]},
            {"key":"# comment","value":"# not a comment","children":[]},
            {"key":"!tag","value":"!not-a-tag","children":[]},
            {"key":"null","value":"null","children":[]},
            {"key":"level","value":"line one\n\u{2028}!tag: value","children":[]}
        ],"intent":"statement"});
        let (yaml, _) = format(input).unwrap();
        assert_eq!(yaml, "\"true\": \"false\"\n\"123\": \"42\"\n\"# comment\": \"# not a comment\"\n\"!tag\": \"!not-a-tag\"\n\"null\": \"null\"\nlevel: \"line one\\n\\u2028!tag: value\"");
    }

    #[test]
    fn duplicate_keys_are_merged_and_empty_keys_are_rejected() {
        let duplicate_root = json!({"fields":[{"key":"same","value":"one","children":[]},{"key":"same","value":"two","children":[]}],"intent":"statement"});
        assert_eq!(format(duplicate_root).unwrap().0, "same: |2-\n  one\n  two");
        let duplicate_child = json!({"fields":[{"key":"group","value":null,"children":[{"key":"same","value":"one"},{"key":"same","value":"two"}]}],"intent":"statement"});
        assert_eq!(format(duplicate_child).unwrap().0, "group: |2-\n  one\n  two");
        let empty_root = json!({"fields":[{"key":"  ","value":"text","children":[]}],"intent":"statement"});
        assert!(format(empty_root).unwrap_err().contains("must not be empty"));
        let empty_child = json!({"fields":[{"key":"group","value":null,"children":[{"key":"","value":"text"},{"key":"ok","value":"text"}]}],"intent":"statement"});
        assert!(format(empty_child).unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn depth_is_limited_to_three_mappings_and_shapes_are_unambiguous() {
        let three_levels = json!({"fields":[{"key":"deployments","value":null,"children":[
            {"key":"staging","value":null,"children":[{"key":"endpoint","value":"staging.example.com","children":[]},{"key":"timeout","value":"30 seconds","children":[]}]},
            {"key":"production","value":null,"children":[{"key":"endpoint","value":"prod.example.com","children":[]},{"key":"timeout","value":"60 seconds","children":[]}]}
        ]}],"intent":"statement"});
        let (yaml, _) = format(three_levels).unwrap();
        assert_eq!(yaml, "deployments:\n  staging:\n    endpoint: \"staging.example.com\"\n    timeout: \"30 seconds\"\n  production:\n    endpoint: \"prod.example.com\"\n    timeout: \"60 seconds\"");
        let four_levels = json!({"fields":[{"key":"a","value":null,"children":[
            {"key":"b","value":null,"children":[
                {"key":"c","value":null,"children":[{"key":"d","value":"text","children":[]},{"key":"e","value":"other","children":[]}]},
                {"key":"h","value":"sibling","children":[]}
            ]},
            {"key":"g","value":"root","children":[]}
        ]}],"intent":"statement"});
        assert!(format(four_levels).unwrap_err().contains("three mappings"));
        let both = json!({"fields":[{"key":"group","value":"scalar","children":[{"key":"child","value":"text"}]}],"intent":"statement"});
        assert!(format(both).unwrap_err().contains("both a value and child fields"));
        let empty_both = json!({"fields":[{"key":"group","value":"","children":[{"key":"child","value":"text"}]}],"intent":"statement"});
        assert!(format(empty_both).unwrap_err().contains("both a value and child fields"));
        let missing = json!({"fields":[{"key":"group","value":null,"children":[]}],"intent":"statement"});
        assert!(format(missing).unwrap_err().contains("no value or child fields"));
        let empty = json!({"fields":[{"key":"group","value":"","children":[]}],"intent":"statement"});
        assert!(format(empty).unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn local_request_rejects_images_and_uses_local_model_schema_and_thinking_control() {
        let preferences = crate::preferences::Preferences::default();
        let mut body = request_with_preferences("Keep this request.", "", &preferences, &[], "", DEFAULT_PROMPT);
        let local = local_request(&body, &preferences).unwrap();
        assert_eq!(local["model"], "minicpm5-2b-fp8");
        assert_eq!(local["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(local["response_format"]["json_schema"]["schema"]["properties"]["fields"]["type"], "array");
        assert_eq!(local["response_format"]["json_schema"]["schema"]["$defs"]["node"]["type"], "object");
        assert_eq!(local["messages"][0]["role"], "system");
        assert_eq!(local["messages"][1]["role"], "user");
        body["input"][0]["content"] = json!([{"type":"input_text","text":"text"},{"type":"input_image","detail":"low"}]);
        assert!(local_request(&body, &preferences).unwrap_err().contains("text-only"));
    }

    #[test]
    fn local_response_is_normalized_to_responses_shape_with_latency() {
        let preferences = crate::preferences::Preferences::default();
        let response = json!({"model":"minicpm5-2b-fp8","choices":[{"message":{"content":"{\"fields\":[]}","reasoning_content":null},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":11,"completion_tokens":2,"total_tokens":13}});
        let normalized = normalize_local(response, &preferences, 321).unwrap();
        assert_eq!(normalized["status"], "completed");
        assert_eq!(normalized["usage"]["input_tokens"], 11);
        assert_eq!(normalized["usage"]["output_tokens"], 2);
        assert_eq!(normalized["output"][0]["content"][0]["type"], "output_text");
        assert_eq!(normalized["output"][0]["content"][0]["text"], "{\"fields\":[]}");
        assert_eq!(normalized["latency_ms"], 321);
        let incomplete = json!({"choices":[{"message":{"content":"{\"fields\":[]}"},"finish_reason":"length"}]});
        assert_eq!(normalize_local(incomplete, &preferences, 1).unwrap()["status"], "incomplete");
    }
}
