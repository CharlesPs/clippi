use std::sync::mpsc;
use std::time::Duration;

pub fn write_clipboard_url(url: &str) -> bool {
  if url.is_empty() { return false; }
  write_url(url)
}

#[cfg(target_os = "linux")]
fn write_url(url: &str) -> bool {
  use gtk::glib;
  use gtk::glib::translate::ToGlibPtr;
  use std::ffi::CString;
  use std::os::raw::{c_int, c_uint, c_void};
  use std::sync::OnceLock;

  #[repr(C)]
  struct GtkTargetEntry {
    target: *const i8,
    flags: c_uint,
    info: c_uint,
  }

  type GtkClipboardGetFunc = Option<
    unsafe extern "C" fn(
      *mut gtk_sys::GtkClipboard,
      *mut gtk_sys::GtkSelectionData,
      c_uint,
      *mut c_void,
    ),
  >;
  type GtkClipboardClearFunc = Option<
    unsafe extern "C" fn(*mut gtk_sys::GtkClipboard, *mut c_void),
  >;

  extern "C" {
    fn gtk_clipboard_set_with_data(
      clipboard: *mut gtk_sys::GtkClipboard,
      targets: *const GtkTargetEntry,
      n_targets: c_int,
      get_func: GtkClipboardGetFunc,
      clear_func: GtkClipboardClearFunc,
      user_data: *mut c_void,
    );
    fn gtk_selection_data_set(
      selection_data: *mut gtk_sys::GtkSelectionData,
      type_: std::os::raw::c_ulong,
      format: c_int,
      data: *const u8,
      length: c_int,
    );
  }

  static URI_LIST_NAME: OnceLock<CString> = OnceLock::new();
  static TEXT_NAME: OnceLock<CString> = OnceLock::new();
  static URL_STORAGE: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

  let url_owned = url.to_string();
  let (tx, rx) = mpsc::sync_channel::<bool>(1);
  glib::MainContext::default().invoke(move || {
    let display = match gdk::Display::default() {
      Some(value) => value,
      None => { let _ = tx.send(false); return; }
    };
    let clipboard = gtk::Clipboard::for_display(&display, &gdk::SELECTION_CLIPBOARD);

    let url_bytes = url_owned.into_bytes();
    {
      let mut storage = match URL_STORAGE.lock() {
        Ok(value) => value,
        Err(_) => { let _ = tx.send(false); return; }
      };
      *storage = Some(url_bytes);
    }

    let uri_list_name = URI_LIST_NAME.get_or_init(|| CString::new("text/uri-list").unwrap());
    let text_name = TEXT_NAME.get_or_init(|| CString::new("text/plain").unwrap());

    let targets = [
      GtkTargetEntry { target: uri_list_name.as_ptr(), flags: 0, info: 0 },
      GtkTargetEntry { target: text_name.as_ptr(), flags: 0, info: 1 },
    ];

    unsafe extern "C" fn get_func(
      _clipboard: *mut gtk_sys::GtkClipboard,
      selection_data: *mut gtk_sys::GtkSelectionData,
      info: c_uint,
      _user_data: *mut c_void,
    ) {
      unsafe {
        let storage = URL_STORAGE.lock().expect("URL_STORAGE poisoned");
        if let Some(bytes) = storage.as_ref() {
          let target_atom: std::os::raw::c_ulong = if info == 0 {
            gdk::Atom::intern("text/uri-list").value() as std::os::raw::c_ulong
          } else {
            gdk::Atom::intern("text/plain").value() as std::os::raw::c_ulong
          };
          gtk_selection_data_set(
            selection_data,
            target_atom,
            8,
            bytes.as_ptr(),
            bytes.len() as c_int,
          );
        }
      }
    }

    unsafe extern "C" fn clear_func(
      _clipboard: *mut gtk_sys::GtkClipboard,
      _user_data: *mut c_void,
    ) {
      if let Ok(mut storage) = URL_STORAGE.lock() {
        *storage = None;
      }
    }

    let clipboard_ptr = clipboard.to_glib_none().0;
    unsafe {
      gtk_clipboard_set_with_data(
        clipboard_ptr,
        targets.as_ptr(),
        targets.len() as c_int,
        Some(get_func),
        Some(clear_func),
        std::ptr::null_mut(),
      );
    }
    let _ = tx.send(true);
  });
  rx.recv_timeout(Duration::from_millis(500)).unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn write_url(url: &str) -> bool {
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

    let _: bool = msg_send![pasteboard, writeObjects: nsarray];
    true
  }
}

#[cfg(target_os = "macos")]
fn write_url(url: &str) -> bool {
  use objc2::msg_send;
  use objc2::runtime::{AnyClass, AnyObject};

  let pasteboard_class = match AnyClass::get(c"NSPasteboard") { Some(value) => value, None => return false };
  let nsstring_class = match AnyClass::get(c"NSString") { Some(value) => value, None => return false };

  unsafe {
    let pasteboard: *mut AnyObject = msg_send![pasteboard_class, generalPasteboard];
    if pasteboard.is_null() { return false; }
    let _: () = msg_send![pasteboard, clearContents];

    let nsurl_string = match std::ffi::CString::new(url) { Ok(value) => value, Err(_) => return false };
    let url_obj: *mut AnyObject = msg_send![class!(NSURL), URLWithString: nsurl_string.as_ptr()];
    if url_obj.is_null() { return false; }
    let objects: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: url_obj];
    let url_type_key = b"public.file-url\0";
    let url_type: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: url_type_key.as_ptr()];
    let _: bool = msg_send![pasteboard, setPropertyList: objects forType: url_type];

    let text_obj: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: nsurl_string.as_ptr()];
    let text_type_key = b"public.utf8-plain-text\0";
    let text_type: *mut AnyObject = msg_send![nsstring_class, stringWithUTF8String: text_type_key.as_ptr()];
    let _: () = msg_send![pasteboard, setString: text_obj forType: text_type];

    true
  }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn write_url(_url: &str) -> bool { false }