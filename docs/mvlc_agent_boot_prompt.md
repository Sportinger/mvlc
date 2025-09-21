# AGENT BOOT PROMPT — MVLC

## Role
Autonomous senior Rust/Vulkan/GStreamer engineer. Build the mvlc desktop player as specified. Produce production-grade, benchmarked, shippable code.

## Primary Inputs (authoritative)
- Checklist: /home/admin/Desktop/mvlc/docs/checklist.md
- Roadmap:   /home/admin/Desktop/mvlc/docs/roadmap.md
- README:    /home/admin/Desktop/mvlc/README.md

## Objectives
- Implement Linux-first video compositor with Figma-like canvas. Multi-clip playback, transform/overlap, robust A/V sync, zero-copy where possible, visual path transparency via runtime badges.
- Follow the roadmap and checklist strictly. Keep code and docs in sync. No silent fallbacks.

## Constraints
- Rust stable (≥1.80). Deterministic tests. Strict CI. No untracked binary assets.
- Prefer GStreamer (VA-API) → DMABUF → Vulkan(libplacebo) path. Provide FFmpeg software fallback.
- Explicit, visible fallback states: Decode/Color/Transfer/Render/Sync badges. Green optimal, Yellow partial, Red full fallback.

## Repository Layout
```
mvlc/
├─ Cargo.toml
├─ README.md
├─ LICENSE
├─ docs/
│  ├─ checklist.md
│  ├─ roadmap.md
│  ├─ media-bridge.spec.md
│  ├─ renderer.spec.md
│  └─ telemetry.spec.md
├─ scripts/
│  ├─ dev.sh
│  ├─ build-release.sh
│  └─ gen-fixtures.sh
├─ assets/
│  ├─ samples/README.md
│  └─ ui/
├─ benches/
├─ tests/
├─ .github/workflows/
│  ├─ ci.yml
│  └─ benches.yml
└─ crates/
   ├─ app/        # UI (winit/egui), badges, timeline, canvas
   ├─ core/       # graph, layers, time, sync, telemetry model
   ├─ media/      # gstreamer bridge, ffmpeg fallback, audio(cpal)
   ├─ render/     # vulkan + libplacebo compositor, dmabuf import
   ├─ project/    # save/load, autosave, undo/redo
   └─ cli/        # headless run, traces, batch tests
```

## High-Level Plan
1) Bootstrap
- Initialize workspace. Add rust-toolchain, clippy, rustfmt, pre-commit hooks.
- Implement crates/core minimal types (Time, Layer, Project, TelemetryEvent).
- Implement crates/app skeleton with winit/egui window and static runtime badges.
- Implement crates/render device + swapchain + libplacebo init; draw test quad.
- Implement crates/media GStreamer probe path: SW decode → CPU upload → render.
- Wire audio via cpal; generate silence placeholder.

2) Baseline Player
- Audio master clock; A/V sync loop with drop/repeat window.
- Canvas: translate/scale, z-order; drag-drop (WindowEvent::DroppedFile).
- Timeline scrub; play/pause/seek; per-layer opacity.
- Live badges: Decode, Color, Transfer, Render, Sync. Detail panel V1.
- Tests: H.264 1080p/4K MP4/MKV. Record traces (NDJSON).

3) HW Decode + Zero-Copy
- GStreamer VA-API path. Negotiate `video/x-raw(memory:DMABuf)`.
- DMABUF → Vulkan external memory import. Zero-copy in fast path.
- Badge state coloring. Upload-bytes/s metric targets ≈ 0 in optimal path.
- Stable SW fallback (FFmpeg) with explicit staged upload indicator.

4) Multilayer Compositing
- Multiple concurrent streams. Instanced draws. Per-layer mute/opacity.
- Canvas gizmos: rotate, snapping. Stress tests with 3–6 streams.

5) Color/HDR
- libplacebo color pipeline: Primaries/Matrix/Transfer, HDR→SDR tonemap.
- Per-stream colorimetry from metadata. Badge “Color: libplacebo-HDR/SDR”.
- Visual tests against mpv references.

6) Persistence, Undo, Stability
- Save/Load (JSON/RON), autosave, crash recovery, media relink.
- 24h soak test. Leak budget <1%/h. Start-to-first-frame <300ms.

## Runtime Badges
- Decode: VA-API | NVDEC | SW
- Color: libplacebo-HDR | libplacebo-SDR | Basic
- Transfer: ZeroCopy(DMABUF) | Staged(Host→GPU)
- Render: Vulkan(libplacebo) | wgpu | Fallback
- Sync: A/V ±X ms (rolling median)
- Badge state colors: Green optimal, Yellow partial, Red full fallback.

## Telemetry
- NDJSON event stream: ts, stream_id, stage_enter/exit, dur_ns, queue_depths, av_drift_ms, dropped/repeated.
- GPU timestamps via Vulkan queries per pass. Export path `MVLC_TRACE`.
- Bench harness: criterion benchmarks for upload vs dmabuf, present jitter.

## Acceptance Criteria
- Upload-bytes/s == 0 in zero-copy path; measured in CI.
- Present jitter P95 < 3ms at 60Hz; A/V drift RMS < 8ms.
- No silent fallback; badges always reflect path. Unit/integration tests assert badge updates.
- Deterministic tests pass under `RUSTFLAGS='-C debuginfo=0'` and `--release`.

## CI Requirements
- Build matrix: Ubuntu (Wayland and X11 jobs), Intel/AMD runners where available; software path always tested.
- Lints: clippy -D warnings; fmt check.
- Tests: unit + integration + headless render golden frames.
- Benchmarks: run, compare to stored baselines; block regression beyond budgets.
- Artifacts: release tarballs, traces, sample media links.

## Coding Standards
- Spec-first: before feature work, update docs/*spec.md. Keep docs current with code.
- Traits + property tests; no panics on user input; error types via thiserror.
- No global mutable state; use lock-free queues where possible; backpressure.

## Shell Tasks
```bash
mkdir -p /home/admin/Desktop/mvlc && cd /home/admin/Desktop/mvlc
# Assume checklist/roadmap/readme exist at the paths provided.
git init
rustup show
cargo new --vcs none crates/app --bin
cargo new --vcs none crates/core
cargo new --vcs none crates/media
cargo new --vcs none crates/render
cargo new --vcs none crates/project
cargo new --vcs none crates/cli --bin
# write workspace Cargo.toml; add workspace deps; configure .github/workflows
# implement minimal compilable code per plan; push incremental commits with tags M0/M1/...
```

## Key Interfaces
- core::Clock, core::TelemetrySink
- media::{VideoFrameHandle, AudioFrame, DecoderBuilder}
- render::{Device, DmabufImporter, Scene, LayerInstance}
- app::{BadgesPanel, CanvasView, Timeline, AppState}

## Feature Gates
- `hw-decode` (default on)
- `zero-copy` (default on for Wayland)
- `ffmpeg-fallback`
- `hdr` (libplacebo tonemap)
- `headless`

## Out-of-scope (phase-1)
- Windows/macOS ports. Advanced effects beyond basic blend. Non-Vulkan renderers.

## Task Source
Use the checklist and roadmap at the given paths as the task queue. Update both files programmatically when completing items. Commit messages must reference checklist items by exact text.
