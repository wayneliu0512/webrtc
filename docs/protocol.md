# Protocol Documentation

This document describes the exact protocols and message formats used for communication between the frontend and backend.

## 1. Signaling Protocol (WebSocket)

The signaling protocol is used to exchange Session Description Protocol (SDP) messages to establish a WebRTC connection.

- **Transport**: WebSocket (Text frames)
- **URL**: `ws://<server_ip>:<port>/ws` (Default port: 8080)
- **Structure**: JSON

### Messages

#### 1.1. Client Offer (Frontend -> Backend)

Sent by the frontend to initiate the connection. The frontend creates a WebRTC Offer and sends it as a JSON string.

```json
{
  "type": "offer",
  "sdp": "v=0\r\no=...\r\n..."
}
```

#### 1.2. Server Answer (Backend -> Frontend)

Sent by the backend in response to the offer. The backend gathers all ICE candidates (server-reflexive, relay, etc.) locally and bundles them into the SDP answer before sending it. This avoids the need for trickle ICE messages.

```json
{
  "type": "answer",
  "sdp": "v=0\r\no=...\r\n..."
}
```

> **Note**: While the frontend codebase handles `"candidate"` messages, the current backend implementation bundles candidates in the Answer SDP, so separate candidate messages are not currently sent by the server.

---

## 2. Data Channel Protocol (WebRTC)

Once the WebRTC connection is established, a data channel is created for low-latency control messages.

- **Channel Label**: `"data"`
- **Reliability**: Unreliable (ordered=false, maxRetransmits=0) - Tuned for real-time input.
- **Format**: JSON

### 2.1. Input Events (Frontend -> Backend)

Used to send mouse and keyboard events to the remote machine. All messages are wrapped in an `inputevent` object.

#### Pointer Motion

Relative mouse movement.

```json
{
  "inputevent": {
    "pointermotion": {
      "dx": 10, // Delta X (pixels)
      "dy": -5 // Delta Y (pixels)
    }
  }
}
```

#### Pointer Button

Mouse click events.

```json
{
  "inputevent": {
    "pointerbutton": {
      "button": 1, // 1=Left, 2=Middle, 3=Right (Standard Linux codes)
      "pressed": true // true=Down, false=Up
    }
  }
}
```

#### Scroll

Mouse wheel scrolling.

```json
{
  "inputevent": {
    "scroll": {
      "steps_x": 0, // Horizontal scroll
      "steps_y": 1 // Vertical scroll (Positive = Up, Negative = Down usually)
    }
  }
}
```

#### Keyboard Key

Keyboard key presses.

```json
{
  "inputevent": {
    "key": {
      "keycode": 32, // Linux keycode (e.g., 32 = Space in some mappings, verify against codebase/evdev)
      "pressed": true // true=Down, false=Up
    }
  }
}
```

### 2.2. Clipboard Synchronization

Used to sync clipboard text between the local and remote machines.

#### Set Clipboard (Frontend -> Backend)

Sent when the user copies text on the client side.

```json
{
  "clipboard": {
    "settext": "Copied text content"
  }
}
```

#### Get Clipboard (Frontend -> Backend)

Sent by the client to request the current clipboard content from the remote machine.

```json
{
  "clipboard": "gettext"
}
```

#### Receive Clipboard (Backend -> Frontend)

Sent by the backend in response to a content change or a get request.

```json
{
  "clipboard": {
    "text": "Remote clipboard content"
  }
}
```
