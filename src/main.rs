use std::{env, error::Error, path::PathBuf, time::Duration};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Json,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandSpec {
    args: Vec<String>,
    stdin: Option<String>,
    output_mode: OutputMode,
}

#[derive(Debug, Clone)]
struct CliRunner {
    cli_path: PathBuf,
    profile: Option<String>,
    timeout: Duration,
}

#[derive(Debug, Clone, PartialEq)]
enum CommandOutput {
    Json(Value),
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
    /// Render each result with a lightweight template.
    #[serde(default)]
    template: Option<String>,
    /// Summarize the top N result URLs using subscriber summarizer.
    #[serde(default)]
    follow: Option<u32>,
    /// Locally cache this response.
    #[serde(default)]
    local_cache: Option<bool>,
    /// Override local cache TTL in seconds.
    #[serde(default)]
    cache_ttl: Option<u64>,
    /// Optional output format (json, pretty, compact, markdown, csv).
    #[serde(default)]
    format: Option<String>,
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
    /// Output format (json, pretty, compact, markdown).
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
    /// Optional output format (json, pretty, compact, markdown).
    #[serde(default)]
    format: Option<String>,
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
    concurrency: Option<u32>,
    /// Rate limit in requests per minute (default: 60).
    #[serde(default)]
    rate_limit: Option<u32>,
    /// Optional output format (json, pretty, compact, markdown, csv).
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
    /// Render each result with a lightweight template.
    #[serde(default)]
    template: Option<String>,
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
    limit: Option<u32>,
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

        Ok(Self {
            cli_path,
            profile,
            timeout,
        })
    }

    #[cfg(test)]
    fn new(cli_path: PathBuf, timeout: Duration) -> Self {
        Self {
            cli_path,
            profile: None,
            timeout,
        }
    }

    #[cfg(test)]
    fn new_with_profile(cli_path: PathBuf, profile: String, timeout: Duration) -> Self {
        Self {
            cli_path,
            profile: Some(profile),
            timeout,
        }
    }

    async fn run(&self, spec: CommandSpec) -> Result<CommandOutput, RunnerError> {
        let path_display = self.cli_path.display().to_string();
        let mut command = Command::new(&self.cli_path);
        if let Some(profile) = &self.profile {
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

        let mut child = command.spawn().map_err(|error| RunnerError::Spawn {
            path: path_display.clone(),
            message: error.to_string(),
        })?;

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
            Ok(CommandOutput::Text(text)) => CallToolResult::success(vec![Content::text(text)]),
            Err(error) => CallToolResult::error(vec![Content::text(error.to_string())]),
        }
    }
}

#[tool_router]
impl KagiServer {
    #[tool(description = "Search Kagi and return the CLI JSON response.")]
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

    #[tool(description = "Fetch Kagi News stories.")]
    async fn kagi_news(
        &self,
        Parameters(args): Parameters<NewsArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(news(args)).await)
    }

    #[tool(description = "List Kagi News categories.")]
    async fn kagi_news_categories(
        &self,
        Parameters(args): Parameters<LangArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(news_categories(args)).await)
    }

    #[tool(description = "Fetch the Kagi News chaos index.")]
    async fn kagi_news_chaos(
        &self,
        Parameters(args): Parameters<LangArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(news_chaos(args)).await)
    }

    #[tool(description = "Prompt Kagi Assistant.")]
    async fn kagi_assistant(
        &self,
        Parameters(args): Parameters<AssistantArgs>,
    ) -> Result<CallToolResult, McpError> {
        Ok(self.execute(assistant(args)).await)
    }

    #[tool(description = "Prompt Kagi FastGPT.")]
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
}

#[tool_handler]
impl ServerHandler for KagiServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::from_build_env())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(
                "This server wraps the external `kagi` CLI from kagi-cli. Tool results mirror \
                 the CLI output. Pass Kagi credentials through environment variables."
                    .to_string(),
            )
    }
}

