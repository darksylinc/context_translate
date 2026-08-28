# Context Translate - Agent Guide

This document provides AI coding agents with the technical context needed to understand, maintain, and extend the context_translate project.

## Project Overview

**context_translate** is a Rust CLI tool (v0.1.0) that performs AI-powered, context-aware translations for dialogue, subtitles, manga lines, and other text content. It uses local LLMs (via llama.cpp) or OpenAI-compatible APIs to translate text while preserving context from surrounding lines.

### Key Characteristics
- **Stage**: Early alpha (use with caution)
- **Language**: Rust 2024 edition
- **Primary Use Cases**: Manga translation, game localization, video subtitle translation
- **Integration**: Includes Blender 4.2+ plugin for text object export/import

---

## Architecture

### High-Level Structure

```
context_translate/
├── src/
│   ├── main.rs          # CLI entry point, core translation logic
│   ├── error.rs         # Error type definitions
│   ├── ods_reader.rs    # ODS file handling for spreadsheet mode
│   └── open_ai.rs       # HTTP client for AI API interactions
├── blender_plugin/
│   └── text_translator_csv/
│       ├── __init__.py           # Blender addon registration
│       ├── exporter_csv.py       # Export text objects to CSV
│       ├── importer_csv.py       # Import translated CSV to text objects
│       └── importer_animated_subs_csv.py
├── examples/
│   ├── manga/             # System prompt & options for manga translation
│   └── ods/               # System prompt & options for spreadsheet translation
├── Cargo.toml             # Rust dependencies
└── README.md              # User documentation
```

### Core Data Flow

1. **Input**: CSV file with columns: `datablock_name`, `Collection` (speaker), `Text Contents`. ODS file has different structure.
2. **Batching**: Lines are split into batches (default: 20) with pre/post context
3. **Request Generation**: A JSON `TranslationRequest` is built (`previous_context`, `text_to_translate` with per-entry `index`, `future_context`, `destination_language`)
4. **AI API Call**: JSON sent as the user message to local LLM (llama.cpp) or OpenAI-compatible endpoint, with `response_format: {"type": "json_object"}` to constrain output to valid JSON
5. **Response Parsing**: The AI must reply with a JSON `TranslationResponse`; indices are validated as an exact permutation of the batch (order-independent, deterministic failure detection)
6. **Output**: Translated CSV (or ODS, if in ODS mode) with optional back-translation for validation

---

## Module Details

### `main.rs` - Core Application

**Key Structures:**

```rust
// Main data structure for CSV rows
struct BlenderTextRow {
    datablock_name: String,  // Unique identifier (e.g., "Unique Key 001")
    speaker: String,         // Collection name / character name
    text: String,            // Original/translated text
    original: Option<String>,
    original_back: Option<String>,
    remarks: Option<String>,
    confidence: Option<f64>,       // LLM self-assessed 0.0..=1.0
    needs_revision: Option<bool>,  // Multipass hook
}

// LLM JSON protocol (CSV path only)
struct LineItem { speaker: String, text: String }                  // context line
struct IndexedLineItem { index: usize, speaker: String, text: String }
struct TranslationRequest<'a> {   // LLM input
    destination_language: &'a str,
    previous_context: Vec<LineItem>,
    text_to_translate: Vec<IndexedLineItem>,
    future_context: Vec<LineItem>,
}
struct TranslatedItem {           // LLM output entry
    index: usize, text: String,
    confidence: f64, needs_revision: bool,
    #[serde(default)] remarks: String,
}
struct TranslationResponse { translations: Vec<TranslatedItem> }
```

**Key Functions:**

1. **`read_csv(path: &str)`** - Reads CSV with semicolon delimiter (`;`)
2. **`write_csv(path, entries, original_back)`** - Writes translated CSV (includes `Confidence` / `Needs Revision` columns)
3. **`build_translation_request(pre_cxt, to_translate, pos_cxt, dst_language)`** - Serializes the JSON `TranslationRequest` (M pre-context / N to-translate / O post-context lines)
4. **`translate_batch(pre_cxt, batch, pos_cxt, ai_settings, dst_language, error_log)`** - Pure unit of work: build request → call LLM → parse/validate JSON (3 retries, then "AI ERROR. GIVEN UP." fallback rows flagged `needs_revision: true`). Multipass architectures reuse this directly.
5. **`translate_blender_lines(lines, batch_size, pre_ctx, pos_ctx, ai_settings, dst_lang, error_log)`** - Batching loop over `translate_batch`
6. **`process_ai_response(response, entries, orig_prompt, error_log)`** - Parses/validates the JSON response, logs failures to error log

