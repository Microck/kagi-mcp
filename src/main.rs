use std::{
    env,
    error::Error,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use tokio::{io::AsyncWriteExt, process::Command, time::timeout};
use tracing_subscriber::EnvFilter;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const CLI_PATH_ENV: &str = "KAGI_CLI_PATH";
const CLI_PROFILE_ENV: &str = "KAGI_CLI_PROFILE";
const TIMEOUT_ENV: &str = "KAGI_MCP_TIMEOUT_MS";
const STEALTH_ENV: &str = "KAGI_MCP_STEALTH";

const DEFAULT_JITTER_BASE_MS: u64 = 800;
const DEFAULT_JITTER_SPREAD_MS: u64 = 1200;
const DEFAULT_MIN_INTERVAL_MS: u64 = 1500;
const DEFAULT_STEALTH_CACHE_TTL_SECS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Json,
    JsonToToon,
    Toon,
    Text,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum PrivacyMode {
    Unpersonalized,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum StealthMode {
    On,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandSpec {
    args: Vec<String>,
    stdin: Option<String>,
    output_mode: OutputMode,
    profile_override: Option<String>,
}

#[derive(Debug, Clone)]
struct StealthConfig {
    enabled: bool,
    jitter_base_ms: u64,
    jitter_spread_ms: u64,
    min_interval_ms: u64,
    #[expect(dead_code)]
    default_cache_ttl_secs: u64,
}

impl StealthConfig {
    fn disabled() -> Self {
        Self {
            enabled: false,
            jitter_base_ms: DEFAULT_JITTER_BASE_MS,
            jitter_spread_ms: DEFAULT_JITTER_SPREAD_MS,
            min_interval_ms: DEFAULT_MIN_INTERVAL_MS,
            default_cache_ttl_secs: DEFAULT_STEALTH_CACHE_TTL_SECS,
        }
    }
}

#[derive(Debug)]
struct RateLimiter {
    last_call: Mutex<Option<Instant>>,
    min_interval: Duration,
}

impl RateLimiter {
    fn new(min_interval: Duration) -> Self {
        Self {
            last_call: Mutex::new(None),
            min_interval,
        }
    }

    async fn throttle(&self) {
        let now = Instant::now();
        let sleep_dur = {
            let guard = self.last_call.lock().unwrap();
            match *guard {
                Some(last) if now.duration_since(last) < self.min_interval => {
                    self.min_interval - now.duration_since(last)
                }
                _ => Duration::ZERO,
            }
        };
        if !sleep_dur.is_zero() {
            tokio::time::sleep(sleep_dur).await;
        }
        *self.last_call.lock().unwrap() = Some(Instant::now());
    }
}

#[derive(Debug, Clone)]
struct CliRunner {
    cli_path: PathBuf,
    profile: Option<String>,
    timeout: Duration,
    stealth: StealthConfig,
    limiter: Arc<RateLimiter>,
}

#[derive(Debug, Clone, PartialEq)]
enum CommandOutput {
    Json(Value),
    Toon(String),
    Text(String),
}

#[derive(Debug, Error)]
enum RunnerError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("failed to start `{path}`: {message}")]
    Spawn { path: String, message: String },
    #[error("`{path}` timed out after {timeout_ms}ms")]
    Timeout { path: String, timeout_ms: u64 },
    #[error("{message}")]
    CommandFailed { message: String },
    #[error("failed to parse CLI JSON output: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct SearchArgs {
    /// Search query to send to Kagi.
    query: String,
    /// Optional Snap shortcut prefix, for example "reddit".
    #[serde(default)]
    snap: Option<String>,
    /// Optional Kagi lens index.
    #[serde(default)]
    lens: Option<String>,
    /// Optional region code (e.g., "us", "gb", "no_region").
    #[serde(default)]
    region: Option<String>,
    /// Optional time filter (day, week, month, year).
    #[serde(default)]
    time: Option<String>,
    /// Restrict results to pages updated on or after this date.
    #[serde(default)]
    from_date: Option<String>,
    /// Restrict results to pages updated on or before this date.
    #[serde(default)]
    to_date: Option<String>,
    /// Optional order (default, recency, website, trackers).
    #[serde(default)]
    order: Option<String>,
    /// Enable verbatim search mode.
    #[serde(default)]
    verbatim: Option<bool>,
    /// Force personalized search on.
    #[serde(default)]
    personalized: Option<bool>,
    /// Force personalized search off.
    #[serde(default)]
    no_personalized: Option<bool>,
    /// Optional privacy preset. "unpersonalized" disables personalization and defaults region to no_region.
    #[serde(default)]
    privacy_mode: Option<PrivacyMode>,
    /// Enable stealth mode for this request. Adds jitter, caching, and rate limiting.
    #[serde(default)]
    stealth_mode: Option<StealthMode>,
    /// Per-call CLI profile override. Takes precedence over KAGI_CLI_PROFILE.
    #[serde(default)]
    profile: Option<String>,
    /// Render each result with a lightweight template.
    #[serde(default)]
    template: Option<String>,
    /// Summarize the top N result URLs using subscriber summarizer.
    #[serde(default)]
    follow: Option<u64>,
    /// Maximum number of search results to return.
    #[serde(default)]
    limit: Option<u64>,
    /// Search the Kagi News tab instead of web results.
    #[serde(default)]
    news: Option<bool>,
    /// Locally cache this response.
    #[serde(default)]
    local_cache: Option<bool>,
    /// Override local cache TTL in seconds.
    #[serde(default)]
    cache_ttl: Option<u64>,
    /// Optional output format (toon, json, pretty, compact, markdown, csv).
    #[serde(default)]
    format: Option<String>,
    /// Disable colored terminal output for pretty format.
    #[serde(default)]
    no_color: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct SummarizeArgs {
    /// URL to summarize.
    #[serde(default)]
    url: Option<String>,
    /// Text to summarize.
    #[serde(default)]
    text: Option<String>,
    /// Use subscriber summarizer mode.
    #[serde(default)]
    subscriber: Option<bool>,
    /// Subscriber mode only.
    #[serde(default)]
    length: Option<String>,
    /// Public API mode only.
    #[serde(default)]
    engine: Option<String>,
    /// Summary type or mode.
    #[serde(default)]
    summary_type: Option<String>,
    /// Target language code.
    #[serde(default)]
    target_language: Option<String>,
    /// Allow cached responses.
    #[serde(default)]
    cache: Option<bool>,
    /// URLs or text items to summarize through `kagi summarize --filter`.
    #[serde(default)]
    filter_items: Vec<String>,
    /// Locally cache this response.
    #[serde(default)]
    local_cache: Option<bool>,
    /// Override local cache TTL in seconds.
    #[serde(default)]
    cache_ttl: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct ExtractArgs {
    /// HTTPS URL of the page to extract as markdown.
    url: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct NewsArgs {
    /// News category slug.
    #[serde(default)]
    category: Option<String>,
    /// Maximum number of stories.
    #[serde(default)]
    limit: Option<u32>,
    /// News language code.
    #[serde(default)]
    lang: Option<String>,
    /// List built-in content-filter presets instead of stories.
    #[serde(default)]
    list_filter_presets: Option<bool>,
    /// Built-in content-filter preset IDs to apply.
    #[serde(default)]
    filter_preset: Vec<String>,
    /// Custom keywords to filter out from the feed.
    #[serde(default)]
    filter_keyword: Vec<String>,
    /// Filter behavior for matching stories (hide, blur).
    #[serde(default)]
    filter_mode: Option<String>,
    /// Story fields to inspect (title, summary, all).
    #[serde(default)]
    filter_scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct LangArgs {
    /// Optional language code.
    #[serde(default)]
    lang: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct AssistantArgs {
    /// Prompt to send to Kagi Assistant.
    query: String,
    /// Optional existing thread id.
    #[serde(default)]
    thread_id: Option<String>,
    /// Local files to attach to the prompt.
    #[serde(default)]
    attach: Vec<String>,
    /// Saved assistant name, id, or invoke profile slug.
    #[serde(default)]
    assistant: Option<String>,
    /// Output format (toon, json, pretty, compact, markdown).
    #[serde(default)]
    format: Option<String>,
    /// Override the Assistant model slug.
    #[serde(default)]
    model: Option<String>,
    /// Override the Assistant lens id.
    #[serde(default)]
    lens: Option<u64>,
    /// Force web access on.
    #[serde(default)]
    web_access: Option<bool>,
    /// Force web access off.
    #[serde(default)]
    no_web_access: Option<bool>,
    /// Force personalizations on.
    #[serde(default)]
    personalized: Option<bool>,
    /// Force personalizations off.
    #[serde(default)]
    no_personalized: Option<bool>,
    /// Export the Assistant transcript when the wrapped CLI supports it.
    #[serde(default)]
    export: Option<String>,
    /// Disable colored terminal output for pretty format.
    #[serde(default)]
    no_color: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct AssistantReplArgs {
    /// Prompts to feed to the Assistant REPL before sending `/exit`.
    #[serde(default)]
    prompts: Vec<String>,
    /// Optional existing thread id.
    #[serde(default)]
    thread_id: Option<String>,
    /// Saved assistant name, id, or invoke profile slug.
    #[serde(default)]
    assistant: Option<String>,
    /// Override the Assistant model slug.
    #[serde(default)]
    model: Option<String>,
    /// Output format for each response (toon, json, pretty, compact, markdown).
    #[serde(default)]
    format: Option<String>,
    /// Export the REPL transcript when the session exits.
    #[serde(default)]
    export: Option<String>,
    /// Disable colored terminal output.
    #[serde(default)]
    no_color: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct FastGptArgs {
    /// Prompt to send to FastGPT.
    query: String,
    /// Allow cached responses.
    #[serde(default)]
    cache: Option<bool>,
    /// Enable web search.
    #[serde(default)]
    web_search: Option<bool>,
    /// Locally cache this response.
    #[serde(default)]
    local_cache: Option<bool>,
    /// Override local cache TTL in seconds.
    #[serde(default)]
    cache_ttl: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct EnrichArgs {
    /// Query to send to enrichment endpoints.
    query: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct SmallWebArgs {
    /// Maximum number of feed entries.
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct QuickArgs {
    /// Query to get a quick answer for.
    query: String,
    /// Optional output format (toon, json, pretty, compact, markdown).
    #[serde(default)]
    format: Option<String>,
    /// Disable colored terminal output for pretty format.
    #[serde(default)]
    no_color: Option<bool>,
    /// Scope quick answer to a Kagi lens by numeric index.
    #[serde(default)]
    lens: Option<String>,
    /// Locally cache this response.
    #[serde(default)]
    local_cache: Option<bool>,
    /// Override local cache TTL in seconds.
    #[serde(default)]
    cache_ttl: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct TranslateArgs {
    /// Text to translate.
    text: String,
    /// Source language code (default: auto).
    #[serde(default)]
    from: Option<String>,
    /// Target language code (default: en).
    #[serde(default)]
    to: Option<String>,
    /// Translation quality preference.
    #[serde(default)]
    quality: Option<String>,
    /// Translation model override.
    #[serde(default)]
    model: Option<String>,
    /// Prediction text to bias the translation.
    #[serde(default)]
    prediction: Option<String>,
    /// Predicted source language code.
    #[serde(default)]
    predicted_language: Option<String>,
    /// Formality setting.
    #[serde(default)]
    formality: Option<String>,
    /// Speaker gender hint.
    #[serde(default)]
    speaker_gender: Option<String>,
    /// Addressee gender hint.
    #[serde(default)]
    addressee_gender: Option<String>,
    /// Language complexity setting.
    #[serde(default)]
    language_complexity: Option<String>,
    /// Translation style setting.
    #[serde(default)]
    translation_style: Option<String>,
    /// Extra translation context.
    #[serde(default)]
    context: Option<String>,
    /// Dictionary language override.
    #[serde(default)]
    dictionary_language: Option<String>,
    /// Time formatting style.
    #[serde(default)]
    time_format: Option<String>,
    /// Toggle definition-aware translation behavior.
    #[serde(default)]
    use_definition_context: Option<bool>,
    /// Toggle language-feature enrichment.
    #[serde(default)]
    enable_language_features: Option<bool>,
    /// Preserve source formatting when possible.
    #[serde(default)]
    preserve_formatting: Option<bool>,
    /// Raw JSON array passed through as context_memory.
    #[serde(default)]
    context_memory_json: Option<String>,
    /// Skip alternative translations.
    #[serde(default)]
    no_alternatives: Option<bool>,
    /// Skip word insights.
    #[serde(default)]
    no_word_insights: Option<bool>,
    /// Skip suggestions.
    #[serde(default)]
    no_suggestions: Option<bool>,
    /// Skip alignments.
    #[serde(default)]
    no_alignments: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct BatchArgs {
    /// Search queries to run in parallel.
    #[serde(default)]
    queries: Vec<String>,
    /// Search queries to pass through stdin.
    #[serde(default)]
    stdin_queries: Vec<String>,
    /// Maximum concurrent requests (default: 3).
    #[serde(default)]
    concurrency: Option<u64>,
    /// Rate limit in requests per minute (default: 60).
    #[serde(default)]
    rate_limit: Option<u32>,
    /// Optional output format (toon, json, pretty, compact, markdown, csv).
    #[serde(default)]
    format: Option<String>,
    /// Optional Snap shortcut prefix for every query.
    #[serde(default)]
    snap: Option<String>,
    /// Scope all searches to a Kagi lens by numeric index.
    #[serde(default)]
    lens: Option<String>,
    /// Restrict results to a Kagi region code.
    #[serde(default)]
    region: Option<String>,
    /// Restrict results to a recent time window.
    #[serde(default)]
    time: Option<String>,
    /// Restrict results to pages updated on or after this date.
    #[serde(default)]
    from_date: Option<String>,
    /// Restrict results to pages updated on or before this date.
    #[serde(default)]
    to_date: Option<String>,
    /// Reorder search results.
    #[serde(default)]
    order: Option<String>,
    /// Enable verbatim search mode for all batch requests.
    #[serde(default)]
    verbatim: Option<bool>,
    /// Force personalized search on for all batch requests.
    #[serde(default)]
    personalized: Option<bool>,
    /// Force personalized search off for all batch requests.
    #[serde(default)]
    no_personalized: Option<bool>,
    /// Optional privacy preset. "unpersonalized" disables personalization and defaults region to no_region.
    #[serde(default)]
    privacy_mode: Option<PrivacyMode>,
    /// Enable stealth mode for this request. Adds jitter, caching, and rate limiting.
    #[serde(default)]
    stealth_mode: Option<StealthMode>,
    /// Per-call CLI profile override. Takes precedence over KAGI_CLI_PROFILE.
    #[serde(default)]
    profile: Option<String>,
    /// Render each result with a lightweight template.
    #[serde(default)]
    template: Option<String>,
    /// Maximum number of search results to return per query.
    #[serde(default)]
    limit: Option<u64>,
    /// Disable colored terminal output for pretty format.
    #[serde(default)]
    no_color: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct AskPageArgs {
    /// URL of the page to ask about.
    url: String,
    /// Question to ask about the page.
    question: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct ThreadIdArgs {
    /// Assistant thread ID.
    thread_id: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct ThreadExportArgs {
    /// Assistant thread ID.
    thread_id: String,
    /// Export format (markdown, json).
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct HistoryListArgs {
    /// Maximum local history entries to return.
    #[serde(default)]
    limit: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct SitePrefSetArgs {
    /// Domain to configure.
    domain: String,
    /// Preference mode: block, lower, normal, higher, or pin.
    mode: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct SitePrefDomainArgs {
    /// Domain to remove.
    domain: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct AuthSetArgs {
    /// Kagi API token to save into `.kagi.toml`.
    #[serde(default)]
    api_token: Option<String>,
    /// Kagi session token or full Session Link URL to save into `.kagi.toml`.
    #[serde(default)]
    session_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct LensTargetArgs {
    /// Lens id or exact lens name.
    target: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct LensCreateArgs {
    /// Lens display name.
    name: String,
    #[serde(flatten)]
    options: LensOptions,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct LensUpdateArgs {
    /// Lens id or exact lens name.
    target: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(flatten)]
    options: LensOptions,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq, Default)]
struct LensOptions {
    #[serde(default)]
    included_sites: Option<String>,
    #[serde(default)]
    included_keywords: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    before_date: Option<String>,
    #[serde(default)]
    after_date: Option<String>,
    #[serde(default)]
    excluded_sites: Option<String>,
    #[serde(default)]
    excluded_keywords: Option<String>,
    #[serde(default)]
    shortcut: Option<String>,
    #[serde(default)]
    autocomplete_keywords: Option<bool>,
    #[serde(default)]
    no_autocomplete_keywords: Option<bool>,
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    file_type: Option<String>,
    #[serde(default)]
    share_with_team: Option<bool>,
    #[serde(default)]
    no_share_with_team: Option<bool>,
    #[serde(default)]
    share_copy_code: Option<bool>,
    #[serde(default)]
    no_share_copy_code: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct AssistantCustomTargetArgs {
    /// Custom assistant id or exact assistant name.
    target: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct AssistantCustomCreateArgs {
    /// Assistant name.
    name: String,
    #[serde(flatten)]
    options: AssistantCustomOptions,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct AssistantCustomUpdateArgs {
    /// Custom assistant id or exact assistant name.
    target: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(flatten)]
    options: AssistantCustomOptions,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq, Default)]
struct AssistantCustomOptions {
    #[serde(default)]
    bang_trigger: Option<String>,
    #[serde(default)]
    web_access: Option<bool>,
    #[serde(default)]
    no_web_access: Option<bool>,
    #[serde(default)]
    lens: Option<String>,
    #[serde(default)]
    personalized: Option<bool>,
    #[serde(default)]
    no_personalized: Option<bool>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    instructions: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct CustomBangTargetArgs {
    /// Bang id, exact name, or trigger.
    target: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct CustomBangCreateArgs {
    /// Bang display name.
    name: String,
    /// Bang trigger without the leading `!`.
    trigger: String,
    #[serde(flatten)]
    options: CustomBangOptions,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct CustomBangUpdateArgs {
    /// Bang id, exact name, or trigger.
    target: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    #[serde(flatten)]
    options: CustomBangOptions,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq, Default)]
struct CustomBangOptions {
    #[serde(default)]
    template: Option<String>,
    #[serde(default)]
    snap_domain: Option<String>,
    #[serde(default)]
    regex_pattern: Option<String>,
    #[serde(default)]
    shortcut_menu: Option<bool>,
    #[serde(default)]
    no_shortcut_menu: Option<bool>,
    #[serde(default)]
    open_snap_domain: Option<bool>,
    #[serde(default)]
    no_open_snap_domain: Option<bool>,
    #[serde(default)]
    open_base_path: Option<bool>,
    #[serde(default)]
    no_open_base_path: Option<bool>,
    #[serde(default)]
    encode_placeholder: Option<bool>,
    #[serde(default)]
    no_encode_placeholder: Option<bool>,
    #[serde(default)]
    plus_for_space: Option<bool>,
    #[serde(default)]
    no_plus_for_space: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct RedirectTargetArgs {
    /// Redirect id or exact rule text.
    target: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct RedirectCreateArgs {
    /// Full regex|replacement redirect rule.
    rule: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct RedirectUpdateArgs {
    /// Redirect id or exact rule text.
    target: String,
    /// Full replacement regex|replacement redirect rule.
    rule: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct WatchArgs {
    /// Search query to monitor.
    query: String,
    /// Poll interval in seconds.
    #[serde(default)]
    interval: Option<u64>,
    /// Number of polls to run. Omit or pass 0 for CLI default behavior.
    #[serde(default)]
    count: Option<u32>,
    /// Optional output format (toon, json, pretty, compact, markdown, csv).
    #[serde(default)]
    format: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct NotifyArgs {
    /// Search query to run before notifying.
    #[serde(default)]
    query: Option<String>,
    /// Kagi News category to fetch before notifying.
    #[serde(default)]
    news_category: Option<String>,
    /// Webhook endpoint that receives the JSON payload.
    webhook_url: String,
    /// Only send when `watch` detects a changed search result set.
    #[serde(default)]
    change_only: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct CompletionArgs {
    /// Shell completion target: bash, zsh, fish, or powershell.
    shell: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct SkillGetArgs {
    /// Embedded skill name. Defaults to the core `kagi` skill.
    #[serde(default)]
    name: Option<String>,
    /// Print the full skill body without frontmatter.
    #[serde(default)]
    full: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema, PartialEq, Eq)]
struct SkillPathArgs {
    /// Embedded skill name. Omit to print the embedded skill root locator.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Clone)]
struct KagiServer {
    runner: CliRunner,
    tool_router: ToolRouter<KagiServer>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let runner = CliRunner::from_env()?;
    let service = KagiServer::new(runner).serve(stdio()).await?;

    tracing::info!("kagi-mcp started");
    service.waiting().await?;
    Ok(())
}

impl CliRunner {
    fn from_env() -> Result<Self, RunnerError> {
        let cli_path = env::var(CLI_PATH_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("kagi"));
        let profile = env::var(CLI_PROFILE_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty());
        let timeout = match env::var(TIMEOUT_ENV) {
            Ok(raw) => {
                let value = raw.parse::<u64>().map_err(|_| {
                    RunnerError::Config(format!(
                        "{TIMEOUT_ENV} must be a positive integer in milliseconds"
                    ))
                })?;
                if value == 0 {
                    return Err(RunnerError::Config(format!(
                        "{TIMEOUT_ENV} must be greater than 0"
                    )));
                }
                Duration::from_millis(value)
            }
            Err(_) => Duration::from_millis(DEFAULT_TIMEOUT_MS),
        };

        let stealth_enabled = env::var(STEALTH_ENV)
            .map(|v| !v.is_empty() && v != "0" && v.to_lowercase() != "false")
            .unwrap_or(false);

        let stealth = if stealth_enabled {
            StealthConfig {
                enabled: true,
                jitter_base_ms: DEFAULT_JITTER_BASE_MS,
                jitter_spread_ms: DEFAULT_JITTER_SPREAD_MS,
                min_interval_ms: DEFAULT_MIN_INTERVAL_MS,
                default_cache_ttl_secs: DEFAULT_STEALTH_CACHE_TTL_SECS,
            }
        } else {
            StealthConfig::disabled()
        };

        let min_interval = if stealth.enabled {
            Duration::from_millis(stealth.min_interval_ms)
        } else {
            Duration::ZERO
        };

        Ok(Self {
            cli_path,
            profile,
            timeout,
            stealth,
            limiter: Arc::new(RateLimiter::new(min_interval)),
        })
    }

    #[cfg(test)]
    fn new(cli_path: PathBuf, timeout: Duration) -> Self {
        Self {
            cli_path,
            profile: None,
            timeout,
            stealth: StealthConfig::disabled(),
            limiter: Arc::new(RateLimiter::new(Duration::ZERO)),
        }
    }

    #[cfg(test)]
    fn new_with_profile(cli_path: PathBuf, profile: String, timeout: Duration) -> Self {
        Self {
            cli_path,
            profile: Some(profile),
            timeout,
            stealth: StealthConfig::disabled(),
            limiter: Arc::new(RateLimiter::new(Duration::ZERO)),
        }
    }

    #[cfg(test)]
    fn new_stealthy(cli_path: PathBuf, timeout: Duration) -> Self {
        let stealth = StealthConfig {
            enabled: true,
            jitter_base_ms: 1,
            jitter_spread_ms: 1,
            min_interval_ms: 1,
            default_cache_ttl_secs: 60,
        };
        Self {
            cli_path,
            profile: None,
            timeout,
            stealth,
            limiter: Arc::new(RateLimiter::new(Duration::from_millis(1))),
        }
    }

    async fn run(&self, spec: CommandSpec) -> Result<CommandOutput, RunnerError> {
        // Rate-limit to enforce minimum interval between calls.
        self.limiter.throttle().await;

        // Add deterministic jitter based on args hash to break machine-rhythmic patterns.
        if self.stealth.enabled {
            let spread = self.stealth.jitter_spread_ms;
            if spread > 0 {
                let hash = spec.args.iter().fold(0u64, |acc, s| {
                    acc.wrapping_add(s.bytes().map(|b| b as u64).sum::<u64>())
                });
                let jitter_ms = self.stealth.jitter_base_ms + (hash % spread);
                tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
            }
        }

        let path_display = self.cli_path.display().to_string();
        let mut command = Command::new(&self.cli_path);
        // Per-call profile takes precedence, then runner default.
        let effective_profile = spec.profile_override.as_deref().or(self.profile.as_deref());
        if let Some(profile) = effective_profile {
            command.args(["--profile", profile]);
        }
        command
            .args(&spec.args)
            .stdin(if spec.stdin.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let mut spawn_attempts = 0;
        let mut child = loop {
            match command.spawn() {
                Ok(child) => break child,
                Err(error) if error.raw_os_error() == Some(26) && spawn_attempts < 5 => {
                    spawn_attempts += 1;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => {
                    return Err(RunnerError::Spawn {
                        path: path_display.clone(),
                        message: error.to_string(),
                    });
                }
            }
        };

        if let Some(stdin) = spec.stdin {
            let mut child_stdin = child.stdin.take().ok_or_else(|| RunnerError::Spawn {
                path: path_display.clone(),
                message: "failed to open subprocess stdin".to_string(),
            })?;
            child_stdin
                .write_all(stdin.as_bytes())
                .await
                .map_err(|error| RunnerError::Spawn {
                    path: path_display.clone(),
                    message: error.to_string(),
                })?;
        }

        let output = timeout(self.timeout, child.wait_with_output())
            .await
            .map_err(|_| RunnerError::Timeout {
                path: path_display.clone(),
                timeout_ms: self.timeout.as_millis() as u64,
            })?
            .map_err(|error| RunnerError::Spawn {
                path: path_display.clone(),
                message: error.to_string(),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        let stderr = String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_string();

        if !output.status.success() {
            let message = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                match output.status.code() {
                    Some(code) => format!("`{path_display}` exited with status {code}"),
                    None => format!("`{path_display}` terminated by signal"),
                }
            };
            return Err(RunnerError::CommandFailed { message });
        }

        match spec.output_mode {
            OutputMode::Json => {
                let value = serde_json::from_str(&stdout)
                    .map_err(|error| RunnerError::Parse(error.to_string()))?;
                Ok(CommandOutput::Json(value))
            }
            OutputMode::JsonToToon => {
                let value: Value = serde_json::from_str(&stdout)
                    .map_err(|error| RunnerError::Parse(error.to_string()))?;
                Ok(CommandOutput::Toon(toon::encode(&value, None)))
            }
            OutputMode::Toon => Ok(CommandOutput::Toon(stdout)),
            OutputMode::Text => Ok(CommandOutput::Text(stdout)),
        }
    }
}

impl KagiServer {
    fn new(runner: CliRunner) -> Self {
        Self {
            runner,
            tool_router: Self::tool_router(),
        }
    }

    async fn execute(&self, spec: CommandSpec) -> CallToolResult {
        match self.runner.run(spec).await {
            Ok(CommandOutput::Json(value)) => json_tool_result(value),
            Ok(CommandOutput::Toon(text)) => CallToolResult::success(vec![Content::text(text)]),
            Ok(CommandOutput::Text(text)) => CallToolResult::success(vec![Content::text(text)]),
            Err(error) => CallToolResult::error(vec![Content::text(error.to_string())]),
        }
    }
}

#[tool_router]
impl KagiServer {
    #[tool(
        description = "Search Kagi and return TOON by default. Pass format=json for structured JSON."
    )]
    async fn kagi_search(
        &self,
        Parameters(args): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(search(args)).await)
    }

    #[tool(description = "Summarize a URL or text using the same options as `kagi summarize`.")]
    async fn kagi_summarize(
        &self,
        Parameters(args): Parameters<SummarizeArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(summarize(args)).await)
    }

    #[tool(description = "Extract a page's full content as markdown through Kagi Extract.")]
    async fn kagi_extract(
        &self,
        Parameters(args): Parameters<ExtractArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(extract(args)).await)
    }

    #[tool(description = "Fetch Kagi News stories as TOON.")]
    async fn kagi_news(
        &self,
        Parameters(args): Parameters<NewsArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(news(args)).await)
    }

    #[tool(description = "List Kagi News categories as TOON.")]
    async fn kagi_news_categories(
        &self,
        Parameters(args): Parameters<LangArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(news_categories(args)).await)
    }

    #[tool(description = "Fetch the Kagi News chaos index as TOON.")]
    async fn kagi_news_chaos(
        &self,
        Parameters(args): Parameters<LangArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(news_chaos(args)).await)
    }

    #[tool(
        description = "Prompt Kagi Assistant and return TOON by default. Pass format=json for structured JSON."
    )]
    async fn kagi_assistant(
        &self,
        Parameters(args): Parameters<AssistantArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant(args)).await)
    }

    #[tool(description = "Run a bounded Kagi Assistant REPL by feeding prompts, then `/exit`.")]
    async fn kagi_assistant_repl(
        &self,
        Parameters(args): Parameters<AssistantReplArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_repl(args)).await)
    }

    #[tool(description = "Prompt Kagi FastGPT as TOON.")]
    async fn kagi_fastgpt(
        &self,
        Parameters(args): Parameters<FastGptArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(fastgpt(args)).await)
    }

    #[tool(description = "Query Kagi web enrichment.")]
    async fn kagi_enrich_web(
        &self,
        Parameters(args): Parameters<EnrichArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(enrich_web(args)).await)
    }

    #[tool(description = "Query Kagi news enrichment.")]
    async fn kagi_enrich_news(
        &self,
        Parameters(args): Parameters<EnrichArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(enrich_news(args)).await)
    }

    #[tool(description = "Fetch the Kagi Small Web feed.")]
    async fn kagi_smallweb(
        &self,
        Parameters(args): Parameters<SmallWebArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(smallweb(args)).await)
    }

    #[tool(description = "Show which Kagi credentials are configured for the wrapped CLI.")]
    async fn kagi_auth_status(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(auth_status()).await)
    }

    #[tool(description = "Run `kagi auth check` through the wrapped CLI.")]
    async fn kagi_auth_check(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(auth_check()).await)
    }

    #[tool(description = "Save API and/or session credentials with `kagi auth set`.")]
    async fn kagi_auth_set(
        &self,
        Parameters(args): Parameters<AuthSetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(auth_set(args)).await)
    }

    #[tool(description = "Get a quick answer with references from Kagi.")]
    async fn kagi_quick(
        &self,
        Parameters(args): Parameters<QuickArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(quick(args)).await)
    }

    #[tool(description = "Translate text through Kagi Translate.")]
    async fn kagi_translate(
        &self,
        Parameters(args): Parameters<TranslateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(translate(args)).await)
    }

    #[tool(description = "Run multiple searches in parallel with rate limiting.")]
    async fn kagi_batch(
        &self,
        Parameters(args): Parameters<BatchArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(batch(args)).await)
    }

    #[tool(description = "Ask Kagi Assistant about a specific web page.")]
    async fn kagi_ask_page(
        &self,
        Parameters(args): Parameters<AskPageArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(ask_page(args)).await)
    }

    #[tool(description = "List Assistant conversation threads.")]
    async fn kagi_assistant_thread_list(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_thread_list()).await)
    }

    #[tool(description = "Get an Assistant thread by ID.")]
    async fn kagi_assistant_thread_get(
        &self,
        Parameters(args): Parameters<ThreadIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_thread_get(args)).await)
    }

    #[tool(description = "Export an Assistant thread to markdown or JSON.")]
    async fn kagi_assistant_thread_export(
        &self,
        Parameters(args): Parameters<ThreadExportArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_thread_export(args)).await)
    }

    #[tool(description = "Delete an Assistant thread.")]
    async fn kagi_assistant_thread_delete(
        &self,
        Parameters(args): Parameters<ThreadIdArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_thread_delete(args)).await)
    }

    #[tool(description = "List local kagi-cli command history entries.")]
    async fn kagi_history_list(
        &self,
        Parameters(args): Parameters<HistoryListArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(history_list(args)).await)
    }

    #[tool(description = "Return local kagi-cli history statistics.")]
    async fn kagi_history_stats(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(history_stats()).await)
    }

    #[tool(description = "List local kagi-cli site preferences.")]
    async fn kagi_site_pref_list(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(site_pref_list()).await)
    }

    #[tool(description = "Set a local kagi-cli domain preference.")]
    async fn kagi_site_pref_set(
        &self,
        Parameters(args): Parameters<SitePrefSetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(site_pref_set(args)).await)
    }

    #[tool(description = "Remove a local kagi-cli domain preference.")]
    async fn kagi_site_pref_remove(
        &self,
        Parameters(args): Parameters<SitePrefDomainArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(site_pref_remove(args)).await)
    }

    #[tool(description = "List available Kagi lenses.")]
    async fn kagi_lens_list(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(lens_list()).await)
    }

    #[tool(description = "Get one Kagi lens by id or exact name.")]
    async fn kagi_lens_get(
        &self,
        Parameters(args): Parameters<LensTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(lens_get(args)).await)
    }

    #[tool(description = "Create a Kagi lens.")]
    async fn kagi_lens_create(
        &self,
        Parameters(args): Parameters<LensCreateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(lens_create(args)).await)
    }

    #[tool(description = "Update a Kagi lens by id or exact name.")]
    async fn kagi_lens_update(
        &self,
        Parameters(args): Parameters<LensUpdateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(lens_update(args)).await)
    }

    #[tool(description = "Delete a Kagi lens by id or exact name.")]
    async fn kagi_lens_delete(
        &self,
        Parameters(args): Parameters<LensTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(lens_delete(args)).await)
    }

    #[tool(description = "Enable a Kagi lens by id or exact name.")]
    async fn kagi_lens_enable(
        &self,
        Parameters(args): Parameters<LensTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(lens_enable(args)).await)
    }

    #[tool(description = "Disable a Kagi lens by id or exact name.")]
    async fn kagi_lens_disable(
        &self,
        Parameters(args): Parameters<LensTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(lens_disable(args)).await)
    }

    #[tool(description = "List custom and built-in assistants visible to the account.")]
    async fn kagi_assistant_custom_list(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_custom_list()).await)
    }

    #[tool(description = "Get a custom assistant by id or exact name.")]
    async fn kagi_assistant_custom_get(
        &self,
        Parameters(args): Parameters<AssistantCustomTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_custom_get(args)).await)
    }

    #[tool(description = "Create a custom assistant.")]
    async fn kagi_assistant_custom_create(
        &self,
        Parameters(args): Parameters<AssistantCustomCreateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_custom_create(args)).await)
    }

    #[tool(description = "Update a custom assistant by id or exact name.")]
    async fn kagi_assistant_custom_update(
        &self,
        Parameters(args): Parameters<AssistantCustomUpdateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_custom_update(args)).await)
    }

    #[tool(description = "Delete a custom assistant by id or exact name.")]
    async fn kagi_assistant_custom_delete(
        &self,
        Parameters(args): Parameters<AssistantCustomTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant_custom_delete(args)).await)
    }

    #[tool(description = "List custom bangs.")]
    async fn kagi_bang_custom_list(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(bang_custom_list()).await)
    }

    #[tool(description = "Get a custom bang by id, exact name, or trigger.")]
    async fn kagi_bang_custom_get(
        &self,
        Parameters(args): Parameters<CustomBangTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(bang_custom_get(args)).await)
    }

    #[tool(description = "Create a custom bang.")]
    async fn kagi_bang_custom_create(
        &self,
        Parameters(args): Parameters<CustomBangCreateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(bang_custom_create(args)).await)
    }

    #[tool(description = "Update a custom bang by id, exact name, or trigger.")]
    async fn kagi_bang_custom_update(
        &self,
        Parameters(args): Parameters<CustomBangUpdateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(bang_custom_update(args)).await)
    }

    #[tool(description = "Delete a custom bang by id, exact name, or trigger.")]
    async fn kagi_bang_custom_delete(
        &self,
        Parameters(args): Parameters<CustomBangTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(bang_custom_delete(args)).await)
    }

    #[tool(description = "List redirect rules.")]
    async fn kagi_redirect_list(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(redirect_list()).await)
    }

    #[tool(description = "Get a redirect rule by id or exact rule text.")]
    async fn kagi_redirect_get(
        &self,
        Parameters(args): Parameters<RedirectTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(redirect_get(args)).await)
    }

    #[tool(description = "Create a redirect rule.")]
    async fn kagi_redirect_create(
        &self,
        Parameters(args): Parameters<RedirectCreateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(redirect_create(args)).await)
    }

    #[tool(description = "Update a redirect rule by id or exact rule text.")]
    async fn kagi_redirect_update(
        &self,
        Parameters(args): Parameters<RedirectUpdateArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(redirect_update(args)).await)
    }

    #[tool(description = "Delete a redirect rule by id or exact rule text.")]
    async fn kagi_redirect_delete(
        &self,
        Parameters(args): Parameters<RedirectTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(redirect_delete(args)).await)
    }

    #[tool(description = "Enable a redirect rule by id or exact rule text.")]
    async fn kagi_redirect_enable(
        &self,
        Parameters(args): Parameters<RedirectTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(redirect_enable(args)).await)
    }

    #[tool(description = "Disable a redirect rule by id or exact rule text.")]
    async fn kagi_redirect_disable(
        &self,
        Parameters(args): Parameters<RedirectTargetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(redirect_disable(args)).await)
    }

    #[tool(description = "Run `kagi watch`; pass a finite count to keep this request bounded.")]
    async fn kagi_watch(
        &self,
        Parameters(args): Parameters<WatchArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(watch(args)).await)
    }

    #[tool(description = "Run a search or news fetch and post the JSON payload to a webhook.")]
    async fn kagi_notify(
        &self,
        Parameters(args): Parameters<NotifyArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(notify(args)).await)
    }

    #[tool(description = "Generate a shell completion script for the wrapped kagi CLI.")]
    async fn kagi_generate_completion(
        &self,
        Parameters(args): Parameters<CompletionArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(generate_completion(args)).await)
    }

    #[tool(description = "Load the core embedded kagi-cli agent usage guide.")]
    async fn kagi_agent(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(agent()).await)
    }

    #[tool(description = "List embedded kagi-cli skills available from the wrapped CLI.")]
    async fn kagi_skills_list(&self) -> Result<CallToolResult, McpError> {
        Ok(self.execute(skills_list()).await)
    }

    #[tool(description = "Load an embedded, version-matched kagi-cli skill.")]
    async fn kagi_skills_get(
        &self,
        Parameters(args): Parameters<SkillGetArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(skills_get(args)).await)
    }

    #[tool(description = "Print the embedded kagi-cli skill locator.")]
    async fn kagi_skills_path(
        &self,
        Parameters(args): Parameters<SkillPathArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(skills_path(args)).await)
    }
}

