# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build Commands

```bash
# Build (release mode recommended for performance)
cargo build --release

# Run the application
cargo run --release

# Check for compilation errors without building
cargo check

# Run tests
cargo test
```

## Manage Dependencies

```bash
# Add a new dependency
cargo add <crate-name>

# Update dependencies
cargo update

# Clean build artifacts
cargo clean
```

## GPU / wgpu

This is a Rust project using wgpu for GPU compute. When writing wgpu code, verify API names against the actual source/docs before using them - e.g., `PollType` vs `Maintain` changed across versions. Use `Grep` to check existing usage patterns in the codebase first.

## Workflow

Always run `cargo build` and `cargo test` after making changes. Do not consider a task complete until compilation succeeds and tests pass.

## System Requirements

### Linux
- **Wayland session** with XDG Desktop Portal support (not X11)
- **PipeWire** for screen capture
- **GStreamer** libraries (gstreamer, gstreamer-app, gstreamer-video, gstreamer-allocators)
- **Vulkan** runtime with `VK_KHR_external_memory_fd` and `VK_EXT_external_memory_dma_buf` extensions (for zero-copy DMA-BUF import)
- Native dependencies: `pkg-config`, GStreamer dev packages, PipeWire dev packages, Vulkan dev packages

### Windows
- Windows 10+ with Windows Graphics Capture API
- DirectX 11/12 capable GPU

### macOS
- macOS 13+ with ScreenCaptureKit
- Metal-capable GPU

## Architecture Overview

Cocuyo is a cross-platform screen capture application with ambient lighting support. On Linux it captures via PipeWire/Wayland; on Windows it uses the Windows Graphics Capture API; on macOS it uses ScreenCaptureKit. The UI is built with **iced** (v0.14, custom fork) using the wgpu backend with custom window decorations (daemon mode, multi-window). Rust edition is **2024**.

### Workspace Layout

The project is a Cargo workspace. The binary crate `cocuyo` lives at the repo root (`src/`) and depends on path-local sibling crates under `crates/`:

- **`crates/core`** (`cocuyo-core`) — Platform-agnostic frame transport. Owns `FrameData` + per-variant payload types (`HeldFrame` on Windows, IOSurface wrapper on macOS, DMA-BUF read helpers on Linux), `RecordingCommand`/`RecordingEvent`/`RecordingState`, `texture_format` helpers. Takes the heavy target-cfg deps (`windows`, `windows-capture` types, `nix`+`drm-fourcc`, `screencapturekit`) so the binary's translation unit doesn't have to. Stable, cache-hit on incremental builds.
- **`crates/sampling`** (`cocuyo-sampling`) — `SamplingStrategy` trait, `BoxedStrategy`, all strategies (`Average`/`Max`/`Min`/`Palette`), `GpuSampler` + `SamplingWorker`, WGSL compute shaders, `Region`/`ContainLayout`. Depends on `cocuyo-core` and (target-cfg) the platform crates for GPU texture imports inside `gpu.rs`.
- **`crates/platform-windows`** (`cocuyo-platform-windows`) — Windows recording subscription, `CaptureTarget`/`PickerTab`/`PickerIntent`, `dx12_import`, `SharedTexturePool`. Owns `windows-capture` and the Direct3D `windows` features.
- **`crates/platform-linux`** (`cocuyo-platform-linux`) — PipeWire stream, portal session, GStreamer pipeline, Vulkan DMA-BUF import, format tables. Owns `gstreamer*`, `pipewire`, `ashpd`, `ash`, `nix`.
- **`crates/platform-macos`** (`cocuyo-platform-macos`) — ScreenCaptureKit recording subscription, Metal IOSurface import. Owns `screencapturekit`, `metal`, `objc2`.

The binary crate (`src/`) keeps only what is edited often: `main.rs`, `app.rs`, `config.rs`, `adapters.rs`, `ambient.rs`, `gpu_context.rs`, `perf_stats.rs`, `theme.rs`, `tray.rs`, all of `screen/*` and `widget/*`. Editing UI code only recompiles the binary; the platform/sampling/core crates stay cached. Crate folder names drop the `cocuyo-` prefix (`crates/core`, `crates/sampling`, etc.) but the package names retain it.

Shared dependency versions are centralized in `[workspace.dependencies]` in the root `Cargo.toml`; sub-crates reference them with `.workspace = true`.

### Data Flow

