use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

mod clipboard_files;
mod clipboard_writer;
mod file_server;

use file_server::{SharedFile, SharedFileRegistry, SharedSource};
use std::sync::Arc as SharedArc;

const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);
const MEMORY_THRESHOLD: u64 = 1024 * 1024;

struct SyncState(Mutex<Vec<oneshot::Sender<()>>>);

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ConnectRequest { endpoint: String, room: String, client_id: String }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayRequest { port: u16, room: String, client_id: String }

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Envelope { #[serde(rename = "type")] kind: String, room: String, sender_id: String, text: String }

#[derive(Clone, Serialize)]
struct Status { state: &'static str, detail: String }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardUpdate { text: String }

#[derive(Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ClipboardFile { path: String, name: String, size: u64, mime: String }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardFilesEvent { files: Vec<ClipboardFile> }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShareFailed { name: Option<String>, error: String }

struct SharedFileServerPort(Mutex<Option<u16>>);
struct SharedFileServerShutdown(Mutex<Option<oneshot::Sender<()>>>);

fn status(app: &AppHandle, state: &'static str, detail: impl Into<String>) {
  let _ = app.emit("sync-status", Status { state, detail: detail.into() });
}
fn clipboard_update(app: &AppHandle, text: String) {
  let _ = app.emit("clipboard-update", ClipboardUpdate { text });
}
fn clipboard_files_changed(app: &AppHandle, files: Vec<ClipboardFile>) {
  let _ = app.emit("clipboard-files", ClipboardFilesEvent { files });
}
fn share_failed(app: &AppHandle, name: Option<String>, error: String) {
  let _ = app.emit("share-failed", ShareFailed { name, error });
}

#[cfg(target_os = "linux")]
fn default_shared_dir() -> PathBuf { file_server::shared_dir() }
#[cfg(not(target_os = "linux"))]
fn default_shared_dir() -> PathBuf { PathBuf::from("/tmp/clippi-shared") }

fn set_clipboard_text_plain(text: &str) -> bool {
  match arboard::Clipboard::new() {
    Ok(mut cb) => cb.set_text(text).is_ok(),
    Err(_) => false,
  }
}

#[cfg(target_os = "linux")]
fn run_set_clipboard_text(app: &AppHandle, text: &str) -> bool {
  use std::sync::mpsc;
  let (tx, rx) = mpsc::sync_channel::<bool>(1);
  let text_owned = text.to_string();
  let is_url = text_owned.starts_with("http://") || text_owned.starts_with("https://");
  let _ = app.run_on_main_thread(move || {
    let result = if is_url {
      clipboard_writer::write_clipboard_url(&text_owned)
    } else {
      set_clipboard_text_plain(&text_owned)
    };
    let _ = tx.send(result);
  });
  rx.recv_timeout(std::time::Duration::from_millis(1500)).unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn run_set_clipboard_text(app: &AppHandle, text: &str) -> bool {
  let _ = app;
  if text.starts_with("http://") || text.starts_with("https://") {
    clipboard_writer::write_clipboard_url(text)
  } else {
    set_clipboard_text_plain(text)
  }
}

async fn clipboard_read(app: &AppHandle) -> Result<String, String> {
  use std::sync::mpsc;
  let (tx, rx) = mpsc::sync_channel::<Result<String, String>>(1);
  app.run_on_main_thread(move || {
    let result = (|| -> Result<String, String> {
      let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
      clipboard.get_text().map_err(|e| e.to_string())
    })();
    let _ = tx.send(result);
  }).map_err(|e| e.to_string())?;
  match tokio::task::spawn_blocking(move || {
    rx.recv_timeout(std::time::Duration::from_millis(1500))
  }).await {
    Ok(Ok(value)) => value,
    Ok(Err(_)) => Err("Canal cerrado".into()),
    Err(_) => Err("Tiempo de espera agotado".into()),
  }
}

async fn sync_loop(app: AppHandle, request: ConnectRequest, mut stop: oneshot::Receiver<()>) {
  let (stream, _) = match tokio_tungstenite::connect_async(request.endpoint.as_str()).await { Ok(value) => value, Err(error) => { status(&app, "error", format!("No se pudo conectar: {error}")); return; } };
  status(&app, "connected", format!("Conectado a {}", request.endpoint));
  let (mut writer, mut reader) = stream.split();
  let join = Envelope { kind: "join".into(), room: request.room.clone(), sender_id: request.client_id.clone(), text: String::new() };
  if let Ok(value) = serde_json::to_string(&join) { let _ = writer.send(tokio_tungstenite::tungstenite::Message::Text(value.into())).await; }
  let mut timer = tokio::time::interval(POLL_INTERVAL);
  let mut last_seen: Option<String> = None;
  let mut last_files: Vec<ClipboardFile> = Vec::new();
  loop {
    tokio::select! {
      _ = &mut stop => { let _ = writer.send(tokio_tungstenite::tungstenite::Message::Close(None)).await; return; }
      _ = timer.tick() => {
        let (files, text_result) = tokio::join!(
          clipboard_files::read_clipboard_files(app.clone()),
          clipboard_read(&app)
        );
        if !files.is_empty() && files != last_files {
          last_files = files.clone();
          process_shared_files(&app, &files);
          clipboard_files_changed(&app, files);
        } else {
          last_files.clear();
        }
        if let Ok(text) = text_result {
          if last_seen.as_ref() != Some(&text) {
            let packet = Envelope { kind: "clipboard".into(), room: request.room.clone(), sender_id: request.client_id.clone(), text: text.clone() };
            let sent = match serde_json::to_string(&packet) {
              Ok(value) => writer.send(tokio_tungstenite::tungstenite::Message::Text(value.into())).await.is_ok(),
              Err(_) => false,
            };
            if sent {
              last_seen = Some(text.clone());
              clipboard_update(&app, text);
              status(&app, "sent", "Texto enviado al otro dispositivo");
            } else {
              status(&app, "error", "Se perdió la conexión con el relay");
              return;
            }
          }
        }
      },
      incoming = reader.next() => match incoming {
        Some(Ok(message)) if message.is_text() => if let Ok(packet) = serde_json::from_str::<Envelope>(message.to_text().unwrap_or("")) {
          if packet.kind == "join" && packet.room == request.room && packet.sender_id != request.client_id {
            let _ = writer.send(tokio_tungstenite::tungstenite::Message::Text(message.into_text().unwrap_or_default().into())).await;
          } else if packet.kind == "clipboard" && packet.room == request.room && packet.sender_id != request.client_id {
            if run_set_clipboard_text(&app, &packet.text) {
              last_seen = Some(packet.text.clone());
              clipboard_update(&app, packet.text);
              status(&app, "received", "Texto recibido y escrito en el portapapeles");
            }
          }
        },
        Some(Ok(_)) => {},
        Some(Err(error)) => { status(&app, "error", format!("Conexión cerrada: {error}")); return; },
        None => { status(&app, "disconnected", "El relay cerró la conexión"); return; }
      }
    }
  }
}

fn process_shared_files(app: &AppHandle, files: &[ClipboardFile]) {
  let local_files: Vec<&ClipboardFile> = files.iter()
    .filter(|f| !f.path.starts_with("http://") && !f.path.starts_with("https://"))
    .collect();
  if local_files.is_empty() { return; }
  let registry: State<SharedFileRegistry> = app.state();
  let port_state: State<SharedFileServerPort> = app.state();
  let port = match port_state.0.lock() {
    Ok(guard) => match *guard {
      Some(p) => p,
      None => {
        share_failed(app, None, "El servidor de archivos no está listo".into());
        return;
      }
    },
    Err(_) => return,
  };
  let ip = match local_ip_address::local_ip() {
    Ok(value) => value.to_string(),
    Err(error) => {
      share_failed(app, None, format!("No se pudo detectar la IP local: {error}"));
      return;
    }
  };
  registry.clear();
  let mut urls = Vec::new();
  let shared_dir = default_shared_dir();
  for file in local_files {
    match save_shared_file(app, &registry, file, &shared_dir, port, &ip) {
      Ok(url) => urls.push(url),
      Err(error) => share_failed(app, Some(file.name.clone()), error),
    }
  }
  if !urls.is_empty() {
    let body = urls.join("\n");
    let wrote = run_set_clipboard_text(app, &body);
    if wrote {
      status(app, "info", format!("Compartido {} archivo(s). Pegá con Ctrl+V donde quieras.", urls.len()));
    } else {
      share_failed(app, None, "No se pudo escribir la URL en el portapapeles".into());
    }
  }
}

fn save_shared_file(
  app: &AppHandle,
  registry: &SharedFileRegistry,
  file: &ClipboardFile,
  shared_dir: &Path,
  port: u16,
  ip: &str,
) -> Result<String, String> {
  let id = Uuid::new_v4().to_string();
  let name = file.name.clone();
  let size = file.size;
  let mime = file.mime.clone();
  if file.path.starts_with("http://") || file.path.starts_with("https://") {
    return Err("El archivo de origen ya es una URL HTTP; no se puede compartir".into());
  }
  let source = if size < MEMORY_THRESHOLD {
    let bytes = std::fs::read(&file.path).map_err(|e| format!("No se pudo leer {}: {}", file.path, e))?;
    SharedSource::Memory(Arc::new(bytes))
  } else {
    if let Err(error) = std::fs::create_dir_all(shared_dir) {
      return Err(format!("No se pudo crear {}: {}", shared_dir.display(), error));
    }
    let target = shared_dir.join(&id);
    std::fs::copy(&file.path, &target).map_err(|e| format!("No se pudo copiar a {}: {}", target.display(), e))?;
    SharedSource::Disk(target)
  };
  registry.insert(SharedFile { id: id.clone(), name: name.clone(), size, source });
  let url = if name.is_empty() {
    format!("http://{}:{}/clippi/{}", ip, port, id)
  } else {
    let encoded_name = url_encode(&name);
    let encoded_mime = url_encode(&mime);
    format!("http://{}:{}/clippi/{}?n={}&s={}&m={}", ip, port, id, encoded_name, size, encoded_mime)
  };
  let _ = app;
  Ok(url)
}

fn url_encode(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for byte in s.bytes() {
    match byte {
      b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
      _ => out.push_str(&format!("%{:02X}", byte)),
    }
  }
  out
}

#[tauri::command]
fn connect(request: ConnectRequest, app: AppHandle, sync: State<SyncState>) -> Result<(), String> {
  if !request.endpoint.starts_with("ws://") && !request.endpoint.starts_with("wss://") { return Err("La dirección debe comenzar con ws:// o wss://".into()); }
  if request.room.trim().is_empty() { return Err("La sala no puede estar vacía".into()); }
  for stop in sync.0.lock().map_err(|_| "Estado bloqueado")?.drain(..) { let _ = stop.send(()); }
  let (stop_tx, stop_rx) = oneshot::channel();
  sync.0.lock().map_err(|_| "Estado bloqueado")?.push(stop_tx);
  std::thread::spawn(move || {
    let runtime = tokio::runtime::Runtime::new().expect("No se pudo crear el runtime");
    runtime.block_on(sync_loop(app, request, stop_rx));
  });
  Ok(())
}

#[tauri::command]
fn start_relay(request: RelayRequest, app: AppHandle, sync: State<SyncState>) -> Result<(), String> {
  if request.port == 0 { return Err("El puerto debe estar entre 1 y 65535".into()); }
  if request.room.trim().is_empty() { return Err("La sala no puede estar vacía".into()); }
  for stop in sync.0.lock().map_err(|_| "Estado bloqueado")?.drain(..) { let _ = stop.send(()); }
  let (relay_stop_tx, relay_stop_rx) = oneshot::channel();
  let (client_stop_tx, client_stop_rx) = oneshot::channel();
  { let mut stops = sync.0.lock().map_err(|_| "Estado bloqueado")?; stops.push(relay_stop_tx); stops.push(client_stop_tx); }
  std::thread::spawn(move || {
    let runtime = tokio::runtime::Runtime::new().expect("No se pudo crear el runtime");
    runtime.block_on(async move {
      let relay_app = app.clone();
      let relay = relay_loop(relay_app, request.port, relay_stop_rx);
      let client = async {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        sync_loop(app, ConnectRequest { endpoint: format!("ws://127.0.0.1:{}", request.port), room: request.room, client_id: request.client_id }, client_stop_rx).await;
      };
      tokio::join!(relay, client);
    });
  });
  Ok(())
}

#[tauri::command]
fn disconnect(app: AppHandle, sync: State<SyncState>, server_shutdown: State<SharedFileServerShutdown>) -> Result<(), String> {
  for stop in sync.0.lock().map_err(|_| "Estado bloqueado")?.drain(..) { let _ = stop.send(()); }
  if let Ok(mut shutdown) = server_shutdown.0.lock() {
    if let Some(tx) = shutdown.take() { let _ = tx.send(()); }
  }
  status(&app, "disconnected", "Desconectado");
  Ok(())
}

type RelayPeers = Arc<Mutex<Vec<mpsc::UnboundedSender<tokio_tungstenite::tungstenite::Message>>>>;

async fn relay_loop(app: AppHandle, port: u16, mut stop: oneshot::Receiver<()>) {
  let listener = match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
    Ok(listener) => listener,
    Err(error) => { status(&app, "error", format!("No se pudo abrir el puerto {port}: {error}")); return; }
  };
  status(&app, "server_started", format!("Relay activo en el puerto {port}"));
  let peers: RelayPeers = Arc::new(Mutex::new(Vec::new()));
  loop {
    tokio::select! {
      _ = &mut stop => return,
      accepted = listener.accept() => match accepted {
        Ok((stream, _)) => { tokio::spawn(relay_connection(stream, peers.clone())); },
        Err(error) => { status(&app, "error", format!("Error de relay: {error}")); return; }
      }
    }
  }
}

async fn relay_connection(stream: tokio::net::TcpStream, peers: RelayPeers) {
  let websocket = match tokio_tungstenite::accept_async(stream).await { Ok(socket) => socket, Err(_) => return };
  let (mut writer, mut reader) = websocket.split();
  let (tx, mut rx) = mpsc::unbounded_channel();
  if let Ok(mut list) = peers.lock() { list.push(tx); }
  loop {
    tokio::select! {
      outgoing = rx.recv() => match outgoing {
        Some(message) => if writer.send(message).await.is_err() { return; },
        None => return,
      },
      incoming = reader.next() => match incoming {
        Some(Ok(message)) if message.is_text() => {
          let text = message.into_text().unwrap_or_default().to_string();
          let kind = serde_json::from_str::<Envelope>(&text).map(|p| p.kind).unwrap_or_default();
          if matches!(kind.as_str(), "join" | "clipboard") {
            let outbound = tokio_tungstenite::tungstenite::Message::Text(text.into());
            if let Ok(mut list) = peers.lock() { list.retain(|peer| peer.send(outbound.clone()).is_ok()); }
          }
        },
        Some(Ok(_)) => {},
        _ => return
      }
    }
  }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let shared_registry = SharedArc::new(SharedFileRegistry::new());
  let _ = std::fs::create_dir_all(default_shared_dir());

  let registry_for_setup = shared_registry.clone();
  let (server_port_tx, server_port_rx) = oneshot::channel::<u16>();
  let (server_shutdown_tx, server_shutdown_rx) = oneshot::channel::<()>();

  tauri::Builder::default()
    .manage(SyncState(Mutex::new(Vec::new())))
    .manage(shared_registry)
    .manage(SharedFileServerPort(Mutex::new(None)))
    .manage(SharedFileServerShutdown(Mutex::new(Some(server_shutdown_tx))))
    .invoke_handler(tauri::generate_handler![connect, start_relay, disconnect])
    .on_window_event(|window, event| {
      if let WindowEvent::CloseRequested { api, .. } = event {
        window.hide().ok();
        api.prevent_close();
      }
    })
    .setup(move |app| {
      let show_hide = MenuItem::with_id(app, "show_hide", "Mostrar/Ocultar", true, None::<&str>)?;
      let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
      let menu = Menu::with_items(app, &[&show_hide, &quit])?;
      TrayIconBuilder::new()
        .icon(app.default_window_icon().cloned().expect("falta el icono de la app"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
          "show_hide" => {
            if let Some(window) = app.get_webview_window("main") {
              if window.is_visible().unwrap_or(false) { window.hide().ok(); } else { window.show().ok(); window.set_focus().ok(); }
            }
          }
          "quit" => app.exit(0),
          _ => {}
        })
        .on_tray_icon_event(|tray, event| {
          if let TrayIconEvent::Click { button: tauri::tray::MouseButton::Left, button_state: tauri::tray::MouseButtonState::Up, .. } = event {
            let app = tray.app_handle();
            if let Some(window) = app.get_webview_window("main") {
              if window.is_visible().unwrap_or(false) { window.hide().ok(); } else { window.show().ok(); window.set_focus().ok(); }
            }
          }
        })
        .build(app)?;

      let registry = registry_for_setup.clone();
      std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("No se pudo crear el runtime");
        runtime.block_on(async move {
          if let Err(error) = file_server::run_file_server(registry, server_port_tx, server_shutdown_rx).await {
            let _ = error;
          }
        });
      });
      let app_handle = app.handle().clone();
      std::thread::spawn(move || {
        if let Ok(port) = server_port_rx.blocking_recv() {
          let state: State<SharedFileServerPort> = app_handle.state();
          {
            let Ok(mut guard) = state.0.lock() else { return };
            *guard = Some(port);
          };
        }
      });

      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error al ejecutar la aplicación");
}