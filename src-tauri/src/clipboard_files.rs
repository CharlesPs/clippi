use std::path::Path;
use std::time::Duration;

use tauri::AppHandle;

use crate::ClipboardFile;

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
  clipboard.wait_for_uris().into_iter().map(|s| s.to_string()).collect()
}

#[cfg(target_os = "macos")]
fn read_paths_platform() -> Vec<String> {
  use objc2::msg_send;
  use objc2::runtime::{AnyClass, AnyObject};

  let pasteboard_class = match AnyClass::get(c"NSPasteboard") { Some(value) => value, None => return Vec::new() };
  let nsstring_class = match AnyClass::get(c"NSString") { Some(value) => value, None => return Vec::new() };

  unsafe {
    let pasteboard: *mut AnyObject = msg_send![pasteboard_class, generalPasteboard];
    if pasteboard.is_null() { return Vec::new(); }

    // Prefer the modern public.file-url (NSURL items).
    let url_key = b"public.file-url\0";
    let url_type: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: url_key.as_ptr()];
    let urls: *mut AnyObject = msg_send![pasteboard, propertyListForType: url_type];
    let mut result = nsurl_array_to_paths(urls);

    // Fallback to the legacy NSFilenamesPboard (NSString items).
    if result.is_empty() {
      let filenames_key = b"NSFilenamesPboard\0";
      let filenames_type: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: filenames_key.as_ptr()];
      result = nsstring_array_to_paths(msg_send![pasteboard, propertyListForType: filenames_type]);
    }
    result
  }
}

#[cfg(target_os = "macos")]
unsafe fn nsstring_array_to_paths(array: *mut objc2::runtime::AnyObject) -> Vec<String> {
  use objc2::msg_send;
  use objc2::runtime::AnyObject;
  if array.is_null() { return Vec::new(); }
  let count: usize = msg_send![array, count];
  let mut result = Vec::with_capacity(count);
  for index in 0..count {
    let item: *mut AnyObject = msg_send![array, objectAtIndex: index];
    if item.is_null() { continue; }
    let utf8: *const i8 = msg_send![item, UTF8String];
    if !utf8.is_null() {
      let s = std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned();
      if !s.is_empty() { result.push(s); }
    }
  }
  result
}

#[cfg(target_os = "macos")]
unsafe fn nsurl_array_to_paths(array: *mut objc2::runtime::AnyObject) -> Vec<String> {
  use objc2::msg_send;
  use objc2::runtime::AnyObject;
  if array.is_null() { return Vec::new(); }
  let count: usize = msg_send![array, count];
  let mut result = Vec::with_capacity(count);
  for index in 0..count {
    let item: *mut AnyObject = msg_send![array, objectAtIndex: index];
    if item.is_null() { continue; }
    // NSURL.path returns NSString*. Use UTF8String to get a C string.
    let path_nsstr: *mut AnyObject = msg_send![item, path];
    if path_nsstr.is_null() { continue; }
    let utf8: *const i8 = msg_send![path_nsstr, UTF8String];
    if utf8.is_null() { continue; }
    let s = std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned();
    if !s.is_empty() { result.push(s); }
  }
  result
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_paths_platform() -> Vec<String> {
  Vec::new()
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
