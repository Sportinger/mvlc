# Vulkan Migration Plan

## Phase 0 – Baseline Extraction ✅
- Isolate the legacy wgpu presenter while preparing a toggleable preview path.

## Phase 1 – Swapchain Bootstrap ✅
- Vulkan swapchain clears, resizes, and presents when `MVLC_VULKAN_SWAPCHAIN=1`.

## Phase 2 – UI Bridge (In Progress)
- CPU bridge: wgpu off-screen renderer uploads egui output into Vulkan swapchain overlays. ✅
- Keep wgpu-based presenter as default during iteration.
- Track Vulkan UI bridge work in `crates/render/src/ui_bridge.rs`.
- Next focus areas to drop the CPU readback bridge:
  - [x] Reuse persistent wgpu resources (texture + readback buffer) instead of recreating them every frame.
  - [x] Introduce a Vulkan-native egui renderer so UI meshes land straight in a command buffer (no host copies).
  - [ ] Once libplacebo owns the swapchain image, composite UI via libplacebo or a shared Vulkan render pass.
- Tracking: `UiBackend` in `crates/render/src/ui_bridge.rs` now isolates the wgpu bridge; the `VulkanNative` variant is ready for a real libplacebo-powered overlay path.
  - `MVLC_VULKAN_UI_NATIVE=1` flips the bridge to the Vulkan path while keeping the wgpu fallback as default.

## Phase 3 – libplacebo Test Quad (Pending)
- Introduce feature-gated libplacebo bootstrap on the Vulkan device. ✅ (vendor stub in place)
- Render the canonical libplacebo quad into the swapchain image.
- Flip Render/Color badges to the Vulkan/libplacebo path.
- Replace stub with real libplacebo integration (headers + context creation).

## Phase 4 – Full Integration (Pending)
- Move video compositing and UI overlays fully onto Vulkan/libplacebo.
- Enable DMA-BUF import for hardware decode.

## Phase 5 – Cleanup & Documentation (Pending)
- Remove legacy wgpu surface path once feature parity is achieved.
- Update specs/checklist; address outstanding warnings in render/media crates.