#[tool_handler]
impl ServerHandler for KagiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(
                "This server wraps the external `kagi` CLI from kagi-cli. Structured tool results \
                 default to TOON text for token-efficient model context; pass format=json on \
                 supported tools when structured MCP content is required. Pass Kagi credentials \
                 through environment variables."
                    .to_string(),
            )
    }
}

fn search(mut args: SearchArgs) -> CommandSpec {
    let output_mode = if args.template.is_some() {
        OutputMode::Text
    } else {
        output_mode_for_format(args.format.as_deref())
    };
    let stealth_active = args.stealth_mode == Some(StealthMode::On);
    let is_news = args.news == Some(true);
    apply_privacy_mode(
        args.privacy_mode,
        &mut args.region,
        args.personalized,
        &mut args.no_personalized,
        !is_news,
    );
    // Stealth mode auto-enables local caching with a generous TTL.
    if stealth_active {
        args.local_cache.get_or_insert(true);
        args.cache_ttl.get_or_insert(DEFAULT_STEALTH_CACHE_TTL_SECS);
    }
    let mut argv = vec!["search".to_string(), args.query];
    push_opt_value(&mut argv, "--snap", args.snap);
    push_opt_value(&mut argv, "--lens", args.lens);
    push_opt_value(&mut argv, "--region", args.region);
    push_opt_value(&mut argv, "--time", args.time);
    push_opt_value(&mut argv, "--from-date", args.from_date);
    push_opt_value(&mut argv, "--to-date", args.to_date);
    push_opt_value(&mut argv, "--order", args.order);
    push_opt_flag(&mut argv, "--verbatim", args.verbatim);
    push_opt_flag(&mut argv, "--personalized", args.personalized);
    push_opt_flag(&mut argv, "--no-personalized", args.no_personalized);
    push_opt_value(&mut argv, "--template", args.template.clone());
    push_opt_u64(&mut argv, "--follow", args.follow);
    push_opt_u64(&mut argv, "--limit", args.limit);
    push_opt_flag(&mut argv, "--news", args.news);
    push_opt_flag(&mut argv, "--local-cache", args.local_cache);
    push_opt_u64(&mut argv, "--cache-ttl", args.cache_ttl);
    push_cli_format(&mut argv, args.format, output_mode);
    push_opt_flag(&mut argv, "--no-color", args.no_color);
    let mut spec = command_spec(argv, output_mode);
    spec.profile_override = args.profile;
    spec
}

