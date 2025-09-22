# Vulkan Migration Plan

## Phase 0 – Baseline Extraction ✅
Status: Completed (legacy wgpu presenter left intact while factoring rendering modules)
- Audit legacy renderer entry points and isolate wgpu-specific logic.
- Establish toggles/env flags for alternative presenters.

## Phase 1 – Swapchain Bootstrap ✅
Status: Completed (`MVLC_VULKAN_SWAPCHAIN=1` launches the native Vulkan swapchain that clears the backbuffer and handles resize/present).
- Create Vulkan instance/device/queues and per-frame sync objects.
- Wire swapchain acquisition/presentation and command buffer recording (clear pass) into `VulkanRenderer`.
- Keep wgpu path as default for day-to-day use.

## Phase 2 – UI Bridge (In Progress)
Goals:
- Render egui/canvas output into an off-screen target (initially CPU-uploaded).
- Composite the off-screen image into the Vulkan swapchain in preview mode.
- Maintain feature parity: drag/drop, badges, and canvas interaction visible when preview flag is set.

Immediate tasks:
1. Allocate shared textures for egui output and copy/upload into Vulkan images.
2. Introduce a “UI bridge” module to manage egui textures (see `crates/render/src/ui_bridge.rs`).
3. Update `run_vulkan` loop to drive egui logic and overlay the UI on top of the cleared swapchain.

## Phase 3 – libplacebo Test Quad (Pending)
- Add libplacebo bindings (`libplacebo-sys` or wrapper crate).
- Initialize libplacebo GPU context on the Vulkan device and render the test quad.
- Flip Render/Color badges to `Vulkan(libplacebo)` when active.

## Phase 4 – Full Integration (Pending)
- Move video frame compositing from egui/wgpu path into Vulkan/libplacebo.
- Introduce DMA-BUF import plumbing for hardware decode path.
- Remove reliance on egui textures for video layers.

## Phase 5 – Cleanup & Documentation (Pending)
- Delete unused wgpu surface pipeline once Vulkan path covers all features.
- Document renderer architecture (spec updates) and checklist adjustments.
- Address outstanding lint warnings in render/media crates.
