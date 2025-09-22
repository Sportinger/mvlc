# Vulkan Swapchain & libplacebo Migration Plan

## Goal
Replace the wgpu-based swapchain with a native Vulkan pipeline that drives libplacebo for color-correct rendering, while keeping egui-based UI functional. Outcome: M0 checklist items “Vulkan Swapchain init” and “libplacebo minimal binden und Test-Quad rendern” checked, and groundwork laid for DMA-BUF zero-copy in M2.

## Constraints & Risks
- wgpu currently owns the window surface; Vulkan swapchain must take exclusive control.
- egui must continue rendering (either via Vulkan backend or offscreen wgpu texture composited into Vulkan).
- Need deterministic teardown/reset paths to avoid device-loss crashes.
- libplacebo requires Vulkan instance/device setup with feature negotiation; limited bindings available in Rust.

## Phased Approach

### Phase 0 — Baseline Extraction
1. Audit `GraphicsState` in `crates/app/src/main.rs` and isolate all wgpu-only dependencies.
2. Introduce an abstraction so the UI loop can plug in a “presenter” that provides `egui::Context` input/output without assuming wgpu.

### Phase 1 — Vulkan Swapchain Bootstrap
1. Create new module `crates/render/src/device.rs` (or similar) that owns:
   - Vulkan instance/device/queue selection via `ash` + `ash-window`.
   - Surface creation from winit window.
   - Swapchain lifecycle (image acquisition, present).
2. Provide a simple color-only render pass that clears to black and draws a full-screen triangle (no libplacebo yet).
3. Replace `GraphicsState` usage in the app with the Vulkan swapchain presenter; ensure event loop requests redraw and presents frames successfully.
4. Integrate egui by rendering into an offscreen wgpu texture (temporary) and compositing into Vulkan via sampled image, or by adopting egui’s custom Vulkan backend (decision to be made during Phase 1).

### Phase 2 — libplacebo Integration (Test Quad)
1. Add libplacebo bindings to `crates/render/Cargo.toml` (likely `libplacebo-sys` initially).
2. Initialize libplacebo context (`pl_context`, `pl_gpu`) using the Vulkan device handles from Phase 1.
3. Create a libplacebo swapchain or FBO targeting our Vulkan swapchain images.
4. Render the canonical libplacebo test quad (color bars) each frame; verify output.
5. Surface status via Render/Color badges (turn “Render” to `Vulkan(libplacebo)` and “Color” away from fallback).

### Phase 3 — UI Reconciliation
1. If egui was rendered offscreen in Phase 1, move to a Vulkan-native egui backend or integrate libplacebo compositing to blend UI layer over video.
2. Update `AppState`/canvas drawing to feed frames into libplacebo instead of wgpu textures.
3. Provide minimal tracing/metrics for swapchain frame times, errors.

### Phase 4 — Cleanup & Checklist Update
1. Remove legacy wgpu structures/modules no longer used.
2. Ensure `docs/checklist.md` items for swapchain and libplacebo quad are ticked with commit references.
3. Add documentation in `docs/renderer.spec.md` (or create) summarizing the new pipeline.
4. Re-run `cargo fmt`, `cargo clippy`, `cargo run` to validate.

## Open Questions
- Preferred strategy for egui rendering during migration (Vulkan backend vs. offscreen + composite).
- libplacebo binding selection (`libplacebo` crate vs. `libplacebo-sys` + manual wrappers).
- Minimum Vulkan features/extensions we must enforce (e.g., descriptor indexing for libplacebo).

## Next Steps
- Decide egui integration approach and document pros/cons.
- Begin Phase 0 implementation, keeping commits focused per phase.
