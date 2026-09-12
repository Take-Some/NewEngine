use std::time::Instant;

use newengine_material_domain_api::{
    LitPipeline, MaterialDomainError, MaterialDomainResult, MaterialPipelineBuildProfile,
    MaterialRenderDevice, LIT_INSTANCE_VERTEX_STRIDE,
};
use newengine_primitives::PrimitiveVertex;
use newengine_render_api::*;

use crate::manifest::StandardLitShaderManifest;

#[path = "pipeline_descriptors.rs"]
mod descriptors;
#[path = "pipeline_finish.rs"]
mod finish;
#[path = "pipeline_resources.rs"]
mod resources;
#[path = "pipeline_shaders.rs"]
mod shaders;

use shaders::create_manifest_shader;

const WARMUP_CPU_BUDGET_MS: f32 = 4.0;

/// Incremental Standard pipeline builder.
///
/// The old builder materialized every shader/resource/pipeline in one call. That
/// made a cold driver/cache path indistinguishable from a hung loading screen.
/// This state is retained by the provider and advances under the loading
/// projection. A warm cache may complete several cheap stages in one frame; a
/// slow stage yields immediately so the event loop can present the loading UI.
pub(super) struct PendingLitPipelineBuild {
    profile: MaterialPipelineBuildProfile,
    manifest: StandardLitShaderManifest,
    stage: u8,

    vs: Option<ShaderId>,
    fs: Option<ShaderId>,
    gbuffer_fs: Option<ShaderId>,
    gbuffer_terrain_fs: Option<ShaderId>,
    terrain_fs: Option<ShaderId>,
    shadow_vs: Option<ShaderId>,
    shadow_fs: Option<ShaderId>,
    instanced_vs: Option<ShaderId>,
    instanced_fs: Option<ShaderId>,
    shadow_instanced_vs: Option<ShaderId>,
    skinned_vs: Option<ShaderId>,
    shadow_skinned_vs: Option<ShaderId>,

    bgl: Option<BindGroupLayoutId>,
    skin_bgl: Option<BindGroupLayoutId>,
    white_texture: Option<TextureId>,
    flat_normal_texture: Option<TextureId>,
    repeat_sampler: Option<SamplerId>,
    clamp_sampler: Option<SamplerId>,

    pipeline: Option<PipelineId>,
    double_sided_pipeline: Option<PipelineId>,
    terrain_pipeline: Option<PipelineId>,
    gbuffer_terrain_pipeline: Option<PipelineId>,
    gbuffer_pipeline: Option<PipelineId>,
    gbuffer_double_sided_pipeline: Option<PipelineId>,
    shadow_pipeline: Option<PipelineId>,
    shadow_double_sided_pipeline: Option<PipelineId>,
    instanced_pipeline: Option<PipelineId>,
    instanced_double_sided_pipeline: Option<PipelineId>,
    instanced_alpha_pipeline: Option<PipelineId>,
    instanced_alpha_double_sided_pipeline: Option<PipelineId>,
    decal_instanced_pipeline: Option<PipelineId>,
    decal_instanced_double_sided_pipeline: Option<PipelineId>,
    gbuffer_instanced_pipeline: Option<PipelineId>,
    gbuffer_instanced_double_sided_pipeline: Option<PipelineId>,
    sky_instanced_pipeline: Option<PipelineId>,
    shadow_instanced_pipeline: Option<PipelineId>,
    shadow_instanced_double_sided_pipeline: Option<PipelineId>,
    skinned_pipeline: Option<PipelineId>,
    skinned_double_sided_pipeline: Option<PipelineId>,
    skinned_alpha_pipeline: Option<PipelineId>,
    skinned_alpha_double_sided_pipeline: Option<PipelineId>,
    gbuffer_skinned_pipeline: Option<PipelineId>,
    gbuffer_skinned_double_sided_pipeline: Option<PipelineId>,
    shadow_skinned_pipeline: Option<PipelineId>,
    shadow_skinned_double_sided_pipeline: Option<PipelineId>,
}

