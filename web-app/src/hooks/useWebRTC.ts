import { useState, useRef, useCallback } from "react";
import type { InputEvent } from "../types/InputEvent";

export type ConnectionStatus =
  | "Disconnected"
  | "Generating Offer"
  | "Waiting for Answer"
  | "WebRTC Connected";

export interface UseWebRTC {
  remoteStream: MediaStream | null;
  logs: string[];
  connectionStatus: ConnectionStatus;
  isConnecting: boolean;
  generateOffer: () => Promise<string>;
  setRemoteAnswer: (answerSdp: string) => Promise<void>;
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
      if (pc.connectionState === "connected") {
        setConnectionStatus("WebRTC Connected");
      } else if (
        pc.connectionState === "disconnected" ||
        pc.connectionState === "failed"
      ) {
        setConnectionStatus("Disconnected");
      }
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

  const generateOffer = useCallback(async (): Promise<string> => {
    if (connectionStatus !== "Disconnected") {
      addLog("Already connected or connecting. Disconnect first.");
      throw new Error("Already connected");
    }
    setIsConnecting(true);
    setConnectionStatus("Generating Offer");

    const pc = initPeerConnection();
    setupDataChannel(pc);

    addLog("Creating Offer...");
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);

    return new Promise<string>((resolve) => {
      const checkState = () => {
        if (pc.iceGatheringState === "complete") {
          pc.removeEventListener("icegatheringstatechange", checkState);
          addLog("ICE Gathering Complete.");
          if (pc.localDescription) {
            // We wrap it in the same JSON structure as before for compatibility,
            // or just plain SDP? user asked for "SDP content".
            // The backend expects `WebRtcSignalChannel` JSON structure: { type: "offer", sdp: "..." }
            const jsonOffer = {
              type: "offer",
              sdp: pc.localDescription.sdp,
            };
            setConnectionStatus("Waiting for Answer");
            resolve(JSON.stringify(jsonOffer));
          }
        }
      };

      if (pc.iceGatheringState === "complete") {
        checkState();
      } else {
        pc.addEventListener("icegatheringstatechange", checkState);
      }
    });
  }, [addLog, connectionStatus, initPeerConnection, setupDataChannel]);

  const setRemoteAnswer = useCallback(
    async (answerJson: string) => {
      if (!pcRef.current) {
        addLog("No PeerConnection initialized.");
        return;
      }

      try {
        const msg = JSON.parse(answerJson);
        if (msg.type === "answer") {
          addLog("Setting Remote Description (Answer)...");
          await pcRef.current.setRemoteDescription(
            new RTCSessionDescription(msg),
          );
          addLog("Remote Description Set.");
        } else {
          addLog("Invalid answer format. Expected type 'answer'.");
        }
      } catch (e) {
        addLog("Failed to parse answer: " + e);
      }
    },
    [addLog],
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
    generateOffer,
    setRemoteAnswer,
    sendInputEvent,
    sendClipboard,
    sendClipboardGet,
    disconnect: cleanup,
  };
};
