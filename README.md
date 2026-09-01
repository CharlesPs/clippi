# Clipboard Sync (MVP LAN)

Sincroniza **texto del portapapeles** y **archivos** entre Ubuntu (Wayland) y macOS mediante un relay WebSocket dentro de la misma red. El cliente Tauri usa Rust para observar el portapapeles y transferir archivos; la interfaz sólo dispara acciones (conectar, elegir archivos, elegir carpeta).

## Ejecutar el cliente

Antes de compilar, instala Rust. En Ubuntu también se requieren las bibliotecas de Tauri; en macOS, las Command Line Tools de Xcode:

```bash
# Ubuntu 26.04
sudo apt update
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh

# macOS (en Terminal)
xcode-select --install
curl --proto '=https' --tlsv1.2 https://sh.rustup.rs -sSf | sh
```

Reabre la terminal tras instalar Rust y ejecuta en cada equipo:

```bash
npm install
npm run tauri dev
```

## Elegir servidor o cliente

Cada instalación puede asumir cualquiera de los dos papeles:

1. En un equipo, selecciona **Servidor**, conserva el puerto `8787` e inicia el relay. Permite el puerto TCP `8787` en su cortafuegos.
3. Averigua la IP LAN de ese equipo, por ejemplo `192.168.1.20`.
4. En cada otro equipo, selecciona **Cliente** y usa `ws://192.168.1.20:8787`. Escribe la misma sala en todos los clientes que deban sincronizarse.

El relay se ejecuta dentro de la aplicación Tauri. El script `npm run relay` se conserva únicamente como alternativa de desarrollo sin interfaz.

En Ubuntu/Wayland, ejecuta la app dentro de tu sesión gráfica normal; el proceso necesita las variables de sesión (`WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`). En macOS, el primer acceso puede requerir conceder el permiso de portapapeles que indique el sistema.

## Texto

1. Abre la app en Ubuntu y macOS.
2. Inicia el servidor en una de ellas y conecta la otra como cliente usando su IP LAN.
3. Configura la misma sala en los clientes (por ejemplo `prueba-casa-42`).
4. Copia texto en un cliente y pégalo en el otro.

## Archivos

Además del texto, puedes pasar archivos entre los dos dispositivos:

- **Enviar**: pulsa **Enviar archivos…** y elige uno o varios, o arrástralos directamente sobre la ventana.
- **Selección del portapapeles**: cuando copias archivos en Nautilus (Ctrl+C) — u otro explorador que use el portapapeles estándar — aparecen listados automáticamente en la sección **Selección del portapapeles**. Pulsa **Enviar selección** para transmitirlos sin tener que volver a elegirlos.
- **Recibidos**: se guardan en la carpeta indicada en **Carpeta de recibidos** (por defecto `~/Desktop/clippi`). Cambia la carpeta con el botón **Elegir…** o escribe la ruta directamente.
- Si el nombre ya existe, se añade un sufijo numérico (`archivo-1.ext`, `archivo-2.ext`).
- La barra de progreso muestra el avance de cada transferencia; la lista **Últimos recibidos** mantiene los 20 últimos con su ruta.

El límite por archivo es 8 GB. La transferencia se divide en bloques de 64 KB; los archivos muy grandes tardan unos segundos más en arrancar pero siguen el mismo flujo.

La lectura del portapapeles para archivos usa `gtk::Clipboard::wait_for_uris` en Linux (sirve para X11 y Wayland) y `NSPasteboard` con `NSFilenamesPboard` en macOS; otros sistemas no están soportados en este MVP.

## Límites deliberados del MVP

- Un destinatario por transferencia: la sala puede tener varios clientes conectados, pero los archivos sólo se entregan al resto (el emisor nunca recibe su propio archivo de vuelta).
- El relay no cifra ni autentica: úsalo únicamente en una LAN de confianza.
- No hay reconexión automática ni historial de archivos persistentes (la lista de recibidos se reinicia al cerrar la app).
- Cancelación parcial: al pulsar **Desconectar** o desconectar la red, las transferencias en curso se abortan; el receptor elimina el archivo parcial.

El siguiente paso, una vez validado este flujo, es añadir pairing y autenticación antes de cualquier relay por Internet.