impl PendingLitPipelineBuild {
    pub(super) fn new(
        profile: MaterialPipelineBuildProfile,
        manifest: StandardLitShaderManifest,
    ) -> Self {
        Self {
            profile,
            manifest,
            stage: 0,
            vs: None,
            fs: None,
            gbuffer_fs: None,
            gbuffer_terrain_fs: None,
            terrain_fs: None,
            shadow_vs: None,
            shadow_fs: None,
            instanced_vs: None,
            instanced_fs: None,
            shadow_instanced_vs: None,
            skinned_vs: None,
            shadow_skinned_vs: None,
            bgl: None,
            skin_bgl: None,
            white_texture: None,
            flat_normal_texture: None,
            repeat_sampler: None,
            clamp_sampler: None,
            pipeline: None,
            double_sided_pipeline: None,
            terrain_pipeline: None,
            gbuffer_terrain_pipeline: None,
            gbuffer_pipeline: None,
            gbuffer_double_sided_pipeline: None,
            shadow_pipeline: None,
            shadow_double_sided_pipeline: None,
            instanced_pipeline: None,
            instanced_double_sided_pipeline: None,
            instanced_alpha_pipeline: None,
            instanced_alpha_double_sided_pipeline: None,
            decal_instanced_pipeline: None,
            decal_instanced_double_sided_pipeline: None,
            gbuffer_instanced_pipeline: None,
            gbuffer_instanced_double_sided_pipeline: None,
            sky_instanced_pipeline: None,
            shadow_instanced_pipeline: None,
            shadow_instanced_double_sided_pipeline: None,
            skinned_pipeline: None,
            skinned_double_sided_pipeline: None,
            skinned_alpha_pipeline: None,
            skinned_alpha_double_sided_pipeline: None,
            gbuffer_skinned_pipeline: None,
            gbuffer_skinned_double_sided_pipeline: None,
            shadow_skinned_pipeline: None,
            shadow_skinned_double_sided_pipeline: None,
        }
    }

    pub(super) fn advance(
        &mut self,
        r: &mut dyn MaterialRenderDevice,
    ) -> MaterialDomainResult<Option<LitPipeline>> {
        let frame_started = Instant::now();
        let start_stage = self.stage;
        let mut operations = 0_u32;

        loop {
            if self.stage >= 39 {
                let pipeline = self.finish()?;
                newengine_ulog_api::ulog::info!(
                    "standard material domain: staged pipeline ready stages={} operations_this_frame={} elapsed_ms={:.2} deferred_pipelines={}",
                    self.stage,
                    operations,
                    frame_started.elapsed().as_secs_f32() * 1000.0,
                    self.profile.deferred_pipelines,
                );
                return Ok(Some(pipeline));
            }

            let op_started = Instant::now();
            self.advance_one(r)?;
            operations = operations.saturating_add(1);
            let op_ms = op_started.elapsed().as_secs_f32() * 1000.0;
            if op_ms >= 8.0 {
                newengine_ulog_api::ulog::warn!(
                    "standard material domain: warmup stage slow stage={} elapsed_ms={:.2} deferred_pipelines={} action='yield_after_stage'",
                    self.stage.saturating_sub(1),
                    op_ms,
                    self.profile.deferred_pipelines,
                );
                break;
            }
            if frame_started.elapsed().as_secs_f32() * 1000.0 >= WARMUP_CPU_BUDGET_MS {
                break;
            }
        }

        newengine_ulog_api::ulog::debug!(
            "standard material domain: staged warmup progress stage={} previous_stage={} operations={} elapsed_ms={:.2} budget_ms={:.2}",
            self.stage,
            start_stage,
            operations,
            frame_started.elapsed().as_secs_f32() * 1000.0,
            WARMUP_CPU_BUDGET_MS,
        );
        Ok(None)
    }

