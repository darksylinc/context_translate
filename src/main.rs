use clap::Parser;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{env, fs::File, io::Read, io::Write as OtherWrite};

use crate::error::Error;

mod error;
mod ods_reader;
mod open_ai;

#[derive(Debug, Default, Serialize, Deserialize, Clone)]
struct BlenderTextRow {
    datablock_name: String,
    #[serde(rename = "Collection")]
    speaker: String,
    #[serde(rename = "Text Contents")]
    text: String,
    #[serde(rename = "Original")]
    original: Option<String>,
    #[serde(rename = "Original Back")]
    original_back: Option<String>,
    #[serde(rename = "Remarks")]
    remarks: Option<String>,
    #[serde(rename = "Confidence", default)]
    confidence: Option<f64>,
    #[serde(rename = "Needs Revision", default)]
    needs_revision: Option<bool>,
}

// ─────────────────────────────────────────────────────────────────────────────
// LLM JSON protocol
//
// The user message sent to the LLM is a pure JSON object (TranslationRequest)
// and the LLM must reply with a pure JSON object (TranslationResponse).
// This replaces the old brittle {SPK} / {RMK} free-text tagging.
// ─────────────────────────────────────────────────────────────────────────────

/// A single dialogue line (speaker + text) used for context.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct LineItem {
    speaker: String,
    text: String,
}

/// A line to be translated. Carries an `index` which the LLM must echo back
/// in its response, so entries can be matched deterministically regardless of
/// the order the LLM returns them in.
///
/// On revision passes (multipass), `previous_translation` / `previous_remarks`
/// carry the earlier attempt so the LLM can critique and improve it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexedLineItem {
    index: usize,
    speaker: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_translation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_remarks: Option<String>,
}

/// LLM input: M lines of previous context, N lines to translate, O lines of
/// future context. M/N/O are user-adjustable via --pre-ctx/--batch-size/--pos-ctx.
#[derive(Debug, Serialize)]
struct TranslationRequest<'a> {
    destination_language: &'a str,
    previous_context: Vec<LineItem>,
    text_to_translate: Vec<IndexedLineItem>,
    future_context: Vec<LineItem>,
}

/// One translated entry as returned by the LLM.
#[derive(Debug, Clone, Deserialize)]
struct TranslatedItem {
    index: usize,
    text: String,
    /// LLM self-assessed confidence in the translation, 0.0..=1.0.
    confidence: f64,
    /// Multipass hook: true when the LLM wants a second look (insufficient
    /// context, genuine ambiguity, etc).
    needs_revision: bool,
    /// Free-form commentary, in English.
    #[serde(default)]
    remarks: String,
}

/// LLM output: only the translated entries from `text_to_translate`.
#[derive(Debug, Deserialize)]
struct TranslationResponse {
    translations: Vec<TranslatedItem>,
}

fn read_csv(path: &str) -> Result<Vec<BlenderTextRow>, csv::Error> {
    let file = File::open(path)?;
    let mut rdr = csv::ReaderBuilder::new().delimiter(b';').from_reader(file);

    let mut entries = Vec::new();

    for result in rdr.deserialize() {
        let rec: BlenderTextRow = result?;
        entries.push(rec);
    }

    Ok(entries)
}

fn write_csv(
    path: &str,
    entries: Vec<BlenderTextRow>,
    original_back: Vec<BlenderTextRow>,
) -> Result<(), csv::Error> {
    let file = File::create(path)?;
    let mut wr = csv::WriterBuilder::new().delimiter(b';').from_writer(file);
    for (entry, back) in entries.into_iter().zip(original_back) {
        let row = BlenderTextRow {
            datablock_name: entry.datablock_name,
            speaker: entry.speaker,
            text: entry.text,
            original: entry.original,
            original_back: Some(back.text),
            remarks: entry.remarks,
            confidence: entry.confidence,
            needs_revision: entry.needs_revision,
        };
        wr.serialize(row)?;
    }
    Ok(())
}