fn summarize(args: SummarizeArgs) -> CommandSpec {
    let mut argv = vec!["summarize".to_string()];
    push_opt_value(&mut argv, "--url", args.url);
    push_opt_value(&mut argv, "--text", args.text);
    if args.subscriber.unwrap_or(false) {
        argv.push("--subscriber".to_string());
    }
    push_opt_value(&mut argv, "--length", args.length);
    push_opt_value(&mut argv, "--engine", args.engine);
    push_opt_value(&mut argv, "--summary-type", args.summary_type);
    push_opt_value(&mut argv, "--target-language", args.target_language);
    push_opt_bool(&mut argv, "--cache", args.cache);
    push_opt_flag(&mut argv, "--local-cache", args.local_cache);
    push_opt_u64(&mut argv, "--cache-ttl", args.cache_ttl);
    let stdin = if args.filter_items.is_empty() {
        None
    } else {
        argv.push("--filter".to_string());
        Some(format!("{}\n", args.filter_items.join("\n")))
    };

    CommandSpec {
        args: argv,
        stdin,
        output_mode: OutputMode::JsonToToon,
        profile_override: None,
    }
}

fn extract(args: ExtractArgs) -> CommandSpec {
    command_spec(vec!["extract".to_string(), args.url], OutputMode::Text)
}

