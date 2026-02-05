import { useState, useCallback, useRef } from "react";
import { Container, Typography, CssBaseline } from "@mui/material";
import { createTheme, ThemeProvider } from "@mui/material/styles";
import { useWebRTC } from "./hooks/useWebRTC";
import { ControlPanel } from "./components/ControlPanel";
import { VideoDisplay } from "./components/VideoDisplay";
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
  const localClipboardRef = useRef(""); // Use ref to keep track of latest value without re-binding callbacks

  const handleClipboardReceived = useCallback((text: string) => {
    setLocalClipboard(text);
    localClipboardRef.current = text; // Update ref when received from remote
  }, []);

  // Update ref when user types
  const handleLocalClipboardChange = useCallback((text: string) => {
    setLocalClipboard(text);
    localClipboardRef.current = text;
  }, []);

  const {
    remoteStream,
    logs,
    connectionStatus,
    isConnecting,
    generateOffer,
    setRemoteAnswer,
    disconnect,
    sendInputEvent,
    sendClipboard,
    sendClipboardGet,
  } = useWebRTC(handleClipboardReceived);

  const handleLockChange = useCallback(
    (locked: boolean) => {
      if (locked) {
        // Entered remote control -> Send local clipboard to remote
        // Use ref to get latest value without adding state as dependency
        console.log("Expected sendClipboard with:", localClipboardRef.current);
        sendClipboard(localClipboardRef.current);
      } else {
        // Exited remote control -> Fetch remote clipboard to local
        sendClipboardGet();
      }
    },
    [sendClipboard, sendClipboardGet], // Removed localClipboard dependency to keep callback stable
  );

  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <Container maxWidth={false} disableGutters sx={{ py: 4 }}>
        <Typography variant="h4" component="h1" gutterBottom sx={{ mb: 4 }}>
          Rust WebRTC Demo (React)
        </Typography>

        <ControlPanel
          onGenerateOffer={generateOffer}
          onSetRemoteAnswer={setRemoteAnswer}
          onDisconnect={disconnect}
          status={connectionStatus}
          isConnecting={isConnecting}
        />

        <ClipboardSimulator
          value={localClipboard}
          onChange={handleLocalClipboardChange}
        />

        <LogViewer logs={logs} />

        <VideoDisplay
          remoteStream={remoteStream}
          onInput={sendInputEvent}
          onLockChange={handleLockChange}
        />
      </Container>
    </ThemeProvider>
  );
}

export default App;
