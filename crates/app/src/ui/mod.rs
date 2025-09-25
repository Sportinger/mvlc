use crate::app_state::AppState;
use crate::badges::BadgeState;
use crate::transport::format_timecode;
use egui::{self, Layout};
use mvlc_core::Layer;
use mvlc_media::{check_dmabuf_support, check_vaapi_support};

pub fn show_ui(ctx: &egui::Context, app_state: &mut AppState) {
    update_badges_for_state(app_state);

    egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("MVLC - Runtime Status:");

            for badge in &app_state.badges {
                let color = badge.state.color();
                let text = badge.state.text();

                ui.colored_label(color, format!("{} {}: {}", text, badge.name, badge.value));
                ui.separator();
            }
        });
    });

    egui::TopBottomPanel::bottom("transport_bar")
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(16, 16, 16)))
        .show(ctx, |ui| {
            ui.set_height(72.0);

            ui.with_layout(Layout::left_to_right(egui::Align::Center), |ui| {
                let duration = app_state.transport.duration_seconds;

                if ui.button("⏮ Prev").clicked() {
                    app_state.transport.position_seconds = 0.0;
                }

                let play_label = if app_state.transport.is_playing {
                    "⏸ Pause"
                } else {
                    "▶ Play"
                };
                if ui.button(play_label).clicked() {
                    app_state.transport.is_playing = !app_state.transport.is_playing;
                }

                if ui.button("⏹ Stop").clicked() {
                    app_state.transport.is_playing = false;
                    app_state.transport.position_seconds = 0.0;
                }

                if ui.button("⏭ Next").clicked() {
                    app_state.transport.position_seconds = duration.max(0.0);
                }

                ui.add_space(16.0);

                let range = 0.0..=duration.max(1.0);
                let slider = egui::Slider::new(&mut app_state.transport.position_seconds, range)
                    .show_value(false)
                    .text("Timeline");

                if duration <= 0.0 {
                    ui.add_enabled(false, slider);
                } else {
                    ui.add(slider);
                }

                app_state.transport.position_seconds = app_state
                    .transport
                    .position_seconds
                    .clamp(0.0, duration.max(0.0));

                ui.add_space(12.0);

                let position_str = format_timecode(app_state.transport.position_seconds);
                let duration_str = if duration > 0.0 {
                    format_timecode(duration)
                } else {
                    "--:--".to_string()
                };

                ui.label(format!("{} / {}", position_str, duration_str));
            });
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(egui::Color32::BLACK))
        .show(ctx, |ui| {
            let canvas_response = ui.allocate_rect(
                ui.available_rect_before_wrap(),
                egui::Sense::click_and_drag(),
            );

            if canvas_response.clicked() {
                let canvas_pos = app_state.canvas_viewport.screen_to_canvas(
                    [
                        canvas_response.interact_pointer_pos().unwrap_or_default().x,
                        canvas_response.interact_pointer_pos().unwrap_or_default().y,
                    ],
                    canvas_response.rect,
                );

                let mut clicked_layer = None;
                let mut clicked_handle = None;

                for layer in app_state.project.layers.layers().iter().rev() {
                    if let Some(handle) = app_state
                        .canvas_interaction
                        .hit_test_handle(canvas_pos, layer, 20.0)
                    {
                        clicked_handle = Some((layer.id, handle));
                        break;
                    } else if app_state.canvas_interaction.hit_test(
                        canvas_pos,
                        layer,
                        &app_state.canvas_viewport,
                    ) {
                        clicked_layer = Some(layer.id);
                        break;
                    }
                }

                if let Some((layer_id, _handle)) = clicked_handle {
                    app_state.canvas_interaction.selected_layer = Some(layer_id);
                    if let Some(layer) = app_state.project.layers.get_layer(layer_id) {
                        app_state
                            .canvas_interaction
                            .start_drag(canvas_pos, layer, 20.0);
                    }
                } else if let Some(layer_id) = clicked_layer {
                    app_state.canvas_interaction.selected_layer = Some(layer_id);
                    if let Some(layer) = app_state.project.layers.get_layer(layer_id) {
                        app_state
                            .canvas_interaction
                            .start_drag(canvas_pos, layer, 20.0);
                    }
                } else {
                    app_state.canvas_interaction.selected_layer = None;
                    app_state.canvas_interaction.stop_drag();
                }
            }

            if canvas_response.drag_started() {
                if let Some(layer_id) = app_state.canvas_interaction.selected_layer {
                    let canvas_pos = app_state.canvas_viewport.screen_to_canvas(
                        [
                            canvas_response.interact_pointer_pos().unwrap_or_default().x,
                            canvas_response.interact_pointer_pos().unwrap_or_default().y,
                        ],
                        canvas_response.rect,
                    );
                    if let Some(layer) = app_state.project.layers.get_layer(layer_id) {
                        app_state
                            .canvas_interaction
                            .start_drag(canvas_pos, layer, 20.0);
                    }
                }
            }

            if canvas_response.dragged() && app_state.canvas_interaction.is_dragging {
                let current_pos = app_state.canvas_viewport.screen_to_canvas(
                    [
                        canvas_response.interact_pointer_pos().unwrap_or_default().x,
                        canvas_response.interact_pointer_pos().unwrap_or_default().y,
                    ],
                    canvas_response.rect,
                );

                let delta = [
                    current_pos[0] - app_state.canvas_interaction.drag_start[0],
                    current_pos[1] - app_state.canvas_interaction.drag_start[1],
                ];

                if let Some(layer_id) = app_state.canvas_interaction.selected_layer {
                    if let Some(layer) = app_state.project.layers.get_layer_mut(layer_id) {
                        if let Some(handle) = app_state.canvas_interaction.gizmo_handle {
                            match handle {
                                mvlc_core::GizmoHandle::ScaleTopLeft => {
                                    layer.transform.scale[0] *= (1.0 - delta[0] / 100.0).max(0.1);
                                    layer.transform.scale[1] *= (1.0 - delta[1] / 100.0).max(0.1);
                                }
                                mvlc_core::GizmoHandle::ScaleTopRight => {
                                    layer.transform.scale[0] *= (1.0 + delta[0] / 100.0).max(0.1);
                                    layer.transform.scale[1] *= (1.0 - delta[1] / 100.0).max(0.1);
                                }
                                mvlc_core::GizmoHandle::ScaleBottomLeft => {
                                    layer.transform.scale[0] *= (1.0 - delta[0] / 100.0).max(0.1);
                                    layer.transform.scale[1] *= (1.0 + delta[1] / 100.0).max(0.1);
                                }
                                mvlc_core::GizmoHandle::ScaleBottomRight => {
                                    layer.transform.scale[0] *= (1.0 + delta[0] / 100.0).max(0.1);
                                    layer.transform.scale[1] *= (1.0 + delta[1] / 100.0).max(0.1);
                                }
                                mvlc_core::GizmoHandle::Rotate => {
                                    layer.transform.rotation += delta[0] * 0.01;
                                }
                                mvlc_core::GizmoHandle::Move => {
                                    layer.transform.translation[0] += delta[0];
                                    layer.transform.translation[1] += delta[1];
                                }
                                mvlc_core::GizmoHandle::None => {}
                            }
                        } else {
                            layer.transform.translation[0] += delta[0];
                            layer.transform.translation[1] += delta[1];
                        }
                    }
                }

                app_state.canvas_interaction.drag_start = current_pos;
            }

            if !canvas_response.is_pointer_button_down_on()
                && app_state.canvas_interaction.is_dragging
            {
                app_state.canvas_interaction.stop_drag();
            }

            let painter = ui.painter();
            let canvas_rect = canvas_response.rect;
            painter.rect_filled(canvas_rect, 0.0, egui::Color32::BLACK);

            let layers: Vec<Layer> = app_state.project.layers.render_order().cloned().collect();

            for layer in layers {
                if !layer.visible {
                    continue;
                }

                draw_layer(ui, painter, canvas_rect, &layer, app_state);
            }
        });
}

