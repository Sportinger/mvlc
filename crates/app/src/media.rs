use std::path::Path;

use crate::app_state::AppState;
use crate::video_layer::VideoLayerState;
use mvlc_core::StreamId;
use mvlc_media::HardwareVideoDecoder;

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

            if let Some(layer) = app_state.project.layers.get_layer_mut(layer_id) {
                layer.media_path = Some(media_path.clone());
                layer.set_position(0.0, 0.0);
                layer.set_scale(1.0, 1.0);
                layer.play();
                layer.set_stream(stream_id);
            }

            if let Some(ref audio) = app_state.audio_output {
                app_state
                    .av_sync_manager
                    .add_stream(stream_id, audio.master_clock().clone(), 30.0);
            }

            let autoplay = if app_state.video_layers.is_empty() {
                if let Some(audio) = &app_state.audio_output {
                    audio.clear_buffer();
                }
                true
            } else {
                app_state.transport.is_playing
            };

            let audio_sink = if app_state.video_layers.is_empty() {
                app_state
                    .audio_output
                    .as_ref()
                    .map(|audio| audio.sample_sink())
            } else {
                None
            };

            match HardwareVideoDecoder::new(stream_id, &media_path, audio_sink) {
                Ok(decoder) => {
                    app_state
                        .video_layers_mut()
                        .insert(layer_id, VideoLayerState::new(decoder));

                    app_state.schedule_fit_to_view(layer_id);

                    if autoplay {
                        app_state.play_all();
                    } else {
                        app_state.update_transport_from_decoders();
                    }

                    tracing::info!("Decoder ready for layer '{}'", layer_name);
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
