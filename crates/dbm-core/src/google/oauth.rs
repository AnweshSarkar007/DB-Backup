//! Google OAuth 2.0 desktop (loopback) flow with PKCE (Section 14).
//!
//! We request the narrowest useful scope — `drive.file` (only files the app creates) — and never ask
//! the user to paste a token. The refresh token is returned to the caller, which stores it in the OS
//! credential store. The client_secret for an "installed/desktop" client is not truly confidential
//! and may omit its client secret for public Desktop clients.

use crate::error::{BackupError, Result};
use base64::Engine;
use serde::Deserialize;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
/// App-created files only — the least-privilege Drive scope.
pub const DRIVE_FILE_SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

/// OAuth client credentials for a Google "Desktop app" client.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: Option<String>,
}

/// Parse the Desktop app credentials JSON downloaded from Google Cloud Console.
pub fn parse_desktop_client_json(input: &str) -> Result<OAuthConfig> {
    #[derive(Deserialize)]
    struct Document {
        installed: Option<Client>,
    }
    #[derive(Deserialize)]
    struct Client {
        client_id: String,
        client_secret: Option<String>,
    }
    let doc: Document =
        serde_json::from_str(input).map_err(|e| BackupError::GoogleAuthenticationExpired {
            detail: format!("invalid Google OAuth JSON: {e}"),
        })?;
    let client = doc
        .installed
        .ok_or_else(|| BackupError::GoogleAuthenticationExpired {
            detail: "expected Google Cloud Console Desktop app JSON with an 'installed' section"
                .into(),
        })?;
    if client.client_id.trim().is_empty() {
        return Err(BackupError::GoogleAuthenticationExpired {
            detail: "OAuth JSON has an empty client_id".into(),
        });
    }
    Ok(OAuthConfig {
        client_id: client.client_id,
        client_secret: client.client_secret.filter(|s| !s.trim().is_empty()),
    })
}

/// Tokens returned by Google.
#[derive(Debug, Clone, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: i64,
    #[serde(default)]
    pub token_type: String,
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Generate a PKCE (verifier, S256 challenge) pair.
fn pkce() -> (String, String) {
    use sha2::{Digest, Sha256};
    let mut seed = Vec::new();
    seed.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    seed.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    let verifier = b64url(&seed);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Run the full loopback authorization flow, returning tokens (including a refresh token).
pub async fn run_loopback_flow<F>(
    cfg: &OAuthConfig,
    scope: &str,
    open_browser: F,
) -> Result<(Tokens, String)>
where
    F: FnOnce(&str) -> std::result::Result<(), String>,
{
    // Bind a loopback listener on an ephemeral port for the redirect.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| BackupError::Internal {
            detail: format!("oauth listener: {e}"),
        })?;
    let port = listener
        .local_addr()
        .map_err(|e| BackupError::Internal {
            detail: e.to_string(),
        })?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let (verifier, challenge) = pkce();
    let state = uuid::Uuid::new_v4().to_string();

    let auth_url = format!(
        "{AUTH_ENDPOINT}?client_id={cid}&redirect_uri={ru}&response_type=code&scope={scope}\
         &access_type=offline&prompt=consent&code_challenge={ch}&code_challenge_method=S256&state={st}",
        cid = urlencode(&cfg.client_id),
        ru = urlencode(&redirect_uri),
        scope = urlencode(scope),
        ch = challenge,
        st = state,
    );

    open_browser(&auth_url).map_err(|e| BackupError::GoogleAuthenticationExpired {
        detail: format!("could not open the authorization page: {e}"),
    })?;

    // Wait (with a timeout) for Google to redirect back with ?code=...
    let code = tokio::time::timeout(Duration::from_secs(300), accept_code(&listener, &state))
        .await
        .map_err(|_| BackupError::GoogleAuthenticationExpired {
            detail: "timed out waiting for Google authorization".into(),
        })??;

    let tokens = exchange_code(cfg, &code, &redirect_uri, &verifier).await?;
    Ok((tokens, redirect_uri))
}

