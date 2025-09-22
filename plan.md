# Vulkan Migration Plan

## Goal
Complete the transition from the legacy wgpu swapchain to a Vulkan + libplacebo renderer while keeping egui-based tooling usable. Track progress across distinct phases so intermediate builds remain testable.

## Phase Outline
1. **Swapchain Bootstrap** *(done)* — bring up a native Vulkan swapchain that clears the window and handles resize/present correctly. Keep the old wgpu path available as the default experience.
2. **UI Bridge** *(in progress)* — render egui + canvas output off-screen and composite into the Vulkan backbuffer (temporary CPU upload is acceptable). Ensure both pipelines can be toggled at runtime for comparison.
3. **libplacebo Test Quad** — initialize libplacebo on the Vulkan device, render the canonical quad, and route badges to the new path.
4. **Full Integration** — replace the wgpu presenter entirely: video compositing, UI, and badges all flow through Vulkan/libplacebo. Enable DMA-BUF import wiring.
5. **Cleanup** — delete unused wgpu surface code, document the renderer pipeline, and stabilize API for later DMA-BUF work.

## Open Questions
- Preferred strategy for egui compositing (CPU upload vs. shared image interop).
- libplacebo binding choice (`libplacebo-sys` vs. higher-level crate).

## Next Steps
- Implement Phase 2: add an off-screen egui render target and copy it into the Vulkan swapchain in the preview mode (`MVLC_VULKAN_SWAPCHAIN=1`).
- Update the checklist items when Vulkan becomes the default presenter and the libplacebo quad renders successfully.
