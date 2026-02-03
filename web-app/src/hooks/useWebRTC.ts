import { useState, useRef, useCallback } from "react";

export interface UseWebRTC {
  localStream: MediaStream | null;
  remoteStream: MediaStream | null;
  logs: string[];
  connectionStatus: string;
  isConnecting: boolean;
  connect: () => Promise<void>;
  sendMessage: (msg: string) => void;
  disconnect: () => void;
}

export const useWebRTC = (): UseWebRTC => {
  const [localStream, setLocalStream] = useState<MediaStream | null>(null);
  const [remoteStream, setRemoteStream] = useState<MediaStream | null>(null);
  const [logs, setLogs] = useState<string[]>([]);
  const [connectionStatus, setConnectionStatus] =
    useState<string>("Disconnected");
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
    if (dcRef.current) {
      dcRef.current = null;
    }
    setLocalStream((prev) => {
      if (prev) {
        prev.getTracks().forEach((track) => track.stop());
      }
      return null;
    });
    setRemoteStream(null);
    setConnectionStatus("Disconnected");
    setIsConnecting(false);
  }, []);

  const setupDataChannel = useCallback(
    (channel: RTCDataChannel) => {
      dcRef.current = channel;
      channel.onopen = () => {
        addLog("DataChannel OPEN!");
        setConnectionStatus("WebRTC Connected");
        setIsConnecting(false);
      };
      channel.onmessage = (event) => {
        addLog(`Received (DC): ${event.data}`);
      };
      channel.onclose = () => {
        addLog("DataChannel CLOSED");
        cleanup();
      };
    },
    [addLog, cleanup],
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

  const connect = useCallback(async () => {
    if (isConnecting || connectionStatus === "WebRTC Connected") return;

    setIsConnecting(true);
    addLog("Getting User Media...");

    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        video: true,
        audio: false,
      });
      setLocalStream(stream);

      const pc = new RTCPeerConnection({
        // iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
      });
      pcRef.current = pc;

      stream.getTracks().forEach((track) => {
        addLog(`Adding track: ${track.id}`);
        pc.addTrack(track, stream);
      });

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
          // In a full implementation, we might send this immediately if using trickle ICE,
          // but the original code gathered all then sent offer.
          // However, the original code had a handler that printed it.
          // The signaling logic below handles sending "candidate" messages if needed,
          // OR the main offer logic waits for gathering complete.
          // Original code: sendOffer() is called after 'complete'.
        }
      };

      pc.onconnectionstatechange = async () => {
        addLog(`PC State: ${pc.connectionState}`);
        if (pc.connectionState === "connected") {
          // Stats logic omitted for brevity, but can be added back if needed
        }
      };

      pc.ondatachannel = (event) => {
        addLog(`DataChannel received: ${event.channel.label}`);
        setupDataChannel(event.channel);
      };

      const dataChannel = pc.createDataChannel("data", {
        ordered: false,
        maxRetransmits: 0,
      });
      setupDataChannel(dataChannel);

      addLog("Connecting to WebSocket...");
      const ws = new WebSocket(`ws://${location.host}/ws`);
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
    } catch (err) {
      addLog("Error getting user media: " + err);
      cleanup();
    }
  }, [
    addLog,
    cleanup,
    setupDataChannel,
    isConnecting,
    connectionStatus,
    createAndSendOffer,
  ]);

  const sendMessage = useCallback(
    (msg: string) => {
      if (dcRef.current && dcRef.current.readyState === "open") {
        dcRef.current.send(msg);
        addLog(`Sent (DC): ${msg}`);
      } else {
        addLog("DataChannel not open");
      }
    },
    [addLog],
  );

  return {
    localStream,
    remoteStream,
    logs,
    connectionStatus,
    isConnecting,
    connect,
    sendMessage,
    disconnect: cleanup,
  };
};
