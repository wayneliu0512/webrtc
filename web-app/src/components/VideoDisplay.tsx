import { useRef, useEffect, useCallback, useState } from "react";
import { Box, Paper, Typography, Button } from "@mui/material";
import type { InputEvent } from "../types/InputEvent";
import { getEvdevKeycode } from "../utils/evdev";

interface VideoDisplayProps {
  remoteStream: MediaStream | null;
  onInput?: (event: InputEvent) => void;
}

export const VideoDisplay = ({ remoteStream, onInput }: VideoDisplayProps) => {
  const remoteVideoRef = useRef<HTMLVideoElement>(null);
  const [isLocked, setIsLocked] = useState(false);

  useEffect(() => {
    if (remoteVideoRef.current && remoteStream) {
      remoteVideoRef.current.srcObject = remoteStream;
    }
  }, [remoteStream]);

  const handleMouseMove = useCallback(
    (e: MouseEvent) => {
      if (!onInput) return;
      if (e.movementX === 0 && e.movementY === 0) return;
      onInput({
        pointermotion: {
          dx: e.movementX,
          dy: e.movementY,
        },
      });
    },
    [onInput],
  );

  const handleMouseDown = useCallback(
    (e: MouseEvent) => {
      if (!onInput) return;
      onInput({
        pointerbutton: {
          button: e.button, // 0=left, 1=middle, 2=right
          pressed: true,
        },
      });
    },
    [onInput],
  );

  const handleMouseUp = useCallback(
    (e: MouseEvent) => {
      if (!onInput) return;
      onInput({
        pointerbutton: {
          button: e.button,
          pressed: false,
        },
      });
    },
    [onInput],
  );

  const handleWheel = useCallback(
    (e: WheelEvent) => {
      if (!onInput) return;
      const stepX = e.deltaX ? (e.deltaX > 0 ? 1 : -1) : 0;
      const stepY = e.deltaY ? (e.deltaY > 0 ? -1 : 1) : 0; // Usually scroll down (positive delta) means content moves up (negative space) -> typical scroll down is negative Y step in some protocols, or positive?
      onInput({
        scroll: {
          steps_x: stepX,
          steps_y: stepY,
        },
      });
      e.preventDefault();
    },
    [onInput],
  );

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (!onInput || e.repeat) return; // Ignore repeat for now? Or handle it.
      const keycode = getEvdevKeycode(e.code);
      if (keycode) {
        onInput({
          key: {
            keycode,
            pressed: true,
          },
        });
        e.preventDefault();
      }
    },
    [onInput],
  );

  const handleKeyUp = useCallback(
    (e: KeyboardEvent) => {
      if (!onInput) return;
      const keycode = getEvdevKeycode(e.code);
      if (keycode) {
        onInput({
          key: {
            keycode,
            pressed: false,
          },
        });
        e.preventDefault();
      }
    },
    [onInput],
  );

  useEffect(() => {
    const handleLockChange = () => {
      if (document.pointerLockElement === remoteVideoRef.current) {
        setIsLocked(true);
        document.addEventListener("mousemove", handleMouseMove);
        document.addEventListener("mousedown", handleMouseDown);
        document.addEventListener("mouseup", handleMouseUp);
        document.addEventListener("wheel", handleWheel, { passive: false });
        document.addEventListener("keydown", handleKeyDown);
        document.addEventListener("keyup", handleKeyUp);
      } else {
        setIsLocked(false);
        document.removeEventListener("mousemove", handleMouseMove);
        document.removeEventListener("mousedown", handleMouseDown);
        document.removeEventListener("mouseup", handleMouseUp);
        document.removeEventListener("wheel", handleWheel);
        document.removeEventListener("keydown", handleKeyDown);
        document.removeEventListener("keyup", handleKeyUp);
      }
    };

    document.addEventListener("pointerlockchange", handleLockChange);
    return () => {
      document.removeEventListener("pointerlockchange", handleLockChange);
      // Ensure cleanup of other listeners if component unmounts while locked
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mousedown", handleMouseDown);
      document.removeEventListener("mouseup", handleMouseUp);
      document.removeEventListener("wheel", handleWheel);
      document.removeEventListener("keydown", handleKeyDown);
      document.removeEventListener("keyup", handleKeyUp);
    };
  }, [
    handleMouseMove,
    handleMouseDown,
    handleMouseUp,
    handleWheel,
    handleKeyDown,
    handleKeyUp,
  ]);

  const toggleLock = () => {
    if (!remoteVideoRef.current) return;
    if (!isLocked) {
      remoteVideoRef.current.requestPointerLock();
    }
  };

  return (
    <Box sx={{ mt: 2, width: "100%" }}>
      <Paper elevation={3} sx={{ p: 1, textAlign: "center" }}>
        <Box
          sx={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            mb: 1,
          }}
        >
          <Typography variant="h6">Remote Screen</Typography>
          <Button variant="outlined" size="small" onClick={toggleLock}>
            {isLocked ? "Unlock Mouse (Esc)" : "Click Video to Control"}
          </Button>
        </Box>
        <video
          ref={remoteVideoRef}
          autoPlay
          playsInline
          style={{
            width: "100%",
            borderRadius: "4px",
            cursor: isLocked ? "none" : "default",
          }}
          onClick={toggleLock}
        />
        {isLocked && (
          <Typography variant="caption" color="text.secondary">
            Press ESC to exit remote control
          </Typography>
        )}
      </Paper>
    </Box>
  );
};