fn draw_layer(
    _ui: &egui::Ui,
    painter: &egui::Painter,
    canvas_rect: egui::Rect,
    layer: &Layer,
    app_state: &AppState,
) {
    let layer_pos = app_state
        .canvas_viewport
        .canvas_to_screen(layer.transform.translation, canvas_rect);

    let (video_width, video_height) = app_state
        .video_layers
        .get(&layer.id)
        .and_then(|state| state.dimensions())
        .unwrap_or((400.0, 225.0));

    let layer_size = [
        video_width * layer.transform.scale[0] * app_state.canvas_viewport.zoom,
        video_height * layer.transform.scale[1] * app_state.canvas_viewport.zoom,
    ];

    let layer_rect = egui::Rect::from_center_size(
        egui::pos2(layer_pos[0], layer_pos[1]),
        egui::vec2(layer_size[0], layer_size[1]),
    );

    if !canvas_rect.intersects(layer_rect) {
        return;
    }

    let mut drew_video = false;
    if let Some(video_state) = app_state.video_layers.get(&layer.id) {
        if let Some(texture_id) = video_state.texture_id() {
            let tint_alpha = (layer.opacity.clamp(0.0, 1.0) * 255.0) as u8;
            painter.image(
                texture_id,
                layer_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::from_rgba_unmultiplied(255, 255, 255, tint_alpha),
            );
            drew_video = true;
        }
    }

    if !drew_video {
        let layer_color = if Some(layer.id) == app_state.canvas_interaction.selected_layer {
            egui::Color32::from_rgb(100, 150, 200)
        } else {
            egui::Color32::from_rgb(80, 80, 80)
        };

        painter.rect_filled(layer_rect, 2.0, layer_color);
    }

    painter.rect_stroke(
        layer_rect,
        2.0,
        egui::Stroke::new(2.0, egui::Color32::WHITE),
    );

    painter.text(
        layer_rect.center_top() + egui::vec2(0.0, 16.0),
        egui::Align2::CENTER_TOP,
        &layer.name,
        egui::FontId::proportional(16.0),
        egui::Color32::WHITE,
    );

    if Some(layer.id) == app_state.canvas_interaction.selected_layer {
        let handle_color = egui::Color32::from_rgb(255, 255, 0);
        let handle_size = 8.0;
        let layer_size_vec = egui::vec2(layer_size[0], layer_size[1]);
        let corners = [
            egui::pos2(-layer_size_vec.x / 2.0, -layer_size_vec.y / 2.0),
            egui::pos2(layer_size_vec.x / 2.0, -layer_size_vec.y / 2.0),
            egui::pos2(-layer_size_vec.x / 2.0, layer_size_vec.y / 2.0),
            egui::pos2(layer_size_vec.x / 2.0, layer_size_vec.y / 2.0),
        ];

        for corner in corners.iter() {
            let handle_pos = egui::pos2(layer_pos[0] + corner.x, layer_pos[1] + corner.y);
            painter.circle_filled(handle_pos, handle_size, handle_color);
        }

        let rotate_pos = egui::pos2(layer_pos[0], layer_pos[1] - layer_size[1] / 2.0 - 20.0);
        painter.circle_filled(rotate_pos, handle_size, handle_color);
        painter.line_segment(
            [
                egui::pos2(layer_pos[0], layer_pos[1] - layer_size[1] / 2.0),
                rotate_pos,
            ],
            egui::Stroke::new(1.0, handle_color),
        );
    }
}

