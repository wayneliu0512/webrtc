import { useRef, useEffect } from "react";
import { Box, Paper, Typography, Grid } from "@mui/material";

interface VideoDisplayProps {
  localStream: MediaStream | null;
  remoteStream: MediaStream | null;
}

export const VideoDisplay = ({
  localStream,
  remoteStream,
}: VideoDisplayProps) => {
  const localVideoRef = useRef<HTMLVideoElement>(null);
  const remoteVideoRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    if (localVideoRef.current && localStream) {
      localVideoRef.current.srcObject = localStream;
    }
  }, [localStream]);

  useEffect(() => {
    if (remoteVideoRef.current && remoteStream) {
      remoteVideoRef.current.srcObject = remoteStream;
    }
  }, [remoteStream]);

  return (
    <Box sx={{ mt: 2 }}>
      <Grid container spacing={2}>
        <Grid size={{ xs: 12, md: 6 }}>
          <Paper elevation={3} sx={{ p: 1, textAlign: "center" }}>
            <Typography variant="h6" gutterBottom>
              Local Video
            </Typography>
            <video
              ref={localVideoRef}
              autoPlay
              muted
              playsInline
              style={{ width: "100%", maxWidth: "320px", borderRadius: "4px" }}
            />
          </Paper>
        </Grid>
        <Grid size={{ xs: 12, md: 6 }}>
          <Paper elevation={3} sx={{ p: 1, textAlign: "center" }}>
            <Typography variant="h6" gutterBottom>
              Remote Video
            </Typography>
            <video
              ref={remoteVideoRef}
              autoPlay
              playsInline
              style={{ width: "100%", maxWidth: "320px", borderRadius: "4px" }}
            />
          </Paper>
        </Grid>
      </Grid>
    </Box>
  );
};
