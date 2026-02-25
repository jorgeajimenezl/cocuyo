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

## Architecture Overview

Cocuyo is a cross-platform screen capture application with ambient lighting support. On Linux it captures via PipeWire/Wayland; on Windows it uses the Windows Graphics Capture API. The UI is built with **iced** (v0.14) using the wgpu backend with custom window decorations (daemon mode, multi-window). Rust edition is **2024**.

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

#### Ambient Lighting
1. User selects WiZ smart bulbs via `BulbSetup` screen
2. Regions are auto-created per selected bulb, editable on the video preview via `widget/region_overlay.rs`
3. Each region has a configurable sampling strategy (Average, Max, Min)
4. On "Start Ambient", original bulb states are saved, recording starts
5. Each frame: `frame.convert_to_cpu()` → per-region `sampling::sample_region()` → `ambient::dispatch_bulb_colors()` to WiZ bulbs (throttled to every 150ms)
6. On "Stop Ambient", original bulb states are restored

### Key Components

- **`main.rs`** - Application entry point, loads config, sets wgpu adapter via env vars, initializes GStreamer/PipeWire (Linux), launches iced daemon
- **`app.rs`** - `Cocuyo` application state, iced message handling, multi-window management (`Main`, `Settings`, `BulbSetup`, `CapturePicker`). Recording state, regions, bulb state, and config owned directly (no mutexes).
- **`frame.rs`** - Platform-agnostic `FrameData` enum with `DmaBuf` (Linux), `D3DShared` (Windows), and `Cpu` variants
- **`recording.rs`** - Platform-agnostic recording types (`RecordingCommand`, `RecordingEvent`) with conditional re-exports from platform modules
- **`config.rs`** - Persistent app configuration via TOML (`~/.config/cocuyo/config.toml`). Stores preferred adapter, backend, saved bulbs, selected bulb MACs.
- **`adapters.rs`** - GPU adapter enumeration and selection (`GpuAdapter`, `GpuAdapterSelection`)
- **`ambient.rs`** - WiZ smart bulb discovery, color mapping, state save/restore, frame sampling dispatch
- **`region.rs`** - Screen capture region definitions and coordinate transformations
- **`theme.rs`** - Custom iced theme and styling

#### Sampling Module (`sampling/`)
- **`mod.rs`** - `SamplingStrategy` trait, `BoxedStrategy` type-erased wrapper (for iced pick_list), `sample_region()` function, strategy registry
- **`average.rs`** - `Average` strategy: computes mean RGB across sampled pixels
- **`max.rs`** - `Max` strategy: finds brightest pixel by luminance
- **`min.rs`** - `Min` strategy: finds darkest pixel by luminance

#### Widget Module (`widget/`)
- **`mod.rs`** - Module exports, `Element<'a, Message>` type alias
- **`video_shader.rs`** - Custom iced shader widget for rendering video frames (DMA-BUF/D3DShared/CPU)
- **`video_shader.wgsl`** - WGSL shader for fullscreen video rendering with aspect ratio correction
- **`title_bar.rs`** - Custom window title bar (drag, minimize, maximize, close)
- **`region_overlay.rs`** - Interactive canvas overlay for drawing/editing capture regions on the preview

#### Screen Module (`screen/`)
- **`main_window.rs`** - Main control window (preview, controls, region list, status bar)
- **`settings.rs`** - GPU adapter selection and backend selection (Linux GStreamer backend)
- **`bulb_setup.rs`** - WiZ bulb discovery and selection UI
- **`capture_picker.rs`** - Windows-only capture target picker (monitors, windows)

#### Platform: Linux (`platform/linux/`)
- **`recording.rs`** - Linux recording subscription via PipeWire as `iced::Subscription`
- **`stream.rs`** - PipeWire stream setup, portal session, frame processing pipeline (DMA-BUF and CPU paths). Uses bounded `mpsc::Sender<Arc<FrameData>>` with backpressure.
- **`gst_pipeline.rs`** - GStreamer video converter with GPU backend detection (CUDA, OpenGL, CPU). `GstVideoConverter` handles appsrc → videoconvert → appsink.
- **`vulkan_dmabuf.rs`** - Vulkan DMA-BUF import: creates VkImage with external memory, wraps into wgpu texture via `wgpu_hal`
- **`dmabuf_handler.rs`** - DMA-BUF metadata extraction from PipeWire buffers (fd, stride, format, dimensions, modifier) and pixel reading
- **`formats.rs`** - Unified video format conversion tables: PipeWire SPA → GStreamer, PipeWire SPA → DRM fourcc, DRM → Vulkan format, DRM → wgpu format, importability check

#### Platform: Windows (`platform/windows/`)
- **`recording.rs`** - Windows recording subscription using `windows-capture` crate
- **`capture_target.rs`** - `CaptureTarget` enum (Monitor/Window), `PickerTab`, `PickerIntent`
- **`dx12_import.rs`** - DX12 shared texture import into wgpu
- **`shared_texture.rs`** - `SharedTexturePool` and `SharedTextureSlot` for GPU-GPU zero-copy frame delivery via shared D3D11 textures with keyed mutexes

### Threading Model

- Main thread: iced event loop (daemon mode with multi-window)
- Recording lifecycle driven by `iced::Subscription` — when `is_recording` is true, the subscription spawns a capture thread:
  - **Linux**: `std::thread::spawn` for PipeWire main loop
  - **Windows**: `CaptureHandler::start_free_threaded` for Windows Graphics Capture
- Frame data sent via bounded `tokio::sync::mpsc::channel(2)` with backpressure (frames dropped when full)
- Frames delivered to UI as `Message::RecordingEvent(Frame(Arc<FrameData>))` — no polling tick
- Recording state updated via `Message::RecordingEvent(StateChanged(...))` — owned by UI, no mutexes
- Bidirectional control: subscription sends `RecordingEvent::Ready(cmd_tx)` at startup; app stores the `cmd_tx` sender for issuing `RecordingCommand::Stop`
- Graceful stop: app sends `RecordingCommand::Stop` via command channel → subscription cleans up capture resources → emits `StateChanged(Idle)` → app sets `is_recording = false`
- **Windows GPU sync**: `SharedTexturePool` uses `Arc::strong_count` for slot availability (backpressure), `IDXGIKeyedMutex` for GPU synchronization, event query + `Flush()` + spin-wait for GPU completion
- **Linux DMA-BUF lifetime**: `HeldBuffer` deque (3 entries) manages PipeWire buffer lifetimes

### Supported Video Formats

RGB, RGBA, RGBx, BGRx, YUY2, I420 - all converted to RGBA via GStreamer (Linux)

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