fn news(args: NewsArgs) -> CommandSpec {
    let mut argv = vec!["news".to_string()];
    push_opt_value(&mut argv, "--category", args.category);
    push_opt_u32(&mut argv, "--limit", args.limit);
    push_opt_value(&mut argv, "--lang", args.lang);
    push_opt_flag(&mut argv, "--list-filter-presets", args.list_filter_presets);
    push_repeated_value(&mut argv, "--filter-preset", args.filter_preset);
    push_repeated_value(&mut argv, "--filter-keyword", args.filter_keyword);
    push_opt_value(&mut argv, "--filter-mode", args.filter_mode);
    push_opt_value(&mut argv, "--filter-scope", args.filter_scope);

    command_spec(argv, OutputMode::JsonToToon)
}

fn news_categories(args: LangArgs) -> CommandSpec {
    let mut argv = vec!["news".to_string(), "--list-categories".to_string()];
    push_opt_value(&mut argv, "--lang", args.lang);

    command_spec(argv, OutputMode::JsonToToon)
}

fn news_chaos(args: LangArgs) -> CommandSpec {
    let mut argv = vec!["news".to_string(), "--chaos".to_string()];
    push_opt_value(&mut argv, "--lang", args.lang);

    command_spec(argv, OutputMode::JsonToToon)
}

fn assistant(args: AssistantArgs) -> CommandSpec {
    let output_mode = output_mode_for_format(args.format.as_deref());
    let mut argv = vec!["assistant".to_string(), args.query];
    push_opt_value(&mut argv, "--thread-id", args.thread_id);
    push_repeated_value(&mut argv, "--attach", args.attach);
    push_opt_value(&mut argv, "--assistant", args.assistant);
    push_cli_format(&mut argv, args.format, output_mode);
    push_opt_flag(&mut argv, "--no-color", args.no_color);
    push_opt_value(&mut argv, "--model", args.model);
    push_opt_u64(&mut argv, "--lens", args.lens);
    push_opt_flag(&mut argv, "--web-access", args.web_access);
    push_opt_flag(&mut argv, "--no-web-access", args.no_web_access);
    push_opt_flag(&mut argv, "--personalized", args.personalized);
    push_opt_flag(&mut argv, "--no-personalized", args.no_personalized);
    push_opt_value(&mut argv, "--export", args.export);

    command_spec(argv, output_mode)
}

fn assistant_repl(args: AssistantReplArgs) -> CommandSpec {
    let mut argv = vec!["assistant".to_string(), "repl".to_string()];
    push_opt_value(&mut argv, "--thread-id", args.thread_id);
    push_opt_value(&mut argv, "--assistant", args.assistant);
    push_opt_value(&mut argv, "--model", args.model);
    push_opt_value(&mut argv, "--format", args.format);
    push_opt_value(&mut argv, "--export", args.export);
    push_opt_flag(&mut argv, "--no-color", args.no_color);

    let mut stdin = args.prompts.join("\n");
    if !stdin.is_empty() {
        stdin.push('\n');
    }
    stdin.push_str("/exit\n");

    CommandSpec {
        args: argv,
        stdin: Some(stdin),
        output_mode: OutputMode::Text,
        profile_override: None,
    }
}

fn fastgpt(args: FastGptArgs) -> CommandSpec {
    let mut argv = vec!["fastgpt".to_string(), args.query];
    push_opt_bool(&mut argv, "--cache", args.cache);
    push_opt_bool(&mut argv, "--web-search", args.web_search);
    push_opt_flag(&mut argv, "--local-cache", args.local_cache);
    push_opt_u64(&mut argv, "--cache-ttl", args.cache_ttl);

    command_spec(argv, OutputMode::JsonToToon)
}

fn enrich_web(args: EnrichArgs) -> CommandSpec {
    command_spec(
        vec!["enrich".to_string(), "web".to_string(), args.query],
        OutputMode::JsonToToon,
    )
}

fn enrich_news(args: EnrichArgs) -> CommandSpec {
    command_spec(
        vec!["enrich".to_string(), "news".to_string(), args.query],
        OutputMode::JsonToToon,
    )
}

fn smallweb(args: SmallWebArgs) -> CommandSpec {
    let mut argv = vec!["smallweb".to_string()];
    push_opt_u32(&mut argv, "--limit", args.limit);

    command_spec(argv, OutputMode::JsonToToon)
}

