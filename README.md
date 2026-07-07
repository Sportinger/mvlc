# mvlc - Modern Video Layered Compositor

mvlc is an early desktop video-canvas prototype. The current branch is a
debuggable Windows-friendly build that can load multiple videos, tile them on
an egui/wgpu canvas, and report runtime performance once per second.

The long-term target is a Linux-first Vulkan/libplacebo compositor with
hardware decode and zero-copy DMA-BUF. That target is documented below, but it
is not the current runtime path.

## Current Status

Branch: `GPTfast`

Current build:

- App binary: `mvlc-app`
- UI/windowing: `winit` + `egui`
- UI render path: `egui-wgpu` on a `wgpu` swapchain
- Media path: external `ffmpeg`/`ffprobe` processes
- Decode output: raw `RGBA` frames over stdout
- Upload path: CPU memory to `wgpu` texture upload
- Multi-video: one FFmpeg process per video
- Queueing: bounded 2-frame channel per stream; stale frames are dropped
- Default decode scale: max width `640`, aspect-ratio preserved
- Default decode FPS: source FPS capped at `30`
- Audio output: `cpal` silence/output clock path exists; FFmpeg media audio is not wired
- Vulkan path: prototype exists behind env flags, not the default app path
- GStreamer/VA-API/DMA-BUF/libplacebo: target architecture, not active in this debug build

Verified locally with:

```powershell
.\target\debug\mvlc-app.exe `
  "E:\_A11\Anduril Unveils Roadrunner  Roadrunner-M.mp4" `
  "E:\_A11\Betaflight  FPV Freestyle.mp4" `
  "E:\_A11\streifen.mp4"
```

Observed debug run: 3 videos at `640x360`, source FPS capped to `24/30/30`,
UI around 60 FPS after startup warmup.

## What Works Now

- Start the app with one or more video file paths as CLI arguments.
- Drag and drop video files into the running window.
- Multiple loaded videos are placed on the canvas and tiled automatically.
- Layers can be moved/scaled through the existing canvas interaction code.
- Play, pause, stop and seek controls are present.
- Runtime badges are shown in the toolbar.
- Runtime debug metrics are written to stdout when enabled.

## Known Limits

- This is a debug playback path, not a production-quality player.
- No zero-copy: every decoded frame is copied from FFmpeg stdout into CPU memory and uploaded to GPU.
- No hardware decode control from mvlc; FFmpeg uses whatever its default build/runtime chooses.
- No audio decode/mix from FFmpeg video files.
- Seek updates logical position but does not restart/reseek the FFmpeg process.
- Pause stops accepting frames but does not pause the FFmpeg process.
- Per-stream frame queues are intentionally tiny; slow UI/render frames drop stale video frames.
- Color management, HDR, libplacebo and Vulkan compositing are placeholders/prototypes.
- Runtime badges still include target-path language and are not a full truth source yet.

## Requirements

Current debug build on Windows:

- Rust toolchain with Cargo
- `ffmpeg` and `ffprobe` available on `PATH`
- A GPU/backend supported by `wgpu`

Check:

```powershell
cargo --version
ffmpeg -version
ffprobe -version
```

Linux target work will additionally require Vulkan, GStreamer, VA-API and
driver-specific packages. That is target work, not required for the current
FFmpeg debug path.

## Build

```powershell
cargo build -p mvlc-app
```

Release build:

```powershell
cargo build -p mvlc-app --release
```

## Run

Debug build:

```powershell
.\target\debug\mvlc-app.exe "path\to\video.mp4"
```

Multiple videos:

```powershell
.\target\debug\mvlc-app.exe "video-a.mp4" "video-b.mp4" "video-c.mp4"
```

Release build:

```powershell
.\target\release\mvlc-app.exe "path\to\video.mp4"
```

## Debug And Smoke Tests

Enable runtime metrics:

```powershell
$env:MVLC_DEBUG = "1"
.\target\debug\mvlc-app.exe "path\to\video.mp4"
```

Run a bounded smoke test:

```powershell
$env:MVLC_DEBUG = "1"
$env:MVLC_EXIT_AFTER_SECS = "10"
.\target\debug\mvlc-app.exe "video-a.mp4" "video-b.mp4" "video-c.mp4"
```

Typical `mvlc_debug` line:

```text
ui_fps=60.0 worst_frame_ms=17.8 slow_frames=14 layers=3 videos=3 playing=true decoded_frames=1116 decode_fps=80.5 upload=900.0 KB/frame
```

Metric meaning:

- `ui_fps`: egui/wgpu redraw loop FPS over the last second
- `worst_frame_ms`: slowest UI frame in the last second
- `slow_frames`: UI frames over 16.7 ms in the last second
- `layers`: project layer count
- `videos`: active decoder count
- `playing`: transport state
- `decoded_frames`: total frames accepted by the app
- `decode_fps`: accepted decoded frames per second since app start
- `upload`: average CPU-to-GPU upload bytes per accepted frame

## Environment Variables

Current variables:

- `MVLC_DEBUG=1|true|yes|on`: enables once-per-second runtime debug logging.
- `MVLC_EXIT_AFTER_SECS=N`: exits automatically after `N` seconds; useful for smoke tests.
- `MVLC_MAX_DECODE_WIDTH=N`: max decoded video width before upload. Default: `640`.
- `MVLC_VULKAN_SWAPCHAIN=1|true|yes|on`: starts the Vulkan prototype instead of default wgpu app path.
- `MVLC_VULKAN_UI_NATIVE=1`: attempts native Vulkan egui UI in the Vulkan prototype path.

Planned target variables, not fully wired in the current debug path:

- `MVLC_HWDECODE=1|0`
- `MVLC_ZERO_COPY=1|0`
- `MVLC_COLOR=placebo|basic`
- `MVLC_PRESENT=mailbox|fifo`
- `MVLC_TRACE=path.ndjson`

## Current Architecture

```text
CLI paths / Drag-and-drop
        |
        v
  AppState / LayerStack
        |
        v
  HardwareVideoDecoder facade
        |
        v
  ffprobe -> stream metadata
  ffmpeg  -> RGBA rawvideo stdout
        |
        v
  bounded channel, newest accepted frame wins
        |
        v
  wgpu texture upload
        |
        v
  egui painter on wgpu swapchain
