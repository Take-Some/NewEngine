use super::*;

impl RenderFrameOrchestrator {
    pub(in super::super) fn trace_feature_extract_profile(
        frame_index: u64,
        trace_frame: bool,
        features: &FeatureExtractionFrame,
        ui_layers: &UiLayerDrawPacketSet,
        thread_pool: Option<&ThreadPoolHandle>,
    ) {
        let feature_ms = features.profile_total_ms();
        if !timed_profile_due(frame_index, trace_frame, feature_ms) {
            return;
        }
        let breakdown = features.profile_breakdown();
        let ui_stats = if ui_layers.is_empty() {
            "ui_layers=none".to_owned()
        } else {
            let domains = ui_layers
                .packets
                .iter()
                .map(|packet| {
                    format!(
                        "{}:{}",
                        packet.domain.as_str(),
                        Self::ui_draw_list_stats(&packet.draw_list)
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ");
            format!("ui_layers(count={} {})", ui_layers.packets.len(), domains)
        };
        emit_timed_profile_deferred(
            thread_pool,
            "render feature profile",
            frame_index,
            trace_frame,
            feature_ms,
            breakdown,
            ui_stats,
        );
    }

    pub(in super::super) fn ui_draw_list_stats(ui: &UiDrawList) -> String {
        let tex_set_bytes: usize = ui
            .texture_delta
            .set
            .values()
            .map(|texture| texture.rgba8.len())
            .sum();
        let patch_bytes: usize = ui
            .texture_delta
            .patches
            .iter()
            .map(|patch| patch.rgba8.len())
            .sum();
        let mut paint_text = 0usize;
        let mut paint_vector = 0usize;
        let mut paint_images = 0usize;
        let mut first_font_ref = None;
        let mut first_vector_ref = None;
        for command in &ui.paint.commands {
            match command {
                UiPaintCommand::Text(text) => {
                    paint_text += 1;
                    if first_font_ref.is_none() && !text.font_ref.trim().is_empty() {
                        first_font_ref = Some(text.font_ref.as_str());
                    }
                }
                UiPaintCommand::Vector(vector) => {
                    paint_vector += 1;
                    if first_vector_ref.is_none() && !vector.vector.uri.trim().is_empty() {
                        first_vector_ref = Some(vector.vector.uri.as_str());
                    }
                }
                UiPaintCommand::Image(_) => paint_images += 1,
                _ => {}
            }
        }

        format!(
            "ui(mesh_vertices={} mesh_indices={} mesh_cmds={} paint_cmds={} paint_text={} paint_vector={} paint_images={} paint_diags={} first_font_ref={:?} first_vector_ref={:?} tex_set={} tex_set_bytes={} patches={} patch_bytes={} free={})",
            ui.mesh.vertices.len(),
            ui.mesh.indices.len(),
            ui.mesh.cmds.len(),
            ui.paint.commands.len(),
            paint_text,
            paint_vector,
            paint_images,
            ui.paint.diagnostics.len(),
            first_font_ref,
            first_vector_ref,
            ui.texture_delta.set.len(),
            tex_set_bytes,
            ui.texture_delta.patches.len(),
            patch_bytes,
            ui.texture_delta.free.len(),
        )
    }

    pub(in super::super) fn trace_cpu_profile(
        frame_index: u64,
        trace_frame: bool,
        profile: &FrameCpuProfile,
        thread_pool: Option<&ThreadPoolHandle>,
    ) {
        let total_ms = profile.total_ms();
        if !timed_profile_due(frame_index, trace_frame, total_ms) {
            return;
        }
        emit_timed_profile_deferred(
            thread_pool,
            "render cpu profile",
            frame_index,
            trace_frame,
            total_ms,
            profile.breakdown(),
            String::new(),
        );
    }
}
