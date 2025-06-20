use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tokio::fs;
use toml;

#[derive(Deserialize)]
struct TranscriptRequest {
    text: String,
}

#[derive(Clone, Deserialize)]
struct Config {
    openai_api_url: String,
    openai_model: String,
    webhook_url: String,
    webhook_template: String,
}

struct SharedState {
    buffer: String,
    last_received: Instant,
    last_summary: Instant,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            buffer: String::new(),
            last_received: Instant::now(),
            last_summary: Instant::now(),
        }
    }
}

type StateHandle = Arc<Mutex<SharedState>>;

async fn load_config() -> Config {
    let path = env::var("CONFIG_FILE").unwrap_or_else(|_| "server-rs/config.toml".to_string());
    let text = fs::read_to_string(&path).await.expect("failed to read config file");
    toml::from_str(&text).expect("invalid config")
}

async fn add_transcript(
    State(state): State<StateHandle>,
    Json(req): Json<TranscriptRequest>,
) -> StatusCode {
    let mut s = state.lock().await;
    if !s.buffer.is_empty() {
        s.buffer.push('\n');
    }
    s.buffer.push_str(&req.text);
    s.last_received = Instant::now();
    StatusCode::OK
}

async fn summarize_text(text: &str, key: &str, api_url: &str, model: &str) -> Result<String, String> {
    let client = reqwest::Client::new();
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": format!("Summarize the following text in a few sentences:\n{}", text)}]
    });

    let res = client
        .post(api_url)
        .bearer_auth(key)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        return Err(format!("OpenAI error: {}", res.status()));
    }

    let resp_json: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(resp_json["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string())
}

async fn post_webhook(url: &str, template: &str, summary: &str) {
    let payload = template.replace("{summary}", summary);
    let client = reqwest::Client::new();
    let _ = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await;
}

#[tokio::main]
async fn main() {
    let key = env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");
    let config = load_config().await;

    let state = Arc::new(Mutex::new(SharedState::default()));
    let state_bg = state.clone();
    let key_bg = key.clone();
    let cfg_bg = config.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            let mut s = state_bg.lock().await;
            let now = Instant::now();
            let since_recv = now.duration_since(s.last_received);
            let since_summary = now.duration_since(s.last_summary);

            if !s.buffer.is_empty() && since_summary >= Duration::from_secs(300) {
                let text = std::mem::take(&mut s.buffer);
                s.last_summary = now;
                drop(s);
                if let Ok(summary) = summarize_text(&text, &key_bg, &cfg_bg.openai_api_url, &cfg_bg.openai_model).await {
                    post_webhook(&cfg_bg.webhook_url, &cfg_bg.webhook_template, &summary).await;
                }
                continue;
            }

            if since_recv >= Duration::from_secs(3600) && !s.buffer.is_empty() {
                let text = std::mem::take(&mut s.buffer);
                s.last_summary = now;
                drop(s);
                if let Ok(summary) = summarize_text(&text, &key_bg, &cfg_bg.openai_api_url, &cfg_bg.openai_model).await {
                    post_webhook(&cfg_bg.webhook_url, &cfg_bg.webhook_template, &summary).await;
                }
            }
        }
    });

    let app = Router::new()
        .route("/transcript", post(add_transcript))
        .with_state(state);
    let addr = SocketAddr::from(([127, 0, 0, 1], 8000));
    println!("Listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
