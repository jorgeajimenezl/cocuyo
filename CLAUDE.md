# CLAUDE.md

## Build

```bash
cargo build --release   # release mode recommended for perf
cargo run --release
cargo check             # fast compile check
cargo test
cargo add <crate>       # add dependency
cargo update
cargo clean
```

## Workflow

Always run `cargo check` and `cargo test` after changes. Do not consider a task complete until both pass.

## Code style

Rust edition **2024**. Shared dep versions live in `[workspace.dependencies]` in the root `Cargo.toml`; sub-crates reference them with `.workspace = true`.

## GPU / wgpu

Verify wgpu API names against source or docs before using — they change across versions (e.g. `PollType` vs `Maintain`). Use `Grep` to check existing usage in the codebase first.

## System requirements

### Linux
- Wayland + XDG Desktop Portal (not X11)
- PipeWire, GStreamer (`gstreamer`, `gstreamer-app`, `gstreamer-video`, `gstreamer-allocators`)
- Vulkan with `VK_KHR_external_memory_fd` + `VK_EXT_external_memory_dma_buf` (DMA-BUF zero-copy)
- Native: `pkg-config`, GStreamer/PipeWire/Vulkan dev packages

### Windows
- Windows 10+ with Windows Graphics Capture API
- DirectX 11/12 GPU

### macOS
- macOS 13+ with ScreenCaptureKit
- Metal GPU

## Architecture

Cross-platform screen capture + ambient lighting app. UI built with **iced** (custom fork, wgpu backend).

Cargo workspace — binary at `src/`, platform-agnostic crates under `crates/`:
- **`cocuyo-core`** — `FrameData` (`Cpu | Gpu(Box<dyn GpuFrame>)`), `GpuFrame` trait, `ImportGuard`. Only deps: `wgpu`, `tracing`, `tokio`.
- **`cocuyo-sampling`** — sampling strategies, `GpuSampler`/`SamplingWorker`, WGSL shaders. No platform deps.
- **`cocuyo-platform-linux/windows/macos`** — platform recording + `impl GpuFrame` for `DmaBufFrame` / `HeldFrame` / `IOSurfaceFrame`.

Adding a new capture backend = one struct + one `impl GpuFrame` in a platform crate. Core, sampling, and the widget don't change.

## Non-obvious constraints

**Frame channel**: bounded `tokio::sync::mpsc::channel(2)` — frames are **dropped** (never queued) when full. Do not change this to unbounded; it would break backpressure and stall the capture thread.

**GPU sampler channel**: `std::sync::mpsc::sync_channel(1)` with `try_send`. Callers must handle the `Busy` variant — do not unwrap or block.

**Zero-copy import paths** (`DmaBufFrame`, `HeldFrame`, `IOSurfaceFrame`): calling `mark_import_failed()` disables the path permanently for the session via an atomic flag. Once disabled it won't recover without a restart. Don't call it on transient errors.

**macOS Metal calls**: must be wrapped in `screencapturekit::metal::autoreleasepool(...)` to prevent ObjC object leaks and Cocoa run-loop re-entrancy inside the winit event handler.

**Windows GPU resources**: always acquire the keyed mutex before GPU ops on D3D11/DXGI shared textures, check access flags (READ + WRITE), and watch resource lifetimes. Use-after-free and mutex misuse here have been recurring bug sources.
