# mvlc — Roadmap

Status: Draft v0.1
Scope: Linux-first desktop video player & compositor with Figma‑like Canvas

---

## 0. Leitbild

* Ziel: Realtime‑Playback mehrerer Videos auf einer frei editierbaren 2D‑Canvas (Position/Skalierung/Rotation/Überblendung), robustes A/V‑Sync, minimale Latenz, Zero‑Copy‑Pfade wo möglich.
* Anspruch: „Best of best“ Stack: HW‑Decode, Farbpipeline, HDR/Tonemapping, DMA‑BUF Zero‑Copy, Vulkan‑Renderer, deterministisches Scheduling.

## 1. Prinzipien

* **Single Source of Truth**: Audio‑Clock als Master; Video folgt.
* **Zero‑Copy bevorzugt**: DMA‑BUF Import/Export, externer Speicher für Vulkan.
* **Explizite Pfad‑Transparenz**: UI zeigt aktiv Dekoder/Renderer/Copy‑Pfad.
* **Fail‑gracefully**: Sauberes Fallback auf SW‑Decode und CPU‑Uploads.
* **Determinismus**: Keine „best effort“ Zeitplanung; harte Toleranzen pro Frame.
* **Composable Core**: Mediengraph (Nodes/Edges) + eigenständiger Renderer.
* **Testbarkeit**: Headless Render/Decode‑Tests + deterministische Goldens.

## 2. Architektur (Backbone)

* **Decode/Mux**: GStreamer (`gstreamer-rs`) als Primärpfad; optional FFmpeg‑Backupschicht.
* **HW‑Decode**: VA‑API → DMA‑BUF Frames; perspektivisch Vulkan Video.
* **Render/Color**: Vulkan + libplacebo für YUV→RGB, OETF/EOTF, HDR→SDR.
* **UI/Canvas**: winit + egui; Gizmos, Timeline, Layer‑Panel.
* **Audio**: cpal + swresample, Audio‑Ringbuffer, Lautstärke pro Layer.
* **Job‑System**: Thread‑Pools, lock‑arme Queues, back‑pressure.
* **Projektformat**: Serde (JSON/RON), stabile IDs, Undo/Redo‑Journal.

## 3. Pfad‑Transparenz (Visual)

### 3.1 Runtime‑Badges (Toolbar rechtsoben)

* **Decode**: `VA-API | NVDEC | SW`
* **Color**: `libplacebo-HDR | libplacebo-SDR | Basic`
* **Transfer**: `ZeroCopy(DMABUF) | Staged(Host->GPU)`
* **Render**: `Vulkan(libplacebo) | wgpu | Fallback`
* **Sync**: `A/V in ±X ms` (Rolling Median)

### 3.2 Farbcodes

* Grün: Optimalpfad.
* Gelb: Teil‑Fallback.
* Rot: Voll‑Fallback (SW‑Decode + CPU‑Upload).

### 3.3 Detail‑Panel

* Per‑Stream Karte: Codec, Auflösung, Bit‑Tiefe, Chroma, HW‑Surface‑Typ, Colorimetrie (Primaries/Matrix/Transfer), Queue‑Füllstände, Dropped/Repeated Frames, Upload‑Bytes/s.

## 4. Telemetrie & Debuggability

* Frame‑Traces (JSON Lines): `ts, stream_id, stage_enter/exit, dur_ns`.
* A/V‑Drift‑Historie, Jitter‑Histogramme, GC/Allocator‑Spikes.
* GPU‑Timing via Vulkan timestamps (begin/end pass).
* Toggle „Record Trace“ in UI; Artefakt‑Export.

## 5. Milestones

### M0 — Bootstrap (2–3 Wochen)

* Workspace, CI, Clippy/Format, minimaler App-Loop (winit/egui).
* GStreamer Probe: Single-Stream Playback, SW-Decode → CPU→GPU Upload.
* Vulkan Swapchain + libplacebo Einbindung, einfacher Textur-Quad.
* **Neu:** egui wird derzeit über einen wgpu-Swapchain-Fallback präsentiert, bis der Vulkanpfad bereit steht.
* Toolbar mit statischen Badges.

### M1 — Baseline Player (2–4 Wochen)

* Audio‑Clock Master, A/V Sync, Play/Pause/Seek, Timeline‑Scrub.
* Canvas: Translate/Scale; Layer‑Z; Drag‑Drop (`WindowEvent::DroppedFile`).
* Runtime‑Badges live verdrahtet; Detail‑Panel V1.
* Testmatrix: H.264 1080p/4K, MP4/MKV.

### P2 — UI Bridge (laufend)

