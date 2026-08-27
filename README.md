# Clipboard Sync (MVP LAN)

Sincroniza **texto** del portapapeles entre Ubuntu (Wayland) y macOS mediante un relay WebSocket dentro de la misma red. El cliente Tauri usa Rust para observar, leer y escribir el portapapeles; no hay acceso al portapapeles desde JavaScript.

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
2. Averigua la IP LAN de ese equipo, por ejemplo `192.168.1.20`.
3. En cada otro equipo, selecciona **Cliente** y usa `ws://192.168.1.20:8787`. Escribe la misma sala en todos los clientes que deban sincronizarse.

El relay se ejecuta dentro de la aplicación Tauri. El script `npm run relay` se conserva únicamente como alternativa de desarrollo sin interfaz.

En Ubuntu/Wayland, ejecuta la app dentro de tu sesión gráfica normal; el proceso necesita las variables de sesión (`WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`). En macOS, el primer acceso puede requerir conceder el permiso de portapapeles que indique el sistema.

1. Abre la app en Ubuntu y macOS.
2. Inicia el servidor en una de ellas y conecta la otra como cliente usando su IP LAN.
3. Configura la misma sala en los clientes (por ejemplo `prueba-casa-42`).
4. Copia texto en un cliente y pégalo en el otro.

## Límites deliberados del MVP

- Solo texto; no imágenes ni archivos.
- El relay no cifra ni autentica: úsalo únicamente en una LAN de confianza.
- No hay reconexión automática ni historial todavía.

El siguiente paso, una vez validado este flujo, es añadir pairing y autenticación antes de cualquier relay por Internet.
