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
use tokio::{process::Command, time::timeout};
use tracing_subscriber::EnvFilter;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const CLI_PATH_ENV: &str = "KAGI_CLI_PATH";
const TIMEOUT_ENV: &str = "KAGI_MCP_TIMEOUT_MS";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputMode {
    Json,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandSpec {
    args: Vec<String>,
    output_mode: OutputMode,
}

#[derive(Debug, Clone)]
struct CliRunner {
    cli_path: PathBuf,
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
    /// Optional Kagi lens index.
    #[serde(default)]
    lens: Option<String>,
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

        Ok(Self { cli_path, timeout })
    }

    #[cfg(test)]
    fn new(cli_path: PathBuf, timeout: Duration) -> Self {
        Self { cli_path, timeout }
    }

    async fn run(&self, spec: CommandSpec) -> Result<CommandOutput, RunnerError> {
        let path_display = self.cli_path.display().to_string();
        let mut command = Command::new(&self.cli_path);
        command
            .args(&spec.args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        let child = command.spawn().map_err(|error| RunnerError::Spawn {
            path: path_display.clone(),
            message: error.to_string(),
        })?;

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
    let mut argv = vec!["search".to_string(), args.query];
    push_opt_value(&mut argv, "--lens", args.lens);
    CommandSpec {
        args: argv,
        output_mode: OutputMode::Json,
    }
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

    CommandSpec {
        args: argv,
        output_mode: OutputMode::Json,
    }
}

fn news(args: NewsArgs) -> CommandSpec {
    let mut argv = vec!["news".to_string()];
    push_opt_value(&mut argv, "--category", args.category);
    push_opt_u32(&mut argv, "--limit", args.limit);
    push_opt_value(&mut argv, "--lang", args.lang);

    CommandSpec {
        args: argv,
        output_mode: OutputMode::Json,
    }
}

fn news_categories(args: LangArgs) -> CommandSpec {
    let mut argv = vec!["news".to_string(), "--list-categories".to_string()];
    push_opt_value(&mut argv, "--lang", args.lang);

    CommandSpec {
        args: argv,
        output_mode: OutputMode::Json,
    }
}

fn news_chaos(args: LangArgs) -> CommandSpec {
    let mut argv = vec!["news".to_string(), "--chaos".to_string()];
    push_opt_value(&mut argv, "--lang", args.lang);

    CommandSpec {
        args: argv,
        output_mode: OutputMode::Json,
    }
}

fn assistant(args: AssistantArgs) -> CommandSpec {
    let mut argv = vec!["assistant".to_string(), args.query];
    push_opt_value(&mut argv, "--thread-id", args.thread_id);

    CommandSpec {
        args: argv,
        output_mode: OutputMode::Json,
    }
}

fn fastgpt(args: FastGptArgs) -> CommandSpec {
    let mut argv = vec!["fastgpt".to_string(), args.query];
    push_opt_bool(&mut argv, "--cache", args.cache);
    push_opt_bool(&mut argv, "--web-search", args.web_search);

    CommandSpec {
        args: argv,
        output_mode: OutputMode::Json,
    }
}

fn enrich_web(args: EnrichArgs) -> CommandSpec {
    CommandSpec {
        args: vec!["enrich".to_string(), "web".to_string(), args.query],
        output_mode: OutputMode::Json,
    }
}

fn enrich_news(args: EnrichArgs) -> CommandSpec {
    CommandSpec {
        args: vec!["enrich".to_string(), "news".to_string(), args.query],
        output_mode: OutputMode::Json,
    }
}

fn smallweb(args: SmallWebArgs) -> CommandSpec {
    let mut argv = vec!["smallweb".to_string()];
    push_opt_u32(&mut argv, "--limit", args.limit);

    CommandSpec {
        args: argv,
        output_mode: OutputMode::Json,
    }
}

fn auth_status() -> CommandSpec {
    CommandSpec {
        args: vec!["auth".to_string(), "status".to_string()],
        output_mode: OutputMode::Text,
    }
}

fn auth_check() -> CommandSpec {
    CommandSpec {
        args: vec!["auth".to_string(), "check".to_string()],
        output_mode: OutputMode::Text,
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

fn push_opt_u32(argv: &mut Vec<String>, flag: &str, value: Option<u32>) {
    if let Some(value) = value {
        argv.push(flag.to_string());
        argv.push(value.to_string());
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
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
    };

    use tempfile::tempdir;

    use super::*;

    fn write_fixture(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("kagi");
        fs::write(&path, body).expect("fixture script should write");
        let mut perms = fs::metadata(&path)
            .expect("fixture metadata should exist")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("fixture should be executable");
        path
    }

    #[test]
    fn builds_search_args() {
        let spec = search(SearchArgs {
            query: "rust".to_string(),
            lens: Some("2".to_string()),
        });

        assert_eq!(
            spec.args,
            vec!["search", "rust", "--lens", "2"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
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

    #[test]
    fn wraps_json_with_text_and_structured_content() {
        let value = serde_json::json!({ "data": ["a", "b"] });
        let result = json_tool_result(value.clone());

        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.structured_content, Some(value));
        assert_eq!(result.content.len(), 1);
    }
}