#### Linux
1. **Portal Session** (`platform/linux/stream.rs:open_portal`) - Uses ashpd to request screen capture permission via XDG Desktop Portal
2. **PipeWire Stream** (`platform/linux/stream.rs:start_streaming`) - Connects to PipeWire node using the portal-provided file descriptor
3. **Frame Processing** (`platform/linux/stream.rs`) - Three paths, tried in priority order:
   - **DMA-BUF zero-copy Vulkan import** - When the buffer has a linear modifier and an importable DRM format (Xrgb8888, Argb8888, Abgr8888, Xbgr8888), the DMA-BUF fd is sent directly to the renderer which imports it as a wgpu texture via Vulkan external memory (`platform/linux/vulkan_dmabuf.rs`)
   - **DMA-BUF via GStreamer** - For DMA-BUF buffers with non-linear modifiers or non-importable formats, GStreamer converts to RGBA via CPU
   - **CPU copy fallback** - Traditional memory copy when no DMA-BUF is available
4. **Format Conversion** (`platform/linux/gst_pipeline.rs`) - GStreamer pipeline converts input formats to RGBA. Supports GPU-accelerated backends (CUDA, VA-API) and CPU fallback
5. **Display** - iced shader widget (`widget/video_shader.rs`) renders frames using wgpu, importing DMA-BUF textures directly or uploading CPU RGBA data

#### Windows
1. **Capture Target Selection** (`screen/capture_picker.rs`) - User selects a monitor or window via picker UI
2. **Windows Graphics Capture** (`platform/windows/recording.rs`) - Uses `windows-capture` crate with `GraphicsCaptureApiHandler`
3. **Frame Processing** - Two paths:
   - **Zero-copy**: `SharedTexturePool` copies WGC texture to a shared D3D11 texture with NT handle, sent as `FrameData::D3DShared`
   - **CPU fallback**: Reads pixels from GPU to CPU memory, sent as `FrameData::Cpu`
4. **Display** - `widget/video_shader.rs` imports shared texture via DX12 HAL or uploads CPU data

#### macOS
1. **Content Sharing Picker** - System content sharing picker for screen/window selection
2. **ScreenCaptureKit** (`platform/macos/recording.rs`) - Uses `screencapturekit` crate for screen capture with configurable frame rate and resolution
3. **Frame Processing** - IOSurface zero-copy path:
   - **Zero-copy**: IOSurface sent as `FrameData::IOSurface`, imported into Metal texture via `metal_import.rs`
   - **CPU fallback**: Pixel data read from IOSurface, sent as `FrameData::Cpu`
4. **Display** - `widget/video_shader.rs` imports Metal texture via wgpu HAL or uploads CPU data

#### Ambient Lighting
1. User selects WiZ smart bulbs via `BulbSetup` screen
2. Regions are auto-created per selected bulb, editable on the video preview via `widget/region_overlay.rs`
3. Each region has a configurable sampling strategy (Average, Max, Min, Palette)
4. On "Start Ambient", original bulb states are saved, recording starts, `SamplingWorker` spawns a background GPU sampling thread
5. Each frame: `SamplingWorker::try_send()` submits frame + regions to the GPU sampler thread → compute shaders sample all GPU-capable regions in one submission → CPU fallback for unsupported strategies → `ambient::dispatch_bulb_colors()` to WiZ bulbs (throttled to every 150ms)
6. On "Stop Ambient", original bulb states are restored

### Key Components

- **`main.rs`** (binary) - Application entry point, loads config, sets wgpu adapter via env vars, initializes GStreamer/PipeWire (Linux), launches iced daemon
- **`app.rs`** (binary) - `Cocuyo` application state, iced message handling, multi-window management (`Main`, `Settings`, `BulbSetup`, `CapturePicker`). Recording state, regions, bulb state, and config owned directly (no mutexes).
- **`cocuyo-core::frame`** - Platform-agnostic `FrameData` enum with `DmaBuf` (Linux), `IOSurface` (macOS), `D3DShared` (Windows), and `Cpu` variants
- **`cocuyo-core::recording`** - Platform-agnostic recording types (`RecordingCommand`, `RecordingEvent`, `RecordingState`)
- **`config.rs`** (binary) - Persistent app configuration via TOML (`~/.config/cocuyo/config.toml`). Stores preferred adapter, backend, saved bulbs, selected bulb MACs, capture settings (fps limit, resolution scale), ambient settings (update interval, brightness, color temp), and UI preferences (minimize to tray, perf overlay).
- **`adapters.rs`** (binary) - GPU adapter enumeration and selection (`GpuAdapter`, `GpuAdapterSelection`)
- **`ambient.rs`** (binary) - WiZ smart bulb discovery, color mapping, state save/restore, frame sampling dispatch
- **`cocuyo-sampling::region`** - Screen capture region definitions and coordinate transformations (`Region`, `ContainLayout`)
- **`theme.rs`** (binary) - Custom iced theme and styling
- **`gpu_context.rs`** - Global `OnceLock` storing the wgpu `Device`/`Queue` for use outside the shader widget (set once from `VideoPipeline::new()`)
- **`tray.rs`** - System tray icon and menu (Windows/macOS only; Linux stub). Menu items: Show/Hide, Start/Stop Ambient, Exit
- **`perf_stats.rs`** - Performance metrics tracking with EMA smoothing (alpha=0.05): effective FPS, frame interval, sampling time, bulb dispatch duration. Fingerprinting for HUD cache invalidation

