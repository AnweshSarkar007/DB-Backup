//! Minimal Google Drive API v3 client (Section 14): ensure a folder, resumable upload, size check,
//! delete, and free-space query. Scoped to `drive.file` (app-created files only).

use crate::error::{BackupError, Result};
use serde::Deserialize;
use std::path::Path;

const FILES: &str = "https://www.googleapis.com/drive/v3/files";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3/files?uploadType=resumable";
const ABOUT: &str = "https://www.googleapis.com/drive/v3/about?fields=storageQuota";

#[derive(Debug, Deserialize)]
struct FileMeta {
    id: String,
    #[serde(default)]
    size: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FileList {
    #[serde(default)]
    files: Vec<FileMeta>,
}

/// Ensure a folder named `name` exists under `parent` (or My Drive root if None); returns its id.
pub async fn ensure_folder(access_token: &str, name: &str, parent: Option<&str>) -> Result<String> {
    let client = reqwest::Client::new();
    // Look for an existing non-trashed folder with this name under the parent.
    let mut q = format!(
        "mimeType='application/vnd.google-apps.folder' and name='{}' and trashed=false",
        name.replace('\'', "\\'")
    );
    if let Some(p) = parent {
        q.push_str(&format!(" and '{p}' in parents"));
    }
    let resp = client
        .get(FILES)
        .bearer_auth(access_token)
        .query(&[("q", q.as_str()), ("fields", "files(id,name)")])
        .send()
        .await
        .map_err(gerr)?;
    let list: FileList = parse(resp).await?;
    if let Some(f) = list.files.into_iter().next() {
        return Ok(f.id);
    }
    // Create it.
    let mut meta = serde_json::json!({
        "name": name,
        "mimeType": "application/vnd.google-apps.folder",
    });
    if let Some(p) = parent {
        meta["parents"] = serde_json::json!([p]);
    }
    let resp = client
        .post(FILES)
        .bearer_auth(access_token)
        .json(&meta)
        .send()
        .await
        .map_err(gerr)?;
    let created: FileMeta = parse(resp).await?;
    Ok(created.id)
}

/// Find a file by name within a folder; returns its id if present.
pub async fn find_file(access_token: &str, folder_id: &str, name: &str) -> Result<Option<String>> {
    let client = reqwest::Client::new();
    let q = format!("name='{}' and '{folder_id}' in parents and trashed=false", name.replace('\'', "\\'"));
    let resp = client
        .get(FILES)
        .bearer_auth(access_token)
        .query(&[("q", q.as_str()), ("fields", "files(id,name)")])
        .send()
        .await
        .map_err(gerr)?;
    let list: FileList = parse(resp).await?;
    Ok(list.files.into_iter().next().map(|f| f.id))
}

/// Resumable upload of a local file into `folder_id` under `name`. Returns the new file id.
pub async fn upload_resumable(
    access_token: &str,
    folder_id: &str,
    name: &str,
    local: &Path,
) -> Result<String> {
    let client = reqwest::Client::new();
    let meta = serde_json::json!({ "name": name, "parents": [folder_id] });

    // 1. Initiate the resumable session → session URI in the Location header.
    let init = client
        .post(UPLOAD)
        .bearer_auth(access_token)
        .header("X-Upload-Content-Type", "application/octet-stream")
        .json(&meta)
        .send()
        .await
        .map_err(gerr)?;
    if !init.status().is_success() {
        return Err(BackupError::GoogleUploadFailed {
            detail: format!("init failed: {}", init.text().await.unwrap_or_default()),
        });
    }
    let session_uri = init
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| BackupError::GoogleUploadFailed { detail: "no upload session URI".into() })?
        .to_string();

    // 2. Upload the bytes in one PUT, streaming the file so we never load it fully into memory.
    let file = tokio::fs::File::open(local).await?;
    let len = file.metadata().await?.len();
    let stream = tokio_util::io::ReaderStream::new(file);
    let put = client
        .put(&session_uri)
        .header(reqwest::header::CONTENT_LENGTH, len)
        .body(reqwest::Body::wrap_stream(stream))
        .send()
        .await
        .map_err(gerr)?;
    if !put.status().is_success() {
        return Err(BackupError::GoogleUploadFailed {
            detail: format!("upload failed: {}", put.text().await.unwrap_or_default()),
        });
    }
    let created: FileMeta = parse(put).await?;
    Ok(created.id)
}

/// Remote file size in bytes.
pub async fn file_size(access_token: &str, file_id: &str) -> Result<u64> {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{FILES}/{file_id}"))
        .bearer_auth(access_token)
        .query(&[("fields", "id,size")])
        .send()
        .await
        .map_err(gerr)?;
    let meta: FileMeta = parse(resp).await?;
    Ok(meta.size.and_then(|s| s.parse().ok()).unwrap_or(0))
}

pub async fn delete_file(access_token: &str, file_id: &str) -> Result<()> {
    let client = reqwest::Client::new();
    let resp = client
        .delete(format!("{FILES}/{file_id}"))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(gerr)?;
    if resp.status().is_success() || resp.status().as_u16() == 404 {
        Ok(())
    } else {
        Err(BackupError::GoogleUploadFailed {
            detail: format!("delete failed: {}", resp.text().await.unwrap_or_default()),
        })
    }
}

/// Remaining Drive storage in bytes (best-effort; unlimited quotas return u64::MAX).
pub async fn available_space(access_token: &str) -> Result<u64> {
    #[derive(Deserialize)]
    struct Quota {
        #[serde(default)]
        limit: Option<String>,
        #[serde(default)]
        usage: Option<String>,
    }
    #[derive(Deserialize)]
    struct About {
        #[serde(rename = "storageQuota")]
        storage_quota: Quota,
    }
    let client = reqwest::Client::new();
    let resp = client.get(ABOUT).bearer_auth(access_token).send().await.map_err(gerr)?;
    let about: About = parse(resp).await?;
    match (about.storage_quota.limit, about.storage_quota.usage) {
        (Some(l), Some(u)) => {
            let l: u64 = l.parse().unwrap_or(u64::MAX);
            let u: u64 = u.parse().unwrap_or(0);
            Ok(l.saturating_sub(u))
        }
        _ => Ok(u64::MAX), // unlimited / pooled quota
    }
}

/// The email address of the authorized account (for display).
pub async fn account_email(access_token: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct User {
        #[serde(rename = "emailAddress", default)]
        email_address: String,
    }
    #[derive(Deserialize)]
    struct About {
        user: User,
    }
    let client = reqwest::Client::new();
    let resp = client
        .get("https://www.googleapis.com/drive/v3/about?fields=user")
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(gerr)?;
    let about: About = parse(resp).await?;
    Ok(about.user.email_address)
}

fn gerr(e: reqwest::Error) -> BackupError {
    BackupError::GoogleUploadFailed { detail: e.to_string() }
}

async fn parse<T: for<'de> Deserialize<'de>>(resp: reqwest::Response) -> Result<T> {
    if !resp.status().is_success() {
        let s = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(BackupError::GoogleUploadFailed { detail: format!("HTTP {s}: {body}") });
    }
    resp.json::<T>().await.map_err(gerr)
}
