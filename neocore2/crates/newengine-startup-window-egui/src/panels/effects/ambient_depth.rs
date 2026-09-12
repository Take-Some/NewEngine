#![forbid(unsafe_op_in_unsafe_fn)]

use eframe::egui;

use super::super::super::app::PreStartGraphicsApp;
use super::super::super::widgets::{
    engine_toggle, float_parameter_row, integer_parameter_row, mark_custom_if_changed,
    section_card, setting_label,
};

impl PreStartGraphicsApp {
    pub(super) fn show_ambient_depth(&mut self, ui: &mut egui::Ui) {
        section_card(
            ui,
            "Ambient & Depth",
            "Scene-depth effects controlled before the renderer graph is built",
            |ui| {
                egui::Grid::new("newengine_prestart_ambient_depth")
                    .num_columns(2)
                    .spacing([28.0, 11.0])
                    .show(ui, |ui| {
                        setting_label(ui, "SSAO", "Screen-space ambient occlusion");
                        let changed =
                            engine_toggle(ui, &mut self.settings.graphics.ssao_enabled, "Enabled");
                        mark_custom_if_changed(&mut self.settings, changed);
                        ui.end_row();
                        float_parameter_row(
                            ui,
                            "SSAO radius (world)",
                            self.settings.graphics.ssao_enabled,
                            &mut self.settings.graphics.ssao_radius_ws,
                            0.05..=10.0,
                            0.02,
                        );
                        float_parameter_row(
                            ui,
                            "SSAO intensity",
                            self.settings.graphics.ssao_enabled,
                            &mut self.settings.graphics.ssao_intensity,
                            0.0..=4.0,
                            0.02,
                        );
                        integer_parameter_row(
                            ui,
                            "SSAO quality steps",
                            self.settings.graphics.ssao_enabled,
                            &mut self.settings.graphics.ssao_quality_steps,
                            4..=64,
                        );
                        setting_label(
                            ui,
                            "SSAO resolution",
                            "Half resolution reduces bandwidth and fill cost",
                        );
                        let changed = ui
                            .add_enabled(
                                self.settings.graphics.ssao_enabled,
                                egui::Checkbox::new(
                                    &mut self.settings.graphics.ssao_half_resolution,
                                    "Half resolution",
                                ),
                            )
                            .changed();
                        mark_custom_if_changed(&mut self.settings, changed);
                        ui.end_row();

                        setting_label(
                            ui,
                            "Screen-space reflections",
                            "Deferred-only raster reflections from scene color + GBuffer depth/normal/material",
                        );
                        let changed =
                            engine_toggle(ui, &mut self.settings.graphics.ssr_enabled, "Enabled");
                        mark_custom_if_changed(&mut self.settings, changed);
                        ui.end_row();
                        float_parameter_row(
                            ui,
                            "SSR intensity",
                            self.settings.graphics.ssr_enabled,
                            &mut self.settings.graphics.ssr_intensity,
                            0.0..=2.0,
                            0.01,
                        );
                        float_parameter_row(
                            ui,
                            "SSR max distance (m)",
                            self.settings.graphics.ssr_enabled,
                            &mut self.settings.graphics.ssr_max_distance_m,
                            2.0..=200.0,
                            1.0,
                        );
                        float_parameter_row(
                            ui,
                            "SSR thickness (m)",
                            self.settings.graphics.ssr_enabled,
                            &mut self.settings.graphics.ssr_thickness_m,
                            0.02..=2.0,
                            0.01,
                        );
                        float_parameter_row(
                            ui,
                            "SSR stride (m)",
                            self.settings.graphics.ssr_enabled,
                            &mut self.settings.graphics.ssr_stride_m,
                            0.05..=4.0,
                            0.02,
                        );
                        float_parameter_row(
                            ui,
                            "SSR roughness cutoff",
                            self.settings.graphics.ssr_enabled,
                            &mut self.settings.graphics.ssr_roughness_cutoff,
                            0.05..=1.0,
                            0.01,
                        );
                        integer_parameter_row(
                            ui,
                            "SSR max steps",
                            self.settings.graphics.ssr_enabled,
                            &mut self.settings.graphics.ssr_max_steps,
                            8..=128,
                        );

                        setting_label(
                            ui,
                            "Depth of field",
                            "Allows view-provided focus and blur parameters",
                        );
                        let changed = engine_toggle(
                            ui,
                            &mut self.settings.graphics.depth_of_field_enabled,
                            "Enabled",
                        );
                        mark_custom_if_changed(&mut self.settings, changed);
                        ui.end_row();

                        setting_label(
                            ui,
                            "Motion blur",
                            "Allows view-provided motion blur parameters",
                        );
                        let changed = engine_toggle(
                            ui,
                            &mut self.settings.graphics.motion_blur_enabled,
                            "Enabled",
                        );
                        mark_custom_if_changed(&mut self.settings, changed);
                        ui.end_row();
                    });
            },
        );
    }
}