fn search(args: SearchArgs) -> CommandSpec {
    let output_mode = if args.template.is_some() {
        OutputMode::Text
    } else {
        output_mode_for_format(args.format.as_deref())
    };
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
    push_opt_u32(&mut argv, "--follow", args.follow);
    push_opt_flag(&mut argv, "--local-cache", args.local_cache);
    push_opt_u64(&mut argv, "--cache-ttl", args.cache_ttl);
    push_opt_value(&mut argv, "--format", args.format);
    command_spec(argv, output_mode)
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
        output_mode: OutputMode::Json,
    }
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

    command_spec(argv, OutputMode::Json)
}

fn news_categories(args: LangArgs) -> CommandSpec {
    let mut argv = vec!["news".to_string(), "--list-categories".to_string()];
    push_opt_value(&mut argv, "--lang", args.lang);

    command_spec(argv, OutputMode::Json)
}

fn news_chaos(args: LangArgs) -> CommandSpec {
    let mut argv = vec!["news".to_string(), "--chaos".to_string()];
    push_opt_value(&mut argv, "--lang", args.lang);

    command_spec(argv, OutputMode::Json)
}

fn assistant(args: AssistantArgs) -> CommandSpec {
    let output_mode = output_mode_for_format(args.format.as_deref());
    let mut argv = vec!["assistant".to_string(), args.query];
    push_opt_value(&mut argv, "--thread-id", args.thread_id);
    push_repeated_value(&mut argv, "--attach", args.attach);
    push_opt_value(&mut argv, "--assistant", args.assistant);
    push_opt_value(&mut argv, "--format", args.format);
    push_opt_value(&mut argv, "--model", args.model);
    push_opt_u64(&mut argv, "--lens", args.lens);
    push_opt_flag(&mut argv, "--web-access", args.web_access);
    push_opt_flag(&mut argv, "--no-web-access", args.no_web_access);
    push_opt_flag(&mut argv, "--personalized", args.personalized);
    push_opt_flag(&mut argv, "--no-personalized", args.no_personalized);

    command_spec(argv, output_mode)
}

fn fastgpt(args: FastGptArgs) -> CommandSpec {
    let mut argv = vec!["fastgpt".to_string(), args.query];
    push_opt_bool(&mut argv, "--cache", args.cache);
    push_opt_bool(&mut argv, "--web-search", args.web_search);
    push_opt_flag(&mut argv, "--local-cache", args.local_cache);
    push_opt_u64(&mut argv, "--cache-ttl", args.cache_ttl);

    command_spec(argv, OutputMode::Json)
}

fn enrich_web(args: EnrichArgs) -> CommandSpec {
    command_spec(
        vec!["enrich".to_string(), "web".to_string(), args.query],
        OutputMode::Json,
    )
}

fn enrich_news(args: EnrichArgs) -> CommandSpec {
    command_spec(
        vec!["enrich".to_string(), "news".to_string(), args.query],
        OutputMode::Json,
    )
}

fn smallweb(args: SmallWebArgs) -> CommandSpec {
    let mut argv = vec!["smallweb".to_string()];
    push_opt_u32(&mut argv, "--limit", args.limit);

    command_spec(argv, OutputMode::Json)
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
    push_opt_value(&mut argv, "--format", args.format);
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
    command_spec(argv, OutputMode::Json)
}

fn batch(args: BatchArgs) -> CommandSpec {
    let output_mode = if args.template.is_some() {
        OutputMode::Text
    } else {
        output_mode_for_format(args.format.as_deref())
    };
    let mut argv = vec!["batch".to_string()];
    argv.extend(args.queries);
    push_opt_u32(&mut argv, "--concurrency", args.concurrency);
    push_opt_u32(&mut argv, "--rate-limit", args.rate_limit);
    push_opt_value(&mut argv, "--format", args.format);
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
    let stdin = if args.stdin_queries.is_empty() {
        None
    } else {
        Some(format!("{}\n", args.stdin_queries.join("\n")))
    };
    CommandSpec {
        args: argv,
        stdin,
        output_mode,
    }
}

