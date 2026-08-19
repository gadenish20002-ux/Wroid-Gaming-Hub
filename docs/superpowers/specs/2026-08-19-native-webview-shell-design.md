# Native WebView Shell Design

**Date:** 2026-08-19

## Goal

Launch Wroid Gaming Hub and Controls Studio as first-class Linux desktop
windows instead of tabs in the user's default browser. Preserve the existing
Hub UI, Rust loopback APIs, profiles, input runtime, and game-session behavior.

The result is the first native desktop packaging slice. It is deliberately not
a rewrite of the interface into native widgets.

## Current Problem

The installed desktop entry executes `wroid hub`. That command starts the Hub
HTTP server on an ephemeral loopback port, generates a per-session token, and
passes the tokenized URL to `xdg-open`. The application therefore looks like a
website, exposes its internal URL in browser chrome, and depends on browser tab
lifecycle. Controls Studio uses the same browser-first pattern.

## Chosen Approach

Use GTK 3 with WebKitGTK 4.1 as a thin Linux-only desktop shell around the
existing local applications. Those ABIs are already present on the reference
host and use the same GTK generation. The shell is part of the Rust Wroid
executable and is initialized only for graphical commands. Normal CLI, daemon,
helper, and game worker behavior remains unchanged.

This approach was selected over:

- Tauri, which would retain WebKitGTK while adding a second frontend build
  system and unnecessary Node.js/bundling complexity;
- a full GTK widget rewrite, which would duplicate the already working Hub and
  Controls Studio before gameplay readiness is complete.

## Scope

This slice includes:

- a native Hub window with standard minimize, maximize, and close controls;
- a native Controls Studio window using the same reusable shell;
- single-instance activation for the main Hub;
- explicit browser fallback for diagnostics;
- secure navigation restrictions and deterministic server cleanup;
- desktop installation and Arch/AUR dependency documentation;
- automated lifecycle, routing, launcher, and security tests;
- a manual installed-application smoke test on KDE Wayland.

It does not include a native-widget UI rewrite, tray integration, an always-on
game overlay, Windows/macOS support, or changes to the gameplay input path.
Play Store and Waydroid continue to open as their own application windows.

## Architecture

### Local application server

Extract the shared server lifecycle from the current blocking `run_hub` and
editor entry points. Starting a local application returns a typed handle:

- the exact loopback origin and unguessable access token;
- a shared shutdown signal;
- a bounded join operation for its server thread.

The listener continues binding only to `127.0.0.1` on an ephemeral port. HTTP
request parsing, authentication, upload limits, profile validation, and action
handlers remain unchanged.

Headless tests and `--browser` diagnostics use the same server handle. There is
no second implementation of the Hub API.

### Native shell

A small `desktop_webview` module owns GTK application/window construction and
WebKit configuration. It accepts only a typed local-application handle, not an
arbitrary URL supplied from the command line.

The Hub uses application ID `io.wroid.GamingHub`. GTK application activation
raises the existing Hub window when the launcher is invoked again. The first
activation starts the local server and loads its authenticated URL in the
WebView.

Controls Studio uses the same shell implementation in a separate non-singleton
Wroid process. Each explicit editor action opens one profile window; multiple
editor windows are allowed. Existing profile-path validation remains
authoritative.

Initial windows use a normal system title bar, a 1280x800 default size, and a
1024x640 minimum. The existing responsive web layout fills the content area.

### Command behavior

`wroid hub` becomes native-window mode by default. `wroid hub --browser` keeps
the current browser behavior for diagnostics. `--no-open` remains a headless
server mode and cannot be combined with `--browser`.

The Controls Studio command follows the same policy: native by default with an
explicit browser fallback. Internal actions that open Controls Studio select
native mode automatically.

The desktop entry remains `wroid hub`, so reinstalling the desktop application
switches the menu launcher to native behavior without adding another public
binary or changing user profiles.

## Lifecycle and Data Flow

1. GTK registers or activates the Wroid application.
2. The first activation starts the selected loopback application server.
3. The server returns its private URL directly in process memory.
4. The WebView loads that URL; the token never appears in process arguments,
   desktop files, logs intended for normal users, or browser history.
5. UI requests continue using the existing authenticated HTTP API.
6. Closing the native window signals server shutdown and performs a bounded
   join before process exit.
7. The existing in-page close action signals the same shutdown path and closes
   the native window.

If the WebView fails before the first page load, Wroid shows a native error
dialog with a concise dependency or initialization error and shuts down the
server. It does not silently fall back to the browser. Browser fallback is
explicit so packaging failures cannot be mistaken for correct native behavior.

## Navigation and Security

The WebView may load only the exact origin generated by its server handle,
including the assigned port. It rejects:

- navigation to another host, scheme, or loopback port;
- new-window and popup requests;
- direct access to `file:`, `data:`, and custom schemes;
- TLS exception handling and developer tools in release builds.

Normal file selection remains enabled for the existing APK upload flow. The
selected file still passes through the current bounded, ticket-based HTTP
upload and inspection pipeline; the WebView never grants the backend an
arbitrary filesystem path.

External documentation links, if added later, must be opened through an
explicit allowlisted system-browser action rather than normal WebView
navigation.

## Packaging

The desktop release dynamically links the distribution GTK 3 and WebKitGTK 4.1
libraries. The Arch/AUR package declares their runtime packages explicitly, so
the package manager prevents an incomplete installation. `wroid desktop
install` continues copying Wroid, `wroidd`, and the staged helper.

The existing application ID, desktop file, icon, configuration directory, and
closed-source release model remain unchanged.

## Testing

Automated tests cover:

- native, browser, and headless CLI mode validation;
- desktop entry generation with native mode as the default;
- exact-origin navigation policy;
- rejection of popup and non-HTTP local navigation;
- server shutdown from both window close and in-page close;
- bounded cleanup after WebView initialization failure;
- Hub single-instance activation policy;
- Controls Studio native-mode command construction;
- preservation of existing Hub, editor, upload, compatibility, and session
  tests.

The full workspace test, Clippy, JavaScript checks, and release build remain
mandatory.

Manual acceptance on the reference KDE Wayland host requires:

1. Launching **Wroid Gaming Hub** from the application menu opens one Wroid
   window and no browser tab.
2. Launching it again raises the same window.
3. Library, System, Play Store, APK selection, refresh, and close actions work.
4. **Edit controls** opens Controls Studio in a Wroid window.
5. Closing every Wroid window leaves no Hub/editor listener or orphan process.
6. A Standoff 2 managed session still starts and restores Waydroid normally.

## Rollout

Native mode becomes the default only after the installed release passes the
manual acceptance list. The explicit browser mode remains available as a
diagnostic escape hatch, not as an automatic fallback.