**Response Parsing Rules** (see `process_ai_response_impl`):
- Strips markdown code fences if present; slices from first `{` to last `}` to tolerate prose
- `serde_json::from_str::<TranslationResponse>`
- Validates count matches and that echoed `index` values form an exact permutation of `0..n` (reordered output is fine; missing/duplicate/out-of-range indices are deterministic errors)
- Maps results back to input order; carries `confidence`/`needs_revision`/`remarks` into rows

**Translation Loop Pattern:**

```rust
// step_by over entries; each batch gets pre/post context slices
for i in (0..entries.len()).step_by(entries_per_query) {
    let from = i;
    let to = std::cmp::min(i + entries_per_query, entries.len());
    let pre_cxt = &entries[from.saturating_sub(pre_context_lines)..from];
    let pos_cxt = &entries[to..std::cmp::min(to + pos_context_lines, entries.len())];

    // translate_batch: build JSON request -> run_prompt -> parse JSON (3 retries)
    let translated = translate_batch(pre_cxt, &entries[from..to], pos_cxt,
        ai_settings, dst_language, error_log).await;
    output.extend(translated);
}
// TODO(multipass): rows with needs_revision == true can be re-sent through
// translate_batch with wider context and merged back by datablock_name.
```

### `open_ai.rs` - API Communication

**Purpose**: Handles HTTP requests to AI endpoints (local llama.cpp or OpenAI-compatible)

**Key Structures:**

```rust
pub struct AiSettings<'a> {
    pub endpoint: String,         // API URL (e.g., "http://127.0.0.1:8081/v1/chat/completions")
    pub api_key: String,          // Bearer token
    pub system_prompt: String,    // System instruction text
    pub model: String,            // Model name (e.g., "Qwen3.5-35B-A3B-UD-Q4_K_L.gguf")
    pub timeout_secs: u64,        // Request timeout
    pub extra_options: Option<&'a Map<String, Value>>, // Additional JSON fields
    pub response_format: Option<&'a Value>, // e.g. {"type": "json_object"} for CSV path
    pub debug: bool,              // Enable debug output
}
```

**Function `run_prompt`**: 
- Builds JSON request with system + user messages
- Applies timeout using `tokio::time::timeout`
- Handles authentication headers
- Merges `response_format` (if set) and extra_options into request body
- Returns AI response content string

### `ods_reader.rs` - Spreadsheet Mode

**Purpose**: Handles ODS files with multiple language columns (key-based translation)

**Key Structures:**

```rust
struct Entry {
    key_name: String,   // Unique identifier
    text: String,       // Translation in specific language
}

struct LangSet {
    lang: String,       // Language name (from header row)
    entries: Vec<Entry>,
}
```

**Workflow:**
1. Load ODS file, extract columns specified by `--ods-key-mode-columns`
2. First column is always the source language
3. Subsequent columns are target languages to translate
4. For each target column, translate entries keyed by `key_name`

---

## CLI Interface

### Command-Line Arguments (from `clap` derive)

| Argument | Required | Description |
|----------|----------|-------------|
| `--src-csv` | ✓ | Input CSV/ODS file path |
| `--dst-csv` | ✓ | Output CSV/ODS file path |
| `--dst-lang` | ✓ | Target language (e.g., "Japanese") |
| `--model` | ✓ | LLM model name |
| `--endpoint` | ✓ | API endpoint URL |
| `--api-key` |   | API key (or env: `OPENAI_API_KEY`) |
| `--system-prompt` |   | Custom system prompt file |
| `--batch-size` |   | Lines per batch (default: 20) |
| `--pre-ctx` |   | Preceding context lines (default: 3) |
| `--pos-ctx` |   | Following context lines (default: 3) |
| `--src-lang` |   | Source language (enables back-translation) |
| `--timeout-secs` |   | Request timeout in seconds |
| `--llm-options` |   | JSON file with extra LLM options |
| `--debug` |   | Enable debug output |
| `--ods-key-mode-columns` |   | Comma-separated column indices for ODS mode |

### Example Usage