#### Sampling Crate (`crates/sampling/` — `cocuyo-sampling`)
- **`lib.rs`** - `SamplingStrategy` trait (with `supports_gpu()` opt-in), `BoxedStrategy` type-erased wrapper (for iced pick_list), `sample_region()` function, `sample_extremum()` unified max/min helper, strategy registry
- **`average.rs`** - `Average` strategy: computes mean RGB across sampled pixels (GPU-capable)
- **`max.rs`** - `Max` strategy: finds brightest pixel by luminance (CPU-only, delegates to `sample_extremum`)
- **`min.rs`** - `Min` strategy: finds darkest pixel by luminance (CPU-only, delegates to `sample_extremum`)
- **`palette.rs`** - `Palette` strategy: dominant color via fixed histogram quantization (8×8×8 bins, GPU-capable). `extract_dominant_from_histogram()` shared between GPU readback and CPU path
- **`gpu.rs`** - `GpuSampler` compute pipeline engine and `SamplingWorker` background thread. Imports frames as wgpu textures (DMA-BUF/D3DShared/CPU), dispatches per-region compute passes (average + palette pipelines), reads back results via mapped buffers. `SamplingWorker` wraps the sampler in a dedicated thread with `try_send()` returning `iced::Task`
- **`gpu_average.wgsl`** - WGSL compute shader for average color sampling (atomic sum + count)
- **`gpu_palette.wgsl`** - WGSL compute shader for histogram-based palette extraction (512-bin atomic histogram)

#### Widget Module (`widget/`)
- **`mod.rs`** - Module exports, `Element<'a, Message>` type alias
- **`video_shader.rs`** - Custom iced shader widget for rendering video frames (DMA-BUF/D3DShared/CPU)
- **`video_shader.wgsl`** - WGSL shader for fullscreen video rendering with aspect ratio correction
- **`title_bar.rs`** - Custom window title bar (drag, minimize, maximize, close)
- **`region_overlay.rs`** - Interactive canvas overlay for drawing/editing capture regions on the preview
- **`perf_hud.rs`** - Performance HUD overlay displaying FPS, sample time, and bulb dispatch metrics

#### Screen Module (`screen/`)
- **`main_window.rs`** - Main control window (preview, controls, region list, status bar)
- **`settings.rs`** - GPU adapter selection and backend selection (Linux GStreamer backend)
- **`bulb_setup.rs`** - WiZ bulb discovery and selection UI
- **`capture_picker.rs`** - Windows-only capture target picker (monitors, windows)

#### Platform: Linux (`crates/platform-linux/` — `cocuyo-platform-linux`)
- **`recording.rs`** - Linux recording subscription via PipeWire as `iced::Subscription`
- **`stream.rs`** - PipeWire stream setup, portal session, frame processing pipeline (DMA-BUF and CPU paths). Uses bounded `mpsc::Sender<Arc<FrameData>>` with backpressure.
- **`gst_pipeline.rs`** - GStreamer video converter with GPU backend detection (CUDA, OpenGL, CPU). `GstVideoConverter` handles appsrc → videoconvert → appsink.
- **`vulkan_dmabuf.rs`** - Vulkan DMA-BUF import: creates VkImage with external memory, wraps into wgpu texture via `wgpu_hal`
- **`dmabuf_handler.rs`** - DMA-BUF metadata extraction from PipeWire buffers (fd, stride, format, dimensions, modifier). The CPU pixel-read fallback (`read_dmabuf_pixels`) lives in `cocuyo-core::linux`.
- **`formats.rs`** - Unified video format conversion tables: PipeWire SPA → GStreamer, PipeWire SPA → DRM fourcc, DRM → Vulkan format, DRM → wgpu format, importability check

#### Platform: Windows (`crates/platform-windows/` — `cocuyo-platform-windows`)
- **`recording.rs`** - Windows recording subscription using `windows-capture` crate
- **`capture_target.rs`** - `CaptureTarget` enum (Monitor/Window), `PickerTab`, `PickerIntent`
- **`dx12_import.rs`** - DX12 shared texture import into wgpu
- **`SharedTexturePool`** - `SharedTexturePool` and `SharedTextureSlot` for GPU-GPU zero-copy frame delivery via shared D3D11 textures with keyed mutexes. The `HeldFrame` type that the pool hands out lives in `cocuyo-core::windows` (so `FrameData::D3DShared` is constructible from both crates).

