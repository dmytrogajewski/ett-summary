use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    routing::post,
    Router,
};
use chrono::{DateTime, Utc};
use hound;
use serde::Deserialize;
use serde_json::json;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::sync::Mutex;
use tokio_postgres::{Client, NoTls};
use toml;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn print_config_template(provider: &str) {
    let url = match provider {
        "openrouter" => "https://openrouter.ai/api/v1/chat/completions",
        "azure" => "https://YOUR-RESOURCE.openai.azure.com/openai/deployments/YOUR-DEPLOYMENT/chat/completions?api-version=2023-09-15-preview",
        "ollama" => "http://localhost:11434/v1/chat/completions",
        "local" => "http://localhost:8080/v1/chat/completions",
        _ => "https://api.openai.com/v1/chat/completions",
    };
    println!(
        r#"openai_api_url = "{url}"
openai_model = "gpt-3.5-turbo"
webhook_url = "https://example.com/webhook"
webhook_template = '{{"summary":"{{summary}}"}}'
whisper_model_path = "models/ggml-base.en.bin"
database_url = "postgres://user:password@localhost/summary"

[[systems]]
key = "default"
initial_prompt = "Summarize this transcription: {{transcription}}"

update_prompt = "Here is text summary:
{{summary}}
Please update this summary with new information from this transcription:
{{transcription}}"
"#,
        url = url
    );
}

#[derive(Clone, Deserialize)]
struct SystemConfig {
    key: String,
    initial_prompt: String,
    update_prompt: String,
}

#[derive(Clone, Deserialize)]
struct Config {
    openai_api_url: String,
    openai_model: String,
    webhook_url: String,
    webhook_template: String,
    whisper_model_path: String,
    database_url: String,
    systems: Vec<SystemConfig>,
}

struct SharedState {
    ctx: WhisperContext,
}

impl SharedState {
    fn new(model_path: &str) -> Self {
        let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())
            .expect("failed to load model");
        Self { ctx }
    }
}

type StateHandle = Arc<Mutex<SharedState>>;

#[derive(Clone)]
struct AppState {
    shared: StateHandle,
    config: Arc<Config>,
    key: Arc<String>,
    db: Arc<Client>,
}

async fn load_config() -> Config {
    let path = env::var("CONFIG_FILE").unwrap_or_else(|_| "server-rs/config.toml".to_string());
    let text = fs::read_to_string(&path)
        .await
        .expect("failed to read config file");
    toml::from_str(&text).expect("invalid config")
}

async fn transcribe_wav(data: Vec<u8>, state: &mut SharedState) -> Result<String, String> {
    let cursor = std::io::Cursor::new(data);
    let mut reader = hound::WavReader::new(cursor).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.channels != 1 || spec.sample_rate != 16_000 {
        return Err("wav must be mono 16kHz".to_string());
    }
    let samples: Vec<i16> = reader
        .samples::<i16>()
        .map(|s| s.unwrap_or_default())
        .collect();
    let mut float_samples = vec![0.0f32; samples.len()];
    whisper_rs::convert_integer_to_float_audio(&samples, &mut float_samples)
        .map_err(|e| e.to_string())?;

    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(Some("en"));
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);

    let mut wstate = state.ctx.create_state().map_err(|e| e.to_string())?;
    wstate
        .full(params, &float_samples[..])
        .map_err(|e| e.to_string())?;

    let num_segments = wstate.full_n_segments().map_err(|e| e.to_string())?;
    let mut text = String::new();
    for i in 0..num_segments {
        let seg = wstate.full_get_segment_text(i).map_err(|e| e.to_string())?;
        text.push_str(seg.trim());
        text.push(' ');
    }
    Ok(text)
}

async fn summarize_text(
    prompt: String,
    key: &str,
    api_url: &str,
    model: &str,
) -> Result<String, String> {
    let client = reqwest::Client::new();
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": prompt}]
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
    if url.is_empty() {
        return;
    }
    let payload = template.replace("{summary}", summary);
    let client = reqwest::Client::new();
    let _ = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await;
}