fn auth_status() -> CommandSpec {
    command_spec(
        vec!["auth".to_string(), "status".to_string()],
        OutputMode::Text,
    )
}

fn auth_check() -> CommandSpec {
    command_spec(
        vec!["auth".to_string(), "check".to_string()],
        OutputMode::Text,
    )
}

fn quick(args: QuickArgs) -> CommandSpec {
    let output_mode = output_mode_for_format(args.format.as_deref());
    let mut argv = vec!["quick".to_string(), args.query];
    push_cli_format(&mut argv, args.format, output_mode);
    push_opt_flag(&mut argv, "--no-color", args.no_color);
    push_opt_value(&mut argv, "--lens", args.lens);
    push_opt_flag(&mut argv, "--local-cache", args.local_cache);
    push_opt_u64(&mut argv, "--cache-ttl", args.cache_ttl);
    command_spec(argv, output_mode)
}

fn translate(args: TranslateArgs) -> CommandSpec {
    let mut argv = vec!["translate".to_string(), args.text];
    push_opt_value(&mut argv, "--from", args.from);
    push_opt_value(&mut argv, "--to", args.to);
    push_opt_value(&mut argv, "--quality", args.quality);
    push_opt_value(&mut argv, "--model", args.model);
    push_opt_value(&mut argv, "--prediction", args.prediction);
    push_opt_value(&mut argv, "--predicted-language", args.predicted_language);
    push_opt_value(&mut argv, "--formality", args.formality);
    push_opt_value(&mut argv, "--speaker-gender", args.speaker_gender);
    push_opt_value(&mut argv, "--addressee-gender", args.addressee_gender);
    push_opt_value(&mut argv, "--language-complexity", args.language_complexity);
    push_opt_value(&mut argv, "--translation-style", args.translation_style);
    push_opt_value(&mut argv, "--context", args.context);
    push_opt_value(&mut argv, "--dictionary-language", args.dictionary_language);
    push_opt_value(&mut argv, "--time-format", args.time_format);
    push_opt_bool(
        &mut argv,
        "--use-definition-context",
        args.use_definition_context,
    );
    push_opt_bool(
        &mut argv,
        "--enable-language-features",
        args.enable_language_features,
    );
    push_opt_bool(&mut argv, "--preserve-formatting", args.preserve_formatting);
    push_opt_value(&mut argv, "--context-memory-json", args.context_memory_json);
    if args.no_alternatives.unwrap_or(false) {
        argv.push("--no-alternatives".to_string());
    }
    if args.no_word_insights.unwrap_or(false) {
        argv.push("--no-word-insights".to_string());
    }
    if args.no_suggestions.unwrap_or(false) {
        argv.push("--no-suggestions".to_string());
    }
    if args.no_alignments.unwrap_or(false) {
        argv.push("--no-alignments".to_string());
    }
    command_spec(argv, OutputMode::JsonToToon)
}

fn batch(mut args: BatchArgs) -> CommandSpec {
    let output_mode = if args.template.is_some() {
        OutputMode::Text
    } else {
        output_mode_for_format(args.format.as_deref())
    };
    let stealth_active = args.stealth_mode == Some(StealthMode::On);
    apply_privacy_mode(
        args.privacy_mode,
        &mut args.region,
        args.personalized,
        &mut args.no_personalized,
        true,
    );
    // Stealth mode caps concurrency and rate limiting.
    if stealth_active {
        args.concurrency.get_or_insert(1);
        args.rate_limit.get_or_insert(20);
    }
    let mut argv = vec!["batch".to_string()];
    argv.extend(args.queries);
    push_opt_u64(&mut argv, "--concurrency", args.concurrency);
    push_opt_u32(&mut argv, "--rate-limit", args.rate_limit);
    push_cli_format(&mut argv, args.format, output_mode);
    push_opt_value(&mut argv, "--snap", args.snap);
    push_opt_value(&mut argv, "--lens", args.lens);
    push_opt_value(&mut argv, "--region", args.region);
    push_opt_value(&mut argv, "--time", args.time);
    push_opt_value(&mut argv, "--from-date", args.from_date);
    push_opt_value(&mut argv, "--to-date", args.to_date);
    push_opt_value(&mut argv, "--order", args.order);
    push_opt_flag(&mut argv, "--verbatim", args.verbatim);
    push_opt_flag(&mut argv, "--personalized", args.personalized);
    push_opt_flag(&mut argv, "--no-personalized", args.no_personalized);
    push_opt_value(&mut argv, "--template", args.template.clone());
    push_opt_u64(&mut argv, "--limit", args.limit);
    push_opt_flag(&mut argv, "--no-color", args.no_color);
    let stdin = if args.stdin_queries.is_empty() {
        None
    } else {
        Some(format!("{}\n", args.stdin_queries.join("\n")))
    };
    CommandSpec {
        args: argv,
        stdin,
        output_mode,
        profile_override: args.profile,
    }
}

fn ask_page(args: AskPageArgs) -> CommandSpec {
    command_spec(
        vec!["ask-page".to_string(), args.url, args.question],
        OutputMode::JsonToToon,
    )
}

fn assistant_thread_list() -> CommandSpec {
    command_spec(
        vec![
            "assistant".to_string(),
            "thread".to_string(),
            "list".to_string(),
        ],
        OutputMode::JsonToToon,
    )
}

fn assistant_thread_get(args: ThreadIdArgs) -> CommandSpec {
    command_spec(
        vec![
            "assistant".to_string(),
            "thread".to_string(),
            "get".to_string(),
            args.thread_id,
        ],
        OutputMode::JsonToToon,
    )
}

fn assistant_thread_export(args: ThreadExportArgs) -> CommandSpec {
    let output_mode = match args.format.as_deref() {
        Some("json") => OutputMode::JsonToToon,
        _ => OutputMode::Text,
    };
    let mut argv = vec![
        "assistant".to_string(),
        "thread".to_string(),
        "export".to_string(),
        args.thread_id,
    ];
    push_opt_value(&mut argv, "--format", args.format);
    command_spec(argv, output_mode)
}

fn assistant_thread_delete(args: ThreadIdArgs) -> CommandSpec {
    command_spec(
        vec![
            "assistant".to_string(),
            "thread".to_string(),
            "delete".to_string(),
            args.thread_id,
        ],
        OutputMode::JsonToToon,
    )
}

fn history_list(args: HistoryListArgs) -> CommandSpec {
    let mut argv = vec!["history".to_string(), "list".to_string()];
    push_opt_u64(&mut argv, "--limit", args.limit);
    command_spec(argv, OutputMode::JsonToToon)
}

fn history_stats() -> CommandSpec {
    command_spec(
        vec!["history".to_string(), "stats".to_string()],
        OutputMode::JsonToToon,
    )
}

fn site_pref_list() -> CommandSpec {
    command_spec(
        vec!["site-pref".to_string(), "list".to_string()],
        OutputMode::JsonToToon,
    )
}

fn site_pref_set(args: SitePrefSetArgs) -> CommandSpec {
    command_spec(
        vec![
            "site-pref".to_string(),
            "set".to_string(),
            args.domain,
            "--mode".to_string(),
            args.mode,
        ],
        OutputMode::JsonToToon,
    )
}

fn site_pref_remove(args: SitePrefDomainArgs) -> CommandSpec {
    command_spec(
        vec!["site-pref".to_string(), "remove".to_string(), args.domain],
        OutputMode::JsonToToon,
    )
}

fn auth_set(args: AuthSetArgs) -> CommandSpec {
    let mut argv = vec!["auth".to_string(), "set".to_string()];
    push_opt_value(&mut argv, "--api-token", args.api_token);
    push_opt_value(&mut argv, "--session-token", args.session_token);
    command_spec(argv, OutputMode::Text)
}

fn lens_list() -> CommandSpec {
    command_spec(
        vec!["lens".to_string(), "list".to_string()],
        OutputMode::JsonToToon,
    )
}

fn lens_get(args: LensTargetArgs) -> CommandSpec {
    command_spec(
        vec!["lens".to_string(), "get".to_string(), args.target],
        OutputMode::JsonToToon,
    )
}

fn lens_create(args: LensCreateArgs) -> CommandSpec {
    let mut argv = vec!["lens".to_string(), "create".to_string(), args.name];
    push_lens_options(&mut argv, args.options);
    command_spec(argv, OutputMode::JsonToToon)
}

fn lens_update(args: LensUpdateArgs) -> CommandSpec {
    let mut argv = vec!["lens".to_string(), "update".to_string(), args.target];
    push_opt_value(&mut argv, "--name", args.name);
    push_lens_options(&mut argv, args.options);
    command_spec(argv, OutputMode::JsonToToon)
}

fn lens_delete(args: LensTargetArgs) -> CommandSpec {
    command_spec(
        vec!["lens".to_string(), "delete".to_string(), args.target],
        OutputMode::JsonToToon,
    )
}

fn lens_enable(args: LensTargetArgs) -> CommandSpec {
    command_spec(
        vec!["lens".to_string(), "enable".to_string(), args.target],
        OutputMode::JsonToToon,
    )
}

fn lens_disable(args: LensTargetArgs) -> CommandSpec {
    command_spec(
        vec!["lens".to_string(), "disable".to_string(), args.target],
        OutputMode::JsonToToon,
    )
}

fn assistant_custom_list() -> CommandSpec {
    command_spec(
        vec![
            "assistant".to_string(),
            "custom".to_string(),
            "list".to_string(),
        ],
        OutputMode::JsonToToon,
    )
}

fn assistant_custom_get(args: AssistantCustomTargetArgs) -> CommandSpec {
    command_spec(
        vec![
            "assistant".to_string(),
            "custom".to_string(),
            "get".to_string(),
            args.target,
        ],
        OutputMode::JsonToToon,
    )
}

fn assistant_custom_create(args: AssistantCustomCreateArgs) -> CommandSpec {
    let mut argv = vec![
        "assistant".to_string(),
        "custom".to_string(),
        "create".to_string(),
        args.name,
    ];
    push_assistant_custom_options(&mut argv, args.options);
    command_spec(argv, OutputMode::JsonToToon)
}

fn assistant_custom_update(args: AssistantCustomUpdateArgs) -> CommandSpec {
    let mut argv = vec![
        "assistant".to_string(),
        "custom".to_string(),
        "update".to_string(),
        args.target,
    ];
    push_opt_value(&mut argv, "--name", args.name);
    push_assistant_custom_options(&mut argv, args.options);
    command_spec(argv, OutputMode::JsonToToon)
}

fn assistant_custom_delete(args: AssistantCustomTargetArgs) -> CommandSpec {
    command_spec(
        vec![
            "assistant".to_string(),
            "custom".to_string(),
            "delete".to_string(),
            args.target,
        ],
        OutputMode::JsonToToon,
    )
}

fn bang_custom_list() -> CommandSpec {
    command_spec(
        vec!["bang".to_string(), "custom".to_string(), "list".to_string()],
        OutputMode::JsonToToon,
    )
}

fn bang_custom_get(args: CustomBangTargetArgs) -> CommandSpec {
    command_spec(
        vec![
            "bang".to_string(),
            "custom".to_string(),
            "get".to_string(),
            args.target,
        ],
        OutputMode::JsonToToon,
    )
}

fn bang_custom_create(args: CustomBangCreateArgs) -> CommandSpec {
    let mut argv = vec![
        "bang".to_string(),
        "custom".to_string(),
        "create".to_string(),
        args.name,
        "--trigger".to_string(),
        args.trigger,
    ];
    push_custom_bang_options(&mut argv, args.options);
    command_spec(argv, OutputMode::JsonToToon)
}

fn bang_custom_update(args: CustomBangUpdateArgs) -> CommandSpec {
    let mut argv = vec![
        "bang".to_string(),
        "custom".to_string(),
        "update".to_string(),
        args.target,
    ];
    push_opt_value(&mut argv, "--name", args.name);
    push_opt_value(&mut argv, "--trigger", args.trigger);
    push_custom_bang_options(&mut argv, args.options);
    command_spec(argv, OutputMode::JsonToToon)
}

