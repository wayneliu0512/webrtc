import { TextField, Paper, Typography, Box } from "@mui/material";

interface ClipboardSimulatorProps {
  value: string;
  onChange: (value: string) => void;
}

export const ClipboardSimulator = ({
  value,
  onChange,
}: ClipboardSimulatorProps) => {
  return (
    <Paper elevation={3} sx={{ p: 2, mt: 2 }}>
      <Typography variant="subtitle1" gutterBottom>
        Local Clipboard Simulator
      </Typography>
      <Box sx={{ mb: 1 }}>
        <Typography variant="body2" color="text.secondary">
          This text field simulates your local clipboard.
          <br />- <strong>Enter Remote (Click Video)</strong>: Text here is sent
          to remote.
          <br />- <strong>Exit Remote (ESC)</strong>: Text here updates from
          remote.
        </Typography>
      </Box>
      <TextField
        fullWidth
        multiline
        rows={3}
        variant="outlined"
        placeholder="Type here to simulate local clipboard content..."
        value={value}
        onChange={(e) => onChange(e.target.value)}
        size="small"
      />
    </Paper>
  );
};
