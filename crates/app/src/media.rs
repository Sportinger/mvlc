use std::path::Path;

use crate::app_state::AppState;
use crate::video_layer::VideoLayerState;
use mvlc_core::StreamId;
use mvlc_media::VideoDecoder;

pub fn handle_dropped_file(app_state: &mut AppState, path: &Path) {
    if let Some(extension) = path.extension() {
        let ext_str = extension.to_string_lossy().to_lowercase();
        if matches!(
            ext_str.as_str(),
            "mp4" | "mkv" | "avi" | "mov" | "webm" | "m4v"
        ) {
            tracing::info!("Video file detected: {}", path.display());

            let layer_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let layer_id = app_state.project.layers.add_layer(layer_name.clone());
            let stream_id = StreamId(layer_id.0 as u64);
            let media_path = path.to_string_lossy().to_string();

            let layer_count = app_state.project.layers.layers().len();

            if let Some(layer) = app_state.project.layers.get_layer_mut(layer_id) {
                layer.media_path = Some(media_path.clone());
                layer.set_position(
                    (layer_count as f32 - 1.0) * 50.0,
                    (layer_count as f32 - 1.0) * 30.0,
                );
                layer.set_scale(0.8, 0.8);
                layer.play();
                layer.set_stream(stream_id);
            }

            if let Some(ref audio) = app_state.audio_output {
                app_state
                    .av_sync_manager
                    .add_stream(stream_id, audio.master_clock().clone(), 30.0);
            }

            match VideoDecoder::new(stream_id, &media_path) {
                Ok(decoder) => {
                    if let Err(err) = decoder.play() {
                        tracing::error!("Failed to start decoder for {}: {}", media_path, err);
                    }

                    let duration_seconds = decoder
                        .duration()
                        .map(|ns| ns as f32 / 1_000_000_000.0)
                        .unwrap_or(120.0);

                    app_state.transport.duration_seconds = duration_seconds;
                    app_state.transport.position_seconds = 0.0;
                    app_state.transport.is_playing = true;

                    app_state
                        .video_layers_mut()
                        .insert(layer_id, VideoLayerState::new(decoder));
                    tracing::info!("Decoder started for layer '{}'", layer_name);
                }
                Err(err) => {
                    tracing::error!("Failed to create decoder for {}: {}", media_path, err);
                }
            }

            tracing::info!("Created layer '{}' with video file", layer_name);
        } else {
            tracing::warn!(
                "Unsupported file type: {} (extension: {})",
                path.display(),
                ext_str
            );
        }
    } else {
        tracing::warn!("File has no extension: {}", path.display());
    }
}
