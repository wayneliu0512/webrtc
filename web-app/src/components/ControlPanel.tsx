import { useState } from "react";
import { Box, Button, Chip, TextField } from "@mui/material";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import StopIcon from "@mui/icons-material/Stop";

interface ControlPanelProps {
  onConnect: (url: string) => void;
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
  const [ip, setIp] = useState("10.88.17.213");
  const isConnected = status === "WebRTC Connected";

  const handleConnect = () => {
    const url = `ws://${ip}:8080/ws`;
    onConnect(url);
  };

  return (
    <Box sx={{ display: "flex", alignItems: "center", gap: 2, mb: 2 }}>
      <TextField
        label="Server IP"
        variant="outlined"
        size="small"
        value={ip}
        onChange={(e) => setIp(e.target.value)}
        disabled={isConnected || isConnecting}
        sx={{ width: 150 }}
      />
      {!isConnected ? (
        <Button
          variant="contained"
          color="primary"
          startIcon={<PlayArrowIcon />}
          onClick={handleConnect}
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