```

Important current files:

- `crates/app/src/main.rs`: window loop, startup file loading, debug ticker, env flags.
- `crates/app/src/media.rs`: drag-and-drop and CLI media loading flow.
- `crates/app/src/app_state.rs`: project state, transport, tiling, frame polling.
- `crates/app/src/video_layer.rs`: frame upload to `wgpu` textures.
- `crates/media/src/gstreamer.rs`: current FFmpeg-backed decoder facade.
- `crates/render/src/*`: renderer abstraction, Vulkan prototype, color placeholders.
- `crates/mvlc-core/src/*`: layers, project model, sync and performance counters.

## Workspace Layout

```text
mvlc/
  Cargo.toml
  README.md
  docs/
    roadmap.md
    checklist.md
    mvlc_agent_boot_prompt.md
  crates/
    app/         # winit/egui app, canvas, transport, startup/debug plumbing
    media/       # current FFmpeg decoder facade plus cpal audio scaffolding
    mvlc-core/   # layer/project/time/sync/performance types
    render/      # renderer abstraction, wgpu bridge, Vulkan prototype
    project/     # project crate scaffold
    cli/         # CLI crate scaffold
```

## Runtime Badges

The UI currently shows badges for:

- `Decode`
- `Transfer`
- `Performance`
- `Color`
- `Render`
- `Sync`

Treat them as coarse status indicators. They are not yet a complete source of
truth because the current FFmpeg debug backend and the target GStreamer/Vulkan
backend share some old badge labels.

## Target Architecture

The intended production architecture is still:

- Linux-first media stack
- GStreamer primary decode path
- VA-API hardware decode where available
- DMA-BUF zero-copy transfer into Vulkan
- Vulkan compositor
- libplacebo color pipeline for YUV/RGB, HDR/SDR and tonemapping
- Audio master clock for A/V sync
- Explicit runtime badges for decode, color, transfer, render and sync paths
- Trace/benchmark artifacts for frame timing and upload costs

Target diagram:

```text
GStreamer / VA-API
        |
        | DMA-BUF
        v
Vulkan external memory
        |
        v
libplacebo color/render path
        |
        v
winit/egui desktop UI
```

## Technical Goals

Current debug build goals:

- Make multi-video startup reproducible.
- Keep UI responsive under multiple videos by capping decode size/FPS.
- Drop stale frames instead of growing unbounded queues.
- Log enough metrics to diagnose whether decode, upload or UI is the bottleneck.

Production goals:

- Zero-copy upload bytes in the optimal path.
- Stable frame pacing with low present jitter.
- Correct A/V sync using audio as master clock.
- Correct color pipeline and HDR handling.
- Accurate path transparency in UI and logs.

## Troubleshooting

`ffmpeg` or `ffprobe` not found:

```powershell
ffmpeg -version
ffprobe -version
```

If more videos stutter, lower decode width:

```powershell
$env:MVLC_MAX_DECODE_WIDTH = "480"
```

If startup should be testable without manually closing the window:

```powershell
$env:MVLC_EXIT_AFTER_SECS = "10"
```

If the app shows gray layer boxes instead of video:

- Check stdout for `FFmpeg decoder stream ...`.
- Check that `decoded_frames` increases in `mvlc_debug`.
- Check that `ffmpeg` can open the file directly.

If UI FPS drops below 60:

- Check `worst_frame_ms` and `slow_frames`.
- Lower `MVLC_MAX_DECODE_WIDTH`.
- Use fewer simultaneous videos.
- Build with `--release` for a more realistic performance check.

## Roadmap

Detailed target planning lives in `docs/roadmap.md`.

The README is the current runtime contract. Roadmap items are not guarantees
until implemented and verified in this branch.

## License

Workspace metadata currently declares `MIT OR Apache-2.0`.
Third-party tools and libraries keep their own licenses.
