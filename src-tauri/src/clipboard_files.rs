use std::path::Path;
use std::time::Duration;

use crate::ClipboardFile;

pub async fn read_clipboard_files() -> Vec<ClipboardFile> {
  match tokio::task::spawn_blocking(read_clipboard_files_blocking).await {
    Ok(value) => value,
    Err(_) => Vec::new(),
  }
}

fn read_clipboard_files_blocking() -> Vec<ClipboardFile> {
  let paths = read_paths();
  paths.into_iter().map(enrich).collect()
}

fn enrich(path: String) -> ClipboardFile {
  let name = Path::new(&path)
    .file_name()
    .and_then(|s| s.to_str())
    .unwrap_or("")
    .to_string();
  let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
  let mime = guess_mime(&path);
  ClipboardFile { path, name, size, mime }
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

#[cfg(target_os = "linux")]
fn read_paths() -> Vec<String> {
  let ctx = glib::MainContext::default();
  let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<String>>(1);
  ctx.invoke(move || {
    let display = match gdk::Display::default() { Some(value) => value, None => { let _ = tx.send(Vec::new()); return; } };
    let clipboard = gtk::Clipboard::for_display(&display, &gdk::SELECTION_CLIPBOARD);
    let result = clipboard.wait_for_uris().into_iter().map(|s| s.to_string()).collect();
    let _ = tx.send(result);
  });
  rx.recv_timeout(Duration::from_millis(1200)).unwrap_or_default()
}

#[cfg(target_os = "macos")]
fn read_paths() -> Vec<String> {
  use objc2::msg_send;
  use objc2::runtime::{AnyClass, AnyObject};

  let pasteboard_class = match AnyClass::get(c"NSPasteboard") { Some(value) => value, None => return Vec::new() };
  let nsstring_class = match AnyClass::get(c"NSString") { Some(value) => value, None => return Vec::new() };

  unsafe {
    let pasteboard: *mut AnyObject = msg_send![pasteboard_class, generalPasteboard];
    if pasteboard.is_null() { return Vec::new(); }

    let filenames_key = b"NSFilenamesPboard\0";
    let filenames_type: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: filenames_key.as_ptr()];
    let mut result: Vec<String> = ns_array_to_string_vec(msg_send![pasteboard, propertyListForType: filenames_type]);

    if result.is_empty() {
      let url_key = b"public.file-url\0";
      let url_type: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: url_key.as_ptr()];
      let urls = msg_send![pasteboard, propertyListForType: url_type];
      if !urls.is_null() {
        let count: usize = msg_send![urls, count];
        for index in 0..count {
          let item: *mut AnyObject = msg_send![urls, objectAtIndex: index];
          if item.is_null() { continue; }
          let utf8: *const i8 = msg_send![item, UTF8String];
          if !utf8.is_null() {
            let s = std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned();
            if !s.is_empty() { result.push(s); }
          }
        }
      }
    }
    result
  }
}

#[cfg(target_os = "macos")]
unsafe fn ns_array_to_string_vec(array: *mut objc2::runtime::AnyObject) -> Vec<String> {
  use objc2::msg_send;
  use objc2::runtime::AnyObject;
  if array.is_null() { return Vec::new(); }
  let count: usize = msg_send![array, count];
  let mut result = Vec::with_capacity(count);
  for index in 0..count {
    let item: *mut AnyObject = msg_send![array, objectAtIndex: index];
    if item.is_null() { continue; }
    let utf8: *const i8 = msg_send![item, UTF8String];
    if utf8.is_null() { continue; }
    let s = std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned();
    if !s.is_empty() { result.push(s); }
  }
  result
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_paths() -> Vec<String> {
  Vec::new()
}