//! Multiple model backends for the same JSON contract. Keys stay in env vars.
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_REPLY: u64 = 400_000;
const TIMEOUT: Duration = Duration::from_secs(150);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Codex,
    ClaudeCli,
    GeminiCli,
    Compatible,
    Anthropic,
    Gemini,
}

#[derive(Clone, Debug)]
pub struct Provider {
    kind: Kind,
    name: String,
    model: String,
    url: String,
    key_env: String,
}

pub const NAMES: &str = "codex, claude-cli, gemini-cli, openai, anthropic, gemini, \
openai-compatible, ollama, lmstudio, openrouter, groq, deepseek, mistral, xai, together";

fn preset(name: &str) -> Option<(Kind, &'static str, &'static str, &'static str)> {
    Some(match name {
        "codex" => (Kind::Codex, "gpt-6-luna", "", ""),
        "claude-cli" => (Kind::ClaudeCli, "sonnet", "", ""),
        "gemini-cli" => (Kind::GeminiCli, "gemini-2.5-flash", "", ""),
        "openai" => (
            Kind::Compatible,
            "gpt-6",
            "https://api.openai.com/v1",
            "OPENAI_API_KEY",
        ),
        "anthropic" | "claude" => (
            Kind::Anthropic,
            "claude-sonnet-4-5",
            "https://api.anthropic.com/v1",
            "ANTHROPIC_API_KEY",
        ),
        "gemini" | "google" => (
            Kind::Gemini,
            "gemini-2.5-flash",
            "https://generativelanguage.googleapis.com/v1beta",
            "GEMINI_API_KEY",
        ),
        "ollama" => (
            Kind::Compatible,
            "llama3.2",
            "http://127.0.0.1:11434/v1",
            "",
        ),
        "lmstudio" => (
            Kind::Compatible,
            "local-model",
            "http://127.0.0.1:1234/v1",
            "",
        ),
        "openrouter" => (
            Kind::Compatible,
            "",
            "https://openrouter.ai/api/v1",
            "OPENROUTER_API_KEY",
        ),
        "groq" => (
            Kind::Compatible,
            "",
            "https://api.groq.com/openai/v1",
            "GROQ_API_KEY",
        ),
        "deepseek" => (
            Kind::Compatible,
            "deepseek-chat",
            "https://api.deepseek.com/v1",
            "DEEPSEEK_API_KEY",
        ),
        "mistral" => (
            Kind::Compatible,
            "",
            "https://api.mistral.ai/v1",
            "MISTRAL_API_KEY",
        ),
        "xai" => (Kind::Compatible, "", "https://api.x.ai/v1", "XAI_API_KEY"),
        "together" => (
            Kind::Compatible,
            "",
            "https://api.together.xyz/v1",
            "TOGETHER_API_KEY",
        ),
        "openai-compatible" | "custom" => (Kind::Compatible, "", "", "CODEGYM_API_KEY"),
        _ => return None,
    })
}

impl Provider {
    pub fn new(
        provider: Option<String>,
        model: Option<String>,
        url: Option<String>,
        key_env: Option<String>,
    ) -> Result<Self, String> {
        let name = provider
            .or_else(|| std::env::var("CODEGYM_PROVIDER").ok())
            .unwrap_or_else(|| "codex".into())
            .to_ascii_lowercase();
        let (kind, model_default, url_default, key_default) = preset(&name)
            .ok_or_else(|| format!("Невідомий провайдер {name}. Варіанти: {NAMES}"))?;
        let model = model
            .or_else(|| std::env::var("CODEGYM_MODEL").ok())
            .unwrap_or_else(|| model_default.into());
        if model.trim().is_empty() || model.len() > 160 || model.contains(['\r', '\n']) {
            return Err(format!("Для {name} вкажи --model."));
        }
        let url = url
            .or_else(|| std::env::var("CODEGYM_BASE_URL").ok())
            .unwrap_or_else(|| url_default.into());
        if matches!(kind, Kind::Compatible | Kind::Anthropic | Kind::Gemini) {
            validate_url(&url)?;
        }
        let key_env = key_env
            .or_else(|| std::env::var("CODEGYM_API_KEY_ENV").ok())
            .unwrap_or_else(|| key_default.into());
        if !key_env.is_empty()
            && (key_env.len() > 128
                || !key_env.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                || !key_env
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            return Err("Некоректна назва змінної середовища API-ключа.".into());
        }
        Ok(Self {
            kind,
            name,
            model,
            url,
            key_env,
        })
    }

