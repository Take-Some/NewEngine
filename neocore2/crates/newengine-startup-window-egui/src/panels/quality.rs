#![forbid(unsafe_op_in_unsafe_fn)]

use eframe::egui;

use newengine_core::startup_window::{GraphicsPreset, ShadowQuality, TextureQuality};

use super::super::app::PreStartGraphicsApp;
use super::super::widgets::{
    compact_choice_button, engine_toggle, format_anisotropy, preset_choice_button, section_card,
    setting_label, warning_banner,
};

#[inline]
fn shadow_cascade_label(value: u32) -> String {
    if value == 0 {
        "Auto (scene)".to_owned()
    } else if value == 1 {
        "1 cascade".to_owned()
    } else {
        format!("{value} cascades")
    }
}

#[inline]
fn shadow_map_label(value: u32) -> String {
    if value == 0 {
        "Auto (scene)".to_owned()
    } else {
        format!("{value} x {value}")
    }
}

impl PreStartGraphicsApp {
    pub(crate) fn show_quality(&mut self, ui: &mut egui::Ui) {
        section_card(
            ui,
            "Graphics Baseline",
            "Presets are starting points; any manual edit switches the profile to Custom",
            |ui| {
                ui.horizontal_wrapped(|ui| {
                    for preset in GraphicsPreset::ALL {
                        let selected = self.settings.graphics.preset == preset;
                        if preset_choice_button(ui, preset, selected).clicked()
                            && !matches!(preset, GraphicsPreset::Custom)
                        {
                            self.settings.graphics.apply_preset(preset);
                        }
                    }
                });
            },
        );

        ui.add_space(12.0);
        section_card(
            ui,
            "Resource Quality",
            "Texture residency and material sampling policy",
            |ui| {
                egui::Grid::new("newengine_prestart_resource_quality")
                    .num_columns(2)
                    .spacing([28.0, 12.0])
                    .show(ui, |ui| {
                        setting_label(
                            ui,
                            "Texture quality",
                            "Controls the selected texture quality tier",
                        );
                        let before = self.settings.graphics.texture_quality;
                        egui::ComboBox::from_id_salt("newengine_texture_quality")
                            .width(210.0)
                            .selected_text(before.label())
                            .show_ui(ui, |ui| {
                                for value in TextureQuality::ALL {
                                    ui.selectable_value(
                                        &mut self.settings.graphics.texture_quality,
                                        value,
                                        value.label(),
                                    );
                                }
                            });
                        if self.settings.graphics.texture_quality != before {
                            self.settings.graphics.mark_custom();
                        }
                        ui.end_row();

                        setting_label(
                            ui,
                            "Anisotropic filtering",
                            "Improves oblique texture sampling",
                        );
                        let before = self.settings.graphics.anisotropy;
                        egui::ComboBox::from_id_salt("newengine_anisotropy")
                            .width(210.0)
                            .selected_text(format_anisotropy(before))
                            .show_ui(ui, |ui| {
                                for value in [0_u8, 2, 4, 8, 16] {
                                    ui.selectable_value(
                                        &mut self.settings.graphics.anisotropy,
                                        value,
                                        format_anisotropy(value),
                                    );
                                }
                            });
                        if self.settings.graphics.anisotropy != before {
                            self.settings.graphics.mark_custom();
                        }
                        ui.end_row();

                        setting_label(
                            ui,
                            "View distance",
                            "Controls how far authored-world cells remain resident for rendering",
                        );
                        let before = self.settings.graphics.view_distance_meters;
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings.graphics.view_distance_meters,
                                100.0..=10_000.0,
                            )
                            .step_by(50.0)
                            .suffix(" m"),
                        );
                        if (self.settings.graphics.view_distance_meters - before).abs()
                            > f32::EPSILON
                        {
                            self.settings.graphics.mark_custom();
                        }
                        ui.end_row();

                        setting_label(
                            ui,
                            "LOD distance",
                            "Scales runtime visibility and LOD distance thresholds; 1.0x preserves authored/default distances",
                        );
                        let before = self.settings.graphics.lod_distance_scale;
                        ui.add(
                            egui::Slider::new(
                                &mut self.settings.graphics.lod_distance_scale,
                                0.5..=2.0,
                            )
                            .step_by(0.05)
                            .suffix("x"),
                        );
                        if (self.settings.graphics.lod_distance_scale - before).abs() > f32::EPSILON {
                            self.settings.graphics.mark_custom();
                        }
                        ui.end_row();
                    });
            },
        );

        ui.add_space(12.0);
        section_card(
            ui,
            "Shadow Pipeline",
            "Global shadow gate and runtime quality contract",
            |ui| {
                ui.horizontal(|ui| {
                    let changed = engine_toggle(
                        ui,
                        &mut self.settings.graphics.shadows_enabled,
                        "Dynamic shadows",
                    );
                    if changed {
                        if self.settings.graphics.shadows_enabled
                            && matches!(self.settings.graphics.shadow_quality, ShadowQuality::Off)
                        {
                            self.settings.graphics.shadow_quality = ShadowQuality::Balanced;
                        } else if !self.settings.graphics.shadows_enabled {
                            self.settings.graphics.shadow_quality = ShadowQuality::Off;
                        }
                        self.settings.graphics.mark_custom();
                    }
                });
                ui.add_space(10.0);
                ui.add_enabled_ui(self.settings.graphics.shadows_enabled, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for quality in ShadowQuality::ALL {
                            if matches!(quality, ShadowQuality::Off) {
                                continue;
                            }
                            let selected = self.settings.graphics.shadow_quality == quality;
                            if compact_choice_button(ui, quality.label(), selected).clicked() {
                                self.settings.graphics.shadow_quality = quality;
                                self.settings.graphics.mark_custom();
                            }
                        }
                    });
                    ui.add_space(10.0);
                    egui::Grid::new("newengine_prestart_shadow_details")
                        .num_columns(2)
                        .spacing([28.0, 12.0])
                        .show(ui, |ui| {
                            setting_label(
                                ui,
                                "CSM cascades",
                                "Auto keeps the scene value; an explicit value overrides directional shadow cascades for this launch",
                            );
                            let before = self.settings.graphics.shadow_cascade_count;
                            egui::ComboBox::from_id_salt("newengine_shadow_cascade_count")
                                .width(210.0)
                                .selected_text(shadow_cascade_label(before))
                                .show_ui(ui, |ui| {
                                    for value in [0_u32, 1, 2, 3, 4] {
                                        ui.selectable_value(
                                            &mut self.settings.graphics.shadow_cascade_count,
                                            value,
                                            shadow_cascade_label(value),
                                        );
                                    }
                                });
                            if self.settings.graphics.shadow_cascade_count != before {
                                self.settings.graphics.mark_custom();
                            }
                            ui.end_row();

                            setting_label(
                                ui,
                                "Shadow map size",
                                "Directional shadow map resolution. Auto preserves the scene-authored size",
                            );
                            let before = self.settings.graphics.shadow_map_resolution;
                            egui::ComboBox::from_id_salt("newengine_shadow_map_resolution")
                                .width(210.0)
                                .selected_text(shadow_map_label(before))
                                .show_ui(ui, |ui| {
                                    for value in [0_u32, 256, 512, 1024, 2048, 4096, 8192, 16284] {
                                        ui.selectable_value(
                                            &mut self.settings.graphics.shadow_map_resolution,
                                            value,
                                            shadow_map_label(value),
                                        );
                                    }
                                });
                            if self.settings.graphics.shadow_map_resolution != before {
                                self.settings.graphics.mark_custom();
                            }
                            ui.end_row();
                        });
                });

                if matches!(
                    self.settings.graphics.shadow_quality,
                    ShadowQuality::Cinematic
                ) {
                    ui.add_space(10.0);
                    warning_banner(
                        ui,
                        "Cinematic shadows may increase atlas pressure and per-frame sampling cost.",
                    );
                }
            },
        );
    }
}