* wgpu Offscreen‑Pfad zeichnet egui in persistente Texture + Readback‑Buffer, Upload in Vulkan Swapchain.
* `UiBackend` Enum kapselt wgpu‑Bridge und reserviert Slot für Vulkan‑native Variante.
* Ziel: libplacebo übernimmt Swapchain‑Overlay, Vulkan‑Backend fällt dann nahtlos ein.
* Vulkan-native egui-Kommandos stehen hinter `MVLC_VULKAN_UI_NATIVE`, wgpu-CPU-Bridge bleibt Fallback.

### M2 — HW‑Decode + Zero‑Copy (4–6 Wochen)

* VA‑API Dekoder, Negotiation `video/x-raw(memory:DMABuf)`.
* DMA‑BUF Import in Vulkan, externe Speicherbindung.
* Pfad‑Erkennung + Farbcodes.
* Leistungstests: CPU‑Zeit, Upload‑Bytes ≈ 0 im Optimum.

### M3 — Mehrspur‑Compositing (3–5 Wochen)

* Mehrere gleichzeitige Video‑Layer; instanzierte Draws.
* Per‑Layer Opacity, Alpha‑Blend, per‑Layer Mute.
* Canvas‑Gizmos: Scale‑Handles, Rotation, Snapping.

### M4 — Farbpipeline & HDR (3–5 Wochen)

* libplacebo: BT.709/601/2020, HLG/PQ, Tonemapping nach SDR.
* Per‑Stream Colorimetrie aus Metadaten, Matrixwahl automatisch.
* Badge „Color: libplacebo‑HDR/SDR“ mit Param‑Tooltip.

### M5 — Stabilität, Persistenz, Undo (2–3 Wochen)

* Projekt‑Save/Load, Autosave, Undo/Redo Journal.
* Crash‑Recovery, Medien‑Relink.

### M6 — Erweiterungen (laufend)

* Vulkan Video (wenn Treiber stabil).
* Effekte (Blend‑Modes, Masken), Audio‑Mixer multi‑source.
* Headless Renderer für Exporte.

## 6. AI‑Agent‑Coding‑Praktiken (Durchgängig)

* **Spec‑first**: Jede Komponente mit `.spec.md` inkl. Inputs/Outputs, Fehlerfällen, Telemetrie.
* **Contracts**: Rust Traits + Property‑Tests; pro Trait Referenz‑Fakes.
* **Deterministische Tests**: Seeded RNGs, feste Timelines, Golden‑Frame‑Diff.
* **Observability‑Gates**: PRs erfordern Metriken/Logs + Badge‑Verdrahtung.
* **Auto‑Bench**: CI misst FPS, Latenz, Upload‑Bytes; Regression‑Budget.
* **Policy**: Keine stillen Fallbacks; UI‑Badge Pflicht bei Pfadwechsel.

## 7. Definition of Done (pro Milestone)

* Alle Specs erfüllt, keine „TODO: later“ in Codepfaden.
* Reproduzierbare Benchmarks grün, A/V‑Drift < 8 ms RMS bei 60 Hz.
* Crash‑Freiheit > 24 h Dauerschleife, Memory‑Leak < 1%/h.
* Telemetrie vollständig, Badges korrekt, Tests grün.

## 8. Technical KPIs

* Upload‑Bytes/s (Soll 0 im Zero‑Copy Pfad).
* Present‑Jitter (P95 < 3 ms).
* Dropped/Repeated Frames pro Minute.
* CPU‑% pro Stream, GPU‑Zeit pro Frame.
* Start‑to‑First‑Frame < 300 ms.

## 9. Risiko‑Register + Gegenmaßnahmen

* **HW‑Interop instabil** → Feature‑Gate, geprüfter SW‑Fallback.
* **Treiberfragmentierung** → Per‑Vendor Quirks‑Layer, Auto‑Blacklist.
* **A/V‑Drift** → Striktes Audio‑Master‑Lock, Frame‑Drop/Repeat Window.
* **Color‑Mismatches** → libplacebo als Kanon, Metadaten‑Validierung.

## 10. Deliverables je Milestone

* Binary + Release Notes + bekannte Limits.
* Sample‑Medien, Repro‑Scripts, Trace‑Artefakte.
* User‑Guide Abschnitt „Pfad‑Transparenz“.

## 11. Offene Punkte (zu präzisieren)

* Ziel‑GPUs und Treiberlinien (Intel/AMD/NVIDIA) für die Optimierung.
* Priorisierte Codecs/Container jenseits H.264/HEVC/VP9/AV1.
* HDR‑Ziel: SDR‑Tonemapping vorerst oder echtes HDR10‑Output.
* Wayland vs. X11 Priorität in der ersten Release.
* Minimal unterstützte Linux‑Distros und Paketform (Flatpak/AppImage).