    pub fn description(&self) -> String {
        let source = if matches!(self.kind, Kind::Codex | Kind::ClaudeCli | Kind::GeminiCli) {
            "CLI / existing login"
        } else {
            "HTTP API"
        };
        let secret = if self.key_env.is_empty() {
            "not required".into()
        } else {
            format!(
                "{} ({})",
                self.key_env,
                if std::env::var(&self.key_env).is_ok_and(|x| !x.is_empty()) {
                    "configured"
                } else {
                    "missing"
                }
            )
        };
        format!(
            "Провайдер: {} | модель: {} | {} | API key: {}",
            self.name, self.model, source, secret
        )
    }

    pub fn generate(&self, prompt: &str, schema: &str) -> Result<String, String> {
        if prompt.len() > 120_000 {
            return Err("Запит до AI надто великий.".into());
        }
        let schema: Value =
            serde_json::from_str(schema).map_err(|e| format!("Некоректна JSON-схема: {e}"))?;
        let response = match self.kind {
            Kind::Codex | Kind::ClaudeCli | Kind::GeminiCli => self.cli(prompt, &schema)?,
            Kind::Compatible | Kind::Anthropic | Kind::Gemini => self.http(prompt, &schema)?,
        };
        normalize_json(&response)
    }

    fn cli(&self, prompt: &str, schema: &Value) -> Result<String, String> {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("codegym-ai-{}-{n}", std::process::id()));
        fs::create_dir(&dir).map_err(|e| e.to_string())?;
        let outcome = self.cli_in_dir(&dir, prompt, schema);
        let _ = fs::remove_dir_all(&dir);
        outcome
    }

    fn cli_in_dir(&self, dir: &Path, prompt: &str, schema: &Value) -> Result<String, String> {
        let stdout_path = dir.join("stdout.txt");
        let stderr_path = dir.join("stderr.txt");
        let bin = match self.kind {
            Kind::Codex => std::env::var("CODEGYM_CODEX_BIN")
                .unwrap_or_else(|_| if cfg!(windows) { "codex.cmd" } else { "codex" }.into()),
            Kind::ClaudeCli => std::env::var("CODEGYM_CLAUDE_BIN").unwrap_or_else(|_| {
                if cfg!(windows) {
                    "claude.cmd"
                } else {
                    "claude"
                }
                .into()
            }),
            Kind::GeminiCli => std::env::var("CODEGYM_GEMINI_BIN").unwrap_or_else(|_| {
                if cfg!(windows) {
                    "gemini.cmd"
                } else {
                    "gemini"
                }
                .into()
            }),
            _ => return Err("Некоректний CLI-провайдер.".into()),
        };
        let mut command = Command::new(bin);
        command.current_dir(dir);
        let stdin_text = if self.kind == Kind::Codex {
            let schema_path = dir.join("schema.json");
            fs::write(&schema_path, schema.to_string()).map_err(|e| e.to_string())?;
            command
                .arg("exec")
                .args([
                    "--ephemeral",
                    "--ignore-user-config",
                    "--ignore-rules",
                    "--skip-git-repo-check",
                    "--sandbox",
                    "read-only",
                    "--color",
                    "never",
                    "--output-schema",
                ])
                .arg(&schema_path)
                .arg("--output-last-message")
                .arg(dir.join("answer.json"));
            for opt in [
                "approval_policy=\"never\"",
                "features.hooks=false",
                "features.shell_tool=false",
                "features.shell_snapshot=false",
                "features.multi_agent=false",
                "features.apps=false",
                "features.remote_plugin=false",
                "features.memories=false",
                "features.goals=false",
                "web_search=\"disabled\"",
                "mcp_servers={}",
                "project_doc_max_bytes=0",
                "model_reasoning_effort=\"low\"",
            ] {
                command.arg("-c").arg(opt);
            }
            command.args(["--model", &self.model, "-"]);
            prompt.to_owned()
        } else {
            let text = format!(
                "Return ONLY a JSON object matching the following schema, without Markdown.\
                 \nSchema:\n{schema}\n\nUser task:\n{prompt}"
            );
            if self.kind == Kind::ClaudeCli {
                command.args([
                    "--print",
                    "--output-format",
                    "json",
                    "--max-turns",
                    "1",
                    "--tools",
                    "",
                    "--model",
                    &self.model,
                ]);
            } else {
                // A disposable cwd prevents project instructions from being picked up.
                // Gemini's plan mode is read-only, with extensions disabled.
                command.args([
                    "--prompt",
                    "",
                    "--output-format",
                    "json",
                    "--approval-mode",
                    "plan",
                    "--extensions",
                    "none",
                    "--model",
                    &self.model,
                ]);
            }
            text
        };
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::from(
                File::create(&stdout_path).map_err(|e| e.to_string())?,
            ))
            .stderr(Stdio::from(
                File::create(&stderr_path).map_err(|e| e.to_string())?,
            ));
        let mut child = command.spawn().map_err(|e| {
            format!(
                "Не вдалося запустити {} CLI ({e}). Перевір встановлення й авторизацію.",
                self.name
            )
        })?;
        if let Some(mut stdin) = child.stdin.take()
            && let Err(e) = stdin.write_all(stdin_text.as_bytes())
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Помилка надсилання запиту в CLI: {e}"));
        }
        let start = Instant::now();
        let exit = loop {
            if let Some(exit) = child.try_wait().map_err(|e| e.to_string())? {
                break exit;
            }
            if start.elapsed() >= TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{} перевищив ліміт {} секунд.",
                    self.name,
                    TIMEOUT.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(70));
        };
        if !exit.success() {
            return Err(format!(
                "{} CLI повернув помилку ({exit}): {}",
                self.name,
                read_limited(&stderr_path, 550).unwrap_or_default()
            ));
        }
        if self.kind == Kind::Codex {
            return read_limited(&dir.join("answer.json"), MAX_REPLY);
        }
        let stdout = read_limited(&stdout_path, MAX_REPLY)?;
        let response: Value = serde_json::from_str(&stdout)
            .map_err(|_| format!("{} CLI повернув некоректний JSON.", self.name))?;
        let path = if self.kind == Kind::ClaudeCli {
            "result"
        } else {
            "response"
        };
        response
            .get(path)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| format!("{} CLI не повернув поле {path}.", self.name))
    }

    fn http(&self, prompt: &str, schema: &Value) -> Result<String, String> {
        let api_key = if self.key_env.is_empty() {
            None
        } else {
            Some(std::env::var(&self.key_env).map_err(|_| {
                format!(
                    "Установи змінну середовища {} для {}.",
                    self.key_env, self.name
                )
            })?)
        };
        let base = self.url.trim_end_matches('/');
        let (endpoint, mut data, header) = match self.kind {
            Kind::Compatible => (
                if base.ends_with("/chat/completions") {
                    base.into()
                } else {
                    format!("{base}/chat/completions")
                },
                json!({"model":self.model,"stream":false,
                "messages":[{"role":"user","content":format!(
                    "Return a JSON object matching this schema:\n{schema}\n\n{prompt}"
                )}]}),
                "Authorization",
            ),
            Kind::Anthropic => (
                if base.ends_with("/messages") {
                    base.into()
                } else {
                    format!("{base}/messages")
                },
                json!({"model":self.model,"max_tokens":6000,
                    "messages":[{"role":"user","content":prompt}],
                    "output_config":{"format":{"type":"json_schema","schema":schema}}}),
                "x-api-key",
            ),
            Kind::Gemini => (
                format!("{base}/models/{}:generateContent", self.model),
                json!({"contents":[{"role":"user","parts":[{"text":prompt}]}],
                    "generationConfig":{"responseMimeType":"application/json",
                        "responseSchema":schema}}),
                "x-goog-api-key",
            ),
            _ => return Err("Некоректний HTTP-провайдер.".into()),
        };
        if self.name == "openai" {
            data["response_format"] = json!({
                "type":"json_schema",
                "json_schema":{
                    "name":"codegym_response",
                    "strict":false,
                    "schema":schema
                }
            });
        }
        // Explicitly prohibit redirects so authorization headers cannot be
        // accidentally forwarded to an unexpected host.
        let client = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout(TIMEOUT)
            .build();
        let mut request = client
            .post(&endpoint)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json");
        if self.kind == Kind::Anthropic {
            request = request.set("anthropic-version", "2023-06-01");
        }
        if let Some(key) = api_key {
            request = if header == "Authorization" {
                request.set(header, &format!("Bearer {key}"))
            } else {
                request.set(header, &key)
            };
        }
        let response = match request.send_json(data) {
            Ok(response) => response,
            Err(ureq::Error::Status(code, _)) => {
                return Err(format!(
                    "{} API: HTTP {code}. Перевір модель, ключ і параметри.",
                    self.name
                ));
            }
            Err(ureq::Error::Transport(err)) => {
                return Err(format!("{} API недоступний: {err}", self.name));
            }
        };
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(MAX_REPLY + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_REPLY {
            return Err("AI-відповідь надто велика.".into());
        }
        let response: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "AI API повернув некоректний JSON.".to_owned())?;
        extract_response(self.kind, &response)
    }
}