/// Builds the JSON `TranslationRequest` sent to the LLM: M lines of previous
/// context, N lines to translate, O lines of future context.
///
/// `previous` (multipass only) carries the previous pass's results aligned
/// with `to_translate`; when present, each entry is annotated with
/// `previous_translation` / `previous_remarks` so the LLM can revise it.
fn build_translation_request(
    pre_cxt: &[BlenderTextRow],
    to_translate: &[BlenderTextRow],
    pos_cxt: &[BlenderTextRow],
    dst_language: &str,
    previous: Option<&[BlenderTextRow]>,
) -> Result<String, serde_json::Error> {
    let request = TranslationRequest {
        destination_language: dst_language,
        previous_context: pre_cxt
            .iter()
            .map(|l| LineItem {
                speaker: l.speaker.clone(),
                text: l.text.clone(),
            })
            .collect(),
        text_to_translate: to_translate
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let prev = previous.and_then(|p| p.get(i));
                IndexedLineItem {
                    index: i,
                    speaker: l.speaker.clone(),
                    text: l.text.clone(),
                    previous_translation: prev
                        .map(|r| r.text.clone())
                        .filter(|t| !t.is_empty()),
                    previous_remarks: prev
                        .map(|r| r.remarks.clone().unwrap_or_default())
                        .filter(|t| !t.is_empty()),
                }
            })
            .collect(),
        future_context: pos_cxt
            .iter()
            .map(|l| LineItem {
                speaker: l.speaker.clone(),
                text: l.text.clone(),
            })
            .collect(),
    };

    serde_json::to_string(&request)
}

fn process_ai_response(
    response: &String,
    entries: &[BlenderTextRow],
    orig_prompt: &String,
    error_log: &mut File,
) -> Result<Vec<BlenderTextRow>, Error> {
    let r = process_ai_response_impl(response, entries);
    match &r {
        Ok(_) => {}
        Err(_) => {
            writeln!(error_log, "# ERROR LOG Invalid response:").ok();
            writeln!(error_log, "==============================").ok();
            writeln!(error_log, "{}", response).ok();
            writeln!(error_log, "==============================").ok();
            writeln!(error_log, "# ERROR LOG Original Prompt:").ok();
            writeln!(error_log, "==============================").ok();
            writeln!(error_log, "{}", orig_prompt).ok();
            writeln!(error_log, "==============================").ok();
        }
    }
    r
}

fn process_ai_response_impl(
    response: &str,
    entries: &[BlenderTextRow],
) -> Result<Vec<BlenderTextRow>, Error> {
    if response.trim().is_empty() {
        return Err(Error::InvalidResponse("empty response".to_string()));
    }

    // LLMs sometimes wrap the JSON in markdown code fences; strip them.
    let mut text = response.trim();
    if let Some(stripped) = text.strip_prefix("```") {
        let after_fence = stripped
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or("");
        if let Some(end) = after_fence.rfind("```") {
            text = after_fence[..end].trim();
        }
    }

    // Tolerate preamble/postamble prose: slice from the first '{' to the last '}'.
    let start = text
        .find('{')
        .ok_or_else(|| Error::InvalidResponse("no JSON object found in response".to_string()))?;
    let end = text
        .rfind('}')
        .ok_or_else(|| Error::InvalidResponse("no JSON object found in response".to_string()))?;
    if end <= start {
        return Err(Error::InvalidResponse(
            "malformed JSON object in response".to_string(),
        ));
    }
    let json = &text[start..=end];

    let parsed: TranslationResponse = serde_json::from_str(json)
        .map_err(|e| Error::InvalidResponse(format!("JSON parse error: {e}")))?;

    if parsed.translations.len() != entries.len() {
        return Err(Error::InvalidResponse(format!(
            "expected {} translations, got {}",
            entries.len(),
            parsed.translations.len()
        )));
    }

    // Validate that the echoed indices form an exact permutation of 0..entries.len().
    // This makes parsing order-independent and deterministically detects
    // missing, duplicated or out-of-range entries.
    let mut seen = vec![false; entries.len()];
    for item in &parsed.translations {
        if item.index >= entries.len() {
            return Err(Error::InvalidResponse(format!(
                "index {} out of range ({} entries)",
                item.index,
                entries.len()
            )));
        }
        if seen[item.index] {
            return Err(Error::InvalidResponse(format!(
                "duplicate index {}",
                item.index
            )));
        }
        seen[item.index] = true;
    }
    if !seen.iter().all(|s| *s) {
        let missing: Vec<usize> = seen
            .iter()
            .enumerate()
            .filter(|(_, s)| !**s)
            .map(|(i, _)| i)
            .collect();
        return Err(Error::InvalidResponse(format!(
            "missing indices: {missing:?}"
        )));
    }

    // Map back to input order.
    let mut by_index = vec![None; entries.len()];
    for item in parsed.translations {
        let idx = item.index;
        by_index[idx] = Some(item);
    }

    let mut translated = Vec::with_capacity(entries.len());
    for (entry, item) in entries.iter().zip(by_index.into_iter()) {
        let Some(item) = item else {
            unreachable!("index permutation validated above");
        };
        translated.push(BlenderTextRow {
            datablock_name: entry.datablock_name.clone(),
            speaker: entry.speaker.clone(),
            text: item.text,
            original: Some(entry.text.clone()),
            original_back: None,
            remarks: Some(item.remarks),
            confidence: Some(item.confidence),
            needs_revision: Some(item.needs_revision),
        });
    }

    Ok(translated)
}

