pub fn write_clipboard_uris(paths: &[String]) -> bool {
  if paths.is_empty() { return false; }
  let body = paths.iter()
    .map(|path| if path.starts_with("file://") { path.clone() } else { format!("file://{path}") })
    .collect::<Vec<_>>()
    .join("\n");
  write_uris(&body)
}

#[cfg(target_os = "linux")]
fn write_uris(body: &str) -> bool {
  let mut child = match std::process::Command::new("wl-copy")
    .args(["-t", "text/uri-list"])
    .stdin(std::process::Stdio::piped())
    .spawn()
  {
    Ok(value) => value,
    Err(_) => return false,
  };
  if let Some(stdin) = child.stdin.as_mut() {
    use std::io::Write;
    let _ = stdin.write_all(body.as_bytes());
  }
  child.wait().map(|status| status.success()).unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn write_uris(body: &str) -> bool {
  use objc2::msg_send;
  use objc2::runtime::{AnyClass, AnyObject};

  let pasteboard_class = match AnyClass::get(c"NSPasteboard") { Some(value) => value, None => return false };
  let nsstring_class = match AnyClass::get(c"NSString") { Some(value) => value, None => return false };

  unsafe {
    let pasteboard: *mut AnyObject = msg_send![pasteboard_class, generalPasteboard];
    if pasteboard.is_null() { return false; }
    let _: () = msg_send![pasteboard, clearContents];

    let urls_type_key = b"public.file-url\0";
    let urls_type: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: urls_type_key.as_ptr()];

    let lines: Vec<&str> = body.lines().collect();
    let count = lines.len();
    let nsarray: *mut AnyObject = msg_send![class!(NSArray), arrayWithCapacity: count];

    for line in lines {
      let nsurl_string = match std::ffi::CString::new(line) { Ok(value) => value, Err(_) => continue };
      let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: nsurl_string.as_ptr()];
      if !url.is_null() {
        let _: *mut AnyObject = msg_send![nsarray, addObject: url];
      }
    }

    let _: bool = msg_send![pasteboard, setPropertyList: nsarray forType: urls_type];
    true
  }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn write_uris(_body: &str) -> bool {
  false
}