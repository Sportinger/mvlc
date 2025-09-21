# mvlc — Modern Video Layered Compositor

Linux‑first Desktop‑Player mit Figma‑ähnlicher Canvas: mehrere Videos gleichzeitig abspielen, frei positionieren, skalieren, rotieren, überblenden. Fokus: Zero‑Copy, präzises A/V‑Sync, HDR‑fähige Farbpipeline.

---

## Status

* **Phase:** Early draft (M0→M1).
* **Roadmap:** siehe [`mvlc-ROADMAP.md`](mvlc-ROADMAP.md).

Badges (geplant): CI • Clippy • Tests • Benchmarks • Coverage • Nightly Artifacts

---

## Kern‑Features

* Mehrspur‑Playback auf Canvas (Translate/Scale/Rotate, Z‑Order, Opacity).
* Pfad‑Transparenz im UI: **Decode**, **Color**, **Transfer**, **Render**, **Sync** als Badges.
* HW‑Decode (VA‑API), Zero‑Copy via **DMA‑BUF** in Vulkan, Fallback auf SW‑Decode.
* libplacebo‑Farbpipeline: korrekte Matrizen/Primaries/Transfers, HDR→SDR Tonemapping.
* Audio‑Masterclock, stabiler Sync, geringe Latenz.
* Drag‑&‑Drop von Dateien aus dem OS, Timeline‑Scrub, Play/Pause/Seek.

---

## Architektur (Kurz)

```
+----------------+   DMABUF    +---------------------+   Vulkan    +------------------+
|  GStreamer     | ==========> |  mvlc-video-bridge  | ==========> |  Renderer/Canvas |
|  (VA-API/Sw)   |             |  (External Memory)  |             |  (libplacebo)    |
+----------------+              +---------------------+             +------------------+
         | packets/frames                                 | UI (winit/egui)
         v                                                 v
   Audio (cpal) <------ Master Clock  -----> Present & A/V Sync
```

**Stacks:**
Decode: `gstreamer-rs` (+ optional FFmpeg Pfad)
Render/Color: Vulkan + `libplacebo`
UI: `winit` + `egui`
Audio: `cpal` (+ `swresample`)

---

## Runtime‑Badges

* **Decode**: `VA-API | NVDEC | SW`
* **Color**: `libplacebo-HDR | libplacebo-SDR | Basic`
* **Transfer**: `ZeroCopy(DMABUF) | Staged(Host->GPU)`
* **Render**: `Vulkan(libplacebo) | wgpu | Fallback`
* **Sync**: `A/V ±X ms` (Rolling Median)
  Farbcodes: Grün = optimal, Gelb = Teil‑Fallback, Rot = Voll‑Fallback.

---

## Systemvoraussetzungen

* Linux mit funktionierender Vulkan‑Runtime und Treibern (Intel/AMD/NVIDIA).
* GStreamer mit VA‑API Plugins (für HW‑Decode).
* FFmpeg/GStreamer Dev‑Headers für Bindings/Build.
* Rust 1.80+ (stable), `pkg-config`.

### Paket‑Hint (Beispiele)

**Ubuntu/Debian**

```bash
sudo apt update
sudo apt install -y build-essential pkg-config cmake git \
  libgstreamer1.0-dev gstreamer1.0-plugins-base gstreamer1.0-plugins-bad \
  gstreamer1.0-vaapi libgstreamer-plugins-bad1.0-dev \
  libvulkan-dev vulkan-tools
```

**Fedora**

```bash
sudo dnf install -y @development-tools pkgconfig cmake git \
  gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-bad-free \
  gstreamer1-vaapi gstreamer1-plugins-bad-free-devel \
  vulkan-loader-devel vulkan-tools
```

**Arch**

```bash
sudo pacman -S --needed base-devel pkgconf cmake git \
  gstreamer gst-plugins-base gst-plugins-bad libva \
  vulkan-icd-loader vulkan-tools
```

---

## Build

```bash
git clone https://example.com/mvlc.git
cd mvlc
cargo build --release
```

### Run (Beispiel)

```bash
RUST_LOG=info ./target/release/mvlc path/to/video.mp4
```

Drag‑&‑Drop mehrerer Dateien in das Fenster zum Hinzufügen von Layern.

### Projektstruktur

```
video-canvas/
  Cargo.toml
  crates/
    app/        # winit/egui UI, Swapchain, Badges, Timeline, Canvas
    media/      # GStreamer/FFmpeg Bridge, HW Surfaces, DMABUF Export
    core/       # Graph, Layer, Time, Sync, Project, Telemetrie
  mvlc-ROADMAP.md
  README.md
```

---

## Konfiguration

Umgebungsvariablen (geplant):

* `MVLC_HWDECODE=1|0` – erzwingt HW‑Decode an/aus.
* `MVLC_ZERO_COPY=1|0` – erzwingt DMABUF Import an/aus.
* `MVLC_COLOR=placebo|basic` – Farbpfadwahl.
* `MVLC_PRESENT=mailbox|fifo` – Present‑Mode Präferenz.
* `MVLC_TRACE=path.ndjson` – schreibt Frame‑Trace.

CLI‑Flags (Beispiele, WIP):

```
--no-hw, --no-zerocopy, --color=placebo|basic, --present=fifo|mailbox,
--trace=FILE, --gpu=N, --disable-vsync
```

---

## Keybindings (MVP)

* **Space**: Play/Pause
* **Arrow Left/Right**: Seek ±1s
* **Ctrl+O**: Datei öffnen
* **Delete**: Selektierte Layer entfernen
* **Mouse**: Drag zum Verschieben, Wheel zum Zoomen, Shift+Wheel = Rotate

---

## Telemetrie & Debug

* UI‑Panel: Per‑Stream Details, Queue‑Füllstände, A/V‑Drift, Dropped/Repeated Frames.
* Trace‑Export: NDJSON Events mit Timestamps, Stadien, Dauer.
* GPU‑Timestamps: Vulkan Queries pro Pass.

---

## Troubleshooting

* Kein VA‑API: Prüfe `vainfo`. Fällt auf SW‑Decode; Badge wird Rot/Gelb.
* Schwarzes Bild: Treiber/ICD prüfen; `vulkaninfo`.
* Ruckler: `RUST_LOG=debug`, A/V‑Drift im Panel prüfen; Test ohne HDR.
* X11/Wayland: Wayland empfohlen (zwp\_linux\_dmabuf\_v1). X11 funktioniert, Zero‑Copy je nach Setup.

---

## Beitrag

* PRs mit Spec‑Docs (`*.spec.md`), Telemetrie‑Hooks und Tests.
* Keine stillen Fallbacks; Badge‑Pflichten beachten.
* Benchmarks in CI: FPS, Jitter, Upload‑Bytes, Start‑Latenz.

---

## Lizenz

TBD (MIT/Apache‑2.0 empfohlen). Drittlibs beachten (GStreamer/FFmpeg/libplacebo).

---

## Danksagung

* libplacebo (Farbraum/HDR), mpv‑Community
* GStreamer (Pipelines, HW‑Decode)
* Vulkan/WGPU‑Ökosystem