/// Accept a single redirect connection and extract the `code`, validating `state`.
async fn accept_code(listener: &tokio::net::TcpListener, expected_state: &str) -> Result<String> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|e| BackupError::Internal {
            detail: format!("oauth accept: {e}"),
        })?;
        let mut buf = vec![0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let first_line = req.lines().next().unwrap_or("");
        // "GET /?code=...&state=... HTTP/1.1"
        let path = first_line.split_whitespace().nth(1).unwrap_or("");
        let (code, state) = parse_query(path);

        let body = "<html><body style='font-family:sans-serif'><h3>Database Backup Manager</h3>\
                    <p>Authorization received. You can close this tab and return to the app.</p></body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = socket.write_all(resp.as_bytes()).await;
        let _ = socket.shutdown().await;

        if let Some(err) = extract_param(path, "error") {
            return Err(BackupError::GoogleAuthenticationExpired {
                detail: format!(
                    "authorization denied: {err}{}",
                    extract_param(path, "error_description")
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default()
                ),
            });
        }
        if let (Some(code), Some(state)) = (code, state) {
            if state != expected_state {
                return Err(BackupError::GoogleAuthenticationExpired {
                    detail: "state mismatch (possible CSRF)".into(),
                });
            }
            return Ok(code);
        }
        // Ignore stray requests (e.g. favicon) and keep waiting.
    }
}

/// Exchange an authorization code for tokens.
async fn exchange_code(
    cfg: &OAuthConfig,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<Tokens> {
    let client = reqwest::Client::new();
    let mut params = vec![
        ("client_id", cfg.client_id.as_str()),
        ("code", code),
        ("code_verifier", verifier),
        ("grant_type", "authorization_code"),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(secret) = cfg.client_secret.as_deref() {
        params.push(("client_secret", secret));
    }
    let resp = client
        .post(TOKEN_ENDPOINT)
        .form(&params)
        .send()
        .await
        .map_err(|e| BackupError::GoogleAuthenticationExpired {
            detail: e.to_string(),
        })?;
    if !resp.status().is_success() {
        let t = resp.text().await.unwrap_or_default();
        return Err(BackupError::GoogleAuthenticationExpired {
            detail: format!("token exchange failed: {t}"),
        });
    }
    resp.json::<Tokens>()
        .await
        .map_err(|e| BackupError::GoogleAuthenticationExpired {
            detail: e.to_string(),
        })
}

/// Obtain a fresh access token from a stored refresh token.
pub async fn refresh(cfg: &OAuthConfig, refresh_token: &str) -> Result<Tokens> {
    let client = reqwest::Client::new();
    let mut params = vec![
        ("client_id", cfg.client_id.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];
    if let Some(secret) = cfg.client_secret.as_deref() {
        params.push(("client_secret", secret));
    }
    let resp = client
        .post(TOKEN_ENDPOINT)
        .form(&params)
        .send()
        .await
        .map_err(|e| BackupError::GoogleAuthenticationExpired {
            detail: e.to_string(),
        })?;
    if !resp.status().is_success() {
        let t = resp.text().await.unwrap_or_default();
        return Err(BackupError::GoogleAuthenticationExpired {
            detail: format!("token refresh failed: {t}"),
        });
    }
    resp.json::<Tokens>()
        .await
        .map_err(|e| BackupError::GoogleAuthenticationExpired {
            detail: e.to_string(),
        })
}

fn parse_query(path: &str) -> (Option<String>, Option<String>) {
    (extract_param(path, "code"), extract_param(path, "state"))
}

fn extract_param(path: &str, key: &str) -> Option<String> {
    let q = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return Some(urldecode(v));
            }
        }
    }
    None
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn urldecode(s: &str) -> String {
    let bytes = s.replace('+', " ");
    let bytes = bytes.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_deterministic_for_verifier() {
        use sha2::{Digest, Sha256};
        let (v, c) = pkce();
        assert_eq!(c, b64url(&Sha256::digest(v.as_bytes())));
    }

    #[test]
    fn query_parsing() {
        assert_eq!(
            extract_param("/?code=abc&state=xyz", "code").as_deref(),
            Some("abc")
        );
        assert_eq!(
            extract_param("/?code=abc&state=xyz", "state").as_deref(),
            Some("xyz")
        );
        assert_eq!(
            extract_param("/?error=access_denied", "error").as_deref(),
            Some("access_denied")
        );
    }

    #[test]
    fn urlencode_roundtrip() {
        assert_eq!(urldecode(&urlencode("a b+c%d/e")), "a b+c%d/e");
    }

    #[test]
    fn parses_desktop_json_and_rejects_web_credentials() {
        let cfg = parse_desktop_client_json(
            r#"{"installed":{"client_id":"desktop-id","client_secret":"secret"}}"#,
        )
        .unwrap();
        assert_eq!(cfg.client_id, "desktop-id");
        assert_eq!(cfg.client_secret.as_deref(), Some("secret"));
        let public =
            parse_desktop_client_json(r#"{"installed":{"client_id":"public-id"}}"#).unwrap();
        assert_eq!(public.client_secret, None);
        assert!(parse_desktop_client_json(r#"{"web":{"client_id":"web-id"}}"#).is_err());
    }
}