fn validate_url(input: &str) -> Result<(), String> {
    if input.len() > 2000 || input.contains(['\r', '\n', '?', '#', '@']) {
        return Err("Некоректний --base-url.".into());
    }
    let url = url::Url::parse(input).map_err(|_| "Некоректна адреса API.".to_owned())?;
    let local = matches!(
        url.host_str().unwrap_or(""),
        "127.0.0.1" | "localhost" | "::1" | "[::1]"
    );
    if url.scheme() != "https" && !(local && url.scheme() == "http") {
        return Err("Віддалені API потребують HTTPS; HTTP дозволено лише для localhost.".into());
    }
    Ok(())
}

fn read_limited(path: &Path, limit: u64) -> Result<String, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > limit {
        return Err("AI CLI повернув надто великий результат.".into());
    }
    fs::read_to_string(path).map_err(|e| e.to_string())
}

fn extract_response(kind: Kind, json: &Value) -> Result<String, String> {
    let text = match kind {
        Kind::Compatible => json
            .pointer("/choices/0/message/content")
            .and_then(Value::as_str),
        Kind::Anthropic => json.pointer("/content/0/text").and_then(Value::as_str),
        Kind::Gemini => json
            .pointer("/candidates/0/content/parts/0/text")
            .and_then(Value::as_str),
        _ => None,
    };
    text.map(str::to_owned)
        .ok_or("Провайдер не повернув текст результату.".into())
}

