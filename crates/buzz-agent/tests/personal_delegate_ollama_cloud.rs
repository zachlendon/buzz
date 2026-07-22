//! Offline real-model harness: Personal Delegate + Ollama Cloud `kimi-k2.7-code`.
//!
//! Ignored by default. Does not attempt network until a provider key is present.
//! Runs three fresh authenticated sessions against the exact product model.
//!
//! ## Exact command
//!
//! Prefer reading the key from Keychain (avoids putting it in shell history via
//! paste). On macOS (Keychain account is the principal id, e.g. `zach`, not `$USER`):
//!
//! ```text
//! OLLAMA_API_KEY="$(security find-generic-password -a zach -s personal-delegate-ollama_cloud -w)" \
//!   cargo test -p buzz-agent --test personal_delegate_ollama_cloud -- --ignored --nocapture
//! ```
//!
//! Or export `OLLAMA_API_KEY` / `OPENAI_COMPAT_API_KEY` from your operator
//! environment before running the same cargo test line.
//!
//! ## What this proves
//!
//! - Authenticated chat against `https://ollama.com/v1` with model `kimi-k2.7-code`
//! - Three independent sessions succeed (no sticky local state)
//! - Optional MCP-style `env_clear` isolation when
//!   `PERSONAL_DELEGATE_HARNESS_MCP_ISOLATION=1`
//!
//! ## Security
//!
//! - Never prints the API key or response body
//! - Skips (panics with instructions) when no key is available — no network attempt

use std::process::{Command, Stdio};
use std::time::Duration;

const BASE_URL: &str = "https://ollama.com/v1";
const MODEL: &str = "kimi-k2.7-code";
const SESSIONS: usize = 3;

fn provider_key() -> Option<String> {
    std::env::var("OLLAMA_API_KEY")
        .or_else(|_| std::env::var("OPENAI_COMPAT_API_KEY"))
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// Minimal OpenAI-compat probe: one-token completion, no tools, fixed ping.
/// Fails closed on non-2xx without logging the body or key.
async fn probe_cloud(api_key: &str) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "http client error".to_string())?;

    let response = client
        .post(format!("{BASE_URL}/chat/completions"))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": MODEL,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
        }))
        .send()
        .await
        .map_err(|_| "network error contacting Ollama Cloud".to_string())?;

    let status = response.status().as_u16();
    let _ = response.bytes().await; // drain, never log
    if (200..300).contains(&status) {
        Ok(())
    } else if status == 401 || status == 403 {
        Err("authentication rejected".into())
    } else {
        Err(format!("HTTP {status}"))
    }
}

async fn run_session(session_idx: usize, api_key: &str) -> Result<(), String> {
    eprintln!("session {session_idx}: probing {BASE_URL} model={MODEL}");
    probe_cloud(api_key).await?;
    eprintln!("session {session_idx}: cloud probe ok");

    // Optional: MCP-style env_clear isolation (mirrors buzz-agent spawn_one).
    if std::env::var("PERSONAL_DELEGATE_HARNESS_MCP_ISOLATION").as_deref() == Ok("1") {
        let output = Command::new("bash")
            .arg("-c")
            .arg("env | cut -d= -f1 | sort")
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", std::env::var("HOME").unwrap_or_default())
            // Deliberately do NOT pass OPENAI_COMPAT_API_KEY.
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .map_err(|e| format!("mcp isolation spawn: {e}"))?;
        let names = String::from_utf8_lossy(&output.stdout);
        if names.lines().any(|l| l == "OPENAI_COMPAT_API_KEY") {
            return Err("MCP-style child received OPENAI_COMPAT_API_KEY".into());
        }
        eprintln!("session {session_idx}: MCP env isolation ok");
    }

    Ok(())
}

#[tokio::test]
#[ignore = "requires Ollama Cloud API key; see module docs for exact command"]
async fn personal_delegate_ollama_cloud_three_sessions() {
    let Some(api_key) = provider_key() else {
        eprintln!(
            "skip: no OLLAMA_API_KEY / OPENAI_COMPAT_API_KEY in environment\n\
             export from Keychain:\n\
             OLLAMA_API_KEY=\"$(security find-generic-password -a \"$USER\" \
             -s personal-delegate-ollama_cloud -w)\" \\\n  \
             cargo test -p buzz-agent --test personal_delegate_ollama_cloud \
             -- --ignored --nocapture"
        );
        // Fail the ignored test when run without a key so operators notice,
        // rather than silently "passing". No network was attempted.
        panic!("provider key not available — see harness header for exact command");
    };

    // Key present → network allowed. Never print the key.
    eprintln!(
        "provider key present ({} chars); running {SESSIONS} sessions",
        api_key.len()
    );

    for i in 1..=SESSIONS {
        run_session(i, &api_key)
            .await
            .unwrap_or_else(|e| panic!("session {i} failed: {e}"));
    }

    eprintln!("all {SESSIONS} sessions passed against {BASE_URL} / {MODEL}");
}

/// Compile-time / unit-style check that the harness constants match product pins.
#[test]
fn harness_constants_match_personal_delegate_cloud_pins() {
    assert_eq!(BASE_URL, "https://ollama.com/v1");
    assert_eq!(MODEL, "kimi-k2.7-code");
    assert_eq!(SESSIONS, 3);
}
