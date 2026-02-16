use icu_locale_core::locale;
use spreadsheet_ods::{CompressionMethod, OdsWriteOptions, Sheet};
use std::io::BufWriter;
use std::{fs::File, io::Write as iowrite};

use crate::error::Error;
use crate::{Args, open_ai};
struct Entry {
    key_name: String,
    text: String,
}

struct LangSet {
    lang: String,
    entries: Vec<Entry>,
}

fn load_ods(path: &str, columns_to_use: &Vec<u32>) -> Vec<LangSet> {
    let book = spreadsheet_ods::read_ods(path).expect(&format!("Error opening ODS {}", path));

    let all = book.sheet(book.sheet_idx("all").unwrap());
    let (num_rows, __) = all.used_grid_size();

    let mut retval = Vec::with_capacity(columns_to_use.len());

    for col in columns_to_use {
        let col = *col;

        let lang = all.value(0, col).as_str_or_default();

        let mut lang_set = LangSet {
            lang: lang.to_string(),
            entries: Vec::with_capacity((num_rows - 1) as usize),
        };

        for row in 1..num_rows {
            let key = all.value(row, 0).as_str_or_default();

            let value = all.value(row, col).as_cow_str_or("");
            if !value.is_empty() && value != "#N/A" {
                lang_set.entries.push(Entry {
                    key_name: key.to_string(),
                    text: value.to_string(),
                });
            } else {
                lang_set.entries.push(Entry {
                    key_name: key.to_string(),
                    text: String::new(),
                });
            }
        }

        retval.push(lang_set);
    }

    retval
}

