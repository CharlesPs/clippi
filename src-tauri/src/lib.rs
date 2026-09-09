use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

mod clipboard_files;
mod clipboard_writer;

const CHUNK_SIZE: usize = 64 * 1024;
const MAX_FILE_SIZE: u64 = 8 * 1024 * 1024 * 1024;

struct SyncState(Mutex<Vec<oneshot::Sender<()>>>);

#[derive(Clone)]
enum SyncCommand { StartFiles(Vec<String>) }
struct SyncCommandSender(Mutex<Option<mpsc::UnboundedSender<SyncCommand>>>);

struct SendCancel(Mutex<bool>);

struct SelfWrittenClipboard(Mutex<HashMap<String, std::time::Instant>>);

struct ReceiveState(Mutex<HashMap<String, ReceiveTransfer>>);
struct ReceiveTransfer { name: String, size: u64, received: u64, path: PathBuf, file: fs::File }

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ConnectRequest { endpoint: String, room: String, client_id: String }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayRequest { port: u16, room: String, client_id: String }

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Envelope { #[serde(rename = "type")] kind: String, room: String, sender_id: String, text: String, #[serde(default, skip_serializing_if = "Vec::is_empty")] features: Vec<String> }

const LOCAL_FEATURES: [&str; 2] = ["text", "files"];

#[derive(Serialize, Deserialize, Clone)]
struct FileMetadata { id: String, name: String, size: u64, mime: String }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileReceived { id: String, name: String, path: String }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileFinalized { id: String, name: String, direction: &'static str }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileFailed { id: String, name: Option<String>, direction: &'static str, error: String }

#[derive(Clone, Serialize)]
struct Status { state: &'static str, detail: String }

#[derive(Clone, Serialize)]
struct ClipboardUpdate { text: String }

#[derive(Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ClipboardFile { path: String, name: String, size: u64, mime: String }

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardFilesEvent { files: Vec<ClipboardFile> }

fn status(app: &AppHandle, state: &'static str, detail: impl Into<String>) {
  let _ = app.emit("sync-status", Status { state, detail: detail.into() });
}
fn clipboard_update(app: &AppHandle, text: String) {
  let _ = app.emit("clipboard-update", ClipboardUpdate { text });
}
fn text_sent(app: &AppHandle, text: String) { let _ = app.emit("text-sent", ClipboardUpdate { text }); }
fn text_received(app: &AppHandle, text: String) { let _ = app.emit("text-received", ClipboardUpdate { text }); }
fn clipboard_files_changed(app: &AppHandle, files: Vec<ClipboardFile>) {
  let _ = app.emit("clipboard-files", ClipboardFilesEvent { files });
}
fn file_received(app: &AppHandle, payload: FileReceived) { let _ = app.emit("file-received", payload); }
fn file_sent(app: &AppHandle, payload: FileFinalized) { let _ = app.emit("file-sent", payload); }
fn file_failed(app: &AppHandle, payload: FileFailed) { let _ = app.emit("file-failed", payload); }

fn default_download_dir() -> PathBuf {
  let base = dirs::desktop_dir()
    .or_else(dirs::home_dir)
    .unwrap_or_else(|| PathBuf::from("."));
  base.join(".clippi").join("received")
}

fn sanitize_filename(name: &str) -> String {
  let trimmed = name.trim();
  let stripped = trimmed.replace(['/', '\\', ':', '\0'], "_");
  let cleaned: String = stripped.chars().filter(|c| !c.is_control()).collect();
  let no_leading_dots = cleaned.trim_start_matches('.');
  let final_name = no_leading_dots.trim();
  if final_name.is_empty() { "archivo-sin-nombre".into() } else { final_name.to_string() }
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
  let candidate = dir.join(name);
  if !candidate.exists() { return candidate; }
  let path = Path::new(name);
  let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("archivo");
  let ext = path.extension().and_then(|s| s.to_str());
  for index in 1..=999 {
    let next = match ext {
      Some(value) => format!("{stem}-{index}.{value}"),
      None => format!("{stem}-{index}"),
    };
    let attempt = dir.join(next);
    if !attempt.exists() { return attempt; }
  }
  candidate
}

#[tauri::command]
fn connect(request: ConnectRequest, app: AppHandle, sync: State<SyncState>, cmd: State<SyncCommandSender>) -> Result<(), String> {
  if !request.endpoint.starts_with("ws://") && !request.endpoint.starts_with("wss://") { return Err("La dirección debe comenzar con ws:// o wss://".into()); }
  if request.room.trim().is_empty() { return Err("La sala no puede estar vacía".into()); }
  for stop in sync.0.lock().map_err(|_| "Estado bloqueado")?.drain(..) { let _ = stop.send(()); }
  let (stop_tx, stop_rx) = oneshot::channel();
  sync.0.lock().map_err(|_| "Estado bloqueado")?.push(stop_tx);
  let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
  *cmd.0.lock().map_err(|_| "Estado bloqueado")? = Some(cmd_tx);
  std::thread::spawn(move || {
    let runtime = tokio::runtime::Runtime::new().expect("No se pudo crear el runtime");
    runtime.block_on(sync_loop(app, request, stop_rx, cmd_rx));
  });
  Ok(())
}

#[tauri::command]
fn start_relay(request: RelayRequest, app: AppHandle, sync: State<SyncState>, cmd: State<SyncCommandSender>) -> Result<(), String> {
  if request.port == 0 { return Err("El puerto debe estar entre 1 y 65535".into()); }
  if request.room.trim().is_empty() { return Err("La sala no puede estar vacía".into()); }
  for stop in sync.0.lock().map_err(|_| "Estado bloqueado")?.drain(..) { let _ = stop.send(()); }
  let (relay_stop_tx, relay_stop_rx) = oneshot::channel();
  let (client_stop_tx, client_stop_rx) = oneshot::channel();
  { let mut stops = sync.0.lock().map_err(|_| "Estado bloqueado")?; stops.push(relay_stop_tx); stops.push(client_stop_tx); }
  let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
  *cmd.0.lock().map_err(|_| "Estado bloqueado")? = Some(cmd_tx);
  std::thread::spawn(move || {
    let runtime = tokio::runtime::Runtime::new().expect("No se pudo crear el runtime");
    runtime.block_on(async move {
      let relay_app = app.clone();
      let relay = relay_loop(relay_app, request.port, relay_stop_rx);
      let client = async {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        sync_loop(app, ConnectRequest { endpoint: format!("ws://127.0.0.1:{}", request.port), room: request.room, client_id: request.client_id }, client_stop_rx, cmd_rx).await;
      };
      tokio::join!(relay, client);
    });
  });
  Ok(())
}

#[tauri::command]
fn disconnect(app: AppHandle, sync: State<SyncState>, cmd: State<SyncCommandSender>) -> Result<(), String> {
  for stop in sync.0.lock().map_err(|_| "Estado bloqueado")?.drain(..) { let _ = stop.send(()); }
  *cmd.0.lock().map_err(|_| "Estado bloqueado")? = None;
  status(&app, "disconnected", "Desconectado");
  Ok(())
}

#[tauri::command]
fn send_files(paths: Vec<String>, cmd: State<SyncCommandSender>) -> Result<(), String> {
  let sender = cmd.0.lock().map_err(|_| "Estado bloqueado")?.clone();
  match sender {
    Some(channel) => channel.send(SyncCommand::StartFiles(paths)).map_err(|_| "No hay sesión activa".to_string()),
    None => Err("Conecta antes de enviar archivos".into()),
  }
}

#[tauri::command]
fn cancel_file(cancel: State<SendCancel>) -> Result<(), String> {
  let mut flag = cancel.0.lock().map_err(|_| "Estado bloqueado")?;
  *flag = true;
  Ok(())
}

#[tauri::command]
fn current_download_dir() -> Result<String, String> {
  Ok(default_download_dir().to_string_lossy().into_owned())
}

#[tauri::command]
async fn paste_received_file(path: String, app: AppHandle) -> Result<(), String> {
  let trimmed = path.trim();
  if trimmed.is_empty() { return Err("Ruta vacía".into()); }
  let names = vec![trimmed.to_string()];
  let wrote = {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::sync_channel::<bool>(1);
    let names_for_main = names.clone();
    let dispatched = app.run_on_main_thread(move || {
      let _ = tx.send(clipboard_writer::write_clipboard_uris(&names_for_main));
    });
    if dispatched.is_err() { return Err("No se pudo escribir en el portapapeles".into()); }
    match tokio::task::spawn_blocking(move || {
      rx.recv_timeout(std::time::Duration::from_millis(1500)).unwrap_or(false)
    }).await {
      Ok(value) => value,
      Err(_) => false,
    }
  };
  if wrote {
    let state: State<SelfWrittenClipboard> = app.state();
    if let Ok(mut recent) = state.0.lock() {
      recent.insert(trimmed.to_string(), std::time::Instant::now());
    };
    Ok(())
  } else {
    Err("No se pudo publicar el archivo en el portapapeles".into())
  }
}

async fn clipboard_read(app: &AppHandle) -> Result<String, String> {
  use std::sync::mpsc;
  let (tx, rx) = mpsc::sync_channel::<Result<String, String>>(1);
  let app_clone = app.clone();
  app.run_on_main_thread(move || {
    let _ = app_clone;
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
async fn clipboard_write(app: &AppHandle, text: String) -> Result<(), String> {
  use std::sync::mpsc;
  let (tx, rx) = mpsc::sync_channel::<Result<(), String>>(1);
  let app_clone = app.clone();
  app.run_on_main_thread(move || {
    let _ = app_clone;
    let result = (|| -> Result<(), String> {
      let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
      clipboard.set_text(text).map_err(|e| e.to_string())
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

async fn sync_loop(app: AppHandle, request: ConnectRequest, mut stop: oneshot::Receiver<()>, mut commands: mpsc::UnboundedReceiver<SyncCommand>) {
  let (stream, _) = match tokio_tungstenite::connect_async(request.endpoint.as_str()).await { Ok(value) => value, Err(error) => { status(&app, "error", format!("No se pudo conectar: {error}")); return; } };
  status(&app, "connected", format!("Conectado a {}", request.endpoint));
  let (mut writer, mut reader) = stream.split();
  let join = Envelope { kind: "join".into(), room: request.room.clone(), sender_id: request.client_id.clone(), text: String::new(), features: LOCAL_FEATURES.iter().map(|value| value.to_string()).collect() };
  if let Ok(value) = serde_json::to_string(&join) { let _ = writer.send(tokio_tungstenite::tungstenite::Message::Text(value.into())).await; }
  let mut timer = tokio::time::interval(std::time::Duration::from_millis(1500));
  let mut last_files: Vec<ClipboardFile> = Vec::new();
  let mut last_seen: Option<String> = None;
  let mut peer_features: HashMap<String, (String, Vec<String>)> = HashMap::new();
  loop {
    tokio::select! {
      _ = &mut stop => { let _ = writer.send(tokio_tungstenite::tungstenite::Message::Close(None)).await; abort_all_receives(&app); return; }
      _ = timer.tick() => {
        let (files, text_result) = tokio::join!(
          clipboard_files::read_clipboard_files(app.clone()),
          clipboard_read(&app)
        );
        if !files.is_empty() {
          last_seen = None;
          if files != last_files {
            let auto_send = !recently_self_written(&app, &files);
            last_files = files.clone();
            clipboard_files_changed(&app, files.clone());
            if auto_send {
              let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
              let cmd_state: State<SyncCommandSender> = app.state();
              if let Ok(sender_opt) = cmd_state.0.lock() {
                if let Some(channel) = sender_opt.as_ref() {
                  let _ = channel.send(SyncCommand::StartFiles(paths));
                };
              };
            }
          }
        } else {
          last_files.clear();
          if let Ok(text) = text_result {
            if last_seen.as_ref() != Some(&text) {
              let packet = Envelope { kind: "clipboard".into(), room: request.room.clone(), sender_id: request.client_id.clone(), text: text.clone(), features: Vec::new() };
              let sent = match serde_json::to_string(&packet) {
                Ok(value) => writer.send(tokio_tungstenite::tungstenite::Message::Text(value.into())).await.is_ok(),
                Err(_) => false,
              };
              if sent { last_seen = Some(text.clone()); clipboard_update(&app, text.clone()); text_sent(&app, text); status(&app, "sent", "Texto enviado al otro dispositivo"); } else { status(&app, "error", "Se perdió la conexión con el relay"); return; }
            }
          }
        }
      },
      command = commands.recv() => match command {
        Some(SyncCommand::StartFiles(paths)) => {
          {
            let cancel: State<SendCancel> = app.state();
            if let Ok(mut flag) = cancel.0.lock() { *flag = false; };
          }
          let fallback = peer_features.iter().any(|(_, (room, features))| room == &request.room && !features.iter().any(|feature| feature == "files"));
          if fallback {
            if !send_filenames_only(&app, &mut writer, &request.room, &request.client_id, &paths).await { status(&app, "error", "Se perdió la conexión con el relay"); return; }
          } else {
            for path in paths {
              if !send_single_file(&app, &mut writer, &request.room, &request.client_id, &path).await { status(&app, "error", "Se perdió la conexión con el relay"); return; }
            }
          }
        }
        None => {}
      },
      incoming = reader.next() => match incoming {
        Some(Ok(message)) if message.is_text() => if let Ok(packet) = serde_json::from_str::<Envelope>(message.to_text().unwrap_or("")) {
          if packet.kind == "join" && packet.room == request.room && packet.sender_id != request.client_id {
            peer_features.insert(packet.sender_id.clone(), (packet.room.clone(), packet.features.clone()));
          } else if packet.kind == "clipboard" && packet.room == request.room && packet.sender_id != request.client_id {
            if clipboard_write(&app, packet.text.clone()).await.is_ok() { last_seen = Some(packet.text.clone()); clipboard_update(&app, packet.text.clone()); text_received(&app, packet.text); status(&app, "received", "Texto recibido y escrito en el portapapeles"); }
          } else if packet.kind == "file_start" && packet.room == request.room && packet.sender_id != request.client_id {
            handle_file_start(&app, &packet.text);
          } else if packet.kind == "file_done" && packet.room == request.room && packet.sender_id != request.client_id {
            handle_file_done(&app, &packet.text);
          } else if packet.kind == "file_cancel" && packet.room == request.room && packet.sender_id != request.client_id {
            handle_file_cancel(&app, &packet.text);
          }
        },
        Some(Ok(message)) if message.is_binary() => {
          let bytes = message.into_data();
          if let Some((id, offset, data)) = parse_chunk(&bytes) {
            handle_chunk(&app, &id, offset, data);
          }
        },
        Some(Ok(_)) => {}, Some(Err(error)) => { status(&app, "error", format!("Conexión cerrada: {error}")); return; }, None => { status(&app, "disconnected", "El relay cerró la conexión"); return; }
      }
    }
  }
}

async fn send_single_file<W>(app: &AppHandle, writer: &mut W, room: &str, sender_id: &str, path: &str) -> bool
where W: SinkExt<tokio_tungstenite::tungstenite::Message> + Unpin,
{
  let path_buf = PathBuf::from(path);
  let file_name = match path_buf.file_name().and_then(|s| s.to_str()) { Some(value) => value.to_string(), None => { file_failed(app, FileFailed { id: String::new(), name: None, direction: "send", error: "Nombre de archivo no válido".into() }); return true; } };
  let metadata = match fs::metadata(&path_buf) { Ok(value) => value, Err(error) => { file_failed(app, FileFailed { id: String::new(), name: Some(file_name), direction: "send", error: format!("No se pudo leer el archivo: {error}") }); return true; } };
  let size = metadata.len();
  if size > MAX_FILE_SIZE { file_failed(app, FileFailed { id: String::new(), name: Some(file_name), direction: "send", error: "El archivo supera el límite de 8 GB".into() }); return true; }
  let id = Uuid::new_v4().to_string();
  let mime = "application/octet-stream".to_string();
  let metadata_payload = serde_json::to_string(&FileMetadata { id: id.clone(), name: file_name.clone(), size, mime }).unwrap_or_default();
  let envelope = Envelope { kind: "file_start".into(), room: room.into(), sender_id: sender_id.into(), text: metadata_payload, features: Vec::new() };
  if writer.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&envelope).unwrap_or_default().into())).await.is_err() { return false; }
  let mut file = match fs::File::open(&path_buf) { Ok(value) => value, Err(error) => { send_cancel_envelope(&id, room, sender_id, writer).await; file_failed(app, FileFailed { id, name: Some(file_name), direction: "send", error: format!("No se pudo abrir: {error}") }); return true; } };
  let mut buffer = vec![0u8; CHUNK_SIZE];
  let mut offset: u64 = 0;
  loop {
    if cancellation_requested(app) {
      send_cancel_envelope(&id, room, sender_id, writer).await;
      file_failed(app, FileFailed { id, name: Some(file_name), direction: "send", error: "Cancelado por el usuario".into() });
      return true;
    }
    let read = match file.read(&mut buffer) { Ok(value) => value, Err(error) => { send_cancel_envelope(&id, room, sender_id, writer).await; file_failed(app, FileFailed { id, name: Some(file_name), direction: "send", error: format!("Error de lectura: {error}") }); return true; } };
    if read == 0 { break; }
    let mut frame = Vec::with_capacity(24 + read);
    let uuid = Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil());
    frame.extend_from_slice(uuid.as_bytes());
    frame.extend_from_slice(&offset.to_be_bytes());
    frame.extend_from_slice(&buffer[..read]);
    if writer.send(tokio_tungstenite::tungstenite::Message::Binary(frame.into())).await.is_err() { return false; }
    offset += read as u64;
  }
  let done = Envelope { kind: "file_done".into(), room: room.into(), sender_id: sender_id.into(), text: id.clone(), features: Vec::new() };
  if writer.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&done).unwrap_or_default().into())).await.is_err() { return false; }
  file_sent(app, FileFinalized { id, name: file_name, direction: "send" });
  true
}

fn cancellation_requested(app: &AppHandle) -> bool {
  let cancel: State<SendCancel> = app.state();
  cancel.0.lock().map(|flag| *flag).unwrap_or(false)
}

async fn send_cancel_envelope<W>(id: &str, room: &str, sender_id: &str, writer: &mut W) -> bool
where W: SinkExt<tokio_tungstenite::tungstenite::Message> + Unpin,
{
  let envelope = Envelope { kind: "file_cancel".into(), room: room.into(), sender_id: sender_id.into(), text: id.to_string(), features: Vec::new() };
  writer.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::to_string(&envelope).unwrap_or_default().into())).await.is_err()
}

async fn send_filenames_only<W>(app: &AppHandle, writer: &mut W, room: &str, sender_id: &str, paths: &[String]) -> bool
where W: SinkExt<tokio_tungstenite::tungstenite::Message> + Unpin,
{
  let basenames: Vec<String> = paths.iter()
    .filter_map(|path| PathBuf::from(path).file_name().and_then(|s| s.to_str()).map(|s| s.to_string()))
    .filter(|name| !name.is_empty())
    .collect();
  if basenames.is_empty() {
    file_failed(app, FileFailed { id: String::new(), name: None, direction: "send", error: "No se pudo obtener el nombre de los archivos".into() });
    return true;
  }
  let combined = basenames.join("\n");
  let envelope = Envelope { kind: "clipboard".into(), room: room.into(), sender_id: sender_id.into(), text: combined.clone(), features: Vec::new() };
  let serialized = match serde_json::to_string(&envelope) { Ok(value) => value, Err(_) => return true };
  if writer.send(tokio_tungstenite::tungstenite::Message::Text(serialized.into())).await.is_err() { return false; }
  status(app, "sent", format!("El receptor no soporta archivos: se envió solo el nombre ({})", combined.replace('\n', ", ")));
  true
}

fn handle_file_start(app: &AppHandle, payload: &str) {
  let metadata: FileMetadata = match serde_json::from_str(payload) { Ok(value) => value, Err(_) => return };
  let id = metadata.id.clone();
  let dir = default_download_dir();
  if let Err(error) = fs::create_dir_all(&dir) {
    file_failed(app, FileFailed { id, name: Some(metadata.name.clone()), direction: "recv", error: format!("No se pudo crear la carpeta: {error}") });
    return;
  }
  let name = sanitize_filename(&metadata.name);
  let final_path = unique_path(&dir, &name);
  let file = match fs::OpenOptions::new().create(true).truncate(true).write(true).open(&final_path) {
    Ok(value) => value,
    Err(error) => {
      file_failed(app, FileFailed { id, name: Some(metadata.name.clone()), direction: "recv", error: format!("No se pudo abrir el archivo: {error}") });
      return;
    }
  };
  let transfer = ReceiveTransfer { name: metadata.name.clone(), size: metadata.size, received: 0, path: final_path, file };
  {
    let state: State<ReceiveState> = app.state();
    if let Ok(mut active) = state.0.lock() {
      active.insert(id.clone(), transfer);
    };
  }
}

fn handle_chunk(app: &AppHandle, id: &str, offset: u64, data: Vec<u8>) {
  struct Failure { name: String, error: String }
  let outcome: Option<Result<(), Failure>> = (|| {
    let state: State<ReceiveState> = app.state();
    let mut active = state.0.lock().ok()?;
    let transfer = active.get_mut(id)?;
    if offset != transfer.received { return Some(Ok(())); }
    if let Err(error) = transfer.file.write_all(&data) {
      let failed_name = transfer.name.clone();
      active.remove(id);
      return Some(Err(Failure { name: failed_name, error: format!("No se pudo escribir el archivo: {error}") }));
    }
    transfer.received += data.len() as u64;
    Some(Ok(()))
  })();
  match outcome {
    Some(Err(failure)) => file_failed(app, FileFailed { id: id.to_string(), name: Some(failure.name), direction: "recv", error: failure.error }),
    _ => {}
  }
}

fn handle_file_done(app: &AppHandle, id: &str) {
  let app_clone = app.clone();
  let id_owned = id.to_string();
  tokio::spawn(async move {
    handle_file_done_async(&app_clone, &id_owned).await;
  });
}

async fn handle_file_done_async(app: &AppHandle, id: &str) {
  let removed: Option<ReceiveTransfer> = {
    let state: State<ReceiveState> = app.state();
    let Ok(mut active) = state.0.lock() else { return };
    active.remove(id)
  };
  let Some(mut transfer) = removed else { return };
  if let Err(error) = transfer.file.flush() {
    file_failed(app, FileFailed { id: id.to_string(), name: Some(transfer.name.clone()), direction: "recv", error: format!("No se pudo cerrar el archivo: {error}") });
    let _ = fs::remove_file(&transfer.path);
    return;
  }
  if transfer.received != transfer.size {
    file_failed(app, FileFailed { id: id.to_string(), name: Some(transfer.name.clone()), direction: "recv", error: "Tamaño incompleto".into() });
    let _ = fs::remove_file(&transfer.path);
    return;
  }
  let final_path = transfer.path.to_string_lossy().into_owned();
  let names = vec![final_path.clone()];
  let wrote = {
    use std::sync::mpsc;
    let (tx, rx) = mpsc::sync_channel::<bool>(1);
    let names_for_main = names.clone();
    let _ = app.run_on_main_thread(move || {
      let _ = tx.send(clipboard_writer::write_clipboard_uris(&names_for_main));
    });
    match tokio::task::spawn_blocking(move || {
      rx.recv_timeout(std::time::Duration::from_millis(1500)).unwrap_or(false)
    }).await {
      Ok(value) => value,
      Err(_) => false,
    }
  };
  if wrote {
    let state: State<SelfWrittenClipboard> = app.state();
    if let Ok(mut recent) = state.0.lock() {
      recent.insert(final_path.clone(), std::time::Instant::now());
    };
  }
  file_received(app, FileReceived { id: id.to_string(), name: transfer.name.clone(), path: final_path });
}

fn handle_file_cancel(app: &AppHandle, id: &str) {
  let removed: Option<ReceiveTransfer> = {
    let state: State<ReceiveState> = app.state();
    let Ok(mut active) = state.0.lock() else { return };
    active.remove(id)
  };
  if let Some(transfer) = removed { let _ = fs::remove_file(&transfer.path); }
}

fn abort_all_receives(app: &AppHandle) {
  let pending: Vec<(String, PathBuf)> = {
    let state: State<ReceiveState> = app.state();
    let Ok(mut active) = state.0.lock() else { return };
    active.drain().map(|(id, transfer)| (id, transfer.path)).collect()
  };
  for (_, path) in pending { let _ = fs::remove_file(&path); }
}

fn recently_self_written(app: &AppHandle, files: &[ClipboardFile]) -> bool {
  if files.is_empty() { return false; }
  let state: State<SelfWrittenClipboard> = app.state();
  let now = std::time::Instant::now();
  let window = std::time::Duration::from_secs(30);
  let mut all_recent = true;
  {
    let Ok(mut recent) = state.0.lock() else { return false };
    for file in files {
      match recent.get(&file.path) {
        Some(timestamp) if now.duration_since(*timestamp) < window => {}
        _ => { all_recent = false; break; }
      }
    }
    if all_recent {
      for file in files {
        recent.remove(&file.path);
      }
    }
  };
  all_recent
}

fn parse_chunk(bytes: &[u8]) -> Option<(String, u64, Vec<u8>)> {
  if bytes.len() < 24 { return None; }
  let id_bytes: [u8; 16] = bytes[0..16].try_into().ok()?;
  let id = Uuid::from_bytes(id_bytes).to_string();
  let offset = u64::from_be_bytes(bytes[16..24].try_into().ok()?);
  Some((id, offset, bytes[24..].to_vec()))
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
          if matches!(kind.as_str(), "clipboard" | "join" | "file_start" | "file_done" | "file_cancel") {
            let outbound = tokio_tungstenite::tungstenite::Message::Text(text.into());
            if let Ok(mut list) = peers.lock() { list.retain(|peer| peer.send(outbound.clone()).is_ok()); }
          }
        },
        Some(Ok(message)) if message.is_binary() => {
          let outbound = tokio_tungstenite::tungstenite::Message::Binary(message.into_data());
          if let Ok(mut list) = peers.lock() { list.retain(|peer| peer.send(outbound.clone()).is_ok()); }
        },
        Some(Ok(_)) => {}, _ => return
      }
    }
  }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  let _ = fs::create_dir_all(default_download_dir());
  tauri::Builder::default()
    .manage(SyncState(Mutex::new(Vec::new())))
    .manage(SyncCommandSender(Mutex::new(None)))
    .manage(SendCancel(Mutex::new(false)))
    .manage(SelfWrittenClipboard(Mutex::new(HashMap::new())))
    .manage(ReceiveState(Mutex::new(HashMap::new())))
    .invoke_handler(tauri::generate_handler![connect, start_relay, disconnect, send_files, cancel_file, current_download_dir, paste_received_file])
    .on_window_event(|window, event| {
      if let WindowEvent::CloseRequested { api, .. } = event {
        window.hide().ok();
        api.prevent_close();
      }
    })
    .setup(|app| {
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
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error al ejecutar la aplicación");
}