fn bang_custom_delete(args: CustomBangTargetArgs) -> CommandSpec {
    command_spec(
        vec![
            "bang".to_string(),
            "custom".to_string(),
            "delete".to_string(),
            args.target,
        ],
        OutputMode::JsonToToon,
    )
}

fn redirect_list() -> CommandSpec {
    command_spec(
        vec!["redirect".to_string(), "list".to_string()],
        OutputMode::JsonToToon,
    )
}

fn redirect_get(args: RedirectTargetArgs) -> CommandSpec {
    command_spec(
        vec!["redirect".to_string(), "get".to_string(), args.target],
        OutputMode::JsonToToon,
    )
}

fn redirect_create(args: RedirectCreateArgs) -> CommandSpec {
    command_spec(
        vec!["redirect".to_string(), "create".to_string(), args.rule],
        OutputMode::JsonToToon,
    )
}

fn redirect_update(args: RedirectUpdateArgs) -> CommandSpec {
    command_spec(
        vec![
            "redirect".to_string(),
            "update".to_string(),
            args.target,
            args.rule,
        ],
        OutputMode::JsonToToon,
    )
}

fn redirect_delete(args: RedirectTargetArgs) -> CommandSpec {
    command_spec(
        vec!["redirect".to_string(), "delete".to_string(), args.target],
        OutputMode::JsonToToon,
    )
}

fn redirect_enable(args: RedirectTargetArgs) -> CommandSpec {
    command_spec(
        vec!["redirect".to_string(), "enable".to_string(), args.target],
        OutputMode::JsonToToon,
    )
}

fn redirect_disable(args: RedirectTargetArgs) -> CommandSpec {
    command_spec(
        vec!["redirect".to_string(), "disable".to_string(), args.target],
        OutputMode::JsonToToon,
    )
}

fn watch(args: WatchArgs) -> CommandSpec {
    let mut argv = vec!["watch".to_string(), args.query];
    push_opt_u64(&mut argv, "--interval", args.interval);
    push_opt_u32(&mut argv, "--count", args.count);
    push_opt_value(&mut argv, "--format", args.format);
    command_spec(argv, OutputMode::Text)
}

fn notify(args: NotifyArgs) -> CommandSpec {
    let mut argv = vec!["notify".to_string()];
    push_opt_value(&mut argv, "--query", args.query);
    push_opt_value(&mut argv, "--news-category", args.news_category);
    argv.push("--webhook-url".to_string());
    argv.push(args.webhook_url);
    push_opt_flag(&mut argv, "--change-only", args.change_only);
    command_spec(argv, OutputMode::JsonToToon)
}

fn generate_completion(args: CompletionArgs) -> CommandSpec {
    command_spec(
        vec!["--generate-completion".to_string(), args.shell],
        OutputMode::Text,
    )
}

fn agent() -> CommandSpec {
    command_spec(vec!["agent".to_string()], OutputMode::Text)
}

fn skills_list() -> CommandSpec {
    command_spec(
        vec!["skills".to_string(), "list".to_string()],
        OutputMode::Text,
    )
}

fn skills_get(args: SkillGetArgs) -> CommandSpec {
    let mut argv = vec![
        "skills".to_string(),
        "get".to_string(),
        args.name.unwrap_or_else(|| "kagi".to_string()),
    ];
    push_opt_flag(&mut argv, "--full", args.full);
    command_spec(argv, OutputMode::Text)
}

fn skills_path(args: SkillPathArgs) -> CommandSpec {
    let mut argv = vec!["skills".to_string(), "path".to_string()];
    if let Some(name) = args.name {
        argv.push(name);
    }
    command_spec(argv, OutputMode::Text)
}

fn command_spec(args: Vec<String>, output_mode: OutputMode) -> CommandSpec {
    CommandSpec {
        args,
        stdin: None,
        output_mode,
        profile_override: None,
    }
}

fn apply_privacy_mode(
    privacy_mode: Option<PrivacyMode>,
    region: &mut Option<String>,
    personalized: Option<bool>,
    no_personalized: &mut Option<bool>,
    allow_no_personalized: bool,
) {
    match privacy_mode {
        Some(PrivacyMode::Unpersonalized) => {
            region.get_or_insert_with(|| "no_region".to_string());
            if allow_no_personalized && personalized != Some(true) && no_personalized.is_none() {
                *no_personalized = Some(true);
            }
        }
        None => {}
    }
}

fn push_opt_value(argv: &mut Vec<String>, flag: &str, value: Option<String>) {
    if let Some(value) = value {
        argv.push(flag.to_string());
        argv.push(value);
    }
}

fn push_cli_format(argv: &mut Vec<String>, format: Option<String>, output_mode: OutputMode) {
    let cli_format = match (format, output_mode) {
        (Some(format), _) => Some(format),
        (None, OutputMode::Toon) => Some("toon".to_string()),
        (None, _) => None,
    };
    push_opt_value(argv, "--format", cli_format);
}

fn push_opt_bool(argv: &mut Vec<String>, flag: &str, value: Option<bool>) {
    if let Some(value) = value {
        argv.push(flag.to_string());
        argv.push(value.to_string());
    }
}

fn push_opt_flag(argv: &mut Vec<String>, flag: &str, value: Option<bool>) {
    if value.unwrap_or(false) {
        argv.push(flag.to_string());
    }
}

fn push_opt_u32(argv: &mut Vec<String>, flag: &str, value: Option<u32>) {
    if let Some(value) = value {
        argv.push(flag.to_string());
        argv.push(value.to_string());
    }
}

fn push_opt_u64(argv: &mut Vec<String>, flag: &str, value: Option<u64>) {
    if let Some(value) = value {
        argv.push(flag.to_string());
        argv.push(value.to_string());
    }
}

fn push_repeated_value(argv: &mut Vec<String>, flag: &str, values: Vec<String>) {
    for value in values {
        argv.push(flag.to_string());
        argv.push(value);
    }
}

fn push_lens_options(argv: &mut Vec<String>, options: LensOptions) {
    push_opt_value(argv, "--included-sites", options.included_sites);
    push_opt_value(argv, "--included-keywords", options.included_keywords);
    push_opt_value(argv, "--description", options.description);
    push_opt_value(argv, "--region", options.region);
    push_opt_value(argv, "--before-date", options.before_date);
    push_opt_value(argv, "--after-date", options.after_date);
    push_opt_value(argv, "--excluded-sites", options.excluded_sites);
    push_opt_value(argv, "--excluded-keywords", options.excluded_keywords);
    push_opt_value(argv, "--shortcut", options.shortcut);
    push_opt_flag(
        argv,
        "--autocomplete-keywords",
        options.autocomplete_keywords,
    );
    push_opt_flag(
        argv,
        "--no-autocomplete-keywords",
        options.no_autocomplete_keywords,
    );
    push_opt_value(argv, "--template", options.template);
    push_opt_value(argv, "--file-type", options.file_type);
    push_opt_flag(argv, "--share-with-team", options.share_with_team);
    push_opt_flag(argv, "--no-share-with-team", options.no_share_with_team);
    push_opt_flag(argv, "--share-copy-code", options.share_copy_code);
    push_opt_flag(argv, "--no-share-copy-code", options.no_share_copy_code);
}

fn push_assistant_custom_options(argv: &mut Vec<String>, options: AssistantCustomOptions) {
    push_opt_value(argv, "--bang-trigger", options.bang_trigger);
    push_opt_flag(argv, "--web-access", options.web_access);
    push_opt_flag(argv, "--no-web-access", options.no_web_access);
    push_opt_value(argv, "--lens", options.lens);
    push_opt_flag(argv, "--personalized", options.personalized);
    push_opt_flag(argv, "--no-personalized", options.no_personalized);
    push_opt_value(argv, "--model", options.model);
    push_opt_value(argv, "--instructions", options.instructions);
}

fn push_custom_bang_options(argv: &mut Vec<String>, options: CustomBangOptions) {
    push_opt_value(argv, "--template", options.template);
    push_opt_value(argv, "--snap-domain", options.snap_domain);
    push_opt_value(argv, "--regex-pattern", options.regex_pattern);
    push_opt_flag(argv, "--shortcut-menu", options.shortcut_menu);
    push_opt_flag(argv, "--no-shortcut-menu", options.no_shortcut_menu);
    push_opt_flag(argv, "--open-snap-domain", options.open_snap_domain);
    push_opt_flag(argv, "--no-open-snap-domain", options.no_open_snap_domain);
    push_opt_flag(argv, "--open-base-path", options.open_base_path);
    push_opt_flag(argv, "--no-open-base-path", options.no_open_base_path);
    push_opt_flag(argv, "--encode-placeholder", options.encode_placeholder);
    push_opt_flag(
        argv,
        "--no-encode-placeholder",
        options.no_encode_placeholder,
    );
    push_opt_flag(argv, "--plus-for-space", options.plus_for_space);
    push_opt_flag(argv, "--no-plus-for-space", options.no_plus_for_space);
}

fn output_mode_for_format(format: Option<&str>) -> OutputMode {
    match format {
        None | Some("toon") => OutputMode::Toon,
        Some("json" | "compact") => OutputMode::Json,
        Some("pretty" | "markdown" | "csv") => OutputMode::Text,
        _ => OutputMode::Json,
    }
}

