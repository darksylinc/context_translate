use clap::{Parser, command};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{env, fmt::Write, fs::File, io::Read, io::Write as OtherWrite};

use crate::error::Error;

mod error;
mod ods_reader;
mod open_ai;

#[derive(Debug, Default, Serialize, Deserialize)]
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
        };
        wr.serialize(row)?;
    }
    Ok(())
}

fn generate_blender_prompt(
    pre_cxt: &[BlenderTextRow],
    to_translate: &[BlenderTextRow],
    pos_cxt: &[BlenderTextRow],
    dst_language: &str,
) -> String {
    let mut prompt = String::new();

    writeln!(prompt, "Translate to {}", dst_language).unwrap();
    writeln!(prompt, "# CONTEXT PREVIOUS BEGIN").unwrap();
    for line in pre_cxt {
        writeln!(prompt, "## {}", line.speaker).unwrap();
        writeln!(prompt, "{}", line.text).unwrap();
    }
    writeln!(prompt, "# CONTEXT PREVIOUS END").unwrap();

    writeln!(prompt, "# TEXT BEGIN").unwrap();
    for line in to_translate {
        writeln!(prompt, "{{SPK}}{}{{SPK}}", line.speaker).unwrap();
        writeln!(prompt, "{}", line.text).unwrap();
    }
    writeln!(prompt, "# TEXT END").unwrap();

    writeln!(prompt, "# CONTEXT AFTER BEGIN").unwrap();
    for line in pos_cxt {
        writeln!(prompt, "## {}", line.speaker).unwrap();
        writeln!(prompt, "{}", line.text).unwrap();
    }
    writeln!(prompt, "# CONTEXT AFTER END").unwrap();

    prompt
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
    response: &String,
    entries: &[BlenderTextRow],
) -> Result<Vec<BlenderTextRow>, Error> {
    if response.is_empty() {
        return Err(error::Error::InvalidTranslation);
    }

    // We can't use response.len() - 1 for out-of-bounds check because that may not be a char boundary.
    // Find the last character.
    let last_char_start = response.char_indices().last().unwrap_or((0, 'A')).0;

    let mut translated = Vec::with_capacity(entries.len());

    let mut start_idx = 0;
    for entry in entries {
        if start_idx >= response.len() {
            // In previous iteration we reached the end but we were expecting more entries.
            return Err(error::Error::InvalidTranslation);
        }

        let speaker_pattern = format!("{{SPK}}{}{{SPK}}", entry.speaker);
        let haystack = response[start_idx..]
            .find(&speaker_pattern)
            .ok_or(error::Error::InvalidTranslation)?;
        start_idx = std::cmp::min(
            start_idx + haystack + speaker_pattern.len() + 1,
            last_char_start,
        );
        let end_idx = match response[start_idx..].find("{SPK}") {
            Some(idx) => start_idx + idx,
            None => match response[start_idx..].find("```") {
                Some(idx) => start_idx + idx,
                None => response.len(),
            },
        };

        let parts = response[start_idx..end_idx]
            .split_once("{RMK}")
            .unwrap_or((&response[start_idx..end_idx], ""));
        let text = parts.0.trim_start().trim_end();
        let remarks = parts.1.trim_start().trim_end();

        translated.push(BlenderTextRow {
            datablock_name: entry.datablock_name.clone(),
            speaker: entry.speaker.clone(),
            text: text.to_string(),
            original: Some(entry.text.clone()),
            original_back: None,
            remarks: Some(remarks.to_string()),
        });

        start_idx = end_idx
    }

    Ok(translated)
}

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

        let entries_to_translate = &entries[from..to];

        let pre_from = from.saturating_sub(pre_context_lines);
        let pre_cxt = &entries[pre_from..from];

        let pos_to = std::cmp::min(to + pos_context_lines, entries.len());
        let pos_cxt = &entries[to..pos_to];

        let prompt = generate_blender_prompt(pre_cxt, entries_to_translate, pos_cxt, dst_language);

        let mut response = open_ai::run_prompt(ai_settings, &prompt).await?;

        let num_retries = 9;

        let mut translated = {
            let mut translated_result = Vec::new();
            for j in 0..num_retries {
                let translated =
                    process_ai_response(&response, entries_to_translate, &prompt, error_log);
                match translated {
                    Ok(t) => translated_result = t,
                    Err(_) => {
                        if j + 1 == num_retries {
                            eprintln!("Invalid Translation Output. Attempt {}. Giving up.", j);
                            for entry in entries_to_translate {
                                translated_result.push(BlenderTextRow {
                                    datablock_name: entry.datablock_name.clone(),
                                    speaker: entry.speaker.clone(),
                                    text: "".to_string(),
                                    original: Some(entry.text.clone()),
                                    original_back: None,
                                    remarks: Some("AI ERROR. GIVEN UP.".to_string()),
                                });
                            }
                        } else {
                            eprintln!("Invalid Translation Output. Attempt {}. Retrying...", j);
                            response = open_ai::run_prompt(ai_settings, &prompt).await?;
                        }
                    }
                }
            }
            translated_result
        };

        output.append(&mut translated);
    }

    Ok(output)
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
    /// LLM Model to use. e.g. "mistralai_Mistral-Small-3.1-24B-Instruct-2503-Q4_K_M.gguf"
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
        debug: args.debug,
    };

    if !args.ods_key_mode_columns.is_empty() {
        ods_reader::translate_key_mode_ods(&args, &mut error_log, &ai_settings).await?;
    } else {
        println!("Opening file {}", args.src_csv);
        let lines = read_csv(&args.src_csv)?;

        // Translate to target lang.
        println!("Begin Translation");
        let translated = translate_blender_lines(
            &lines,
            args.batch_size as usize,
            args.pre_ctx as usize,
            args.pos_ctx as usize,
            &ai_settings,
            &args.dst_lang,
            &mut error_log,
        )
        .await?;

        // Now translate it back to the original lang for validation (if src_lang was provided).
        let original_back = match args.src_lang {
            Some(src_lang) => {
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
