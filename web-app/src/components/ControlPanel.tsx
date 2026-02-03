import { Box, Button, Chip } from "@mui/material";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import StopIcon from "@mui/icons-material/Stop";

interface ControlPanelProps {
  onConnect: () => void;
  onDisconnect: () => void;
  status: string;
  isConnecting: boolean;
}

export const ControlPanel = ({
  onConnect,
  onDisconnect,
  status,
  isConnecting,
}: ControlPanelProps) => {
  const isConnected = status === "WebRTC Connected";

  return (
    <Box sx={{ display: "flex", alignItems: "center", gap: 2, mb: 2 }}>
      {!isConnected ? (
        <Button
          variant="contained"
          color="primary"
          startIcon={<PlayArrowIcon />}
          onClick={onConnect}
          disabled={isConnecting}
        >
          {isConnecting ? "Connecting..." : "Connect"}
        </Button>
      ) : (
        <Button
          variant="contained"
          color="error"
          startIcon={<StopIcon />}
          onClick={onDisconnect}
        >
          Disconnect
        </Button>
      )}

      <Chip
        label={status}
        color={
          isConnected
            ? "success"
            : status === "Disconnected"
              ? "default"
              : "warning"
        }
        variant="outlined"
      />
    </Box>
  );
};