```bash
./context_translate \
    --src-lang "English" \
    --dst-lang "Spanish" \
    --model "Qwen3.5-35B-A3B-UD-Q4_K_L.gguf" \
    --system-prompt examples/manga/system_prompt.txt \
    --endpoint http://127.0.0.1:8081/v1/chat/completions \
    --src-csv input.csv \
    --dst-csv output.csv \
    --timeout-secs 500 \
    --llm-options examples/manga/options.json \
    --api-key "YOUR_KEY" \
    --batch-size 20 \
    --pre-ctx 3 \
    --pos-ctx 3 \
    --debug
```

---

## System Prompts

Two example system prompts are provided, each tailored for different use cases:

### Manga Mode (`examples/manga/system_prompt.txt`)
- **Persona**: "Elara", conversational yet professional
- **Protocol**: JSON in / JSON out (see `main.rs` protocol structs)
- **Key Requirements**:
  - Translate only `text_to_translate`; context is reference-only
  - Reply with a single `{"translations": [...]}` JSON object, no prose/fences
  - Echo each entry's `index`; provide `confidence` (0.0–1.0), `needs_revision` (bool), `remarks` (English)
  - Preserve newlines / per-line length for speech bubbles; never translate speaker names
  - `needs_revision: true` when context is insufficient or the line is genuinely ambiguous (multipass hook)

### ODS Mode (`examples/ods/system_prompt.txt`)
- **Persona**: Neutral, no remarks/commentary
- **Focus**: Game localization
- **Key Requirements**:
  - Handle RTL languages for brace reordering
  - Output format: `# Keyword` + translated text

---

## LLM Options (`options.json`)

Additional JSON fields merged into API request body:

**Manga Example:**
```json
{
  "cache_prompt": true
}
```

**ODS Example:**
```json
{
  "cache_prompt": true
}
```

---

## Error Handling

`src/error.rs` defines the crate error type:

```rust
pub enum Error {
    HttpStatus(u16),          // Non-2xx HTTP response from the AI endpoint
    InvalidTranslation,       // ODS path: free-text response could not be parsed
    InvalidResponse(String),  // CSV path: JSON response failed to parse/validate (carries the reason)
}
```

- Top-level errors propagate with `?` as `Box<dyn std::error::Error>`.
- The CSV path retries a batch up to 3 times on `InvalidResponse`, then emits fallback rows (`text` empty, `remarks` = "AI ERROR. GIVEN UP.", `needs_revision` = true) so the pipeline never hard-fails a whole file.
- Every rejected response and its original JSON request are appended to `errors.log` (created in the working directory).

---

## CSV/ODS Format

### CSV Format (semicolon-delimited)

| Column | Description | Example |
|--------|-------------|---------|
| `datablock_name` | Unique identifier | `Unique Key 001` |
| `Collection` | Speaker/character name | `John` |
| `Text Contents` | Original/translated text | `Hi! Did you enjoy the movie?` |
| `Original` | Original text (output only) | `Hi! Did you enjoy the movie?` |
| `Original Back` | Back-translated text (output only) | `Hi! Did you enjoy the movie?` |
| `Remarks` | AI-generated notes (output only) | Translation notes |
| `Confidence` | LLM self-assessed confidence 0.0–1.0 (output only) | `0.92` |
| `Needs Revision` | LLM flag requesting a review pass (output only) | `true` / `false` |

Input CSVs may omit the output-only columns; `Option` fields + `#[serde(default)]` make them read as empty.

### ODS Format (key-based)

| Column | Description |
|--------|-------------|
| 0 | Key identifier |
| 1 | Source language text |
| 2 | Translated text |
| 3 | Back-translated text (optional) |

---

## Blender Plugin Architecture

### Export (`exporter_csv.py`)

1. Finds all visible FONT objects in current viewport
2. Sorts by position (Z-axis first, then X-axis for same height)
3. Exports to CSV with columns: `datablock_name`, `Collection`, `Text Contents`
4. Uses semicolon delimiter, quotes all fields

### Import (`importer_csv.py`)

1. Reads translated CSV
2. For each row, finds object matching `datablock_name`
3. Duplicates object and its datablock
4. Renames to `{name}_jp`
5. Sets Japanese font (`Bfont Regular`), resolution 12
6. Links to "Japanese Text" collection

### Animated Subs Import (`importer_animated_subs_csv.py`)

- Similar to standard import but handles animated subtitle sequences

---

## Dependencies

### Rust (`Cargo.toml`)

