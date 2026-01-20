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
- **GStreamer** libraries (gstreamer, gstreamer-app, gstreamer-video)
- Native dependencies: `pkg-config`, GStreamer dev packages, PipeWire dev packages

## Architecture Overview

Cocuyo is a Wayland screen capture application that displays real-time screen content using PipeWire.

### Data Flow

1. **Portal Session** (`main.rs:open_portal`) - Uses ashpd to request screen capture permission via XDG Desktop Portal
2. **PipeWire Stream** (`main.rs:start_streaming`) - Connects to PipeWire node using the portal-provided file descriptor
3. **Frame Processing** - Two paths:
   - **DMA-BUF zero-copy path** - Uses `GstDmaBufAllocator` to wrap PipeWire's DMA-BUF fd directly into GStreamer buffers without CPU copies
   - **CPU copy fallback** - Traditional memory copy when DMA-BUF unavailable
4. **Format Conversion** (`gst_pipeline.rs`) - GStreamer pipeline converts any input format to RGBA via `push_dmabuf()` or `push_buffer()`
5. **Display** (`app.rs:CocuyoApp`) - iced renders frames using wgpu backend with multi-window support

### Key Components

- **`main.rs`** - Application entry point, iced daemon setup, recording thread management
- **`app.rs`** - Main application state, elm-like architecture with Message enum, update/view functions, multi-window management
- **`gst_pipeline.rs`** - GStreamer-based video format converter (appsrc → videoconvert → appsink), includes `DmaBufAllocator` for zero-copy buffer wrapping
- **`dmabuf_handler.rs`** - DMA-BUF metadata extraction from PipeWire buffers (fd, stride, format, dimensions)
- **`stream.rs`** - PipeWire streaming and portal session management

### Threading Model

- Main thread: iced daemon event loop
- Separate thread: PipeWire mainloop for frame capture (spawned in `main`)
- Frame data sent via `tokio::sync::mpsc::unbounded_channel`
- Graceful shutdown: When channel closes (window closed), PipeWire thread calls `mainloop.quit()`

### Supported Video Formats

RGB, RGBA, RGBx, BGRx, YUY2, I420 - all converted to RGBA via GStreamer
