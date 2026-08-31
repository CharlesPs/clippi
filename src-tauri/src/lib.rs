use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tokio::sync::{mpsc, oneshot};

struct SyncState(Mutex<Vec<oneshot::Sender<()>>>);

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ConnectRequest { endpoint: String, room: String, client_id: String }

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayRequest { port: u16, room: String, client_id: String }

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipboardMessage { #[serde(rename = "type")] kind: String, room: String, sender_id: String, text: String }

#[derive(Clone, Serialize)]
struct Status { state: &'static str, detail: String }

#[derive(Clone, Serialize)]
struct ClipboardUpdate { text: String }

fn status(app: &AppHandle, state: &'static str, detail: impl Into<String>) {
  let _ = app.emit("sync-status", Status { state, detail: detail.into() });
}

fn clipboard_update(app: &AppHandle, text: String) {
  let _ = app.emit("clipboard-update", ClipboardUpdate { text });
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
fn disconnect(app: AppHandle, sync: State<SyncState>) -> Result<(), String> {
  for stop in sync.0.lock().map_err(|_| "Estado bloqueado")?.drain(..) { let _ = stop.send(()); }
  status(&app, "disconnected", "Desconectado"); Ok(())
}

async fn clipboard_read() -> Result<String, String> {
  tokio::task::spawn_blocking(|| {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.get_text().map_err(|e| e.to_string())
  }).await.map_err(|e| e.to_string())?
}
async fn clipboard_write(text: String) -> Result<(), String> {
  tokio::task::spawn_blocking(move || {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())
  }).await.map_err(|e| e.to_string())?
}

async fn sync_loop(app: AppHandle, request: ConnectRequest, mut stop: oneshot::Receiver<()>) {
  let (stream, _) = match tokio_tungstenite::connect_async(request.endpoint.as_str()).await { Ok(value) => value, Err(error) => { status(&app, "error", format!("No se pudo conectar: {error}")); return; } };
  status(&app, "connected", format!("Conectado a {}", request.endpoint));
  let (mut writer, mut reader) = stream.split();
  let join = ClipboardMessage { kind: "join".into(), room: request.room.clone(), sender_id: request.client_id.clone(), text: String::new() };
  if let Ok(value) = serde_json::to_string(&join) { let _ = writer.send(tokio_tungstenite::tungstenite::Message::Text(value.into())).await; }
  let mut timer = tokio::time::interval(std::time::Duration::from_millis(400));
  let mut last_seen: Option<String> = None;
  loop {
    tokio::select! {
      _ = &mut stop => { let _ = writer.send(tokio_tungstenite::tungstenite::Message::Close(None)).await; return; }
      _ = timer.tick() => match clipboard_read().await {
        Ok(text) if last_seen.as_ref() != Some(&text) => {
          let packet = ClipboardMessage { kind: "clipboard".into(), room: request.room.clone(), sender_id: request.client_id.clone(), text: text.clone() };
          let sent = match serde_json::to_string(&packet) {
            Ok(value) => writer.send(tokio_tungstenite::tungstenite::Message::Text(value.into())).await.is_ok(),
            Err(_) => false,
          };
          if sent { last_seen = Some(text.clone()); clipboard_update(&app, text); status(&app, "sent", "Texto enviado al otro dispositivo"); } else { status(&app, "error", "Se perdió la conexión con el relay"); return; }
        }, _ => {}
      },
      incoming = reader.next() => match incoming {
        Some(Ok(message)) if message.is_text() => if let Ok(packet) = serde_json::from_str::<ClipboardMessage>(message.to_text().unwrap_or("")) {
          if packet.kind == "clipboard" && packet.room == request.room && packet.sender_id != request.client_id {
            if clipboard_write(packet.text.clone()).await.is_ok() { last_seen = Some(packet.text.clone()); clipboard_update(&app, packet.text); status(&app, "received", "Texto recibido y escrito en el portapapeles"); }
          }
        },
        Some(Ok(_)) => {}, Some(Err(error)) => { status(&app, "error", format!("Conexión cerrada: {error}")); return; }, None => { status(&app, "disconnected", "El relay cerró la conexión"); return; }
      }
    }
  }
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
          if serde_json::from_str::<ClipboardMessage>(&text).map(|packet| packet.kind == "clipboard").unwrap_or(false) {
            let outbound = tokio_tungstenite::tungstenite::Message::Text(text.into());
            if let Ok(mut list) = peers.lock() { list.retain(|peer| peer.send(outbound.clone()).is_ok()); }
          }
        },
        Some(Ok(_)) => {}, _ => return
      }
    }
  }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .manage(SyncState(Mutex::new(Vec::new())))
    .invoke_handler(tauri::generate_handler![connect, start_relay, disconnect])
    .on_window_event(|window, event| {
      // Oculta la ventana en vez de cerrar el proceso; se maneja desde el tray.
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