fn process_ai_response(
    response: &String,
    entries: &[Entry],
    orig_prompt: &String,
    error_log: &mut File,
) -> Result<Vec<Entry>, Error> {
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

fn process_ai_response_impl(response: &String, entries: &[Entry]) -> Result<Vec<Entry>, Error> {
    if response.is_empty() {
        return Err(Error::InvalidTranslation);
    }

    let mut translated = Vec::with_capacity(entries.len());

    // We can't use response.len() - 1 for out-of-bounds check because that may not be a char boundary.
    // Find the last character.
    let last_char_start = response.char_indices().last().unwrap_or((0, 'A')).0;

    let mut start_idx = 0;
    for (i, entry) in entries.iter().enumerate() {
        if start_idx >= response.len() {
            // In previous iteration we reached the end but we were expecting more entries.
            return Err(Error::InvalidTranslation);
        }

        let pattern = format!("# {}\n", entry.key_name);
        let haystack = response[start_idx..]
            .find(&pattern)
            .ok_or(Error::InvalidTranslation)?;
        start_idx = std::cmp::min(start_idx + haystack + pattern.len(), last_char_start);
        let end_idx = if i + 1 == entries.len() {
            response.len()
        } else {
            match response[start_idx..].find(&format!("# {}\n", entries[i + 1].key_name)) {
                Some(idx) => start_idx + idx,
                None => return Err(Error::InvalidTranslation),
            }
        };

        let text = response[start_idx..end_idx].trim_start().trim_end();

        translated.push(Entry {
            key_name: entry.key_name.clone(),
            text: text.to_string(),
        });

        start_idx = end_idx
    }

    Ok(translated)
}

async fn translate_lang_set(
    args: &Args,
    dst_lang: &str,
    error_log: &mut File,
    ai_settings: &open_ai::AiSettings<'_>,
    lang_sets: &[LangSet],
    partial_output: bool,
) -> Result<LangSet, Box<dyn std::error::Error>> {
    let (main_lang, context) = lang_sets.split_at(1);

    let src_lang: &LangSet = &main_lang[0];
    let mut dst_lang_set = LangSet {
        lang: dst_lang.to_string(),
        entries: Vec::with_capacity(src_lang.entries.len()),
    };

    let entries_per_query = args.batch_size as usize;
    let num_batches = (src_lang.entries.len() + entries_per_query - 1) / entries_per_query;

    let mut prompt = String::new();
    for i in (0..src_lang.entries.len()).step_by(entries_per_query) {
        println!("Batch ID {} / {}", i / entries_per_query, num_batches);

        let from = i;
        let to = std::cmp::min(i + entries_per_query, src_lang.entries.len());

        prompt.clear();
        prompt += &format!("Translate from {} to: {}", src_lang.lang, dst_lang);

        let entries_to_translate = &src_lang.entries[from..to];

        for (j, e) in entries_to_translate.iter().enumerate() {
            prompt += &format!("\n\n# {}\n{}", e.key_name, e.text);

            if !context.is_empty() {
                let mut had_context = false;
                let old_len = prompt.len();
                prompt += "\n\n## Additional Context";

                for c in context {
                    let ce = &c.entries[from + j];
                    if !ce.text.is_empty() {
                        prompt += &format!("\n\n### {}\n{}", c.lang, ce.text);
                        had_context = true;
                    }
                }
                if !had_context {
                    prompt.truncate(old_len)
                }
            }
        }

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
                                translated_result.push(Entry {
                                    key_name: entry.key_name.to_string(),
                                    text: "AI ERROR. GIVEN UP.".to_string(),
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

        dst_lang_set.entries.append(&mut translated);

        if partial_output {
            write_ods(args, &dst_lang_set, main_lang, None)?;
        }
    }

    Ok(dst_lang_set)
}

fn write_ods(
    args: &Args,
    dst_lang: &LangSet,
    lang_sets: &[LangSet],
    original_back: Option<LangSet>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut sheet = Sheet::new("output");
    let mut wb = spreadsheet_ods::WorkBook::new(locale!("en-US"));

    for (i, e) in dst_lang.entries.iter().enumerate() {
        let row = i as u32 + 1;
        sheet.set_value(row, 0, &e.key_name);
        sheet.set_value(row, 1, &lang_sets[0].entries[i].text);
        sheet.set_value(row, 2, &e.text);

        if let Some(original_back) = &original_back {
            sheet.set_value(row, 3, &original_back.entries[i].text);
        }
    }

    wb.push_sheet(sheet);

    let file = File::create(&args.dst_csv)?;
    let mut write = BufWriter::new(file);
    OdsWriteOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .write_ods(&mut wb, &mut write)?;
    write.flush()?;

    Ok(())
}

pub async fn translate_key_mode_ods(
    args: &Args,
    error_log: &mut File,
    ai_settings: &open_ai::AiSettings<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let columns_to_use: Vec<u32> = args
        .ods_key_mode_columns
        .split(",")
        .map(|v| {
            v.parse()
                .expect("ods_key_mode_columns must contain numbers and commas only!")
        })
        .collect();

    // Process main translations.
    let lang_sets = load_ods(&args.src_csv, &columns_to_use);

    if lang_sets.is_empty() {
        panic!("No languages found in ODS file?!");
    }

    let mut dst_lang = translate_lang_set(
        args,
        &args.dst_lang,
        error_log,
        ai_settings,
        &lang_sets,
        true,
    )
    .await?;

    write_ods(args, &dst_lang, &lang_sets, None)?;

    let original_back = match &args.src_lang {
        Some(src_lang) => {
            println!("Main translation done. Beginning translation of original_back");

            let mut tmp_dst_lang = [dst_lang];
            let result = translate_lang_set(
                args,
                &src_lang,
                error_log,
                ai_settings,
                &tmp_dst_lang,
                false,
            )
            .await?;
            dst_lang = std::mem::replace(
                &mut tmp_dst_lang[0],
                LangSet {
                    lang: String::new(),
                    entries: Vec::new(),
                },
            );
            Some(result)
        }
        None => None,
    };

    write_ods(args, &dst_lang, &lang_sets, original_back)?;

    Ok(())
}
