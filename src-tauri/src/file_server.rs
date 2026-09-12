use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const READ_TIMEOUT: Duration = Duration::from_secs(5);

pub enum SharedSource {
  Memory(Arc<Vec<u8>>),
  Disk(PathBuf),
}

pub struct SharedFile {
  pub id: String,
  pub name: String,
  pub size: u64,
  pub source: SharedSource,
}

#[derive(Default)]
pub struct SharedFileRegistry {
  files: Mutex<HashMap<String, SharedFile>>,
}

impl SharedFileRegistry {
  pub fn new() -> Self { Self::default() }

  pub fn insert(&self, file: SharedFile) {
    if let Ok(mut files) = self.files.lock() {
      files.insert(file.id.clone(), file);
    }
  }

  pub fn clear(&self) {
    if let Ok(mut files) = self.files.lock() {
      files.clear();
    }
  }

  pub fn get(&self, id: &str) -> Option<SharedFile> {
    let files = self.files.lock().ok()?;
    let file = files.get(id)?;
    Some(SharedFile {
      id: file.id.clone(),
      name: file.name.clone(),
      size: file.size,
      source: match &file.source {
        SharedSource::Memory(bytes) => SharedSource::Memory(bytes.clone()),
        SharedSource::Disk(path) => SharedSource::Disk(path.clone()),
      },
    })
  }
}

pub async fn run_file_server(
  registry: Arc<SharedFileRegistry>,
  port_tx: tokio::sync::oneshot::Sender<u16>,
  mut shutdown: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<()> {
  let listener = tokio::net::TcpListener::bind(("0.0.0.0", 0)).await?;
  let actual_port = listener.local_addr()?.port();
  let _ = port_tx.send(actual_port);
  loop {
    tokio::select! {
      _ = &mut shutdown => return Ok(()),
      accepted = listener.accept() => match accepted {
        Ok((stream, _)) => {
          let registry = registry.clone();
          tokio::spawn(async move {
            let _ = tokio::time::timeout(READ_TIMEOUT, handle_request(stream, registry)).await;
          });
        }
        Err(_) => return Ok(()),
      }
    }
  }
}

async fn handle_request(mut stream: tokio::net::TcpStream, registry: Arc<SharedFileRegistry>) -> std::io::Result<()> {
  use tokio::io::{AsyncReadExt, AsyncWriteExt};

  let mut buf = vec![0u8; 8192];
  let mut total = 0;
  let id = loop {
    let n = stream.read(&mut buf[total..]).await?;
    if n == 0 { return Ok(()); }
    total += n;
    if let Some(end) = buf[..total].windows(4).position(|w| w == b"\r\n\r\n") {
      let header = std::str::from_utf8(&buf[..end]).unwrap_or("");
      if let Some(id) = parse_path(header) {
        break id;
      }
      return Ok(());
    }
    if total >= buf.len() {
      buf.extend_from_slice(&[0u8; 4096]);
    }
  };

  let Some(file) = registry.get(&id) else {
    return write_404(&mut stream, id).await;
  };

  let body: Vec<u8> = match &file.source {
    SharedSource::Memory(bytes) => bytes.as_ref().clone(),
    SharedSource::Disk(path) => match tokio::fs::read(path).await {
      Ok(bytes) => bytes,
      Err(_) => return write_404(&mut stream, id).await,
    },
  };

  let header = format!(
    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nContent-Disposition: attachment; filename=\"{}\"\r\nConnection: close\r\n\r\n",
    body.len(),
    sanitize_header(&file.name)
  );
  stream.write_all(header.as_bytes()).await?;
  stream.write_all(&body).await?;
  stream.shutdown().await?;
  Ok(())
}

async fn write_404<S: tokio::io::AsyncWriteExt + Unpin>(stream: &mut S, _id: String) -> std::io::Result<()> {
  let body = b"Not Found";
  let header = format!(
    "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    body.len()
  );
  stream.write_all(header.as_bytes()).await?;
  stream.write_all(body).await?;
  stream.shutdown().await?;
  Ok(())
}

fn parse_path(header: &str) -> Option<String> {
  let first_line = header.lines().next()?;
  let mut parts = first_line.split_whitespace();
  let _method = parts.next()?;
  let target = parts.next()?;
  let path = target.split('?').next()?;
  let id = path.trim_start_matches("/clippi/").trim_start_matches('/');
  if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
    return None;
  }
  Some(id.to_string())
}

fn sanitize_header(name: &str) -> String {
  name.chars()
    .map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' })
    .collect()
}

pub fn shared_dir() -> PathBuf {
  let base = dirs::cache_dir()
    .or_else(dirs::home_dir)
    .unwrap_or_else(|| PathBuf::from("."));
  base.join("clippi").join("shared")
}