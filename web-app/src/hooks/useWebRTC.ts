import { useState, useRef, useCallback } from "react";
import type { InputEvent } from "../types/InputEvent";

export type ConnectionStatus =
  | "Disconnected"
  | "Signaling Connected"
  | "WebRTC Connected";

export interface UseWebRTC {
  remoteStream: MediaStream | null;
  logs: string[];
  connectionStatus: ConnectionStatus;
  isConnecting: boolean;
  connect: (url: string) => Promise<void>;
  sendInputEvent: (event: InputEvent) => void;
  sendClipboard: (text: string) => void;
  sendClipboardGet: () => void;
  disconnect: () => void;
}

export const useWebRTC = (
  onClipboardReceived?: (text: string) => void,
): UseWebRTC => {
  const [remoteStream, setRemoteStream] = useState<MediaStream | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [connectionStatus, setConnectionStatus] =
    useState<ConnectionStatus>("Disconnected");
  const [isConnecting, setIsConnecting] = useState<boolean>(false);

  const pcRef = useRef<RTCPeerConnection | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const dcRef = useRef<RTCDataChannel | null>(null);

  const addLog = useCallback((msg: string) => {
    const time = new Date().toLocaleTimeString();
    const logEntry = `[${time}] ${msg}`;
    console.log(msg);
    setLogs((prev) => [...prev, logEntry]);
  }, []);

  const cleanup = useCallback(() => {
    if (pcRef.current) {
      pcRef.current.close();
      pcRef.current = null;
    }
    if (wsRef.current) {
      wsRef.current.close();
      wsRef.current = null;
    }
    dcRef.current = null;

    setRemoteStream(null);
    setConnectionStatus("Disconnected");
    setIsConnecting(false);
  }, []);

  const initPeerConnection = useCallback(() => {
    const pc = new RTCPeerConnection({});

    pc.addTransceiver("video", { direction: "recvonly" });

    pc.ontrack = (event) => {
      addLog("Track received");
      if (event.streams && event.streams[0]) {
        setRemoteStream(event.streams[0]);
      } else {
        const inboundStream = new MediaStream();
        inboundStream.addTrack(event.track);
        setRemoteStream(inboundStream);
      }
    };

    pc.onicecandidate = (event) => {
      if (event.candidate) {
        addLog("OnIceCandidate: " + event.candidate.candidate);
      }
    };

    pc.onconnectionstatechange = () => {
      addLog(`PC State: ${pc.connectionState}`);
    };

    pcRef.current = pc;
    return pc;
  }, [addLog]);

  const setupDataChannel = useCallback(
    (pc: RTCPeerConnection) => {
      const channel = pc.createDataChannel("data", {
        ordered: false,
        maxRetransmits: 0,
      });

      const handleOpen = () => {
        addLog("DataChannel OPEN!");
        setConnectionStatus("WebRTC Connected");
        setIsConnecting(false);
      };

      const handleMessage = (event: MessageEvent) => {
        try {
          const msg = JSON.parse(event.data);
          if (msg.clipboard) {
            if (msg.clipboard.text) {
              addLog(`Received Clipboard: ${msg.clipboard.text}`);
              if (onClipboardReceived) {
                onClipboardReceived(msg.clipboard.text);
              }
            } else {
              addLog(
                `Received Clipboard event: ${JSON.stringify(msg.clipboard)}`,
              );
            }
          } else {
            addLog(`Received (DC): ${event.data}`);
          }
        } catch {
          addLog(`Received (DC): ${event.data}`);
        }
      };

      const handleClose = () => {
        addLog("DataChannel CLOSED");
        cleanup();
      };

      channel.onopen = handleOpen;
      channel.onmessage = handleMessage;
      channel.onclose = handleClose;

      dcRef.current = channel;

      // Also handle incoming data channels if needed (though we create it here)
      pc.ondatachannel = (event) => {
        addLog(`DataChannel received: ${event.channel.label}`);
        const rxChannel = event.channel;
        rxChannel.onopen = handleOpen;
        rxChannel.onmessage = handleMessage;
        rxChannel.onclose = handleClose;
        dcRef.current = rxChannel;
      };
    },
    [addLog, cleanup, onClipboardReceived],
  );

  const createAndSendOffer = useCallback(
    async (pc: RTCPeerConnection, ws: WebSocket) => {
      addLog("Creating Offer...");
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);

      const checkState = () => {
        if (pc.iceGatheringState === "complete") {
          pc.removeEventListener("icegatheringstatechange", checkState);
          addLog("ICE Gathering Complete. Sending Offer...");
          const localOffer = pc.localDescription;
          if (localOffer && ws.readyState === WebSocket.OPEN) {
            ws.send(JSON.stringify({ type: "offer", sdp: localOffer.sdp }));
          }
        }
      };

      if (pc.iceGatheringState === "complete") {
        checkState();
      } else {
        pc.addEventListener("icegatheringstatechange", checkState);
      }
    },
    [addLog],
  );

  const initWebSocket = useCallback(
    (pc: RTCPeerConnection, url: string) => {
      addLog("Connecting to WebSocket...");
      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        addLog("WebSocket connected");
        setConnectionStatus("Signaling Connected");
        createAndSendOffer(pc, ws);
      };

      ws.onmessage = async (event) => {
        const msg = JSON.parse(event.data);
        if (msg.type === "answer") {
          addLog("Received Answer");
          await pc.setRemoteDescription(new RTCSessionDescription(msg));
        } else if (msg.type === "candidate") {
          addLog("Received ICE Candidate");
          if (msg.candidate) {
            addLog("Add ICE Candidate: " + msg.candidate);
            await pc.addIceCandidate(new RTCIceCandidate(msg.candidate));
          }
        }
      };

      ws.onerror = (e) => addLog("WebSocket error: " + e);
      ws.onclose = () => {
        addLog("WebSocket closed");
        cleanup();
      };
    },
    [addLog, cleanup, createAndSendOffer],
  );

  const connect = useCallback(
    async (url: string) => {
      if (isConnecting || connectionStatus === "WebRTC Connected") return;

      setIsConnecting(true);

      try {
        const pc = initPeerConnection();
        setupDataChannel(pc);
        initWebSocket(pc, url);
      } catch (e) {
        console.error(e);
        cleanup();
      }
    },
    [
      isConnecting,
      connectionStatus,
      cleanup,
      initPeerConnection,
      setupDataChannel,
      initWebSocket,
    ],
  );

  const sendInputEvent = useCallback((event: InputEvent) => {
    if (dcRef.current && dcRef.current.readyState === "open") {
      dcRef.current.send(JSON.stringify({ inputevent: event }));
    }
  }, []);

  const sendClipboard = useCallback(
    (text: string) => {
      if (dcRef.current && dcRef.current.readyState === "open") {
        dcRef.current.send(JSON.stringify({ clipboard: { settext: text } }));
        addLog(`Sent Clipboard SetText: ${text}`);
      }
    },
    [addLog],
  );

  const sendClipboardGet = useCallback(() => {
    if (dcRef.current && dcRef.current.readyState === "open") {
      dcRef.current.send(JSON.stringify({ clipboard: "gettext" }));
      addLog(`Sent Clipboard GetText`);
    }
  }, [addLog]);

  return {
    remoteStream,
    logs,
    connectionStatus,
    isConnecting,
    connect,
    sendInputEvent,
    sendClipboard,
    sendClipboardGet,
    disconnect: cleanup,
  };
};
