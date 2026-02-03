import { useRef, useEffect } from "react";
import { Box, Paper, Typography } from "@mui/material";

interface VideoDisplayProps {
  remoteStream: MediaStream | null;
}

export const VideoDisplay = ({ remoteStream }: VideoDisplayProps) => {
  const remoteVideoRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    if (remoteVideoRef.current && remoteStream) {
      remoteVideoRef.current.srcObject = remoteStream;
    }
  }, [remoteStream]);

  return (
    <Box sx={{ mt: 2, width: "100%" }}>
      <Paper elevation={3} sx={{ p: 1, textAlign: "center" }}>
        <Typography variant="h6" gutterBottom>
          Remote Screen
        </Typography>
        <video
          ref={remoteVideoRef}
          autoPlay
          playsInline
          style={{ width: "100%", borderRadius: "4px" }}
        />
      </Paper>
    </Box>
  );
};
