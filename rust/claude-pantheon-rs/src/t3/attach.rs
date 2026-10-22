//! Attachments for the composer: a file or pasted image becomes a server attachment through
//! `attachments.createUploadUrl` (a signed, short-lived relative URL) and one raw `POST` of the bytes
//! to it (204 on success). The `ChatAttachment` JSON the message carries is returned.

use super::http;
use super::rpc::{Rpc, RpcError};
use super::ws::WsError;
use gtk::gio;
use serde_json::{Value, json};
use std::path::Path;

pub const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_ATTACHMENTS: usize = 10;

#[derive(Clone, Debug)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub is_image: bool,
}

impl Attachment {
    /// The `ChatAttachment` the message carries.
    pub fn to_json(&self) -> Value {
        json!({
            "type": if self.is_image { "image" } else { "file" },
            "id": self.id, "name": self.name, "mimeType": self.mime, "sizeBytes": self.size,
        })
    }
}

#[derive(Debug)]
pub enum AttachError {
    TooLarge(u64),
    Rpc(RpcError),
    Http(String),
    Io(String),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachError::TooLarge(max) => write!(f, "larger than the {} MB limit", max / 1024 / 1024),
            AttachError::Rpc(e) => write!(f, "{}", e.message()),
            AttachError::Http(e) | AttachError::Io(e) => f.write_str(e),
        }
    }
}

impl From<WsError> for AttachError {
    fn from(e: WsError) -> Self {
        AttachError::Http(e.to_string())
    }
}

/// MIME type from the name and the first bytes (gio's content sniffing), as `image/png` or
/// `text/plain`; the server only accepts a few image types for the `image` kind.
pub fn mime_for(name: &str, head: &[u8]) -> (String, bool) {
    let (content_type, _) = gio::content_type_guess(Some(Path::new(name)), head);
    let mime = gio::content_type_get_mime_type(&content_type).map(|m| m.to_string()).unwrap_or_else(|| "application/octet-stream".into());
    let is_image = matches!(mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp");
    (mime, is_image)
}

/// Uploads `bytes` as `name`.
pub async fn upload(server: &str, rpc: &Rpc, name: &str, bytes: Vec<u8>) -> Result<Attachment, AttachError> {
    let (mime, is_image) = mime_for(name, &bytes[..bytes.len().min(4096)]);
    let size = bytes.len() as u64;
    let max = if is_image { MAX_IMAGE_BYTES } else { MAX_FILE_BYTES };
    if size == 0 || size > max {
        return Err(AttachError::TooLarge(max));
    }
    let mut payload = json!({"name": name, "mimeType": mime, "sizeBytes": size});
    if !is_image {
        payload["type"] = json!("file");
    }
    let minted = rpc.call("attachments.createUploadUrl", payload).await.map_err(AttachError::Rpc)?;
    let relative = minted.get("relativeUrl").and_then(Value::as_str).ok_or_else(|| AttachError::Http("no upload URL".into()))?;
    let id = minted.get("attachmentId").and_then(Value::as_str).unwrap_or("").to_string();
    let url = format!("{}{}", server.trim_end_matches('/'), relative);
    let r = http::request("POST", &url, &[("Content-Type", &mime)], Some(&bytes)).await?;
    if r.status != 204 && r.status != 200 {
        return Err(AttachError::Http(format!("upload answered {}: {}", r.status, r.body.chars().take(120).collect::<String>())));
    }
    Ok(Attachment { id, name: name.to_string(), mime, size, is_image })
}

/// Reads a local file and uploads it.
pub async fn upload_path(server: &str, rpc: &Rpc, path: &Path) -> Result<Attachment, AttachError> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let p = path.to_path_buf();
    let bytes = gio::spawn_blocking(move || std::fs::read(&p)).await.map_err(|_| AttachError::Io("read failed".into()))?.map_err(|e| AttachError::Io(e.to_string()))?;
    upload(server, rpc, &name, bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_sniffing() {
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0];
        assert_eq!(mime_for("shot.png", &png), ("image/png".to_string(), true));
        assert!(!mime_for("notes.md", b"# hi\n").1);
        assert_eq!(
            Attachment { id: "a".into(), name: "n.txt".into(), mime: "text/plain".into(), size: 3, is_image: false }.to_json()["type"],
            "file"
        );
    }
}