fn ask_page(args: AskPageArgs) -> CommandSpec {
    command_spec(
        vec!["ask-page".to_string(), args.url, args.question],
        OutputMode::Json,
    )
}

fn assistant_thread_list() -> CommandSpec {
    command_spec(
        vec![
            "assistant".to_string(),
            "thread".to_string(),
            "list".to_string(),
        ],
        OutputMode::Json,
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
        OutputMode::Json,
    )
}

fn assistant_thread_export(args: ThreadExportArgs) -> CommandSpec {
    let mut argv = vec![
        "assistant".to_string(),
        "thread".to_string(),
        "export".to_string(),
        args.thread_id,
    ];
    push_opt_value(&mut argv, "--format", args.format);
    command_spec(argv, OutputMode::Json)
}

fn assistant_thread_delete(args: ThreadIdArgs) -> CommandSpec {
    command_spec(
        vec![
            "assistant".to_string(),
            "thread".to_string(),
            "delete".to_string(),
            args.thread_id,
        ],
        OutputMode::Json,
    )
}

fn history_list(args: HistoryListArgs) -> CommandSpec {
    let mut argv = vec!["history".to_string(), "list".to_string()];
    push_opt_u32(&mut argv, "--limit", args.limit);
    command_spec(argv, OutputMode::Json)
}

fn history_stats() -> CommandSpec {
    command_spec(
        vec!["history".to_string(), "stats".to_string()],
        OutputMode::Json,
    )
}

fn site_pref_list() -> CommandSpec {
    command_spec(
        vec!["site-pref".to_string(), "list".to_string()],
        OutputMode::Json,
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
        OutputMode::Json,
    )
}

fn site_pref_remove(args: SitePrefDomainArgs) -> CommandSpec {
    command_spec(
        vec!["site-pref".to_string(), "remove".to_string(), args.domain],
        OutputMode::Json,
    )
}

fn command_spec(args: Vec<String>, output_mode: OutputMode) -> CommandSpec {
    CommandSpec {
        args,
        stdin: None,
        output_mode,
    }
}

fn push_opt_value(argv: &mut Vec<String>, flag: &str, value: Option<String>) {
    if let Some(value) = value {
        argv.push(flag.to_string());
        argv.push(value);
    }
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

fn output_mode_for_format(format: Option<&str>) -> OutputMode {
    match format {
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
            template: None,
            follow: None,
            local_cache: None,
            cache_ttl: None,
            format: None,
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
            template: None,
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
                args: strings(&["search", "rust", "--lens", "2"]),
                stdin: None,
                output_mode: OutputMode::Json,
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
        args.local_cache = Some(true);
        args.cache_ttl = Some(600);
        args.format = Some("pretty".to_string());

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
                    "--local-cache",
                    "--cache-ttl",
                    "600",
                    "--format",
                    "pretty",
                ]),
                stdin: None,
                output_mode: OutputMode::Text,
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
                output_mode: OutputMode::Json,
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
                ]),
                stdin: Some("zig\ngo\n".to_string()),
                output_mode: OutputMode::Text,
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
                    "--model",
                    "cecil",
                    "--lens",
                    "42",
                    "--no-web-access",
                    "--no-personalized",
                ]),
                stdin: None,
                output_mode: OutputMode::Text,
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
                output_mode: OutputMode::Json,
            }
        );
        assert_eq!(
            history_stats(),
            CommandSpec {
                args: strings(&["history", "stats"]),
                stdin: None,
                output_mode: OutputMode::Json,
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
                output_mode: OutputMode::Json,
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
            })
            .await
            .expect("null stdin should parse");
        let with_stdin = runner
            .run(CommandSpec {
                args: strings(&["summarize", "--filter"]),
                stdin: Some("one\ntwo\n".to_string()),
                output_mode: OutputMode::Json,
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
}
