import { useState, useCallback } from "react";
import { Container, Typography, CssBaseline } from "@mui/material";
import { createTheme, ThemeProvider } from "@mui/material/styles";
import { useWebRTC } from "./hooks/useWebRTC";
import { ControlPanel } from "./components/ControlPanel";
import { VideoDisplay } from "./components/VideoDisplay";
import { ChatPanel } from "./components/ChatPanel";
import { LogViewer } from "./components/LogViewer";
import { ClipboardSimulator } from "./components/ClipboardSimulator";

const theme = createTheme({
  palette: {
    mode: "light",
    primary: {
      main: "#1976d2",
    },
  },
});

function App() {
  const [localClipboard, setLocalClipboard] = useState("");

  const handleClipboardReceived = useCallback((text: string) => {
    setLocalClipboard(text);
  }, []);

  const {
    remoteStream,
    logs,
    connectionStatus,
    isConnecting,
    connect,
    disconnect,
    sendMessage,
    sendInputEvent,
    sendClipboard,
    sendClipboardGet,
  } = useWebRTC(handleClipboardReceived);

  const handleLockChange = useCallback(
    (locked: boolean) => {
      if (locked) {
        // Entered remote control -> Send local clipboard to remote
        sendClipboard(localClipboard);
      } else {
        // Exited remote control -> Fetch remote clipboard to local
        sendClipboardGet();
      }
    },
    [localClipboard, sendClipboard, sendClipboardGet],
  );

  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <Container maxWidth={false} disableGutters sx={{ py: 4 }}>
        <Typography variant="h4" component="h1" gutterBottom sx={{ mb: 4 }}>
          Rust WebRTC Demo (React)
        </Typography>

        <ControlPanel
          onConnect={connect}
          onDisconnect={disconnect}
          status={connectionStatus}
          isConnecting={isConnecting}
        />

        <VideoDisplay
          remoteStream={remoteStream}
          onInput={sendInputEvent}
          onLockChange={handleLockChange}
        />

        <ClipboardSimulator
          value={localClipboard}
          onChange={setLocalClipboard}
        />

        <ChatPanel
          sendMessage={(msg) => sendMessage(msg)} // Revert ChatPanel to just be chat/log for now as requested
          isConnected={connectionStatus === "WebRTC Connected"}
        />

        <LogViewer logs={logs} />
      </Container>
    </ThemeProvider>
  );
}

export default App;
