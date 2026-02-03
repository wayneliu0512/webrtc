import { Container, Typography, CssBaseline } from "@mui/material";
import { createTheme, ThemeProvider } from "@mui/material/styles";
import { useWebRTC } from "./hooks/useWebRTC";
import { ControlPanel } from "./components/ControlPanel";
import { VideoDisplay } from "./components/VideoDisplay";
import { ChatPanel } from "./components/ChatPanel";
import { LogViewer } from "./components/LogViewer";

const theme = createTheme({
  palette: {
    mode: "light",
    primary: {
      main: "#1976d2",
    },
  },
});

function App() {
  const {
    localStream,
    remoteStream,
    logs,
    connectionStatus,
    isConnecting,
    connect,
    disconnect,
    sendMessage,
  } = useWebRTC();

  return (
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <Container maxWidth="md" sx={{ py: 4 }}>
        <Typography variant="h4" component="h1" gutterBottom sx={{ mb: 4 }}>
          Rust WebRTC Demo (React)
        </Typography>

        <ControlPanel
          onConnect={connect}
          onDisconnect={disconnect}
          status={connectionStatus}
          isConnecting={isConnecting}
        />

        <VideoDisplay localStream={localStream} remoteStream={remoteStream} />

        <ChatPanel
          sendMessage={sendMessage}
          isConnected={connectionStatus === "WebRTC Connected"}
        />

        <LogViewer logs={logs} />
      </Container>
    </ThemeProvider>
  );
}

export default App;
