use anyhow::{Result, anyhow};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::os::fd::RawFd;
use tokio::sync::mpsc::UnboundedReceiver;
use tracing::info;

/// Attempts to build a VP9 encoder element.
/// Tries VA-API hardware encoding first, falls back to software vp9enc.
fn build_vp9_encoder() -> Result<gst::Element> {
    // 1. Try VA-API hardware VP9
    if let Ok(hw_enc) = gst::ElementFactory::make("vavp9enc").build() {
        info!("Using VA-API hardware VP9 encoder (vavp9enc)");
        hw_enc.set_property("target-usage", 6u32); // Speed priority (1=quality, 7=speed)
        hw_enc.set_property_from_str("rate-control", "cbr");
        hw_enc.set_property("bitrate", 3000u32); // 3 Mbps – lower = faster encode
        hw_enc.set_property("key-int-max", 60u32); // More frequent keyframes for faster recovery
        hw_enc.set_property("ref-frames", 1u32); // Minimal reference frames for low latency
        return Ok(hw_enc);
    }

    // 2. Software fallback: vp9enc tuned for low latency
    info!("VA-API unavailable, using software VP9 encoder (vp9enc)");
    let sw_enc = gst::ElementFactory::make("vp9enc")
        .build()
        .map_err(|e| anyhow!("Failed to create vp9enc: {}", e))?;
    sw_enc.set_property("deadline", 1i64); // Realtime
    sw_enc.set_property("cpu-used", 16i32); // Max speed (0-16 for VP9, higher = faster)
    sw_enc.set_property("threads", 8i32); // More parallelism
    sw_enc.set_property_from_str("end-usage", "cbr");
    sw_enc.set_property("target-bitrate", 2_500_000i32); // 2.5 Mbps – lower = faster encode
    sw_enc.set_property("keyframe-max-dist", 120i32);
    sw_enc.set_property("lag-in-frames", 0i32); // Zero latency
    sw_enc.set_property("row-mt", true); // Multi-threaded row encoding
    sw_enc.set_property_from_str("error-resilient", "default");

    Ok(sw_enc)
}

/// Builds a GStreamer pipeline that outputs raw VP9 encoded frames (NOT RTP packets).
/// RTP payloading is handled by webrtc-rs's TrackLocalStaticSample instead,
/// which guarantees browser-compatible VP9 RTP format.
pub fn build_gstreamer_pipeline(
    fd_raw: RawFd,
    node_id: u32,
) -> Result<(gst::Pipeline, gst_app::AppSink)> {
    let pipeline = gst::Pipeline::new();

    let src = gst::ElementFactory::make("pipewiresrc")
        .build()
        .map_err(|e| anyhow!("Failed to create src: {}", e))?;
    src.set_property("fd", fd_raw);
    src.set_property("path", &node_id.to_string());
    src.set_property("always-copy", true);

    // Framerate cap: videorate converts variable framerate from pipewiresrc to fixed 60fps
    let videorate = gst::ElementFactory::make("videorate")
        .build()
        .map_err(|e| anyhow!("Failed to create videorate: {}", e))?;

    let capsfilter = gst::ElementFactory::make("capsfilter")
        .build()
        .map_err(|e| anyhow!("Failed to create capsfilter: {}", e))?;
    capsfilter.set_property(
        "caps",
        gst::Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(60, 1))
            .build(),
    );

    // Resolution cap: scale down to max 720p to reduce encoding workload
    let scale = gst::ElementFactory::make("videoscale")
        .build()
        .map_err(|e| anyhow!("Failed to create videoscale: {}", e))?;

    let scale_caps = gst::ElementFactory::make("capsfilter")
        .build()
        .map_err(|e| anyhow!("Failed to create scale capsfilter: {}", e))?;
    scale_caps.set_property(
        "caps",
        gst::Caps::builder("video/x-raw")
            .field("width", 1280i32)
            .field("height", 720i32)
            .build(),
    );

    let conv = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|e| anyhow!("Failed to create conv: {}", e))?;

    let queue = gst::ElementFactory::make("queue")
        .build()
        .map_err(|e| anyhow!("Failed to create queue: {}", e))?;
    queue.set_property("max-size-buffers", 1u32);
    queue.set_property("max-size-bytes", 0u32);
    queue.set_property("max-size-time", 0u64);
    queue.set_property_from_str("leaky", "downstream"); // Drop old frames

    // VP9 encoder (no RTP payloader — webrtc-rs handles that)
    let enc = build_vp9_encoder()?;

    let sink = gst::ElementFactory::make("appsink")
        .build()
        .map_err(|e| anyhow!("Failed to create sink: {}", e))?;
    sink.set_property("sync", false);
    sink.set_property("drop", true);

    // Pipeline: src → conv → videorate → capsfilter(fps) → scale → scale_caps(res) → queue → enc → sink
    // Note: NO rtpvp9pay — raw VP9 frames go to webrtc-rs for RTP payloading
    pipeline
        .add_many(&[
            &src,
            &conv,
            &videorate,
            &capsfilter,
            &scale,
            &scale_caps,
            &queue,
            &enc,
            &sink,
        ])
        .map_err(|e| anyhow!("Failed to add elements: {}", e))?;

    gst::Element::link_many(&[
        &src,
        &conv,
        &videorate,
        &capsfilter,
        &scale,
        &scale_caps,
        &queue,
        &enc,
        &sink,
    ])
    .map_err(|e| anyhow!("Failed to link elements: {}", e))?;

    let appsink = sink
        .downcast::<gst_app::AppSink>()
        .map_err(|_| anyhow!("Failed to cast appsink"))?;

    Ok((pipeline, appsink))
}

pub fn spawn_pli_handler(pipeline: gst::Pipeline, mut pli_rx: UnboundedReceiver<()>) {
    let pipeline_weak = pipeline.downgrade();
    tokio::spawn(async move {
        while let Some(_) = pli_rx.recv().await {
            if let Some(pipeline) = pipeline_weak.upgrade() {
                let struct_ = gst::Structure::builder("GstForceKeyUnit")
                    .field("all-headers", true)
                    .build();
                let event = gst::event::CustomUpstream::new(struct_);
                pipeline.send_event(event);
            } else {
                break;
            }
        }
    });
}