fn normalize_json(input: &str) -> Result<String, String> {
    let fence = char::from(96).to_string().repeat(3);
    let input = input.trim();
    let clean = if input.starts_with(&fence) {
        input
            .strip_prefix(&format!("{fence}json"))
            .or_else(|| input.strip_prefix(&fence))
            .and_then(|s| s.strip_suffix(&fence))
            .unwrap_or(input)
            .trim()
    } else {
        input
    };
    let value: Value = serde_json::from_str(clean).map_err(|_| {
        "Модель повернула невалідний JSON; обери модель, що вміє повертати JSON.".to_owned()
    })?;
    if !value.is_object() {
        return Err("AI має повернути JSON-об'єкт.".into());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    #[test]
    fn all_presets_have_a_known_backend() {
        for name in [
            "codex",
            "claude-cli",
            "gemini-cli",
            "openai",
            "anthropic",
            "gemini",
            "ollama",
            "lmstudio",
            "openrouter",
            "groq",
            "deepseek",
            "mistral",
            "xai",
            "together",
            "openai-compatible",
        ] {
            assert!(preset(name).is_some());
        }
        assert!(preset("invalid").is_none());
    }
    #[test]
    fn parse_and_normalize_json() {
        let fenced = format!(
            "{}json\n{{\"score\":5}}\n{}",
            char::from(96).to_string().repeat(3),
            char::from(96).to_string().repeat(3)
        );
        assert_eq!(normalize_json(&fenced).unwrap(), "{\"score\":5}");
        assert!(normalize_json("broken").is_err());
        assert!(normalize_json("[]").is_err());
    }
    #[test]
    fn read_api_response_shapes() {
        for (kind, data) in [
            (
                Kind::Compatible,
                json!({"choices":[{"message":{"content":"{\"x\":1}"}}]}),
            ),
            (Kind::Anthropic, json!({"content":[{"text":"{\"x\":1}"}]})),
            (
                Kind::Gemini,
                json!({"candidates":[{"content":{"parts":[{"text":"{\"x\":1}"}]}}]}),
            ),
        ] {
            assert_eq!(extract_response(kind, &data).unwrap(), "{\"x\":1}");
        }
    }
    #[test]
    fn remote_urls_must_be_encrypted() {
        assert!(validate_url("http://example.org/v1").is_err());
        assert!(validate_url("https://example.org/v1").is_ok());
        assert!(validate_url("http://127.0.0.1:11434/v1").is_ok());
        assert!(validate_url("https://user:secret@example.org/v1").is_err());
    }

    #[test]
    fn mock_http_requests_work_for_all_three_api_formats() {
        for kind in [Kind::Compatible, Kind::Anthropic, Kind::Gemini] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let expected_path = match kind {
                Kind::Compatible => "/v1/chat/completions",
                Kind::Anthropic => "/v1/messages",
                Kind::Gemini => "/v1/models/test-model:generateContent",
                _ => unreachable!(),
            };
            let returned = match kind {
                Kind::Compatible => json!({"choices":[{"message":{"content":"{\"answer\":42}"}}]}),
                Kind::Anthropic => json!({"content":[{"type":"text","text":"{\"answer\":42}"}]}),
                Kind::Gemini => {
                    json!({"candidates":[{"content":{"parts":[{"text":"{\"answer\":42}"}]}}]})
                }
                _ => unreachable!(),
            };
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(
                    line.starts_with(&format!("POST {expected_path} ")),
                    "{line}"
                );
                let mut size = 0usize;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        size = value.trim().parse().unwrap();
                    }
                }
                assert!(size > 10 && size < 100_000);
                let mut body = vec![0; size];
                reader.read_exact(&mut body).unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                match kind {
                    Kind::Compatible => {
                        assert_eq!(request["messages"][0]["role"], "user");
                        assert_eq!(request["model"], "test-model");
                    }
                    Kind::Anthropic => {
                        assert_eq!(request["output_config"]["format"]["type"], "json_schema");
                        assert_eq!(request["messages"][0]["role"], "user");
                    }
                    Kind::Gemini => {
                        assert_eq!(
                            request["generationConfig"]["responseMimeType"],
                            "application/json"
                        );
                        assert_eq!(request["contents"][0]["role"], "user");
                    }
                    _ => unreachable!(),
                }
                let json = returned.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{json}",
                    json.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
            });
            let provider = Provider {
                kind,
                name: "mock".into(),
                model: "test-model".into(),
                url: format!("http://{addr}/v1"),
                key_env: String::new(),
            };
            assert_eq!(
                provider
                    .generate("Say answer 42", r#"{"type":"object"}"#)
                    .unwrap(),
                r#"{"answer":42}"#
            );
            server.join().unwrap();
        }
    }
}
