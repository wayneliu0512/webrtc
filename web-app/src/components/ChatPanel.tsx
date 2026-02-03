import { useState } from "react";
import { TextField, Button, Paper, Stack } from "@mui/material";
import SendIcon from "@mui/icons-material/Send";

interface ChatPanelProps {
  sendMessage: (msg: string) => void;
  isConnected: boolean;
}

export const ChatPanel = ({ sendMessage, isConnected }: ChatPanelProps) => {
  const [message, setMessage] = useState("");

  const handleSend = () => {
    if (message.trim()) {
      sendMessage(message);
      setMessage("");
    }
  };

  return (
    <Paper elevation={3} sx={{ p: 2, mt: 2 }}>
      <Stack direction="row" spacing={2}>
        <TextField
          fullWidth
          variant="outlined"
          placeholder="Type a message..."
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
          Send
        </Button>
      </Stack>
    </Paper>
  );
};
