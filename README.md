# WebRTC Remote Desktop / Streaming

A high-performance remote desktop and streaming application built with **Rust** (backend) and **React** (frontend). This project leverages WebRTC for low-latency video streaming and bi-directional data communication, enabling real-time screen sharing and remote control capabilities.

## Features

- **Low-Latency Streaming**: Uses GStreamer and WebRTC (VP8 encoding) to stream the desktop screen with minimal delay.
- **Remote Control**: Supports remote mouse and keyboard input forwarding via WebRTC Data Channels.
- **Clipboard Synchronization**: Bi-directional clipboard sharing between the host and the remote client.
- **Wayland Support**: Utilizes `ashpd` (XDG Desktop Portal) for screen capture on Wayland systems.
- **Modern Frontend**: Built with React, TypeScript, Vite, and Material UI for a responsive and intuitive user interface.

## Architecture

### Backend (Rust)

The backend is powered by `axum` and `tokio`. It handles:

- **Signaling**: WebSocket-based signaling server (`/ws`) for exchanging SDP offers/answers and ICE candidates.
- **WebRTC Negotiation**: Manages `RTCPeerConnection` for media and data streams.
- **Media Pipeline**: Constructs a GStreamer pipeline to capture the screen (via PipeWire/Portal), encode it (VP8), and packetize it (RTP) for transmission.
- **Input Injection**: Receives input events from the data channel and injects them into the host system using the Remote Desktop Portal.

### Frontend (React)

The web application provides the viewing and control interface.

- **Video Display**: Renders the incoming remote video stream.
- **Input Capture**: Captures local mouse and keyboard events and transmits them to the backend.
- **Connection Management**: Handles the WebRTC signaling flow (Offer/Answer exchange).

## Prerequisites

- **Rust**: [Install Rust](https://www.rust-lang.org/tools/install)
- **Node.js**: [Install Node.js](https://nodejs.org/) (includes npm)
- **GStreamer**: Development libraries are required for building the backend.
  - _Linux (Debian/Ubuntu)_: `sudo apt install libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav`
  - _Windows_: Install GStreamer development binaries and ensure they are in your `PATH`.

## Getting Started

### 1. Build and Run the Backend

Navigate to the project root:

```bash
# Build the Rust backend
cargo build

# Run the backend server
# Default port: 8080
cargo run
```

The server will start listening on `0.0.0.0:8080`.

### 2. Build and Run the Frontend

The frontend is located in the `web-app` directory.

```bash
cd web-app

# Install dependencies
npm install

# Start the development server
npm run dev
```

Open your browser and navigate to the URL shown by Vite (usually `http://localhost:5173`).

### 3. Usage

1.  Open the web application in a browser.
2.  Click **"Connect"** (or ensuring the connection initiates automatically).
3.  On the host machine (backend), you may see a system prompt asking for permission to share the screen (Screen Cast Portal) and control inputs (Remote Desktop Portal). **Accept these requests**.
4.  Once connected, you should see the remote screen.
5.  Interacting with the video stream (clicking/typing) will send input events to the host.

## Configuration

- **Signaling Server**: The frontend connects to the signaling server at `ws://localhost:8080/ws` by default. This can be configured in `src/hooks/useWebRTC.ts` if needed.
- **Video Codec**: Currently configured to use VP8. This can be adjusted in `src/webrtc/gstreamer.rs`.

## Troubleshooting

- **"Failed to open portal"**: Ensure you are running on a Linux setup that supports XDG Desktop Portals (e.g., GNOME, KDE, Hyprland) and that the relevant portal implementations (xdg-desktop-portal-\*) are installed.
- **No Video**: Check the backend logs (`RUST_LOG=debug cargo run`) for GStreamer errors. Ensure VP8 encoding plugins are installed.
- **Connection Failed**: Verify that both devices can reach each other and that no firewall is blocking the connection. If connecting over the internet, you may need to configure TURN servers in `src/webrtc/mod.rs`.
