import { useState } from "react";
import { Box, Button, Chip, TextField, Paper, Typography } from "@mui/material";
import PlayArrowIcon from "@mui/icons-material/PlayArrow";
import StopIcon from "@mui/icons-material/Stop";
import ContentCopyIcon from "@mui/icons-material/ContentCopy";

interface ControlPanelProps {
  onGenerateOffer: () => Promise<string>;
  onSetRemoteAnswer: (answer: string) => Promise<void>;
  onDisconnect: () => void;
  status: string;
  isConnecting: boolean;
}

export const ControlPanel = ({
  onGenerateOffer,
  onSetRemoteAnswer,
  onDisconnect,
  status,
  isConnecting,
}: ControlPanelProps) => {
  const [offer, setOffer] = useState("");
  const [answer, setAnswer] = useState("");
  const isConnected = status === "WebRTC Connected";
  const isGenerating = status === "Generating Offer";

  const handleGenerateOffer = async () => {
    try {
      const generatedOffer = await onGenerateOffer();
      setOffer(generatedOffer);
    } catch (e) {
      console.error(e);
    }
  };

  const handleSetAnswer = async () => {
    if (answer) {
      await onSetRemoteAnswer(answer);
    }
  };

  const copyToClipboard = () => {
    navigator.clipboard.writeText(offer);
  };

  return (
    <Box sx={{ display: "flex", flexDirection: "column", gap: 2, mb: 2 }}>
      <Box sx={{ display: "flex", alignItems: "center", gap: 2 }}>
        {!isConnected ? (
          <>
            <Button
              variant="contained"
              color="primary"
              startIcon={<PlayArrowIcon />}
              onClick={handleGenerateOffer}
              disabled={isConnecting || isGenerating || !!offer}
            >
              {isGenerating ? "Generating..." : "1. Generate Offer"}
            </Button>
          </>
        ) : (
          <Button
            variant="contained"
            color="error"
            startIcon={<StopIcon />}
            onClick={() => {
              setOffer("");
              setAnswer("");
              onDisconnect();
            }}
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

      {offer && !isConnected && (
        <Paper
          elevation={3}
          sx={{ p: 2, display: "flex", flexDirection: "column", gap: 2 }}
        >
          <Typography variant="subtitle2">
            2. Copy Local Offer (Paste into Backend)
          </Typography>
          <Box sx={{ display: "flex", gap: 1 }}>
            <TextField
              fullWidth
              multiline
              rows={3}
              value={offer}
              InputProps={{
                readOnly: true,
              }}
              variant="outlined"
              size="small"
            />
            <Button
              variant="outlined"
              onClick={copyToClipboard}
              startIcon={<ContentCopyIcon />}
            >
              Copy
            </Button>
          </Box>

          <Typography variant="subtitle2">
            3. Paste Remote Answer (From Backend)
          </Typography>
          <Box sx={{ display: "flex", gap: 1 }}>
            <TextField
              fullWidth
              multiline
              rows={3}
              value={answer}
              onChange={(e) => setAnswer(e.target.value)}
              placeholder='Paste JSON answer here: {"type": "answer", ...}'
              variant="outlined"
              size="small"
            />
            <Button
              variant="contained"
              color="secondary"
              onClick={handleSetAnswer}
              disabled={!answer}
            >
              Set Answer
            </Button>
          </Box>
        </Paper>
      )}
    </Box>
  );
};
