<div align="center">

<img src="assets/logo.png" alt="Boogie Logo" width="132" />

# Boogie — Native Rust Voice AI Assistant

**Ultra-fast, lightweight real-time voice AI assistant for Windows powered by Gemini Live API & a 60 FPS 3D Liquid Glass GLSL Orb.**

![Rust](https://img.shields.io/badge/Rust-Edition_2024-orange?logo=rust&logoColor=white)
![Platform](https://img.shields.io/badge/Platform-Windows_x64-0078D6?logo=windows&logoColor=white)
![Cold Start](https://img.shields.io/badge/Window_Ready-~137ms-22D3EE)
![RAM](https://img.shields.io/badge/Private_RAM-~33_MB-8B5CF6)
![Binary](https://img.shields.io/badge/Single_Binary-28.0_MB-10B981)
![Renderer](https://img.shields.io/badge/UI-egui_+_GLSL_3D_Shader-EC4899)

<br />

<img src="assets/screenshot.png" alt="Boogie 3D Liquid Glass Orb UI" width="100%" />

</div>

---

## Why Rust? — Blazing Fast Startup & Tiny RAM Footprint

Boogie was rewritten from the ground up in **pure Rust** to eliminate interpreter overhead, PyQt/Chromium bloat, and sluggish cold starts — while delivering a **60 FPS raymarched 3D GPU interface**, **real-time screen vision**, and **42 native Windows automation tools** inside a single self-contained `.exe`.

### Visual Performance Comparison

> Measured on Windows 10/11 x64 (`target/release/boogie.exe`, active WASAPI audio streams + OpenGL 3D shader + Gemini Live TLS WebSocket).

```text
⚡ Cold Startup Speed — Launch to Window Ready (shorter is better)
──────────────────────────────────────────────────────────────────────────
Boogie (Rust Native)        █░░░░░░░░░░░░░░░░░░░   0.14s  (137 ms window · 0.74s full GPU+API)
Python + PyQt6              ███████████░░░░░░░░░   2.5 – 4.5s
Electron / Chromium App     ████████████████████   3.5 – 7.0s
```

```text
🧠 Memory Consumption — Active Voice Session (shorter is better)
──────────────────────────────────────────────────────────────────────────
Boogie (Rust Private RAM)   ██░░░░░░░░░░░░░░░░░░   33.3 MB  (~60.8 MB working set w/ GPU + DLLs)
Python + PyQt6              ███████████░░░░░░░░░  240 – 380 MB
Electron / Chromium App     ████████████████████  450 – 800+ MB
```

```text
📦 Footprint on Disk — Ready to Run (shorter is better)
──────────────────────────────────────────────────────────────────────────
Boogie (Single .exe)        ██░░░░░░░░░░░░░░░░░░   28.0 MB  (zero external runtime)
Python + Embedded Env       ████████████░░░░░░░░  350+ MB   (Python + Qt + wheels)
Electron Bundle             ████████████████████  250 – 600 MB
```

### Real Measured Cold-Boot Timeline

| Stage | Elapsed Time | What Happens |
| :--- | :---: | :--- |
| **Process Launch** | `0 ms` | Native binary starts — zero VM/interpreter warmup |
| **WASAPI Audio Ready** | `28 ms` | Native mic (`16 kHz`) & speaker (`24 kHz`) opened via `cpal` |
| **GUI Window Live** | `137 ms` | Frameless Obsidian window visible on screen |
| **Gemini Live Connected** | `~520 ms` | TLS WebSocket `101 Switching Protocols` + session `setupComplete` |
| **3D GPU Orb Rendering** | `~740 ms` | Raymarched Liquid Glass GLSL shader compiled & running at **60 FPS** |

### Side-by-Side Specs

| Metric | Boogie (Rust) | Typical Python + PyQt6 | Typical Electron / WebView |
| :--- | :---: | :---: | :---: |
| **Window Ready** | **~137 ms** | ~2.5 – 4.5 s | ~3.5 – 7.0 s |
| **Private RAM** | **~33 MB** | ~240 – 380 MB | ~450 – 800+ MB |
| **Total Working Set** | **~61 MB** | ~300 – 450 MB | ~550 – 1,000+ MB |
| **Executable Size** | **28.0 MB** (1 file) | 350+ MB (folder) | 250 – 600 MB (folder) |
| **Runtime Needed** | **None** | Python + Qt DLLs | Bundled Chromium + Node |
| **3D Visuals** | **60 FPS GLSL Shader** | CPU / QPainter | WebGL (high RAM) |

---

## Key Features

### 1. Real-Time 3D Liquid Glass / Ferrofluid Orb (`orb_shader.rs`)
- **Pure OpenGL GLSL Raymarching** directly inside `eframe::glow` — zero heavy 3D engine bloat.
- **Audio & Viseme Reactive:** Real-time FFT speech analysis (`dsp.rs` — energy level, vowel openness, and formant width) drives liquid surface waves and internal refracted plasma ribbons at 60 FPS.
- **Smooth State Morphing:** Fluidly blends palette, turbulence, and 3D tilted orbital ring across `Listening` (Violet/Cyan), `Speaking` (Cyan/Mint), `Thinking` (Violet/Amber), `Sleeping`, and `Muted`.
- **Interactive 3D:** Subtle cursor parallax tilts lighting and camera; clicking the orb triggers a liquid energy pulse.

### 2. Modern Frameless Obsidian UI (`gui.rs`)
- **Custom Frameless Window:** Native Neon Boogie app icon (window, taskbar & `.exe`), ultra-slim draggable header, 8-edge resizing, live pulsing status pill, accurate CPU/RAM telemetry, error banners, and crisp vector window controls.
- **First-Run Onboarding Modal:** Launch without manual JSON editing — enter your Gemini API key, select voice, mic/speaker devices, and Push-to-Talk right inside the app.
- **Live Captions + Frosted Glass Drawer:** Clean centered subtitles fade gently below the orb; click captions or the History button to slide up a frosted glass conversation drawer with smart merged sentences and compact tool chips (`⚡ Computer Settings · brightness down`).
- **Floating Glass Control Dock:** Centered pill dock with Mute toggle, 14-bar live spectrum visualizer, instant Interrupt/Stop button, and Conversation Drawer toggle.

### 3. Resilient Voice Pipeline & Real Screen Vision
- **Native WASAPI Audio (`audio.rs`) + Hardware-Aware Echo Guard (`echo.rs`):** Multi-format (`f32`/`i16`/`u16`) capture & playback with anti-alias downsampling, instant speaker buffer clearing on barge-in or server `interrupted`, 400ms speech hangover, `audio/pcm;rate=16000`, and `audioStreamEnd` signaling.
- **Resilient Gemini Live Client (`gemini.rs`, `main.rs`):** Non-blocking background tool execution, panic isolation with automatic supervisor recovery, `interval`-based stall watchdog, `goAway` & 14-minute session rotation, exponential reconnect backoff, and long-term session summary recall.
- **Real Screen & Browser Vision (`screen_process`, `browser_control` `screenshot`):** Captures monitors or Chromium pages, resizes to `≤1280px` JPEG, and streams directly into Gemini Live `realtimeInput.video`.

### 4. Security-First Automation (42 Native Windows Tools)
- **Permission Tiers & Confirmation Gate (`confirm.rs`):**
  - `run_command` auto-runs only a strict read-only single-command allowlist (blocking shell chaining `&&`, `;`, `|`, `>`); all mutating commands require on-screen approval.
  - `send_message`, `kill_process` (exact match + numeric PID + protected system process blocklist), `write_file`, `delete_file`, `move_file`, and destructive `computer_settings` require explicit confirmation with request-ID matching (no stale approvals).
  - All confirmed/denied sensitive actions are recorded in `~/.boogie/audit.log`.
- **Prompt Injection Defense (`util.rs`):** External outputs (`web_scrape`, `web_search`, `read_file`, `read_pdf`, `read_excel`, `clipboard_read`, browser text) are wrapped in `<<<UNTRUSTED source=...>>>` markers and automatically escalate desktop input actions in the same turn to require confirmation.
- **Network & Process Hardening:** SSRF protection blocking loopback/LAN/link-local targets, 15s shared HTTP timeout, Chromium sandbox kept enabled, script/executable launch blocking, and local WebSocket dashboard (`:8765`) disabled by default with Origin checks + random token auth when enabled.

---

## Architecture

```mermaid
flowchart LR
    Mic["WASAPI Mic (16kHz)"] --> DSP["DSP & Echo Guard\n(RMS + FFT Visemes)"]
    DSP --> WS["Gemini Live WebSocket\n(tokio-tungstenite)"]
    WS --> Speaker["WASAPI Speaker (24kHz)\n(Instant Clear on Interrupt)"]
    WS <--> Worker["Isolated Tool Worker\n(42 Windows Actions)"]
    Worker <--> Gate["Permission Tiers &\nConfirm Gate + Audit Log"]
    DSP --> GUI["Frameless egui Window"]
    WS --> GUI
    GUI --> Shader["3D Liquid Glass Orb\n(GLSL Raymarching 60 FPS)"]
    WS -. optional auth .-> Dash["Axum WS Dashboard\n(:8765)"]
```

---

## Quick Start

### Prerequisites
- **Windows 10 / 11 (64-bit)**
- **Rust toolchain** (Edition 2024 — Rust 1.85+ via [rustup.rs](https://rustup.rs))
- A **Google Gemini API key** with Gemini Live API access

### 1. Clone & Build

```powershell
git clone https://github.com/thodark/Boogie-AI-Assistant.git
cd Boogie-AI-Assistant
cargo build --release
```

### 2. Run & Configure

Just launch the binary — if no config exists yet, Boogie opens the **built-in Setup screen** where you can paste your Gemini API key and choose your voice, mic, and speaker:

```powershell
.\target\release\boogie.exe
```

Or configure manually by copying `config/api_keys.example.json` to `target/release/config/api_keys.json` (also searched in `./config/api_keys.json` and `%APPDATA%\Boogie\api_keys.json`):

```json
{
  "gemini_api_key": "YOUR_GEMINI_API_KEY_HERE",
  "model": "models/gemini-3.1-flash-live-preview",
  "api_version": "v1alpha",
  "assistant_name": "Boogie",
  "user_name": "",
  "voice": "Aoede",
  "input_device": "",
  "output_device": "",
  "push_to_talk": false,
  "proactive_audio": true,
  "dashboard_enabled": false
}
```

> **Tip:** Set `RUST_LOG=boogie=debug` if you need detailed diagnostics (defaults to `boogie=info` to keep logs clean and private).

---

## Project Structure

```text
.
├── Cargo.toml                  # Dependencies & binary target
├── Cargo.lock                  # Reproducible dependency lockfile
├── build.rs                    # Embeds Windows .exe icon (assets/logo.ico)
├── assets/
│   ├── logo.png                # Brand logo
│   ├── logo_icon.png           # 256px embedded window/header icon
│   ├── logo.ico                # Multi-res Windows executable icon
│   ├── boogie.rc               # Windows icon resource definition
│   └── screenshot.png          # UI showcase screenshot
├── core/
│   └── prompt.txt              # Embedded system prompt (overridable externally)
├── config/
│   └── api_keys.example.json   # Config template
└── src/
    ├── main.rs                 # Supervisor, non-blocking tool worker & session loop
    ├── gui.rs                  # Frameless Obsidian UI, onboarding, captions, drawer & dock
    ├── orb_shader.rs           # 60 FPS 3D Liquid Glass / Ferrofluid GLSL shader
    ├── gemini.rs               # Gemini Live API WebSocket client (audio + vision)
    ├── audio.rs                # Multi-format WASAPI audio + instant Clear playback
    ├── dsp.rs                  # RMS energy & FFT viseme extraction (rustfft)
    ├── echo.rs                 # Speaker-aware echo guard + hangover + audioStreamEnd
    ├── confirm.rs              # Permission tiers, confirm gate & audit logger
    ├── config.rs               # Multi-path config loader, prompt & session memory
    ├── proactive.rs            # Proactive check-in engine
    ├── dashboard.rs            # Authenticated local Axum WS server (off by default)
    ├── tools.rs                # Tool registry, vision frames & untrusted tagging
    ├── util.rs                 # UTF-8 safe truncation, SSRF guard, shared HTTP & JPEG
    └── actions/                # 16 modules implementing 42 native Windows tools
```

---

## Development & CI

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

GitHub Actions (`.github/workflows/ci.yml`) runs formatting, strict Clippy (`-D warnings`), and the full unit test suite automatically on `windows-latest`.

---

## Tech Stack

| Layer | Crate |
| :--- | :--- |
| **GUI & 3D Shader** | `eframe` / `egui` 0.31 + `glow` (OpenGL GLSL) |
| **Async Runtime** | `tokio` |
| **Audio I/O & DSP** | `cpal` (WASAPI), `rustfft` |
| **Realtime AI** | `tokio-tungstenite` (Gemini Live WebSocket) |
| **Windows Automation** | `windows` (`windows-rs`, Per-Monitor V2 DPI-aware), `screenshots`, `arboard`, `trash` |
| **Browser Control** | `chromiumoxide` (CDP, sandboxed), `reqwest`, `scraper` |
| **Security & Dashboard** | `ring` (random token auth), `url` (SSRF guard), `axum` |