```toml
clap = "4.5.48"           # CLI parsing with derive macros
csv = "1.3.1"             # CSV reading/writing
icu_locale_core = "2.1.1" # Locale utilities for ODS
reqwest = "0.12"          # HTTP client (with json feature)
serde = "1.0"             # Serialization (with derive feature)
serde_json = "1.0.145"    # JSON parsing
spreadsheet-ods = "1.0.2" # ODS file handling
tokio = "1.49.0"          # Async runtime (full features)
writeable = "0.6.2"       # Writeable string type
```

### Python (Blender Plugin)

- Requires Blender 4.2+ (bpy module)
- No external dependencies

---

## Important Considerations

### Performance
- **Current limitation**: Batches are NOT processed concurrently (llama.cpp issues)
- **Batch size**: Larger batches = fewer API calls but higher context window usage
- **Timeout**: Default 500 seconds per request; adjust based on model speed

### Context Management
- **Pre-context**: Lines before current batch (improves translation coherence)
- **Post-context**: Lines after current batch (provides future context)
- **Order matters**: AI translation differs based on line order (conversation flow)

### Security Warnings

From README.md:
1. **Memory exhaustion**: Malicious prompt causing AI to generate huge output
2. **Prompt injection**: AI forced to produce invalid output → infinite retries
3. **Code execution**: If AI has execution capabilities, it will run whatever prompt says

### Localization Notes

- **RTL languages**: Brace reordering may be needed for `{0}`, `{1}` placeholders
- **Newline preservation**: Critical for manga (speech bubble constraints)

---

## Testing

### Unit Tests

`cargo test` runs the JSON protocol tests in `src/main.rs` (`mod tests`): clean/fenced/preamble JSON, reordered indices, and the failure cases (missing/duplicate/out-of-range index, wrong count, non-JSON).

### Test Setup

1. Build project: `cargo build --release`
2. Run test script: `./test.sh`
3. Test input: `test_input.csv`
4. Output: `test_output.csv` (includes `Confidence` / `Needs Revision` columns)

### Manual Testing

```bash
# Debug mode with sample data
./target/release/context_translate \
    --debug \
    --src-csv test_input.csv \
    --dst-csv test_output.csv \
    --dst-lang "Spanish" \
    --model "test_model.gguf" \
    --endpoint "http://localhost:8081/v1/chat/completions" \
    --api-key "test"
```

---

## Code Patterns & Conventions

### Async/Await Pattern

```rust
// All AI calls are async
let response = open_ai::run_prompt(&ai_settings, &prompt).await?;

// Main function is async
async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    // ...
}
```

### Error Propagation

Uses `?` operator with `Box<dyn std::error::Error>` for top-level errors:

```rust
File::open(&args.system_prompt)?.read_to_string(&mut system_prompt)?;
```

---

---

## Extension Points

### Customizing System Prompts

1. Copy example prompt from `examples/manga/` or `examples/ods/`
2. Modify for specific use case
3. Pass via `--system-prompt` argument

---

## Troubleshooting for Agents

### Common Issues

1. **"Invalid Translation" errors**: 
   - Check system prompt is appropriate for target language
   - Verify AI model supports requested language
   - Increase `timeout-secs` if AI is slow

2. **Memory issues**:
   - Reduce `batch-size`
   - Reduce `pre-ctx`/`pos-ctx`
   - Use smaller model

3. **CSV parsing errors**:
   - Verify delimiter is semicolon (`;`), not comma (`,`)
   - Check for unmatched quotes in text fields

4. **ODS mode not working**:
   - Verify `--ods-key-mode-columns` format: `0,1,2` (zero-indexed)
   - Check ODS has correct header row

### Debug Mode

Enable with `--debug` flag:
- Shows full system prompt
- Shows full user prompt
- Shows AI response

### Log Files

- `error.log`: Contains failed responses and original prompts
- Check this file when translations fail

---

## Version Information

- **Project Version**: 0.1.0
- **Rust Edition**: 2024
- **Blender Compatibility**: 4.2 LTS+
- **Last Updated**: 2026-01-23

---

## Contributing

When modifying code:

1. **Preserve error logging**: Always log to error log on translation failure
2. **Test with real data**: Use manga/game translation examples
3. **Update system prompts**: If adding new features, document in system prompt examples

---

## References

- [README.md](./README.md) - User documentation
- [LICENSE](./LICENSE) - GPL 3.0
- [examples/manga/system_prompt.txt](./examples/manga/system_prompt.txt) - Manga translation prompt
- [examples/ods/system_prompt.txt](./examples/ods/system_prompt.txt) - Game localization prompt