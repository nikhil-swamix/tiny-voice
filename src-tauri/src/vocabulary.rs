use std::path::Path;

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Correction {
    pub from: String,
    pub to: String,
    pub context: String,
}

pub fn load_corrections(dir: &Path) -> Result<Vec<Correction>, String> {
    let path = dir.join("vocabulary-corrections.json");
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    };
    let corrections: Vec<Correction> = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
    for correction in &corrections {
        if !valid_term(&correction.from) || !valid_term(&correction.to)
            || correction.context.trim().is_empty() || correction.context.chars().any(char::is_control)
        {
            return Err(format!("Invalid spelling correction in {}", path.display()));
        }
    }
    Ok(corrections)
}

fn valid_term(term: &str) -> bool {
    (2..=28).contains(&term.len())
        && term.chars().next().is_some_and(char::is_alphabetic)
        && term.chars().all(|c| c.is_alphanumeric() || ['-', '+', '.'].contains(&c))
}

// These are contextual spelling hints, never instructions to replace matching words globally.
pub fn correction_hints(corrections: &[Correction]) -> String {
    if corrections.is_empty() {
        return String::new();
    }
    let mut hints = String::from("Contextual spelling hints (not global replacements):");
    for correction in corrections {
        hints.push_str(&format!("\n- {} -> {} only when referring to {}.",
            correction.from, correction.to, correction.context));
    }
    hints.push_str("\nKeep unrelated uses unchanged; if the intended context is uncertain, preserve the original wording.");
    hints
}

// Rank raw observations, not generated notes or guessed phonetic replacements.
pub fn scan(history: &[Value], corrections: &[Correction]) -> Vec<String> {
    let stop: HashSet<_> = "the and for with this that from into have should would could please make use add remove keep fix need want voice task output intent hints completion effort quality check focus pitfall avoidance hint transcript recording instructions settings screen model request update implement improve relevant current previous user only words title point section format allow ensure preserve provide based clear default using include final start stop first last testing suggested approach checks work before after when then more also all not but can its has are was will your our let just each new any how what which their these those about change changes instruction requirement requirements okay hey there you like unclear one behavior clarify avoid overall two test now end-to-end three-word existing while required without validate small confirm high logic such give per verify intelligent meaningful necessary unnecessary effective effectively accordingly specific requested ensure distinguish available unavailable according important appropriately helpful focused sensible practical concise natural readable understand understanding specified desired accurate robust continue continues support supports include includes suitable following better best good great nice done well".split_whitespace().collect();
    let domain: HashSet<_> = "deepgram openai tauri tinyvoice github webview nova-3 gpt api svg acl ocr markdown microphone minibar popover clipboard websocket renderer streaming autocorrect pronunciation latency tokens encoding screenshot screenshots transcript transcripts transcription retranscription reasoning prompt processing frontend backend memory javascript typescript rust powershell luna sol swamix normalization".split_whitespace().collect();
    let mut counts = HashMap::<String, (String, usize, usize, usize)>::new();
    let excluded: HashSet<_> = corrections.iter().map(|correction| correction.from.to_lowercase()).collect();
    for (index, entry) in history.iter().enumerate() {
        let mut seen = HashSet::new();
        // Generated transcripts can repeat an ASR mistake or invent vocabulary.
        let source = entry["raw"].as_str().unwrap_or("");
        for word in source.split(|c: char| !c.is_alphanumeric() && !['-', '+', '.'].contains(&c)) {
            let term = word.trim_matches(['-', '+', '.']);
            let key = term.to_lowercase();
            if term.len() < 3 || term.len() > 28 || stop.contains(key.as_str())
                || excluded.contains(&key) || !term.chars().next().is_some_and(char::is_alphabetic)
            {
                continue;
            }
            let special = (!term.contains('-') && term.chars().skip(1).any(char::is_uppercase))
                || term.chars().any(char::is_numeric);
            let item = counts.entry(key.clone()).or_insert_with(|| (term.to_owned(), 0, 0, index));
            if special {
                if item.0 != term { item.0 = term.to_owned(); }
                item.2 = 8;
            }
            if domain.contains(key.as_str()) { item.2 = item.2.max(6); }
            if seen.insert(key) { item.1 += 1; item.3 = index; }
        }
    }
    let mut terms: Vec<_> = counts.into_values().filter(|(_, count, signal, _)| *count >= 2 && *signal > 0).collect();
    terms.sort_by(|a,b| (b.1 * (b.2 + 1)).cmp(&(a.1 * (a.2 + 1))).then(b.3.cmp(&a.3)).then(a.0.cmp(&b.0)));
    let mut bytes = 0;
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    for term in corrections.iter().map(|correction| correction.to.as_str())
        .chain(terms.iter().map(|(term, _, _, _)| term.as_str()))
    {
        if output.len() == 32 { break; }
        if !seen.insert(term.to_lowercase()) { continue; }
        let needed = term.len() + if output.is_empty() { 0 } else { 2 };
        if bytes + needed > 350 { continue; }
        bytes += needed;
        output.push(term.to_owned());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn pi_corrections() -> Vec<Correction> {
        vec![
            Correction { from: "pie".into(), to: "Pi".into(), context: "the Pi coding-agent harness or its .pi directories, not food or unrelated uses".into() },
            Correction { from: "pai".into(), to: "Pi".into(), context: "the Pi coding-agent harness or its .pi directories, not food or unrelated uses".into() },
        ]
    }

    #[test]
    fn explicit_canonical_spelling_has_priority_without_rewriting_history() {
        let history = vec![
            json!({"raw": "Use the Pie harness with OpenAI and Pai."}),
            json!({"raw": "The Pie harness uses OpenAI and Pai."}),
            json!({"raw": "I ate apple pie, not cake."}),
        ];
        let hints = scan(&history, &pi_corrections());
        assert_eq!(hints.first().map(String::as_str), Some("Pi"));
        assert_eq!(hints.iter().filter(|term| term.as_str() == "Pi").count(), 1);
        assert!(hints.iter().any(|term| term == "OpenAI"));
        assert!(!hints.iter().any(|term| ["pie", "pai"].contains(&term.to_lowercase().as_str())));
    }

    #[test]
    fn generated_notes_and_instruction_text_do_not_create_corrections() {
        let history = vec![
            json!({"raw": "Change the renderer to a microphone.", "transcript": "InventedName OpenAI"}),
            json!({"raw": "Change the renderer to a microphone.", "transcript": "InventedName OpenAI"}),
            json!({"raw": "Rename the pie menu to Pi."}),
        ];
        let hints = scan(&history, &[]);
        assert!(!hints.iter().any(|term| term == "InventedName" || term == "OpenAI"));
        assert!(!hints.iter().any(|term| term == "Pi"));
    }

    #[test]
    fn hints_obey_term_and_joined_utf8_byte_limits() {
        let raw = (0..50).map(|index| format!("VocabÉTerm{index:02}"))
            .collect::<Vec<_>>().join(" ");
        let history = vec![json!({"raw": raw}), json!({"raw": raw})];
        let hints = scan(&history, &pi_corrections());
        assert!(hints.len() <= 32);
        assert!(hints.join(", ").len() <= 350);
        assert_eq!(hints.first().map(String::as_str), Some("Pi"));
        let short_raw = (0..50).map(|index| format!("Term{index:02}"))
            .collect::<Vec<_>>().join(" ");
        let short_history = vec![json!({"raw": short_raw}), json!({"raw": short_raw})];
        assert_eq!(scan(&short_history, &[]).len(), 32);
    }
}
