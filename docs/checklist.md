# mvlc – Checklist

Nutzen: Abhaken während der Umsetzung. Spiegelt Roadmap‑Milestones und Kernaufgaben.

---

## Global

* [ ] Repo initialisiert (Workspace, Lizenzen, CI‑Stub)
* [ ] Coding‑Standards dokumentiert (`*.spec.md`, Telemetrie‑Pflicht)
* [ ] Logging/Tracing Grundsetup (`tracing`, NDJSON Writer)
* [ ] Benchmark‑Skeleton (criterion) aktiv

---

## M0 — Bootstrap

* [ ] `winit`/`egui` Fenster + Eventloop
* [ ] Vulkan Swapchain init
* [ ] libplacebo minimal binden und Test‑Quad rendern
* [ ] GStreamer via `gstreamer-rs` verlinken
* [ ] Single‑Stream SW‑Decode → CPU→GPU Upload → Anzeige
* [ ] Toolbar mit statischen Badges
* [ ] Basis‑Tracing: Frame begin/end

---

## M1 — Baseline Player

* [ ] Audio‑Ausgabe (`cpal`) + Resample
* [ ] Audio‑Masterclock implementiert
* [ ] A/V‑Sync (Drop/Repeat Window)
* [ ] Play/Pause/Seek + Timeline‑Scrub
* [ ] Canvas: Translate/Scale, Z‑Order
* [ ] Drag‑&‑Drop (`DroppedFile`)
* [ ] Badges live verdrahtet (Decode/Color/Transfer/Render/Sync)
* [ ] Detail‑Panel V1: Codec, Auflösung, Queues
* [ ] Tests: 1080p/4K H.264, MP4/MKV

---

## M2 — HW‑Decode + Zero‑Copy

* [ ] VA‑API Decoder in Pipeline
* [ ] Negotiation `video/x-raw(memory:DMABuf)`
* [ ] DMABUF‑Import in Vulkan (External Memory)
* [ ] Zero‑Copy Pfaderkennung + UI‑Farbcodes
* [ ] Metrik: Upload‑Bytes≈0 im Optimum
* [ ] Fallback: SW‑Decode Pfad stabil

---

## M3 — Mehrspur‑Compositing

* [ ] Mehrere Video‑Layer parallel
* [ ] Instanzierte Draws
* [ ] Per‑Layer Opacity, Mute
* [ ] Canvas‑Gizmos: Scale‑Handles, Rotation, Snapping
* [ ] Stress‑Tests (3–6 Streams)

---

## M4 — Farbpipeline & HDR

* [ ] libplacebo Farbraum‑Konfiguration (Primaries/Matrix/Transfer)
* [ ] HDR→SDR Tonemapping
* [ ] Per‑Stream Colorimetrie aus Metadaten
* [ ] Badge „Color: libplacebo‑HDR/SDR“
* [ ] Visual‑Tests gegen Referenz (mpv)

---

## M5 — Stabilität, Persistenz, Undo

* [ ] Projekt Save/Load (JSON/RON)
* [ ] Autosave + Crash‑Recovery
* [ ] Undo/Redo Journal
* [ ] Medien‑Relink Dialog
* [ ] 24h Dauertest ohne Crash/Leak

---

## M6 — Erweiterungen

* [ ] Vulkan Video Feature‑Gate
* [ ] Blend‑Modes, Masken
* [ ] Mehrspur‑Audio‑Mixer
* [ ] Headless Renderer/Export

---

## Telemetrie & KPIs

* [ ] Upload‑Bytes/s (Soll 0 Zero‑Copy)
* [ ] Present‑Jitter P95 < 3 ms
* [ ] A/V‑Drift RMS < 8 ms @60 Hz
* [ ] Dropped/Repeated Frames/min getrackt
* [ ] Start‑to‑First‑Frame < 300 ms

---

## Packaging & Compat

* [ ] Flatpak/AppImage Rezept
* [ ] Wayland bevorzugt, X11 getestet
* [ ] Ziel‑GPU‑Quirks dokumentiert

---

## Docs

* [ ] `media-bridge.spec.md`
* [ ] `renderer.spec.md`
* [ ] `telemetry.spec.md`
* [ ] User‑Guide Abschnitt „Pfad‑Transparenz“

---

## Release Artefakte

* [ ] Binary + Notes + Limits
* [ ] Sample‑Medien + Repro‑Scripts
* [ ] Trace‑Artefakte für Benchmark‑Runs