/// Translate a single batch (N lines) surrounded by context (M pre / O post lines).
///
/// This is the pure unit of work: it builds the JSON request, calls the LLM,
/// and parses/validates the JSON response. Multipass revision passes reuse
/// this directly by rebuilding requests with wider context and the previous
/// pass's results (`previous`) for entries flagged `needs_revision`.
async fn translate_batch(
    pre_cxt: &[BlenderTextRow],
    batch: &[BlenderTextRow],
    pos_cxt: &[BlenderTextRow],
    ai_settings: &open_ai::AiSettings<'_>,
    dst_language: &str,
    error_log: &mut File,
    previous: Option<&[BlenderTextRow]>,
) -> Vec<BlenderTextRow> {
    let request = build_translation_request(pre_cxt, batch, pos_cxt, dst_language, previous)
        .expect("serializing our own TranslationRequest cannot fail");

    let num_retries = 3;

    let mut response = open_ai::run_prompt(ai_settings, &request)
        .await
        .expect("AI request failed");

    for j in 0..num_retries {
        match process_ai_response(&response, batch, &request, error_log) {
            Ok(translated) => return translated,
            Err(e) => {
                if j + 1 == num_retries {
                    eprintln!(
                        "Invalid Translation Output. Attempt {}. Giving up: {e}",
                        j + 1
                    );
                    return batch
                        .iter()
                        .map(|entry| BlenderTextRow {
                            datablock_name: entry.datablock_name.clone(),
                            speaker: entry.speaker.clone(),
                            text: String::new(),
                            original: Some(entry.text.clone()),
                            original_back: None,
                            remarks: Some("AI ERROR. GIVEN UP.".to_string()),
                            confidence: None,
                            needs_revision: Some(true),
                        })
                        .collect();
                }
                eprintln!(
                    "Invalid Translation Output. Attempt {}. Retrying...: {e}",
                    j + 1
                );
                response = open_ai::run_prompt(ai_settings, &request)
                    .await
                    .expect("AI request failed");
            }
        }
    }

    unreachable!("retry loop always returns")
}

