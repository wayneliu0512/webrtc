const connectBtn = document.getElementById("connectBtn");
const statusSpan = document.getElementById("status");
const msgInput = document.getElementById("msgInput");
const sendBtn = document.getElementById("sendBtn");
const logsDiv = document.getElementById("logs");

let pc = null;
let dc = null;
let ws = null;
let candidateQueue = [];

function log(msg) {
  const div = document.createElement("div");
  div.textContent = `[${new Date().toLocaleTimeString()}] ${msg}`;
  logsDiv.appendChild(div);
  logsDiv.scrollTop = logsDiv.scrollHeight;
  console.log(msg);
}


// Move initialization logic to a separate function
async function init() {
  connectBtn.disabled = true;
  log("Getting User Media...");
  
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ video: true, audio: false });
    document.getElementById("localVideo").srcObject = stream;
    
    // Initialize PC before connecting signaling
    pc = new RTCPeerConnection({
      // iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
    });

    stream.getTracks().forEach((track) => {
        log(`Adding track: ${track.id}`);
        pc.addTrack(track, stream);
    });

    pc.ontrack = (event) => {
        log("Track received");
        const remoteVideo = document.getElementById("remoteVideo");
        if (event.streams && event.streams[0]) {
          remoteVideo.srcObject = event.streams[0];
        } else {
          const inboundStream = new MediaStream();
          inboundStream.addTrack(event.track);
          remoteVideo.srcObject = inboundStream;
        }
    };

    pc.onicecandidate = (event) => {
        if (event.candidate) {
            log("OnIceCandidate: " + event.candidate.candidate);
        }
    };

    pc.onconnectionstatechange = async () => {
        log(`PC State: ${pc.connectionState}`);
        if (pc.connectionState === 'connected') {
            try {
                const stats = await pc.getStats();
                let pairFound = false;
                stats.forEach(report => {
                    if (report.type === 'candidate-pair' && report.state === 'succeeded' && report.nominated) {
                        const localCandidate = stats.get(report.localCandidateId);
                        const remoteCandidate = stats.get(report.remoteCandidateId);
                        if (localCandidate && remoteCandidate) {
                            const formatCandidate = (c) => {
                                if (c.candidate) return c.candidate;
                                return `${c.candidateType} ${c.protocol} ${c.address || c.ip}:${c.port}`;
                            };
                            log(`Selected Pair: ${formatCandidate(localCandidate)} <-> ${formatCandidate(remoteCandidate)}`);
                            pairFound = true;
                        }
                    }
                });
            } catch (err) {
                log(`Error getting stats: ${err}`);
            }
        }
    };

    pc.ondatachannel = (event) => {
        log(`DataChannel received: ${event.channel.label}`);
        setupDataChannel(event.channel);
    };

    // Create Data Channel (Frontend is Offerer)
    const dataChannel = pc.createDataChannel("data");
    setupDataChannel(dataChannel);

    log("Connecting to WebSocket...");
    connectWebSocket();

  } catch (err) {
    log("Error getting user media: " + err);
    connectBtn.disabled = false;
  }
}

function connectWebSocket() {
  ws = new WebSocket(`ws://${location.host}/ws`);

  ws.onopen = () => {
    log("WebSocket connected");
    statusSpan.textContent = "Signaling Connected";
    
    // Create and send offer
    createAndSendOffer();
  };

  ws.onmessage = async (event) => {
    const msg = JSON.parse(event.data);
    handleSignalingMessage(msg);
  };

  ws.onerror = (e) => log("WebSocket error: " + e);
  ws.onclose = () => {
    log("WebSocket closed");
    cleanup();
  };
}

connectBtn.onclick = () => {
    if (ws) {
        log("Already connected or connecting...");
        return;
    }
    init();
};

/* 
// OLD startWebRTC function removed as it is merged into init
async function startWebRTC() { ... } 
*/

function setupDataChannel(channel) {
  dc = channel;
  dc.onopen = () => {
    log("DataChannel OPEN!");
    statusSpan.textContent = "WebRTC Connected";
    msgInput.disabled = false;
    sendBtn.disabled = false;
    connectBtn.disabled = true;
  };
  dc.onmessage = (event) => {
    log(`Received (DC): ${event.data}`);
  };
  dc.onclose = () => {
    log("DataChannel CLOSED");
    cleanup();
  };
}

async function handleSignalingMessage(msg) {
  if (msg.type === "answer") {
    log("Received Answer");
    await pc.setRemoteDescription(new RTCSessionDescription(msg));
  } else if (msg.type === "candidate") {
    log("Received ICE Candidate");
    if (msg.candidate) {
      log("Add ICE Candidate: " + msg.candidate);
      await pc.addIceCandidate(new RTCIceCandidate(msg.candidate));
    }
  }
}

async function createAndSendOffer() {
    log("Creating Offer...");
    const offer = await pc.createOffer();
    await pc.setLocalDescription(offer);

    // Wait for ICE gathering to complete
    if (pc.iceGatheringState === 'complete') {
        sendOffer();
    } else {
        const checkState = () => {
            if (pc.iceGatheringState === 'complete') {
                pc.removeEventListener('icegatheringstatechange', checkState);
                sendOffer();
            }
        };
        pc.addEventListener('icegatheringstatechange', checkState);
    }
}

function sendOffer() {
    log("ICE Gathering Complete. Sending Offer...");
    const offer = pc.localDescription;
    ws.send(JSON.stringify({ type: "offer", sdp: offer.sdp }));
}

sendBtn.onclick = () => {
  const text = msgInput.value;
  if (dc && dc.readyState === "open") {
    dc.send(text);
    log(`Sent (DC): ${text}`);
    msgInput.value = "";
  } else {
    log("DataChannel not open");
  }
};

function cleanup() {
  if (pc) pc.close();
  if (ws) ws.close();
  pc = null;
  dc = null;
  ws = null;
  statusSpan.textContent = "Disconnected";
  msgInput.disabled = true;
  sendBtn.disabled = true;
  connectBtn.disabled = false;
}
