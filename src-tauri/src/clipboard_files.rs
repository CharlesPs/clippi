use std::path::Path;
use std::time::Duration;

use tauri::AppHandle;

use crate::ClipboardFile;

// Convert a `file://` URI to a local filesystem path. Handles percent-encoded
// characters and hostnames correctly.
fn uri_to_local_path(uri: &str) -> Option<String> {
  let parsed = url::Url::parse(uri.trim()).ok()?;
  if parsed.scheme() != "file" { return None; }
  let path = parsed.to_file_path().ok()?;
  path.to_str().map(String::from)
}

pub async fn read_clipboard_files(app: AppHandle) -> Vec<ClipboardFile> {
  let paths = read_paths(app).await;
  match tokio::task::spawn_blocking(move || {
    paths.into_iter().map(enrich).collect()
  }).await {
    Ok(value) => value,
    Err(_) => Vec::new(),
  }
}

fn enrich(path: String) -> ClipboardFile {
  if path.starts_with("http://") || path.starts_with("https://") {
    let (name, size) = parse_http_query(&path);
    let mime = guess_mime(&name);
    return ClipboardFile { path, name, size, mime };
  }
  let name = Path::new(&path)
    .file_name()
    .and_then(|s| s.to_str())
    .unwrap_or("")
    .to_string();
  let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
  let mime = guess_mime(&path);
  ClipboardFile { path, name, size, mime }
}

fn parse_http_query(url: &str) -> (String, u64) {
  let parsed = url::Url::parse(url);
  let (mut name, mut size) = (String::new(), 0u64);
  if let Ok(parsed) = parsed {
    for (k, v) in parsed.query_pairs() {
      if k == "n" { name = v.into_owned(); }
      else if k == "s" { if let Ok(n) = v.parse::<u64>() { size = n; } }
    }
  }
  (name, size)
}

fn guess_mime(path: &str) -> String {
  let ext = Path::new(path)
    .extension()
    .and_then(|s| s.to_str())
    .unwrap_or("")
    .to_lowercase();
  let value = match ext.as_str() {
    "pdf" => "application/pdf",
    "txt" | "log" | "csv" | "tsv" => "text/plain",
    "md" | "markdown" => "text/markdown",
    "html" | "htm" => "text/html",
    "css" => "text/css",
    "js" | "mjs" | "cjs" => "application/javascript",
    "ts" | "tsx" => "application/typescript",
    "json" => "application/json",
    "xml" => "application/xml",
    "yml" | "yaml" => "application/yaml",
    "png" => "image/png",
    "jpg" | "jpeg" => "image/jpeg",
    "gif" => "image/gif",
    "webp" => "image/webp",
    "svg" => "image/svg+xml",
    "bmp" => "image/bmp",
    "ico" => "image/x-icon",
    "mp4" | "m4v" => "video/mp4",
    "webm" => "video/webm",
    "mkv" => "video/x-matroska",
    "mov" => "video/quicktime",
    "avi" => "video/x-msvideo",
    "mp3" => "audio/mpeg",
    "wav" => "audio/wav",
    "flac" => "audio/flac",
    "ogg" => "audio/ogg",
    "m4a" => "audio/mp4",
    "zip" => "application/zip",
    "tar" => "application/x-tar",
    "gz" | "tgz" => "application/gzip",
    "bz2" => "application/x-bzip2",
    "xz" => "application/x-xz",
    "7z" => "application/x-7z-compressed",
    "rar" => "application/vnd.rar",
    "rs" => "text/x-rust",
    "py" => "text/x-python",
    "go" => "text/x-go",
    "rb" => "text/x-ruby",
    "java" => "text/x-java",
    "c" | "h" => "text/x-c",
    "cpp" | "hpp" | "cc" => "text/x-c++",
    "sh" => "text/x-shellscript",
    "doc" => "application/msword",
    "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    "xls" => "application/vnd.ms-excel",
    "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    "ppt" => "application/vnd.ms-powerpoint",
    "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    "epub" => "application/epub+zip",
    _ => "application/octet-stream",
  };
  value.to_string()
}