/// Translate all lines, splitting them into batches of `entries_per_query`
/// with `pre_context_lines` / `pos_context_lines` of surrounding context.
async fn translate_blender_lines(
    entries: &Vec<BlenderTextRow>,
    entries_per_query: usize,
    pre_context_lines: usize,
    pos_context_lines: usize,
    ai_settings: &open_ai::AiSettings<'_>,
    dst_language: &str,
    error_log: &mut File,
) -> Result<Vec<BlenderTextRow>, Box<dyn std::error::Error>> {
    let mut output = Vec::new();
    output.reserve_exact(entries.len());
    let num_batches = (entries.len() + entries_per_query - 1) / entries_per_query;
    for i in (0..entries.len()).step_by(entries_per_query) {
        println!("Batch ID {} / {}", i / entries_per_query, num_batches);
        let from = i;
        let to = std::cmp::min(i + entries_per_query, entries.len());

        let pre_from = from.saturating_sub(pre_context_lines);
        let pre_cxt = &entries[pre_from..from];

        let pos_to = std::cmp::min(to + pos_context_lines, entries.len());
        let pos_cxt = &entries[to..pos_to];

        let translated = translate_batch(
            pre_cxt,
            &entries[from..to],
            pos_cxt,
            ai_settings,
            dst_language,
            error_log,
            None,
        )
        .await;

        output.extend(translated);
    }

    // Multipass: rows flagged needs_revision (or below --confidence-threshold)
    // are re-translated by revision_pass() in run(), with progressively wider
    // context and the previous attempt attached to the request.

    Ok(output)
}

/// True when a row should be re-translated on the next revision pass:
/// the LLM flagged it `needs_revision`, or its confidence is below the
/// user-supplied threshold.
fn is_flagged(row: &BlenderTextRow, confidence_threshold: Option<f64>) -> bool {
    row.needs_revision == Some(true)
        || matches!(
            (row.confidence, confidence_threshold),
            (Some(c), Some(t)) if c < t
        )
}

/// One revision pass: re-translate every row in `translated` that
/// `is_flagged`, using context from the ORIGINAL source rows (the true
/// dialogue neighbors, not possibly-wrong pass-1 translations) and attaching
/// the previous attempt via `previous_translation` / `previous_remarks`.
///
/// Each flagged row is sent as its own single-entry batch (N=1) so the full
/// widened context window goes to context. Results are merged back in place;
/// identity fields (datablock_name, speaker, original) are preserved.
///
/// Returns the number of rows re-translated (0 when nothing was flagged —
/// the caller stops the multipass loop then).
async fn revision_pass(
    translated: &mut Vec<BlenderTextRow>,
    source: &[BlenderTextRow],
    base_pre_ctx: usize,
    base_pos_ctx: usize,
    ctx_step: usize,
    pass_number: usize,
    confidence_threshold: Option<f64>,
    ai_settings: &open_ai::AiSettings<'_>,
    dst_language: &str,
    error_log: &mut File,
) -> usize {
    let flagged: Vec<usize> = translated
        .iter()
        .enumerate()
        .filter(|(_, r)| is_flagged(r, confidence_threshold))
        .map(|(i, _)| i)
        .collect();

    if flagged.is_empty() {
        return 0;
    }
    let count = flagged.len();

    let pre_ctx = base_pre_ctx + (pass_number - 1) * ctx_step;
    let pos_ctx = base_pos_ctx + (pass_number - 1) * ctx_step;

    println!(
        "Pass {}: re-translating {} flagged row(s) with {} pre / {} post context",
        pass_number,
        flagged.len(),
        pre_ctx,
        pos_ctx
    );

    for i in flagged {
        let pre_cxt = &source[i.saturating_sub(pre_ctx)..i];
        let pos_cxt = &source[i + 1..std::cmp::min(i + 1 + pos_ctx, source.len())];
        let batch = &source[i..i + 1];
        let previous = &translated[i..i + 1];

        let result = translate_batch(
            pre_cxt,
            batch,
            pos_cxt,
            ai_settings,
            dst_language,
            error_log,
            Some(previous),
        )
        .await;

        // translate_batch returns exactly one row per input row.
        let row = result
            .into_iter()
            .next()
            .expect("single-row batch yields exactly one result row");

        // Merge: keep identity, adopt the new translation + metadata.
        translated[i].text = row.text;
        translated[i].remarks = row.remarks;
        translated[i].confidence = row.confidence;
        translated[i].needs_revision = row.needs_revision;
    }

    count
}