    fn advance_one(&mut self, r: &mut dyn MaterialRenderDevice) -> MaterialDomainResult<()> {
        match self.stage {
            0 => {
                self.vs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Vertex,
                    &self.manifest.shaders.lit_vs,
                    "standard_lit_vs",
                )?)
            }
            1 => {
                self.fs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Fragment,
                    &self.manifest.shaders.lit_fs,
                    "standard_lit_fs",
                )?)
            }
            2 if self.profile.deferred_pipelines => {
                self.gbuffer_fs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Fragment,
                    &self.manifest.shaders.gbuffer_fs,
                    "standard_gbuffer_lit_fs",
                )?)
            }
            3 if self.profile.deferred_pipelines => {
                self.gbuffer_terrain_fs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Fragment,
                    &self.manifest.shaders.gbuffer_terrain_fs,
                    "standard_gbuffer_terrain_fs",
                )?)
            }
            4 => {
                self.terrain_fs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Fragment,
                    &self.manifest.shaders.terrain_fs,
                    "standard_terrain_surface_fs",
                )?)
            }
            5 => {
                self.shadow_vs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Vertex,
                    &self.manifest.shaders.shadow_vs,
                    "standard_sun_shadow_depth_vs",
                )?)
            }
            6 => {
                self.shadow_fs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Fragment,
                    &self.manifest.shaders.shadow_fs,
                    "standard_sun_shadow_depth_fs",
                )?)
            }
            7 => {
                self.instanced_vs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Vertex,
                    &self.manifest.shaders.instanced_vs,
                    "standard_lit_instanced_vs",
                )?)
            }
            8 => {
                self.instanced_fs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Fragment,
                    &self.manifest.shaders.instanced_fs,
                    "standard_lit_instanced_fs",
                )?)
            }
            9 => {
                self.shadow_instanced_vs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Vertex,
                    &self.manifest.shaders.shadow_instanced_vs,
                    "standard_sun_shadow_instanced_vs",
                )?)
            }
            10 => self.create_bind_resources(r)?,
            11 => {
                self.pipeline = Some(r.create_pipeline(self.pipeline_desc(false, false, false)?)?)
            }
            12 => {
                self.double_sided_pipeline =
                    Some(r.create_pipeline(self.pipeline_desc(true, false, false)?)?)
            }
            13 => self.terrain_pipeline = Some(r.create_pipeline(self.terrain_pipeline_desc()?)?),
            14 if self.profile.deferred_pipelines => {
                self.gbuffer_terrain_pipeline =
                    Some(r.create_pipeline(self.gbuffer_terrain_pipeline_desc()?)?)
            }
            15 if self.profile.deferred_pipelines => {
                self.gbuffer_pipeline =
                    Some(r.create_pipeline(self.gbuffer_pipeline_desc(false, false)?)?)
            }
            16 if self.profile.deferred_pipelines => {
                self.gbuffer_double_sided_pipeline =
                    Some(r.create_pipeline(self.gbuffer_pipeline_desc(true, false)?)?)
            }
            17 => {
                self.shadow_pipeline =
                    Some(r.create_pipeline(self.shadow_pipeline_desc(false, false)?)?)
            }
            18 => {
                self.shadow_double_sided_pipeline =
                    Some(r.create_pipeline(self.shadow_pipeline_desc(true, false)?)?)
            }
            19 => {
                self.instanced_pipeline =
                    Some(r.create_pipeline(self.pipeline_desc(false, true, false)?)?)
            }
            20 => {
                self.instanced_double_sided_pipeline =
                    Some(r.create_pipeline(self.pipeline_desc(true, true, false)?)?)
            }
            21 if self.profile.deferred_pipelines => {
                self.gbuffer_instanced_pipeline =
                    Some(r.create_pipeline(self.gbuffer_pipeline_desc(false, true)?)?)
            }
            22 if self.profile.deferred_pipelines => {
                self.gbuffer_instanced_double_sided_pipeline =
                    Some(r.create_pipeline(self.gbuffer_pipeline_desc(true, true)?)?)
            }
            23 => {
                self.sky_instanced_pipeline =
                    Some(r.create_pipeline(self.pipeline_desc(true, true, true)?)?)
            }
            24 => {
                self.shadow_instanced_pipeline =
                    Some(r.create_pipeline(self.shadow_pipeline_desc(false, true)?)?);
                self.shadow_instanced_double_sided_pipeline =
                    Some(r.create_pipeline(self.shadow_pipeline_desc(true, true)?)?);
            }
            25 => {
                self.skinned_vs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Vertex,
                    &self.manifest.shaders.skinned_vs,
                    "standard_lit_skinned_vs",
                )?)
            }
            26 => {
                self.shadow_skinned_vs = Some(create_manifest_shader(
                    r,
                    ShaderStage::Vertex,
                    &self.manifest.shaders.shadow_skinned_vs,
                    "standard_sun_shadow_skinned_vs",
                )?)
            }
            27 => {
                self.skinned_pipeline =
                    Some(r.create_pipeline(self.skinned_pipeline_desc(false, false)?)?)
            }
            28 => {
                self.skinned_double_sided_pipeline =
                    Some(r.create_pipeline(self.skinned_pipeline_desc(true, false)?)?)
            }
            29 if self.profile.deferred_pipelines => {
                self.gbuffer_skinned_pipeline =
                    Some(r.create_pipeline(self.skinned_pipeline_desc(false, true)?)?)
            }
            30 if self.profile.deferred_pipelines => {
                self.gbuffer_skinned_double_sided_pipeline =
                    Some(r.create_pipeline(self.skinned_pipeline_desc(true, true)?)?)
            }
            31 => {
                self.shadow_skinned_pipeline =
                    Some(r.create_pipeline(self.skinned_shadow_pipeline_desc(false)?)?)
            }
            32 => {
                self.shadow_skinned_double_sided_pipeline =
                    Some(r.create_pipeline(self.skinned_shadow_pipeline_desc(true)?)?)
            }
            33 => {
                self.decal_instanced_pipeline =
                    Some(r.create_pipeline(self.decal_pipeline_desc(false, true)?)?)
            }
            34 => {
                self.decal_instanced_double_sided_pipeline =
                    Some(r.create_pipeline(self.decal_pipeline_desc(true, true)?)?)
            }
            35 => {
                self.skinned_alpha_pipeline =
                    Some(r.create_pipeline(self.skinned_alpha_pipeline_desc(false)?)?)
            }
            36 => {
                self.skinned_alpha_double_sided_pipeline =
                    Some(r.create_pipeline(self.skinned_alpha_pipeline_desc(true)?)?)
            }
            37 => {
                self.instanced_alpha_pipeline =
                    Some(r.create_pipeline(self.instanced_alpha_pipeline_desc(false)?)?)
            }
            38 => {
                self.instanced_alpha_double_sided_pipeline =
                    Some(r.create_pipeline(self.instanced_alpha_pipeline_desc(true)?)?)
            }
            _ => {}
        }
        self.stage = self.stage.saturating_add(1);
        Ok(())
    }
}
