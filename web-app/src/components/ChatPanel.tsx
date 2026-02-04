import { useState } from "react";
import { TextField, Button, Paper, Stack } from "@mui/material";
import SendIcon from "@mui/icons-material/Send";

interface ChatPanelProps {
  // Use "sendMessage" prop name to minimize changes in App.tsx if desired, or rename it.
  // Given instructions said "Modify ChatPanel.tsx to use sendClipboardEvent instead of chat message",
  // I will rename the prop to be explicit as per plan.
  sendClipboard: (msg: string) => void;
  isConnected: boolean;
}

export const ChatPanel = ({ sendClipboard, isConnected }: ChatPanelProps) => {
  const [message, setMessage] = useState("");

  const handleSend = () => {
    if (message.trim()) {
      sendClipboard(message);
      setMessage("");
    }
  };

  return (
    <Paper elevation={3} sx={{ p: 2, mt: 2 }}>
      <Stack direction="row" spacing={2}>
        <TextField
          fullWidth
          variant="outlined"
          placeholder="Type text to set on remote clipboard..."
          value={message}
          onChange={(e) => setMessage(e.target.value)}
          disabled={!isConnected}
          size="small"
          onKeyPress={(e) => {
            if (e.key === "Enter") handleSend();
          }}
        />
        <Button
          variant="contained"
          endIcon={<SendIcon />}
          onClick={handleSend}
          disabled={!isConnected || !message.trim()}
        >
          Set Remote Clipboard
        </Button>
      </Stack>
    </Paper>
  );
};
