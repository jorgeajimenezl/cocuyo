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

# Manage Dependencies

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
   - **DMA-BUF path** (`dmabuf_handler.rs`) - Zero-copy GPU buffer access via mmap when available
   - **CPU copy fallback** - Traditional memory copy when DMA-BUF unavailable
4. **Format Conversion** (`gst_pipeline.rs`) - GStreamer pipeline converts any input format to RGBA
5. **Display** (`main.rs:CocuyoApp`) - egui/eframe renders frames using wgpu backend

### Key Components

- **`main.rs`** - Application entry point, PipeWire stream setup, egui UI with custom window frame
- **`gst_pipeline.rs`** - GStreamer-based video format converter (appsrc → videoconvert → appsink)
- **`dmabuf_handler.rs`** - DMA-BUF extraction and mmap for efficient GPU buffer access

### Threading Model

- Main thread: Tokio async runtime for portal communication, then egui event loop
- Separate thread: PipeWire mainloop for frame capture (spawned in `main`)
- Frame data sent via `tokio::sync::mpsc::unbounded_channel`

### Supported Video Formats

RGB, RGBA, RGBx, BGRx, YUY2, I420 - all converted to RGBA via GStreamer
