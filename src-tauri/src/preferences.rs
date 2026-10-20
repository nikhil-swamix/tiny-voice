use serde::{Deserialize, Serialize};
use std::path::Path;
// Languages offered for dictation. `en-IN` is the Indian English default;
// regional English choices map to the "en" hint for cloud transcription, while
// `multi` enables English/Hindi code-switching and automatic cloud detection.
pub const LANGUAGES: [&str; 7] = ["en-IN", "en-US", "en-GB", "en-AU", "en-NZ", "hi", "multi"];
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub model: String, pub reasoning: String, pub send_images: bool, pub language: String,
    pub post_processing_profile: String, pub local_endpoint: String, pub local_model: String,
    pub bypass_gpt_transcribe: bool,
}
impl Default for Preferences {
    fn default() -> Self { Self {
        model: "gpt-6.1-sol".into(), reasoning: "low".into(), send_images: false, language: "en-IN".into(),
        post_processing_profile: "local-sglang".into(), local_endpoint: "http://127.0.0.1:30000/v1".into(),
        local_model: "minicpm5-2b-fp8".into(), bypass_gpt_transcribe: true,
    } }
}
impl Preferences {
    // Invalid or hand-edited files fall back to defaults rather than reaching
    // the processing request or the settings dropdowns.
    pub fn load(path: &Path) -> Self { std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Self>(&b).ok()).filter(|p| p.validate().is_ok()).unwrap_or_default() }
    // Sanitized accessor: returns an ISO language hint for cloud transcription.
    // An empty string means automatic language detection (code-switching).
    pub fn language_code(&self) -> &str {
        match self.language.as_str() {
            "en-IN" | "en-US" | "en-GB" | "en-AU" | "en-NZ" => "en",
            "hi" => "hi",
            "multi" => "",
            _ => "en",
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if !["local-sglang", "openai"].contains(&self.post_processing_profile.as_str()) { return Err("Unsupported post-processing profile".into()); }
        let endpoint = reqwest::Url::parse(&self.local_endpoint).map_err(|_| "Invalid local endpoint URL")?;
        if self.local_endpoint.len() > 2048 || self.local_endpoint.chars().any(char::is_control) || endpoint.scheme() != "http"
            || !matches!(endpoint.host_str(), Some("localhost" | "127.0.0.1" | "[::1]" | "::1"))
            || !endpoint.username().is_empty() || endpoint.password().is_some()
            || endpoint.query().is_some() || endpoint.fragment().is_some()
            || self.local_endpoint.split("://").nth(1).unwrap_or("").split('/').next().unwrap_or("").contains('@')
        { return Err("Local endpoint must use HTTP on localhost, 127.0.0.1, or [::1], without credentials, query, or fragment".into()); }
        if self.local_model.trim().is_empty() || self.local_model.len() > 256 || self.local_model.chars().any(char::is_control) { return Err("Local model must contain 1–256 bytes without control characters".into()); }
        if !["gpt-6.1-sol", "gpt-6-luna", "gpt-6-astra"].contains(&self.model.as_str()) { return Err("Unsupported processing model".into()); }
        if !["low", "medium", "high", "xhigh", "max"].contains(&self.reasoning.as_str()) { return Err("Unsupported reasoning effort".into()); }
        if !LANGUAGES.contains(&self.language.as_str()) { return Err("Unsupported transcription language".into()); }
        Ok(())
    }
}
