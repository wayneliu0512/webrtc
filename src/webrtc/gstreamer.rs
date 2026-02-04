use anyhow::{Result, anyhow};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::os::fd::RawFd;
use tokio::sync::mpsc::UnboundedReceiver;

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

    let conv = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|e| anyhow!("Failed to create conv: {}", e))?;

    let queue = gst::ElementFactory::make("queue")
        .build()
        .map_err(|e| anyhow!("Failed to create queue: {}", e))?;
    queue.set_property("max-size-buffers", 1u32);
    queue.set_property("max-size-bytes", 0u32);
    queue.set_property("max-size-time", 0u64);
    queue.set_property_from_str("leaky", "downstream"); // 2 = downstream (drop old)

    let enc = gst::ElementFactory::make("vp8enc")
        .build()
        .map_err(|e| anyhow!("Failed to create enc: {}", e))?;
    enc.set_property_from_str("error-resilient", "partitions");
    enc.set_property("keyframe-max-dist", 2000i32);
    enc.set_property("deadline", 1i64); // Realtime
    enc.set_property("cpu-used", 16i32);
    enc.set_property("threads", 16i32);
    enc.set_property_from_str("end-usage", "cbr");
    enc.set_property("target-bitrate", 2_000_000i32); // 2 Mbps
    enc.set_property("buffer-size", 100i32);
    enc.set_property("buffer-initial-size", 50i32);
    enc.set_property("buffer-optimal-size", 80i32);
    enc.set_property("lag-in-frames", 0i32);
    enc.set_property_from_str("token-partitions", "8"); // 8 partitions, enables multi-threaded decoding on client

    let pay = gst::ElementFactory::make("rtpvp8pay")
        .build()
        .map_err(|e| anyhow!("Failed to create pay: {}", e))?;

    let sink = gst::ElementFactory::make("appsink")
        .build()
        .map_err(|e| anyhow!("Failed to create sink: {}", e))?;
    sink.set_property("sync", false);
    sink.set_property("drop", true);

    pipeline
        .add_many(&[&src, &conv, &queue, &enc, &pay, &sink])
        .map_err(|e| anyhow!("Failed to add elements: {}", e))?;

    gst::Element::link_many(&[&src, &conv, &queue, &enc, &pay, &sink])
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