#### Platform: macOS (`crates/platform-macos/` — `cocuyo-platform-macos`)
- **`recording.rs`** - macOS recording subscription using `screencapturekit` crate. Configurable frame rate and resolution via `capture_resolution_scale`
- **`metal_import.rs`** - IOSurface import into wgpu texture via Metal HAL (zero-copy). The `strip_stride_padding` helper lives in `cocuyo-core::macos`.

### Threading Model

- Main thread: iced event loop (daemon mode with multi-window)
- Recording lifecycle driven by `iced::Subscription` — when `is_recording` is true, the subscription spawns a capture thread:
  - **Linux**: `std::thread::spawn` for PipeWire main loop
  - **Windows**: `CaptureHandler::start_free_threaded` for Windows Graphics Capture
  - **macOS**: ScreenCaptureKit stream with configurable frame rate and resolution
- Frame data sent via bounded `tokio::sync::mpsc::channel(2)` with backpressure (frames dropped when full)
- Frames delivered to UI as `Message::RecordingEvent(Frame(Arc<FrameData>))` — no polling tick
- Recording state updated via `Message::RecordingEvent(StateChanged(...))` — owned by UI, no mutexes
- Bidirectional control: subscription sends `RecordingEvent::Ready(cmd_tx)` at startup; app stores the `cmd_tx` sender for issuing `RecordingCommand::Stop`
- Graceful stop: app sends `RecordingCommand::Stop` via command channel → subscription cleans up capture resources → emits `StateChanged(Idle)` → app sets `is_recording = false`
- **GPU sampling thread**: `SamplingWorker` spawns a dedicated `gpu-sampler` thread that owns a `GpuSampler` (wgpu Device/Queue). Requests sent via `std::sync::mpsc::sync_channel(1)` (backpressure: `try_send` returns `Busy` if worker is occupied). Results returned via `tokio::sync::oneshot` channel, wrapped in `iced::Task`
- **Windows GPU sync**: `SharedTexturePool` uses `Arc::strong_count` for slot availability (backpressure), `IDXGIKeyedMutex` for GPU synchronization, event query + `Flush()` + spin-wait for GPU completion
- **Linux DMA-BUF lifetime**: `HeldBuffer` deque (3 entries) manages PipeWire buffer lifetimes

### Supported Video Formats

RGB, RGBA, RGBx, BGRx, YUY2, I420 - all converted to BGRA via GStreamer (Linux). CPU frame data is internally stored as BGRA across all platforms.

### DMA-BUF Zero-Copy Path (Linux)

For eligible buffers (linear modifier + importable DRM format), frames bypass GStreamer entirely:
1. PipeWire buffer's DMA-BUF fd is dup'd and sent as `FrameData::DmaBuf`
2. The shader widget (`widget/video_shader.rs`) imports it via `vulkan_dmabuf::import_dmabuf_texture`
3. Vulkan creates a VkImage backed by the imported DMA-BUF memory
4. The VkImage is wrapped into a `wgpu::Texture` via `wgpu_hal`
5. If import fails, `mark_dmabuf_import_failed()` disables the path for all future frames

### D3D Shared Texture Zero-Copy Path (Windows)

For GPU-GPU frame delivery without CPU readback:
1. `SharedTexturePool` pre-allocates D3D11 textures with `NT_HANDLE` sharing
2. Capture handler copies WGC frame to a shared texture slot via `CopyResource`
3. Frame sent as `FrameData::D3DShared` with `Arc<SharedTextureSlot>`
4. The shader widget imports the shared texture into wgpu via DX12 HAL (`dx12_import.rs`)
5. Keyed mutex ensures GPU synchronization between D3D11 capture and D3D12/wgpu rendering

### IOSurface Zero-Copy Path (macOS)

For GPU-GPU frame delivery via Metal:
1. ScreenCaptureKit produces IOSurface-backed frames in BGRA format
2. Frame sent as `FrameData::IOSurface` with the IOSurface handle
3. The shader widget imports the IOSurface into a wgpu texture via Metal HAL (`metal_import.rs`)
4. Metal creates a texture backed by the IOSurface memory (zero-copy)

#### Note:

When dealing with Windows GPU resources (D3D11, DXGI shared textures), pay careful attention to: 1) mutex acquisition before GPU operations, 2) access flags (READ + WRITE), 3) resource lifetime and use-after-free. These have been recurring bug sources.
