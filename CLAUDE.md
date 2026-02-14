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

- **`main.rs`** - Application entry point, spawns recording thread, initializes iced daemon with multi-window support
- **`app.rs`** - `Cocuyo` application state, iced message handling, multi-window management (`Main`, `Settings`, `Preview`)
- **`stream.rs`** - PipeWire stream setup, portal session, frame processing pipeline (DMA-BUF and CPU paths)
- **`gst_pipeline.rs`** - GStreamer video converter with GPU backend detection (CUDA, VA-API, CPU). `GstVideoConverter` handles appsrc → videoconvert → appsink
- **`vulkan_dmabuf.rs`** - Vulkan DMA-BUF import: creates VkImage with external memory, wraps into wgpu texture via `wgpu_hal`. Includes format mapping (DRM → Vulkan → wgpu)
- **`dmabuf_handler.rs`** - DMA-BUF metadata extraction from PipeWire buffers (fd, stride, format, dimensions, modifier)
- **`formats.rs`** - Video format conversion tables (PipeWire SPA → GStreamer, PipeWire SPA → DRM fourcc)
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
- Recording thread: Tokio runtime for portal communication, then PipeWire mainloop for frame capture
- Frame data sent via `tokio::sync::mpsc::unbounded_channel`
- Recording control: `start_recording_tx` channel to start, `AtomicBool` stop flag to stop
- Graceful shutdown: stop flag checked via timer in PipeWire mainloop; channel close triggers mainloop quit

### Supported Video Formats

RGB, RGBA, RGBx, BGRx, YUY2, I420 - all converted to RGBA via GStreamer

### DMA-BUF Zero-Copy Path

For eligible buffers (linear modifier + importable DRM format), frames bypass GStreamer entirely:
1. PipeWire buffer's DMA-BUF fd is dup'd and sent as `FrameData::DmaBuf`
2. The shader widget (`video_shader.rs`) imports it via `vulkan_dmabuf::import_dmabuf_texture`
3. Vulkan creates a VkImage backed by the imported DMA-BUF memory
4. The VkImage is wrapped into a `wgpu::Texture` via `wgpu_hal`
5. If import fails, `mark_dmabuf_import_failed()` disables the path for all future frames