/// Send CSV file to AI for translating.
#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Source Language. Can be left blank to auto-detect BUT "translation back" won't be available.
    /// "translation back" is very helpful for diagnosing if the translated text retained its original meaning.
    /// Highly recommended.
    #[arg(short, long)]
    pub src_lang: Option<String>,
    /// Destination Language to translate to.
    #[arg(short, long)]
    pub dst_lang: String,
    /// OpenAI API key. You can also set the OPENAI_API_KEY environment variable. Cmd line is higher priority.
    #[arg(short, long)]
    pub api_key: Option<String>,
    /// LLM Model to use. e.g. "Qwen3.8-27B-UD-Q4_K_XL.gguf" / "Qwen3.5-35B-A3B-UD-Q4_K_L.gguf"
    #[arg(short, long)]
    pub model: String,

    /// Path to the system prompt location.
    #[arg(long)]
    pub system_prompt: String,

    /// URI to API endpoint, for example https://api.openai.com/v1/chat/completions or
    /// http://127.0.0.1:8081/v1/chat/completions
    #[arg(short, long)]
    pub endpoint: String,

    /// CSV file to translate.
    #[arg(long)]
    pub src_csv: String,
    /// Output CSV file.
    #[arg(long)]
    pub dst_csv: String,

    /// How many lines to translate per AI prompt. Higher values translate faster,
    /// but has a higher chance of being inaccurate or hallucinating.
    /// Extremely high values may cause performance issues due to LLM context window handling.
    #[arg(short, long, default_value_t = 6, value_parser = clap::value_parser!(u16).range(1..))]
    pub batch_size: u16,

    /// How many preceeding lines to send alongside the batch as context.
    /// Very low values may result in less accurate translations.
    /// If increasing this too much, consider raising batch-size instead.
    #[arg(long, default_value_t = 3)]
    pub pre_ctx: u16,

    /// How many subsequent lines to send alongside the batch as context.
    /// Very low values may result in less accurate translations.
    /// If increasing this too much, consider raising batch-size instead.
    #[arg(long, default_value_t = 3)]
    pub pos_ctx: u16,

    /// Total number of translation passes. Pass 1 translates everything; each
    /// further pass re-translates only rows flagged `needs_revision` (or with
    /// confidence below --confidence-threshold), with progressively wider
    /// context. 1 (the default) behaves exactly like single-pass.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..))]
    pub max_passes: u8,

    /// Extra context lines added on each side per revision pass. Pass k uses
    /// --pre-ctx + (k-1)*step pre-context and --pos-ctx + (k-1)*step
    /// post-context.
    #[arg(long, default_value_t = 3)]
    pub revision_ctx_step: u16,

    /// Also re-translate rows whose confidence is below this threshold
    /// (0.0-1.0), in addition to rows flagged `needs_revision`.
    #[arg(long)]
    pub confidence_threshold: Option<f64>,

    /// Optional separate system prompt for revision passes. Defaults to
    /// --system-prompt when omitted.
    #[arg(long)]
    pub revision_system_prompt: Option<String>,

    /// Path to JSON file to customize more options (like temperature, top_p, etc).
    #[arg(long, short)]
    pub llm_options: Option<String>,

    /// Timeout in seconds for each batch before considering it an AI error.
    #[arg(long)]
    pub timeout_secs: u64,

    /// When present, it puts tool into "key / value" mode and open an ODS spreadsheet.
    /// Submit a comma-separated 0-based integers for which columns contain.
    /// The first column is the source language, the other columns add additional context.
    #[arg(long, default_value = "")]
    pub ods_key_mode_columns: String,

    /// Show prompt in stdio.
    #[arg(long)]
    pub debug: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let mut error_log = File::create("errors.log")?;

    println!("Opening System Prompt {}", args.system_prompt);
    let mut system_prompt = String::new();
    File::open(&args.system_prompt)?.read_to_string(&mut system_prompt)?;

    // Read API key from environment variable
    let api_key = match args.api_key {
        Some(ref s) => s.to_string(),
        None => env::var("OPENAI_API_KEY").expect(
            "Please set the OPENAI_API_KEY environment variable or via command line argument. try '--help'",
        ),
    };

    let extra_options: Option<Value> = match args.llm_options {
        Some(ref llm_options_path) => {
            let mut file = File::open(llm_options_path)?;
            let mut json_str = String::new();
            file.read_to_string(&mut json_str)?;
            Some(serde_json::from_str(&json_str).unwrap())
        }
        None => None,
    };

    // The CSV path speaks pure JSON in and out, so ask the provider to
    // constrain the output to valid JSON (grammar-constrained decoding).
    // The ODS path uses a free-text protocol and must stay unconstrained.
    let json_response_format = if args.ods_key_mode_columns.is_empty() {
        Some(serde_json::json!({ "type": "json_object" }))
    } else {
        None
    };

    let ai_settings = open_ai::AiSettings {
        endpoint: args.endpoint.clone(),
        api_key: api_key,
        system_prompt: system_prompt,
        model: args.model.clone(),
        timeout_secs: args.timeout_secs,
        extra_options: match &extra_options {
            Some(extra_options) => Some(extra_options.as_object().unwrap()),
            None => None,
        },
        response_format: json_response_format.as_ref(),
        debug: args.debug,
    };

    if !args.ods_key_mode_columns.is_empty() {
        ods_reader::translate_key_mode_ods(&args, &mut error_log, &ai_settings).await?;
    } else {
        println!("Opening file {}", args.src_csv);
        let lines = read_csv(&args.src_csv)?;

        // Translate to target lang.
        println!("Begin Translation");
        let mut translated = translate_blender_lines(
            &lines,
            args.batch_size as usize,
            args.pre_ctx as usize,
            args.pos_ctx as usize,
            &ai_settings,
            &args.dst_lang,
            &mut error_log,
        )
        .await?;

        // Multipass: re-translate flagged rows (needs_revision or low
        // confidence) with progressively wider context, attaching each row's
        // previous attempt so the LLM can revise it. Stops early when a pass
        // finds nothing left to flag.
        if args.max_passes > 1 {
            let revision_ai_settings = match &args.revision_system_prompt {
                Some(path) => {
                    println!("Opening Revision System Prompt {}", path);
                    let mut revision_prompt = String::new();
                    File::open(path)?.read_to_string(&mut revision_prompt)?;
                    Some(open_ai::AiSettings {
                        system_prompt: revision_prompt,
                        ..ai_settings.clone()
                    })
                }
                None => None,
            };

            let max = args.max_passes as usize;
            for pass in 2..=max {
                let settings = revision_ai_settings.as_ref().unwrap_or(&ai_settings);
                let retranslated = revision_pass(
                    &mut translated,
                    &lines,
                    args.pre_ctx as usize,
                    args.pos_ctx as usize,
                    args.revision_ctx_step as usize,
                    pass,
                    args.confidence_threshold,
                    settings,
                    &args.dst_lang,
                    &mut error_log,
                )
                .await;
                if retranslated == 0 {
                    break;
                }
            }
        }

        // Now translate it back to the original lang for validation (if src_lang was provided).
        let original_back = match args.src_lang {
            Some(src_lang) => {
                {
                    println!("Writing intermediate results to {}", args.dst_csv);
                    let mut blank = Vec::new();
                    blank.resize_with(translated.len(), || BlenderTextRow::default());
                    write_csv(&args.dst_csv, translated.clone(), blank)?;
                }
                println!("Begin Back Translation");
                match translate_blender_lines(
                    &translated,
                    args.batch_size as usize,
                    args.pre_ctx as usize,
                    args.pos_ctx as usize,
                    &ai_settings,
                    &src_lang,
                    &mut error_log,
                )
                .await
                {
                    Ok(r) => r,
                    Err(_) => {
                        eprintln!("Back Translation Error. It won't be available.");
                        let mut blank = Vec::new();
                        blank.resize_with(translated.len(), || BlenderTextRow::default());
                        blank
                    }
                }
            }
            None => {
                let mut blank = Vec::new();
                blank.resize_with(translated.len(), || BlenderTextRow::default());
                blank
            }
        };

        println!("Writing results to {}", args.dst_csv);
        write_csv(&args.dst_csv, translated, original_back)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entries(n: usize) -> Vec<BlenderTextRow> {
        (0..n)
            .map(|i| BlenderTextRow {
                datablock_name: format!("key_{i}"),
                speaker: format!("Speaker {i}"),
                text: format!("original text {i}"),
                original: None,
                original_back: None,
                remarks: None,
                confidence: None,
                needs_revision: None,
            })
            .collect()
    }

    fn item(index: usize, text: &str) -> String {
        format!(
            "{{\"index\": {}, \"text\": \"{}\", \"confidence\": 0.9, \"needs_revision\": false, \"remarks\": \"\"}}",
            index, text
        )
    }

    #[test]
    fn parse_clean_json() {
        let entries = make_entries(2);
        let response = format!(
            "{{\"translations\": [{}, {}]}}",
            item(0, "World of Pastries"),
            item(1, "We searched every bakery in Argentina")
        );
        let result = process_ai_response_impl(&response, &entries).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].text, "World of Pastries");
        assert_eq!(result[0].datablock_name, "key_0");
        assert_eq!(result[0].original.as_deref(), Some("original text 0"));
        assert_eq!(result[0].confidence, Some(0.9));
        assert_eq!(result[0].needs_revision, Some(false));
        assert_eq!(result[1].text, "We searched every bakery in Argentina");
    }

    #[test]
    fn parse_json_in_markdown_fence() {
        let entries = make_entries(1);
        let response = format!(
            "```json\n{{\"translations\": [{}]}}\n```",
            item(0, "translated")
        );
        let result = process_ai_response_impl(&response, &entries).unwrap();
        assert_eq!(result[0].text, "translated");
    }

    #[test]
    fn parse_json_with_preamble() {
        let entries = make_entries(1);
        let response = format!(
            "Here is the translation you requested:\n{{\"translations\": [{}]}}\nHope that helps!",
            item(0, "translated")
        );
        let result = process_ai_response_impl(&response, &entries).unwrap();
        assert_eq!(result[0].text, "translated");
    }

    #[test]
    fn parse_reordered_indices() {
        let entries = make_entries(2);
        // LLM returned index 1 before index 0 — must still map back to input order.
        let response = format!(
            "{{\"translations\": [{}, {}]}}",
            item(1, "second"),
            item(0, "first")
        );
        let result = process_ai_response_impl(&response, &entries).unwrap();
        assert_eq!(result[0].text, "first");
        assert_eq!(result[0].datablock_name, "key_0");
        assert_eq!(result[1].text, "second");
        assert_eq!(result[1].datablock_name, "key_1");
    }

    #[test]
    fn parse_missing_index_fails() {
        let entries = make_entries(2);
        let response = format!("{{\"translations\": [{}]}}", item(0, "only one"));
        assert!(process_ai_response_impl(&response, &entries).is_err());
    }

    #[test]
    fn parse_duplicate_index_fails() {
        let entries = make_entries(2);
        let response = format!("{{\"translations\": [{}, {}]}}", item(0, "a"), item(0, "b"));
        assert!(process_ai_response_impl(&response, &entries).is_err());
    }

    #[test]
    fn parse_out_of_range_index_fails() {
        let entries = make_entries(2);
        let response = format!("{{\"translations\": [{}, {}]}}", item(0, "a"), item(5, "b"));
        assert!(process_ai_response_impl(&response, &entries).is_err());
    }

    #[test]
    fn parse_wrong_count_fails() {
        let entries = make_entries(2);
        let response = format!("{{\"translations\": [{}]}}", item(0, "a"));
        assert!(process_ai_response_impl(&response, &entries).is_err());
    }

    #[test]
    fn parse_not_json_fails() {
        let entries = make_entries(1);
        assert!(process_ai_response_impl("I refuse to answer.", &entries).is_err());
        assert!(process_ai_response_impl("", &entries).is_err());
        assert!(process_ai_response_impl("   \n  ", &entries).is_err());
    }

    #[test]
    fn build_request_json_shape() {
        let pre = make_entries(1);
        let batch = make_entries(2);
        let pos = make_entries(1);
        let json = build_translation_request(&pre, &batch, &pos, "English", None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["destination_language"], "English");
        assert_eq!(value["previous_context"].as_array().unwrap().len(), 1);
        assert_eq!(value["text_to_translate"].as_array().unwrap().len(), 2);
        assert_eq!(value["future_context"].as_array().unwrap().len(), 1);
        let to_translate = value["text_to_translate"].as_array().unwrap();
        assert_eq!(to_translate[0]["index"], 0);
        assert_eq!(to_translate[1]["index"], 1);
        assert_eq!(to_translate[0]["speaker"], "Speaker 0");
        assert_eq!(to_translate[0]["text"], "original text 0");
        // Pass 1 (no previous results) must not carry revision fields.
        assert!(to_translate[0].get("previous_translation").is_none());
        assert!(to_translate[0].get("previous_remarks").is_none());
    }

    #[test]
    fn build_request_with_previous_translation() {
        let batch = make_entries(1);
        // Simulate a pass-1 result: translated text + remarks, aligned with batch.
        let previous = vec![BlenderTextRow {
            text: "World of Invoices".to_string(),
            remarks: Some("ambiguous without context".to_string()),
            ..BlenderTextRow::default()
        }];
        let json = build_translation_request(&[], &batch, &[], "English", Some(&previous)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entry = &value["text_to_translate"].as_array().unwrap()[0];
        assert_eq!(entry["previous_translation"], "World of Invoices");
        assert_eq!(entry["previous_remarks"], "ambiguous without context");
    }

    #[test]
    fn build_request_previous_skips_empty_fields() {
        let batch = make_entries(1);
        // Fallback-style previous result: empty text and remarks must be omitted.
        let previous = vec![BlenderTextRow {
            text: String::new(),
            remarks: None,
            ..BlenderTextRow::default()
        }];
        let json = build_translation_request(&[], &batch, &[], "English", Some(&previous)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entry = &value["text_to_translate"].as_array().unwrap()[0];
        assert!(entry.get("previous_translation").is_none());
        assert!(entry.get("previous_remarks").is_none());
    }

    #[test]
    fn is_flagged_needs_revision() {
        let mut row = BlenderTextRow::default();
        assert!(!is_flagged(&row, None));
        row.needs_revision = Some(true);
        assert!(is_flagged(&row, None));
        row.needs_revision = Some(false);
        assert!(!is_flagged(&row, None));
    }

    #[test]
    fn is_flagged_confidence_threshold() {
        let mut row = BlenderTextRow::default();
        row.confidence = Some(0.5);
        assert!(!is_flagged(&row, None));
        assert!(!is_flagged(&row, Some(0.5)));
        assert!(is_flagged(&row, Some(0.6)));
        assert!(!is_flagged(&row, Some(0.4)));
        // No confidence (e.g. fallback row without metadata) is not flagged
        // by the threshold alone — needs_revision covers those.
        let blank = BlenderTextRow::default();
        assert!(!is_flagged(&blank, Some(0.9)));
    }

    #[test]
    fn build_request_empty_contexts() {
        let batch = make_entries(1);
        let json = build_translation_request(&[], &batch, &[], "Spanish", None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["previous_context"], serde_json::json!([]));
        assert_eq!(value["future_context"], serde_json::json!([]));
    }
}