async fn read_paths(app: AppHandle) -> Vec<String> {
  use std::sync::mpsc;
  let (tx, rx) = mpsc::sync_channel::<Vec<String>>(1);
  let dispatched = app.run_on_main_thread(move || {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
      #[cfg(target_os = "macos")]
      {
        objc2::exception::catch(|| read_paths_platform()).unwrap_or_default()
      }
      #[cfg(not(target_os = "macos"))]
      {
        read_paths_platform()
      }
    }))
    .unwrap_or_default();
    let _ = tx.send(result);
  });
  if dispatched.is_err() {
    return Vec::new();
  }
  match tokio::task::spawn_blocking(move || {
    rx.recv_timeout(Duration::from_millis(1500)).unwrap_or_default()
  }).await {
    Ok(value) => value,
    Err(_) => Vec::new(),
  }
}

#[cfg(target_os = "linux")]
fn read_paths_platform() -> Vec<String> {
  let Some(display) = gdk::Display::default() else { return Vec::new() };
  let clipboard = gtk::Clipboard::for_display(&display, &gdk::SELECTION_CLIPBOARD);
  clipboard
    .wait_for_uris()
    .into_iter()
    .filter_map(|u| uri_to_local_path(&u))
    .collect()
}

#[cfg(target_os = "macos")]
fn read_paths_platform() -> Vec<String> {
  use objc2::msg_send;
  use objc2::runtime::{AnyClass, AnyObject};

  let pasteboard_class = match AnyClass::get(c"NSPasteboard") { Some(value) => value, None => return Vec::new() };
  let nsstring_class = match AnyClass::get(c"NSString") { Some(value) => value, None => return Vec::new() };
  let nsurl_class = match AnyClass::get(c"NSURL") { Some(value) => value, None => return Vec::new() };

  unsafe {
    let pasteboard: *mut AnyObject = msg_send![pasteboard_class, generalPasteboard];
    if pasteboard.is_null() { return Vec::new(); }

    let items: *mut AnyObject = msg_send![pasteboard, pasteboardItems];
    if items.is_null() { return Vec::new(); }
    let count: usize = msg_send![items, count];
    if count == 0 { return Vec::new(); }

    let url_type: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: b"public.file-url\0".as_ptr()];

    let mut result: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for index in 0..count {
      let item: *mut AnyObject = msg_send![items, objectAtIndex: index];
      if item.is_null() { continue; }

      // Get the URL string for the public.file-url type.
      let value: *mut AnyObject = if !url_type.is_null() {
        msg_send![item, stringForType: url_type]
      } else {
        std::ptr::null_mut()
      };
      if value.is_null() { continue; }

      let utf8: *const i8 = msg_send![value, UTF8String];
      if utf8.is_null() { continue; }
      let url_string = std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned();

      // Convert the URL string into an NSURL object.
      let url_str_obj: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: utf8];
      let url: *mut AnyObject = msg_send![nsurl_class, URLWithString: url_str_obj];
      if url.is_null() { continue; }

      // Scoped file URLs (file:///.file/id=...) on macOS 15+ need explicit
      // access before `.path` returns the real filesystem path.
      let _: () = msg_send![url, startAccessingSecurityScopedResource];

      // NSURL.path returns NSString* with the local filesystem path.
      let path_nsstr: *mut AnyObject = msg_send![url, path];
      let resolved = if !path_nsstr.is_null() {
        let p_utf8: *const i8 = msg_send![path_nsstr, UTF8String];
        if !p_utf8.is_null() {
          std::ffi::CStr::from_ptr(p_utf8).to_string_lossy().into_owned()
        } else {
          String::new()
        }
      } else {
        String::new()
      };

      let _: () = msg_send![url, stopAccessingSecurityScopedResource];

      if resolved.is_empty() {
        // Fallback: try the URL's `path` representation via URI conversion.
        if let Some(path) = uri_to_local_path(&url_string) {
          if seen.insert(path.clone()) {
            result.push(path);
          }
        }
        continue;
      }

      if seen.insert(resolved.clone()) {
        result.push(resolved);
      }
    }
    result
  }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_paths_platform() -> Vec<String> {
  Vec::new()
}