fn update_badges_for_state(app_state: &mut AppState) {
    let has_video_files = app_state
        .project
        .layers
        .layers()
        .iter()
        .any(|layer| layer.media_path.is_some());

    let vaapi_available = check_vaapi_support();

    if let Some(decode_badge) = app_state.badges.iter_mut().find(|b| b.name == "Decode") {
        if has_video_files {
            if vaapi_available {
                decode_badge.value = "VA-API (Ready)".to_string();
                decode_badge.state = BadgeState::Optimal;
            } else {
                decode_badge.value = "SW (File Ready)".to_string();
                decode_badge.state = BadgeState::Partial;
            }
        } else if vaapi_available {
            decode_badge.value = "VA-API".to_string();
            decode_badge.state = BadgeState::Optimal;
        } else {
            decode_badge.value = "SW".to_string();
            decode_badge.state = BadgeState::Fallback;
        }
    }

    if let Some(sync_badge) = app_state.badges.iter_mut().find(|b| b.name == "Sync") {
        let overall_stats = app_state.av_sync_manager.overall_stats();

        if overall_stats.frames_presented > 0 {
            let avg_drift = overall_stats.avg_drift_ms;
            if avg_drift.abs() < 2.0 {
                sync_badge.state = BadgeState::Optimal;
                sync_badge.value = format!("A/V ±{:.1}ms", avg_drift);
            } else if avg_drift.abs() < 8.0 {
                sync_badge.state = BadgeState::Partial;
                sync_badge.value = format!("A/V ±{:.1}ms", avg_drift);
            } else {
                sync_badge.state = BadgeState::Fallback;
                sync_badge.value = format!("A/V ±{:.1}ms", avg_drift);
            }
        } else {
            sync_badge.value = "A/V ±0ms".to_string();
            sync_badge.state = BadgeState::Optimal;
        }
    }

    let dmabuf_available = check_dmabuf_support();
    if let Some(transfer_badge) = app_state.badges.iter_mut().find(|b| b.name == "Transfer") {
        if dmabuf_available {
            transfer_badge.value = "Zero-Copy(DMA-BUF)".to_string();
            transfer_badge.state = BadgeState::Optimal;
        } else {
            transfer_badge.value = "Staged(Host->GPU)".to_string();
            transfer_badge.state = BadgeState::Partial;
        }
    }

    let global_perf = app_state.performance_monitor.global_summary();
    if let Some(perf_badge) = app_state
        .badges
        .iter_mut()
        .find(|b| b.name == "Performance")
    {
        perf_badge.value = global_perf.format_upload_bytes();

        if global_perf.is_fully_zero_copy && global_perf.avg_upload_bytes_per_frame < 1024.0 {
            perf_badge.state = BadgeState::Optimal;
        } else if global_perf.zero_copy_streams > 0 {
            perf_badge.state = BadgeState::Partial;
        } else {
            perf_badge.state = BadgeState::Fallback;
        }
    }

    if let Some(render_badge) = app_state.badges.iter_mut().find(|b| b.name == "Render") {
        if app_state.renderer.is_ready() {
            render_badge.value = "Vulkan(DMA-BUF)".to_string();
            render_badge.state = BadgeState::Optimal;
        } else {
            render_badge.value = "Placeholder".to_string();
            render_badge.state = BadgeState::Partial;
        }
    }

    if let Some(color_badge) = app_state.badges.iter_mut().find(|b| b.name == "Color") {
        if let Some(color_pipeline) = &app_state.color_pipeline {
            let status = color_pipeline.status();
            color_badge.value = status.status_string();
            if status.is_hdr_enabled {
                color_badge.state = BadgeState::Optimal;
            } else {
                color_badge.state = BadgeState::Partial;
            }
        } else {
            color_badge.value = "Basic".to_string();
            color_badge.state = BadgeState::Fallback;
        }
    }
}
