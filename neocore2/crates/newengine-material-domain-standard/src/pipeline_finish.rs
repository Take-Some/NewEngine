use super::descriptors::required;
use super::*;

impl PendingLitPipelineBuild {
    pub(super) fn finish(&self) -> MaterialDomainResult<LitPipeline> {
        let pipeline = required(self.pipeline, "pipeline")?;
        let double_sided_pipeline = required(self.double_sided_pipeline, "double_sided_pipeline")?;
        let terrain_pipeline = required(self.terrain_pipeline, "terrain_pipeline")?;
        let instanced_pipeline = required(self.instanced_pipeline, "instanced_pipeline")?;
        let instanced_double_sided_pipeline = required(
            self.instanced_double_sided_pipeline,
            "instanced_double_sided_pipeline",
        )?;
        let instanced_alpha_pipeline =
            required(self.instanced_alpha_pipeline, "instanced_alpha_pipeline")?;
        let instanced_alpha_double_sided_pipeline = required(
            self.instanced_alpha_double_sided_pipeline,
            "instanced_alpha_double_sided_pipeline",
        )?;
        let decal_instanced_pipeline =
            required(self.decal_instanced_pipeline, "decal_instanced_pipeline")?;
        let decal_instanced_double_sided_pipeline = required(
            self.decal_instanced_double_sided_pipeline,
            "decal_instanced_double_sided_pipeline",
        )?;
        let skinned_pipeline = required(self.skinned_pipeline, "skinned_pipeline")?;
        let skinned_double_sided_pipeline = required(
            self.skinned_double_sided_pipeline,
            "skinned_double_sided_pipeline",
        )?;
        let skinned_alpha_pipeline =
            required(self.skinned_alpha_pipeline, "skinned_alpha_pipeline")?;
        let skinned_alpha_double_sided_pipeline = required(
            self.skinned_alpha_double_sided_pipeline,
            "skinned_alpha_double_sided_pipeline",
        )?;
        Ok(LitPipeline {
            bgl: required(self.bgl, "bgl")?,
            skin_bgl: required(self.skin_bgl, "skin_bgl")?,
            white_texture: required(self.white_texture, "white_texture")?,
            flat_normal_texture: required(self.flat_normal_texture, "flat_normal_texture")?,
            repeat_sampler: required(self.repeat_sampler, "repeat_sampler")?,
            clamp_sampler: required(self.clamp_sampler, "clamp_sampler")?,
            vs: required(self.vs, "vs")?,
            fs: required(self.fs, "fs")?,
            terrain_fs: required(self.terrain_fs, "terrain_fs")?,
            shadow_vs: required(self.shadow_vs, "shadow_vs")?,
            shadow_fs: required(self.shadow_fs, "shadow_fs")?,
            skinned_vs: required(self.skinned_vs, "skinned_vs")?,
            shadow_skinned_vs: required(self.shadow_skinned_vs, "shadow_skinned_vs")?,
            pipeline,
            double_sided_pipeline,
            terrain_pipeline,
            gbuffer_terrain_pipeline: if self.profile.deferred_pipelines {
                required(self.gbuffer_terrain_pipeline, "gbuffer_terrain_pipeline")?
            } else {
                terrain_pipeline
            },
            gbuffer_pipeline: if self.profile.deferred_pipelines {
                required(self.gbuffer_pipeline, "gbuffer_pipeline")?
            } else {
                pipeline
            },
            gbuffer_double_sided_pipeline: if self.profile.deferred_pipelines {
                required(
                    self.gbuffer_double_sided_pipeline,
                    "gbuffer_double_sided_pipeline",
                )?
            } else {
                double_sided_pipeline
            },
            gbuffer_instanced_pipeline: if self.profile.deferred_pipelines {
                required(
                    self.gbuffer_instanced_pipeline,
                    "gbuffer_instanced_pipeline",
                )?
            } else {
                instanced_pipeline
            },
            gbuffer_instanced_double_sided_pipeline: if self.profile.deferred_pipelines {
                required(
                    self.gbuffer_instanced_double_sided_pipeline,
                    "gbuffer_instanced_double_sided_pipeline",
                )?
            } else {
                instanced_double_sided_pipeline
            },
            shadow_pipeline: required(self.shadow_pipeline, "shadow_pipeline")?,
            shadow_double_sided_pipeline: required(
                self.shadow_double_sided_pipeline,
                "shadow_double_sided_pipeline",
            )?,
            instanced_vs: required(self.instanced_vs, "instanced_vs")?,
            instanced_fs: required(self.instanced_fs, "instanced_fs")?,
            shadow_instanced_vs: required(self.shadow_instanced_vs, "shadow_instanced_vs")?,
            instanced_pipeline,
            instanced_double_sided_pipeline,
            instanced_alpha_pipeline,
            instanced_alpha_double_sided_pipeline,
            decal_instanced_pipeline,
            decal_instanced_double_sided_pipeline,
            sky_instanced_pipeline: required(
                self.sky_instanced_pipeline,
                "sky_instanced_pipeline",
            )?,
            shadow_instanced_pipeline: required(
                self.shadow_instanced_pipeline,
                "shadow_instanced_pipeline",
            )?,
            shadow_instanced_double_sided_pipeline: required(
                self.shadow_instanced_double_sided_pipeline,
                "shadow_instanced_double_sided_pipeline",
            )?,
            skinned_pipeline,
            skinned_double_sided_pipeline,
            skinned_alpha_pipeline,
            skinned_alpha_double_sided_pipeline,
            gbuffer_skinned_pipeline: if self.profile.deferred_pipelines {
                required(self.gbuffer_skinned_pipeline, "gbuffer_skinned_pipeline")?
            } else {
                skinned_pipeline
            },
            gbuffer_skinned_double_sided_pipeline: if self.profile.deferred_pipelines {
                required(
                    self.gbuffer_skinned_double_sided_pipeline,
                    "gbuffer_skinned_double_sided_pipeline",
                )?
            } else {
                skinned_double_sided_pipeline
            },
            shadow_skinned_pipeline: required(
                self.shadow_skinned_pipeline,
                "shadow_skinned_pipeline",
            )?,
            shadow_skinned_double_sided_pipeline: required(
                self.shadow_skinned_double_sided_pipeline,
                "shadow_skinned_double_sided_pipeline",
            )?,
        })
    }
}