async fn upload_audio(State(app): State<AppState>, mut multipart: Multipart) -> StatusCode {
    let state = &app.shared;
    let cfg = &app.config;
    let key = &app.key;
    let db = &app.db;
    let mut data = None;
    let mut sys_key = None;
    while let Some(field) = multipart.next_field().await.unwrap() {
        if let Some(name) = field.name() {
            match name {
                "file" => {
                    data = Some(field.bytes().await.unwrap().to_vec());
                }
                "system_key" => {
                    sys_key = Some(field.text().await.unwrap());
                }
                _ => {}
            }
        }
    }

    let data = match data {
        Some(d) => d,
        None => return StatusCode::BAD_REQUEST,
    };

    let system_key = match sys_key {
        Some(k) => k,
        None => return StatusCode::BAD_REQUEST,
    };

    let mut s = state.lock().await;
    if db
        .execute(
            "UPDATE state SET last_received = NOW() WHERE system_key=$1",
            &[&system_key],
        )
        .await
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR;
    }
    let transcription = match transcribe_wav(data, &mut s).await {
        Ok(t) => t,
        Err(_) => return StatusCode::BAD_REQUEST,
    };

    drop(s);
    let row = match db
        .query_one(
            "SELECT summary FROM state WHERE system_key=$1",
            &[&system_key],
        )
        .await
    {
        Ok(r) => r,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR,
    };
    let current_summary: String = row.get(0);
    let sys_cfg = match cfg.systems.iter().find(|s| s.key == system_key) {
        Some(c) => c,
        None => return StatusCode::BAD_REQUEST,
    };
    let prompt = if current_summary.is_empty() {
        sys_cfg
            .initial_prompt
            .replace("{transcription}", &transcription)
    } else {
        sys_cfg
            .update_prompt
            .replace("{summary}", &current_summary)
            .replace("{transcription}", &transcription)
    };

    match summarize_text(prompt, &key, &cfg.openai_api_url, &cfg.openai_model).await {
        Ok(sum) => {
            let _ = db
                .execute(
                    "UPDATE state SET summary=$1 WHERE system_key=$2",
                    &[&sum, &system_key],
                )
                .await;
            post_webhook(&cfg.webhook_url, &cfg.webhook_template, &sum).await;
            StatusCode::OK
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn flush_task(db: Arc<Client>) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        if let Ok(rows) = db
            .query("SELECT system_key, summary, last_received FROM state", &[])
            .await
        {
            for row in rows {
                let key: String = row.get(0);
                let summary: String = row.get(1);
                let last_received: DateTime<Utc> = row.get(2);
                if !summary.is_empty()
                    && Utc::now().signed_duration_since(last_received)
                        >= chrono::Duration::seconds(3600)
                {
                    let _ = db
                        .execute("UPDATE state SET summary='' WHERE system_key=$1", &[&key])
                        .await;
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.get(1).map(|s| s == "gen-config").unwrap_or(false) {
        let provider = args.get(2).map(|s| s.as_str()).unwrap_or("openai");
        print_config_template(provider);
        return;
    }

    let key = Arc::new(env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set"));
    let config = load_config().await;
    let (client, connection) = tokio_postgres::connect(&config.database_url, NoTls)
        .await
        .expect("db connect failed");
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("db connection error: {}", e);
        }
    });
    let db = Arc::new(client);
    db.execute(
        "CREATE TABLE IF NOT EXISTS state (
            system_key TEXT PRIMARY KEY,
            summary TEXT NOT NULL,
            last_received TIMESTAMPTZ NOT NULL
        )",
        &[],
    )
    .await
    .expect("create table");

    for sys in &config.systems {
        db.execute(
            "INSERT INTO state (system_key, summary, last_received) VALUES ($1, '', NOW()) ON CONFLICT (system_key) DO NOTHING",
            &[&sys.key],
        )
        .await
        .expect("init row");
    }

    let state = Arc::new(Mutex::new(SharedState::new(&config.whisper_model_path)));

    let db_bg = db.clone();
    tokio::spawn(flush_task(db_bg));

    let app_state = AppState {
        shared: state.clone(),
        config: Arc::new(config.clone()),
        key: key.clone(),
        db: db.clone(),
    };

    let app = Router::new()
        .route("/upload", post(upload_audio))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 8000));
    println!("Listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
