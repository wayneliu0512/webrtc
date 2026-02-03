import { useEffect, useRef } from "react";
import { Paper, Typography, Box } from "@mui/material";

interface LogViewerProps {
  logs: string[];
}

export const LogViewer = ({ logs }: LogViewerProps) => {
  const logsEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    logsEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [logs]);

  return (
    <Paper elevation={3} sx={{ mt: 2, p: 2, bgcolor: "#f5f5f5" }}>
      <Typography variant="h6" gutterBottom>
        Logs
      </Typography>
      <Box
        sx={{
          height: "200px",
          overflowY: "auto",
          fontFamily: "monospace",
          fontSize: "0.875rem",
          bgcolor: "white",
          p: 1,
          borderRadius: 1,
          border: "1px solid #e0e0e0",
        }}
      >
        {logs.map((log, index) => (
          <div key={index}>{log}</div>
        ))}
        <div ref={logsEndRef} />
      </Box>
    </Paper>
  );
};