fn json_tool_result(value: Value) -> CallToolResult {
    let pretty = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    let mut result = CallToolResult::structured(value);
    result.content = vec![Content::text(pretty)];
    result
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use tempfile::tempdir;

    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn search_args(query: &str) -> SearchArgs {
        SearchArgs {
            query: query.to_string(),
            snap: None,
            lens: None,
            region: None,
            time: None,
            from_date: None,
            to_date: None,
            order: None,
            verbatim: None,
            personalized: None,
            no_personalized: None,
            privacy_mode: None,
            stealth_mode: None,
            profile: None,
            template: None,
            follow: None,
            limit: None,
            news: None,
            local_cache: None,
            cache_ttl: None,
            format: None,
            no_color: None,
        }
    }

    fn summarize_args() -> SummarizeArgs {
        SummarizeArgs {
            url: None,
            text: None,
            subscriber: None,
            length: None,
            engine: None,
            summary_type: None,
            target_language: None,
            cache: None,
            filter_items: Vec::new(),
            local_cache: None,
            cache_ttl: None,
        }
    }

    fn assistant_args(query: &str) -> AssistantArgs {
        AssistantArgs {
            query: query.to_string(),
            thread_id: None,
            attach: Vec::new(),
            assistant: None,
            format: None,
            model: None,
            lens: None,
            web_access: None,
            no_web_access: None,
            personalized: None,
            no_personalized: None,
            export: None,
            no_color: None,
        }
    }

    fn assistant_repl_args() -> AssistantReplArgs {
        AssistantReplArgs {
            prompts: Vec::new(),
            thread_id: None,
            assistant: None,
            model: None,
            format: None,
            export: None,
            no_color: None,
        }
    }

    fn batch_args() -> BatchArgs {
        BatchArgs {
            queries: Vec::new(),
            stdin_queries: Vec::new(),
            concurrency: None,
            rate_limit: None,
            format: None,
            snap: None,
            lens: None,
            region: None,
            time: None,
            from_date: None,
            to_date: None,
            order: None,
            verbatim: None,
            personalized: None,
            no_personalized: None,
            privacy_mode: None,
            stealth_mode: None,
            profile: None,
            template: None,
            limit: None,
            no_color: None,
        }
    }

    fn write_fixture(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("kagi");
        let tmp_path = dir.join("kagi.tmp");
        let mut file = fs::File::create(&tmp_path).expect("fixture script should create");
        file.write_all(body.as_bytes())
            .expect("fixture script should write");
        file.sync_all().expect("fixture script should sync");
        drop(file);
        let mut perms = fs::metadata(&tmp_path)
            .expect("fixture metadata should exist")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp_path, perms).expect("fixture should be executable");
        fs::rename(&tmp_path, &path).expect("fixture should move into place");
        path
    }

    #[test]
    fn builds_search_args() {
        let mut args = search_args("rust");
        args.lens = Some("2".to_string());

        assert_eq!(
            search(args),
            CommandSpec {
                args: strings(&["search", "rust", "--lens", "2", "--format", "toon"]),
                stdin: None,
                output_mode: OutputMode::Toon,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_search_toon_args_as_cli_toon() {
        let mut args = search_args("rust");
        args.format = Some("toon".to_string());

        assert_eq!(
            search(args),
            CommandSpec {
                args: strings(&["search", "rust", "--format", "toon"]),
                stdin: None,
                output_mode: OutputMode::Toon,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_search_args_for_v0_5_options() {
        let mut args = search_args("rust");
        args.snap = Some("reddit".to_string());
        args.from_date = Some("2024-01-01".to_string());
        args.to_date = Some("2024-12-31".to_string());
        args.verbatim = Some(true);
        args.personalized = Some(true);
        args.template = Some("{{title}}".to_string());
        args.follow = Some(3);
        args.limit = Some(5);
        args.news = Some(true);
        args.local_cache = Some(true);
        args.cache_ttl = Some(600);
        args.format = Some("pretty".to_string());
        args.no_color = Some(true);

        assert_eq!(
            search(args),
            CommandSpec {
                args: strings(&[
                    "search",
                    "rust",
                    "--snap",
                    "reddit",
                    "--from-date",
                    "2024-01-01",
                    "--to-date",
                    "2024-12-31",
                    "--verbatim",
                    "--personalized",
                    "--template",
                    "{{title}}",
                    "--follow",
                    "3",
                    "--limit",
                    "5",
                    "--news",
                    "--local-cache",
                    "--cache-ttl",
                    "600",
                    "--format",
                    "pretty",
                    "--no-color",
                ]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_summarize_filter_with_controlled_stdin() {
        let mut args = summarize_args();
        args.filter_items = strings(&["https://example.com/a", "plain text"]);
        args.subscriber = Some(true);
        args.cache_ttl = Some(300);

        assert_eq!(
            summarize(args),
            CommandSpec {
                args: strings(&[
                    "summarize",
                    "--subscriber",
                    "--cache-ttl",
                    "300",
                    "--filter"
                ]),
                stdin: Some("https://example.com/a\nplain text\n".to_string()),
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_batch_with_stdin_queries_and_shared_filters() {
        let mut args = batch_args();
        args.queries = strings(&["rust"]);
        args.stdin_queries = strings(&["zig", "go"]);
        args.snap = Some("reddit".to_string());
        args.region = Some("us".to_string());
        args.verbatim = Some(true);
        args.template = Some("{{url}}".to_string());
        args.limit = Some(10);
        args.no_color = Some(true);

        assert_eq!(
            batch(args),
            CommandSpec {
                args: strings(&[
                    "batch",
                    "rust",
                    "--snap",
                    "reddit",
                    "--region",
                    "us",
                    "--verbatim",
                    "--template",
                    "{{url}}",
                    "--limit",
                    "10",
                    "--no-color",
                ]),
                stdin: Some("zig\ngo\n".to_string()),
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_batch_default_toon_as_cli_toon() {
        let mut args = batch_args();
        args.queries = strings(&["rust", "zig"]);

        assert_eq!(
            batch(args),
            CommandSpec {
                args: strings(&["batch", "rust", "zig", "--format", "toon"]),
                stdin: None,
                output_mode: OutputMode::Toon,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_agent_skill_commands() {
        assert_eq!(
            agent(),
            CommandSpec {
                args: strings(&["agent"]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
        assert_eq!(
            skills_list(),
            CommandSpec {
                args: strings(&["skills", "list"]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
        assert_eq!(
            skills_get(SkillGetArgs {
                name: None,
                full: Some(true),
            }),
            CommandSpec {
                args: strings(&["skills", "get", "kagi", "--full"]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
        assert_eq!(
            skills_path(SkillPathArgs {
                name: Some("kagi".to_string()),
            }),
            CommandSpec {
                args: strings(&["skills", "path", "kagi"]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_assistant_prompt_options() {
        let mut args = assistant_args("explain rust");
        args.attach = strings(&["notes.md", "trace.txt"]);
        args.assistant = Some("researcher".to_string());
        args.format = Some("markdown".to_string());
        args.model = Some("cecil".to_string());
        args.lens = Some(42);
        args.no_web_access = Some(true);
        args.no_personalized = Some(true);
        args.export = Some("assistant.json".to_string());
        args.no_color = Some(true);

        assert_eq!(
            assistant(args),
            CommandSpec {
                args: strings(&[
                    "assistant",
                    "explain rust",
                    "--attach",
                    "notes.md",
                    "--attach",
                    "trace.txt",
                    "--assistant",
                    "researcher",
                    "--format",
                    "markdown",
                    "--no-color",
                    "--model",
                    "cecil",
                    "--lens",
                    "42",
                    "--no-web-access",
                    "--no-personalized",
                    "--export",
                    "assistant.json",
                ]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_assistant_toon_args_as_cli_toon() {
        let mut args = assistant_args("explain rust");
        args.format = Some("toon".to_string());

        assert_eq!(
            assistant(args),
            CommandSpec {
                args: strings(&["assistant", "explain rust", "--format", "toon"]),
                stdin: None,
                output_mode: OutputMode::Toon,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_assistant_repl_with_controlled_stdin() {
        let mut args = assistant_repl_args();
        args.prompts = strings(&["first", "second"]);
        args.thread_id = Some("thread_1".to_string());
        args.assistant = Some("researcher".to_string());
        args.model = Some("cecil".to_string());
        args.format = Some("markdown".to_string());
        args.export = Some("transcript.json".to_string());
        args.no_color = Some(true);

        assert_eq!(
            assistant_repl(args),
            CommandSpec {
                args: strings(&[
                    "assistant",
                    "repl",
                    "--thread-id",
                    "thread_1",
                    "--assistant",
                    "researcher",
                    "--model",
                    "cecil",
                    "--format",
                    "markdown",
                    "--export",
                    "transcript.json",
                    "--no-color",
                ]),
                stdin: Some("first\nsecond\n/exit\n".to_string()),
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_assistant_thread_export_modes() {
        assert_eq!(
            assistant_thread_export(ThreadExportArgs {
                thread_id: "thread_1".to_string(),
                format: None,
            }),
            CommandSpec {
                args: strings(&["assistant", "thread", "export", "thread_1"]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
        assert_eq!(
            assistant_thread_export(ThreadExportArgs {
                thread_id: "thread_1".to_string(),
                format: Some("json".to_string()),
            }),
            CommandSpec {
                args: strings(&[
                    "assistant",
                    "thread",
                    "export",
                    "thread_1",
                    "--format",
                    "json",
                ]),
                stdin: None,
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_local_state_commands() {
        assert_eq!(
            history_list(HistoryListArgs { limit: Some(5) }),
            CommandSpec {
                args: strings(&["history", "list", "--limit", "5"]),
                stdin: None,
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            }
        );
        assert_eq!(
            history_stats(),
            CommandSpec {
                args: strings(&["history", "stats"]),
                stdin: None,
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            }
        );
        assert_eq!(
            site_pref_set(SitePrefSetArgs {
                domain: "example.com".to_string(),
                mode: "higher".to_string(),
            }),
            CommandSpec {
                args: strings(&["site-pref", "set", "example.com", "--mode", "higher"]),
                stdin: None,
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            }
        );
    }

    #[test]
    fn builds_extract_command() {
        assert_eq!(
            extract(ExtractArgs {
                url: "https://example.com".to_string(),
            }),
            CommandSpec {
                args: strings(&["extract", "https://example.com"]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
    }

    #[test]
    fn tool_router_exposes_extract() {
        let tools = KagiServer::tool_router().list_all();
        assert!(
            tools.iter().any(|tool| tool.name == "kagi_extract"),
            "expected kagi_extract in tool router, got {tools:?}"
        );
    }

    #[test]
    fn builds_lens_commands() {
        assert_eq!(
            lens_list(),
            CommandSpec {
                args: strings(&["lens", "list"]),
                stdin: None,
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            }
        );

        let spec = lens_create(LensCreateArgs {
            name: "Rust".to_string(),
            options: LensOptions {
                included_sites: Some("doc.rust-lang.org".to_string()),
                shortcut: Some("rs".to_string()),
                no_share_with_team: Some(true),
                ..LensOptions::default()
            },
        });
        assert_eq!(
            spec.args,
            strings(&[
                "lens",
                "create",
                "Rust",
                "--included-sites",
                "doc.rust-lang.org",
                "--shortcut",
                "rs",
                "--no-share-with-team",
            ])
        );
    }

    #[test]
    fn builds_assistant_custom_commands() {
        let spec = assistant_custom_update(AssistantCustomUpdateArgs {
            target: "Researcher".to_string(),
            name: Some("Research Pro".to_string()),
            options: AssistantCustomOptions {
                bang_trigger: Some("research".to_string()),
                no_web_access: Some(true),
                model: Some("cecil".to_string()),
                instructions: Some("Be concise".to_string()),
                ..AssistantCustomOptions::default()
            },
        });
        assert_eq!(
            spec.args,
            strings(&[
                "assistant",
                "custom",
                "update",
                "Researcher",
                "--name",
                "Research Pro",
                "--bang-trigger",
                "research",
                "--no-web-access",
                "--model",
                "cecil",
                "--instructions",
                "Be concise",
            ])
        );
    }

    #[test]
    fn builds_custom_bang_commands() {
        let spec = bang_custom_create(CustomBangCreateArgs {
            name: "Rust docs".to_string(),
            trigger: "rs".to_string(),
            options: CustomBangOptions {
                template: Some("https://doc.rust-lang.org/std/?search=%s".to_string()),
                shortcut_menu: Some(true),
                plus_for_space: Some(true),
                ..CustomBangOptions::default()
            },
        });
        assert_eq!(
            spec.args,
            strings(&[
                "bang",
                "custom",
                "create",
                "Rust docs",
                "--trigger",
                "rs",
                "--template",
                "https://doc.rust-lang.org/std/?search=%s",
                "--shortcut-menu",
                "--plus-for-space",
            ])
        );
    }

    #[test]
    fn builds_redirect_watch_notify_and_auth_set_commands() {
        assert_eq!(
            redirect_update(RedirectUpdateArgs {
                target: "old".to_string(),
                rule: "old|new".to_string(),
            }),
            CommandSpec {
                args: strings(&["redirect", "update", "old", "old|new"]),
                stdin: None,
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            }
        );
        assert_eq!(
            watch(WatchArgs {
                query: "rust".to_string(),
                interval: Some(5),
                count: Some(2),
                format: Some("json".to_string()),
            }),
            CommandSpec {
                args: strings(&[
                    "watch",
                    "rust",
                    "--interval",
                    "5",
                    "--count",
                    "2",
                    "--format",
                    "json",
                ]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
        assert_eq!(
            notify(NotifyArgs {
                query: Some("rust".to_string()),
                news_category: None,
                webhook_url: "https://hooks.example".to_string(),
                change_only: Some(true),
            }),
            CommandSpec {
                args: strings(&[
                    "notify",
                    "--query",
                    "rust",
                    "--webhook-url",
                    "https://hooks.example",
                    "--change-only",
                ]),
                stdin: None,
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            }
        );
        assert_eq!(
            auth_set(AuthSetArgs {
                api_token: Some("api".to_string()),
                session_token: Some("session".to_string()),
            }),
            CommandSpec {
                args: strings(&[
                    "auth",
                    "set",
                    "--api-token",
                    "api",
                    "--session-token",
                    "session",
                ]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
        assert_eq!(
            generate_completion(CompletionArgs {
                shell: "bash".to_string(),
            }),
            CommandSpec {
                args: strings(&["--generate-completion", "bash"]),
                stdin: None,
                output_mode: OutputMode::Text,
                profile_override: None,
            }
        );
    }

    #[tokio::test]
    async fn parses_json_output() {
        let dir = tempdir().expect("tempdir");
        let script = write_fixture(
            dir.path(),
            "#!/usr/bin/env bash\nprintf '{\"ok\":true,\"args\":[\"%s\",\"%s\"]}\\n' \"$1\" \"$2\"\n",
        );
        let runner = CliRunner::new(script, Duration::from_millis(500));

        let output = runner
            .run(CommandSpec {
                args: vec!["search".to_string(), "rust".to_string()],
                stdin: None,
                output_mode: OutputMode::Json,
                profile_override: None,
            })
            .await
            .expect("json output should parse");

        assert_eq!(
            output,
            CommandOutput::Json(serde_json::json!({
                "ok": true,
                "args": ["search", "rust"]
            }))
        );
    }

    #[tokio::test]
    async fn converts_json_output_to_toon() {
        let dir = tempdir().expect("tempdir");
        let script = write_fixture(
            dir.path(),
            "#!/usr/bin/env bash\nprintf '{\"ok\":true,\"results\":[{\"title\":\"Rust\"}]}\\n'\n",
        );
        let runner = CliRunner::new(script, Duration::from_millis(500));

        let output = runner
            .run(CommandSpec {
                args: strings(&["search", "rust", "--format", "json"]),
                stdin: None,
                output_mode: OutputMode::JsonToToon,
                profile_override: None,
            })
            .await
            .expect("json output should convert to TOON");

        let CommandOutput::Toon(text) = output else {
            panic!("expected TOON output");
        };
        assert!(text.contains("ok: true"));
        assert!(text.contains("Rust"));
    }

    #[tokio::test]
    async fn passes_native_toon_output_through() {
        let dir = tempdir().expect("tempdir");
        let script = write_fixture(
            dir.path(),
            "#!/usr/bin/env bash\nprintf 'data[1]{title}:\\n  Rust\\n'\n",
        );
        let runner = CliRunner::new(script, Duration::from_millis(500));

        let output = runner
            .run(CommandSpec {
                args: strings(&["search", "rust", "--format", "toon"]),
                stdin: None,
                output_mode: OutputMode::Toon,
                profile_override: None,
            })
            .await
            .expect("native CLI TOON should pass through");

        assert_eq!(
            output,
            CommandOutput::Toon("data[1]{title}:\n  Rust".to_string())
        );
    }

    #[tokio::test]
    async fn runner_prepends_configured_profile() {
        let dir = tempdir().expect("tempdir");
        let script = write_fixture(
            dir.path(),
            "#!/usr/bin/env bash\nprintf '{\"args\":[\"%s\",\"%s\",\"%s\",\"%s\"]}\\n' \"$1\" \"$2\" \"$3\" \"$4\"\n",
        );
        let runner =
            CliRunner::new_with_profile(script, "work".to_string(), Duration::from_millis(500));

        let output = runner
            .run(CommandSpec {
                args: strings(&["search", "rust"]),
                stdin: None,
                output_mode: OutputMode::Json,
                profile_override: None,
            })
            .await
            .expect("profile-prefixed json output should parse");

        assert_eq!(
            output,
            CommandOutput::Json(serde_json::json!({
                "args": ["--profile", "work", "search", "rust"]
            }))
        );
    }

    #[tokio::test]
    async fn surfaces_stderr_for_failures() {
        let dir = tempdir().expect("tempdir");
        let script = write_fixture(
            dir.path(),
            "#!/usr/bin/env bash\nprintf 'authentication error: invalid token\\n' >&2\nexit 1\n",
        );
        let runner = CliRunner::new(script, Duration::from_millis(500));

        let error = runner
            .run(CommandSpec {
                args: vec!["search".to_string(), "rust".to_string()],
                stdin: None,
                output_mode: OutputMode::Json,
                profile_override: None,
            })
            .await
            .expect_err("runner should return CLI failure");

        assert!(
            error
                .to_string()
                .contains("authentication error: invalid token")
        );
    }

    #[tokio::test]
    async fn runner_sends_controlled_stdin_only_when_requested() {
        let dir = tempdir().expect("tempdir");
        let script = write_fixture(
            dir.path(),
            r#"#!/usr/bin/env bash
payload="$(cat)"
if [ -z "$payload" ]; then
  printf '{"stdin":null}\n'
else
  payload="${payload//$'\n'/\\n}"
  printf '{"stdin":"%s"}\n' "$payload"
fi
"#,
        );
        let runner = CliRunner::new(script, Duration::from_millis(500));

        let no_stdin = runner
            .run(CommandSpec {
                args: strings(&["search", "rust"]),
                stdin: None,
                output_mode: OutputMode::Json,
                profile_override: None,
            })
            .await
            .expect("null stdin should parse");
        let with_stdin = runner
            .run(CommandSpec {
                args: strings(&["summarize", "--filter"]),
                stdin: Some("one\ntwo\n".to_string()),
                output_mode: OutputMode::Json,
                profile_override: None,
            })
            .await
            .expect("controlled stdin should parse");

        assert_eq!(
            no_stdin,
            CommandOutput::Json(serde_json::json!({ "stdin": null }))
        );
        assert_eq!(
            with_stdin,
            CommandOutput::Json(serde_json::json!({ "stdin": "one\ntwo" }))
        );
    }

    #[test]
    fn wraps_json_with_text_and_structured_content() {
        let value = serde_json::json!({ "data": ["a", "b"] });
        let result = json_tool_result(value.clone());

        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content, Some(value));
        assert_eq!(result.content.len(), 1);
    }

    #[test]
    fn default_format_mode_is_toon() {
        assert_eq!(output_mode_for_format(None), OutputMode::Toon);
        assert_eq!(output_mode_for_format(Some("toon")), OutputMode::Toon);
        assert_eq!(output_mode_for_format(Some("json")), OutputMode::Json);
    }

    #[test]
    fn builds_search_with_privacy_mode() {
        let mut args = search_args("rust");
        args.privacy_mode = Some(PrivacyMode::Unpersonalized);
        let spec = search(args);
        assert!(spec.args.contains(&"--no-personalized".to_string()));
        assert!(spec.args.contains(&"--region".to_string()));
        assert!(spec.args.contains(&"no_region".to_string()));
    }

    #[test]
    fn builds_search_privacy_mode_preserves_explicit_region() {
        let mut args = search_args("rust");
        args.region = Some("us".to_string());
        args.privacy_mode = Some(PrivacyMode::Unpersonalized);
        let spec = search(args);
        assert!(spec.args.contains(&"--region".to_string()));
        assert!(spec.args.contains(&"us".to_string()));
        assert!(!spec.args.contains(&"no_region".to_string()));
    }

    #[test]
    fn builds_search_privacy_mode_avoids_news_conflict() {
        let mut args = search_args("rust");
        args.news = Some(true);
        args.privacy_mode = Some(PrivacyMode::Unpersonalized);
        let spec = search(args);
        assert!(!spec.args.contains(&"--no-personalized".to_string()));
        assert!(spec.args.contains(&"--region".to_string()));
        assert!(spec.args.contains(&"no_region".to_string()));
    }

    #[test]
    fn builds_search_stealth_mode_enables_cache() {
        let mut args = search_args("rust");
        args.stealth_mode = Some(StealthMode::On);
        let spec = search(args);
        assert!(spec.args.contains(&"--local-cache".to_string()));
        assert!(spec.args.contains(&"--cache-ttl".to_string()));
    }

    #[test]
    fn builds_search_with_profile_override() {
        let mut args = search_args("rust");
        args.profile = Some("alt-profile".to_string());
        let spec = search(args);
        assert_eq!(spec.profile_override, Some("alt-profile".to_string()));
    }

    #[test]
    fn builds_batch_with_privacy_mode() {
        let mut args = batch_args();
        args.queries = strings(&["rust"]);
        args.privacy_mode = Some(PrivacyMode::Unpersonalized);
        let spec = batch(args);
        assert!(spec.args.contains(&"--no-personalized".to_string()));
        assert!(spec.args.contains(&"--region".to_string()));
        assert!(spec.args.contains(&"no_region".to_string()));
    }

    #[test]
    fn builds_batch_stealth_mode_caps_concurrency() {
        let mut args = batch_args();
        args.queries = strings(&["rust"]);
        args.stealth_mode = Some(StealthMode::On);
        let spec = batch(args);
        assert!(spec.args.contains(&"--concurrency".to_string()));
        assert!(spec.args.contains(&"1".to_string()));
        assert!(spec.args.contains(&"--rate-limit".to_string()));
        assert!(spec.args.contains(&"20".to_string()));
    }

    #[test]
    fn builds_batch_stealth_mode_preserves_explicit_concurrency() {
        let mut args = batch_args();
        args.queries = strings(&["rust"]);
        args.stealth_mode = Some(StealthMode::On);
        args.concurrency = Some(3);
        let spec = batch(args);
        assert!(spec.args.contains(&"--concurrency".to_string()));
        assert!(spec.args.contains(&"3".to_string()));
    }

    #[test]
    fn builds_batch_with_profile_override() {
        let mut args = batch_args();
        args.queries = strings(&["rust"]);
        args.profile = Some("work".to_string());
        let spec = batch(args);
        assert_eq!(spec.profile_override, Some("work".to_string()));
    }

    #[tokio::test]
    async fn stealth_runner_adds_jitter_delay() {
        let dir = tempdir().expect("tempdir");
        let script = write_fixture(
            dir.path(),
            "#!/usr/bin/env bash\nprintf '{\"ok\":true}\\n'\n",
        );
        let runner = CliRunner::new_stealthy(script, Duration::from_secs(5));
        let start = Instant::now();
        let output = runner
            .run(CommandSpec {
                args: strings(&["search", "rust"]),
                stdin: None,
                output_mode: OutputMode::Json,
                profile_override: None,
            })
            .await
            .expect("should succeed");
        let elapsed = start.elapsed();
        // Jitter is 1ms base + 0-1ms spread, so it should complete fast but with some delay.
        assert!(elapsed.as_millis() < 500);
        assert!(matches!(output, CommandOutput::Json(_)));
    }

    #[test]
    fn apply_privacy_mode_defaults_no_region() {
        let mut region = None;
        let mut no_personalized = None;
        apply_privacy_mode(
            Some(PrivacyMode::Unpersonalized),
            &mut region,
            None,
            &mut no_personalized,
            true,
        );
        assert_eq!(region, Some("no_region".to_string()));
        assert_eq!(no_personalized, Some(true));
    }

    #[test]
    fn apply_privacy_mode_respects_explicit_region() {
        let mut region = Some("us".to_string());
        let mut no_personalized = None;
        apply_privacy_mode(
            Some(PrivacyMode::Unpersonalized),
            &mut region,
            None,
            &mut no_personalized,
            true,
        );
        assert_eq!(region, Some("us".to_string()));
    }

    #[test]
    fn apply_privacy_mode_respects_explicit_personalized() {
        let mut region = None;
        let mut no_personalized = None;
        apply_privacy_mode(
            Some(PrivacyMode::Unpersonalized),
            &mut region,
            Some(true),
            &mut no_personalized,
            true,
        );
        assert_eq!(region, Some("no_region".to_string()));
        // When personalized is explicitly true, don't add no-personalized.
        assert_eq!(no_personalized, None);
    }

    #[test]
    fn apply_privacy_mode_none_is_noop() {
        let mut region = None;
        let mut no_personalized = None;
        apply_privacy_mode(None, &mut region, None, &mut no_personalized, true);
        assert_eq!(region, None);
        assert_eq!(no_personalized, None);
    }
}
