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

- **Wayland session** with XDG Desktop Portal support (not X11)
- **PipeWire** for screen capture
- **GStreamer** libraries (gstreamer, gstreamer-app, gstreamer-video, gstreamer-allocators)
- **Vulkan** runtime with `VK_KHR_external_memory_fd` and `VK_EXT_external_memory_dma_buf` extensions (for zero-copy DMA-BUF import)
- Native dependencies: `pkg-config`, GStreamer dev packages, PipeWire dev packages, Vulkan dev packages

## Architecture Overview

Cocuyo is a Wayland screen capture application that displays real-time screen content using PipeWire. The UI is built with **iced** (v0.14) using the wgpu backend with custom window decorations (daemon mode, multi-window).

### Data Flow

1. **Portal Session** (`stream.rs:open_portal`) - Uses ashpd to request screen capture permission via XDG Desktop Portal
2. **PipeWire Stream** (`stream.rs:start_streaming`) - Connects to PipeWire node using the portal-provided file descriptor
3. **Frame Processing** (`stream.rs`) - Three paths, tried in priority order:
   - **DMA-BUF zero-copy Vulkan import** - When the buffer has a linear modifier and an importable DRM format (Xrgb8888, Argb8888, Abgr8888, Xbgr8888), the DMA-BUF fd is sent directly to the renderer which imports it as a wgpu texture via Vulkan external memory (`vulkan_dmabuf.rs`)
   - **DMA-BUF via GStreamer** - For DMA-BUF buffers with non-linear modifiers or non-importable formats, GStreamer converts to RGBA via CPU
   - **CPU copy fallback** - Traditional memory copy when no DMA-BUF is available
4. **Format Conversion** (`gst_pipeline.rs`) - GStreamer pipeline converts input formats to RGBA. Supports GPU-accelerated backends (CUDA, VA-API) and CPU fallback
5. **Display** - iced shader widget (`screen/video_shader.rs`) renders frames using wgpu, importing DMA-BUF textures directly or uploading CPU RGBA data

### Key Components

- **`main.rs`** - Application entry point, initializes GStreamer/PipeWire, detects backends, launches iced daemon
- **`app.rs`** - `Cocuyo` application state, iced message handling, multi-window management (`Main`, `Settings`, `Preview`). Recording state owned directly (no mutexes).
- **`recording.rs`** - Recording lifecycle as an `iced::Subscription` using `iced::stream::channel`. Manages portal session, spawns PipeWire thread, forwards frames as `RecordingEvent`s.
- **`stream.rs`** - PipeWire stream setup, portal session, frame processing pipeline (DMA-BUF and CPU paths). Uses bounded `mpsc::Sender<Arc<FrameData>>` with backpressure.
- **`gst_pipeline.rs`** - GStreamer video converter with GPU backend detection (CUDA, OpenGL, CPU). `GstVideoConverter` handles appsrc → videoconvert → appsink. Helper functions `make_element` and `build_pipeline_with_elements` reduce pipeline builder duplication.
- **`vulkan_dmabuf.rs`** - Vulkan DMA-BUF import: creates VkImage with external memory, wraps into wgpu texture via `wgpu_hal`
- **`dmabuf_handler.rs`** - DMA-BUF metadata extraction from PipeWire buffers (fd, stride, format, dimensions, modifier)
- **`formats.rs`** - Unified video format conversion tables: PipeWire SPA → GStreamer, PipeWire SPA → DRM fourcc, DRM → Vulkan format, DRM → wgpu format, importability check
- **`screen/`** - UI screens:
  - `main_window.rs` - Main control window (start/stop recording, status)
  - `settings.rs` - Backend selection (GPU/CPU)
  - `preview.rs` - Live video preview display
  - `title_bar.rs` - Custom window title bar (drag, minimize, maximize, close)
  - `video_shader.rs` - Custom iced shader widget for rendering video frames (DMA-BUF import or CPU texture upload)
- **`theme.rs`** - Custom iced theme and styling
- **`widget.rs`** - Custom widget type aliases

### Threading Model

- Main thread: iced event loop (daemon mode with multi-window)
- Recording lifecycle driven by `iced::Subscription` — when `is_recording` is true, the subscription spawns a PipeWire thread
- Frame data sent via bounded `tokio::sync::mpsc::channel(2)` with backpressure (frames dropped when full)
- Frames delivered to UI as `Message::RecordingEvent(Frame(Arc<FrameData>))` — no polling tick
- Recording state updated via `Message::RecordingEvent(StateChanged(...))` — owned by UI, no mutexes
- Bidirectional control: subscription sends `RecordingEvent::Ready(cmd_tx)` at startup; app stores the `cmd_tx` sender for issuing `RecordingCommand::Stop`
- Graceful stop: app sends `RecordingCommand::Stop` via command channel → subscription drops `frame_rx` (closing the frame channel) → PipeWire thread detects `Closed` on next `try_send` and calls `mainloop.quit()` → subscription joins PipeWire thread, closes portal session, emits `StateChanged(Idle)` → app sets `is_recording = false` (subscription dropped only after cleanup)

### Supported Video Formats

RGB, RGBA, RGBx, BGRx, YUY2, I420 - all converted to RGBA via GStreamer

### DMA-BUF Zero-Copy Path

For eligible buffers (linear modifier + importable DRM format), frames bypass GStreamer entirely:
1. PipeWire buffer's DMA-BUF fd is dup'd and sent as `FrameData::DmaBuf`
2. The shader widget (`video_shader.rs`) imports it via `vulkan_dmabuf::import_dmabuf_texture`
3. Vulkan creates a VkImage backed by the imported DMA-BUF memory
4. The VkImage is wrapped into a `wgpu::Texture` via `wgpu_hal`
5. If import fails, `mark_dmabuf_import_failed()` disables the path for all future frames
