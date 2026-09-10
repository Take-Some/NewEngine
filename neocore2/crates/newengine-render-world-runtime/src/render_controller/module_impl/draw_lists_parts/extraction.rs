use std::collections::BTreeSet;

use newengine_core::render::{
    RectI32, RenderApi, RenderDrawListKind, RenderGraphPassKind, Viewport,
};
use newengine_core::EngineResult;
use newengine_render_feature_api::{
    RenderDrawListProvider, RuntimeVisibilityPlan, SceneExtractionCtx,
};
use newengine_render_frame_graph::DrawListDesc;

use crate::render_controller::RuntimeRenderController;

#[derive(Clone, Debug)]
pub(crate) struct RuntimeDrawListSet {
    lists: Vec<RuntimeDrawList>,
}

impl RuntimeDrawListSet {
    pub(crate) fn extract(
        visibility: RuntimeVisibilityPlan,
        ctx: &SceneExtractionCtx<'_>,
        providers: &[&dyn RenderDrawListProvider],
    ) -> Self {
        let mut this = Self {
            lists: Vec::with_capacity(6),
        };
        for provider in providers {
            for &kind in provider.provided_draw_lists(ctx) {
                if visibility.allows(kind) {
                    this.push(kind);
                }
            }
        }
        this
    }

    #[inline]
    pub(crate) fn descriptors(&self) -> Vec<DrawListDesc> {
        self.lists
            .iter()
            .map(|list| DrawListDesc::standard(list.kind))
            .collect()
    }

    #[inline]
    pub(super) fn kinds(&self) -> BTreeSet<RenderDrawListKind> {
        self.lists.iter().map(|list| list.kind).collect()
    }

    #[inline]
    pub(super) fn contains(&self, kind: RenderDrawListKind) -> bool {
        self.lists.iter().any(|list| list.kind == kind)
    }

    pub(crate) fn record_pass_state(
        &self,
        ctx: &SceneExtractionCtx<'_>,
        out: &mut DrawListBuildCtx<'_>,
    ) -> EngineResult<()> {
        if self.contains(RenderDrawListKind::ShadowCasters)
            && ctx.render_shadow_map
            && ctx.shadow_frame.cascade_count <= 1
        {
            let extent = ctx.shadow_plan.extent();
            let _ = out.record(RenderDrawListKind::ShadowCasters, move |_this, r| {
                r.set_viewport(Viewport::full(extent))?;
                r.set_scissor(RectI32::new(
                    0,
                    0,
                    extent.width as i32,
                    extent.height as i32,
                ))?;
                Ok(())
            })?;
        }

        if self.contains(RenderDrawListKind::OpaqueForward) {
            let extent = ctx.viewport_extent;
            let _ = out.record(RenderDrawListKind::OpaqueForward, move |_this, r| {
                r.set_viewport(Viewport::full(extent))?;
                r.set_scissor(RectI32::new(
                    0,
                    0,
                    extent.width as i32,
                    extent.height as i32,
                ))?;
                Ok(())
            })?;
        }

        Ok(())
    }

    #[inline]
    pub(super) fn push(&mut self, kind: RenderDrawListKind) {
        if !self.contains(kind) {
            self.lists.push(RuntimeDrawList::new(kind));
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::render_controller::module_impl) struct PrimitiveProviderStageProfile {
    pub(in crate::render_controller::module_impl) sampled: bool,
    pub(in crate::render_controller::module_impl) directional_shadow_ms: f32,
    pub(in crate::render_controller::module_impl) directional_body_ms: f32,
    pub(in crate::render_controller::module_impl) directional_boundary_ms: f32,
    pub(in crate::render_controller::module_impl) directional_cascade_ms: [f32; 4],
    pub(in crate::render_controller::module_impl) directional_skinned_draws: [usize; 4],
    pub(in crate::render_controller::module_impl) shadow_skinned_ms: f32,
    pub(in crate::render_controller::module_impl) shadow_models_ms: f32,
    pub(in crate::render_controller::module_impl) shadow_static_ms: f32,
    pub(in crate::render_controller::module_impl) shadow_static_body_ms: f32,
    pub(in crate::render_controller::module_impl) shadow_static_scan_ms: f32,
    pub(in crate::render_controller::module_impl) shadow_static_plan_ms: f32,
    pub(in crate::render_controller::module_impl) shadow_static_upload_ms: f32,
    pub(in crate::render_controller::module_impl) shadow_static_replay_ms: f32,
    pub(in crate::render_controller::module_impl) local_shadow_ms: f32,
    pub(in crate::render_controller::module_impl) gbuffer_ms: f32,
    pub(in crate::render_controller::module_impl) forward_ms: f32,
}

pub(crate) struct DrawListBuildCtx<'a> {
    controller: &'a mut RuntimeRenderController,
    render: &'a mut dyn RenderApi,
    lists: &'a RuntimeDrawListSet,
    primitive_stage_profile: PrimitiveProviderStageProfile,
}

impl<'a> DrawListBuildCtx<'a> {
    #[inline]
    pub(in crate::render_controller::module_impl) fn new(
        controller: &'a mut RuntimeRenderController,
        render: &'a mut dyn RenderApi,
        lists: &'a RuntimeDrawListSet,
    ) -> Self {
        Self {
            controller,
            render,
            lists,
            primitive_stage_profile: PrimitiveProviderStageProfile::default(),
        }
    }

    #[inline]
    pub(in crate::render_controller::module_impl) fn take_primitive_stage_profile(
        &mut self,
    ) -> PrimitiveProviderStageProfile {
        std::mem::take(&mut self.primitive_stage_profile)
    }

    pub(crate) fn record<T>(
        &mut self,
        kind: RenderDrawListKind,
        record: impl FnOnce(&mut RuntimeRenderController, &mut dyn RenderApi) -> EngineResult<T>,
    ) -> EngineResult<Option<T>> {
        if !self.lists.contains(kind) {
            return Ok(None);
        }

        let controller = &mut *self.controller;
        let render = &mut *self.render;
        let value = super::super::record_draw_list(render, kind, |r| record(controller, r))?;
        Ok(Some(value))
    }

    pub(crate) fn record_shadow_phase<T>(
        &mut self,
        phase: RenderGraphPassKind,
        record: impl FnOnce(&mut RuntimeRenderController, &mut dyn RenderApi) -> EngineResult<T>,
    ) -> EngineResult<Option<T>> {
        if !self.lists.contains(RenderDrawListKind::ShadowCasters) {
            return Ok(None);
        }

        let controller = &mut *self.controller;
        let render = &mut *self.render;
        let value = super::super::record_render_phase(render, phase, |r| record(controller, r))?;
        Ok(Some(value))
    }

    pub(crate) fn record_opaque_phase<T>(
        &mut self,
        phase: RenderGraphPassKind,
        record: impl FnOnce(&mut RuntimeRenderController, &mut dyn RenderApi) -> EngineResult<T>,
    ) -> EngineResult<Option<T>> {
        if !self.lists.contains(RenderDrawListKind::OpaqueForward) {
            return Ok(None);
        }

        let controller = &mut *self.controller;
        let render = &mut *self.render;
        let value = super::super::record_render_phase(render, phase, |r| record(controller, r))?;
        Ok(Some(value))
    }

    pub(crate) fn record_local_shadow_phase<T>(
        &mut self,
        record: impl FnOnce(&mut RuntimeRenderController, &mut dyn RenderApi) -> EngineResult<T>,
    ) -> EngineResult<Option<T>> {
        if !self.lists.contains(RenderDrawListKind::LocalShadowCasters) {
            return Ok(None);
        }
        let controller = &mut *self.controller;
        let render = &mut *self.render;
        let value =
            super::super::record_render_phase(render, RenderGraphPassKind::LocalShadowMap, |r| {
                record(controller, r)
            })?;
        Ok(Some(value))
    }
}

impl<'a> newengine_render_feature_api::DrawListBuildCtx for DrawListBuildCtx<'a> {
    fn record_procedural_terrain_shadow(
        &mut self,
        ctx: &SceneExtractionCtx<'_>,
    ) -> EngineResult<()> {
        if ctx.shadow_frame.cascade_count > 1 {
            for cascade_index in 0..ctx.shadow_frame.cascade_count as usize {
                let cascade = ctx.shadow_frame.cascade(cascade_index);
                let _ =
                    self.record_shadow_phase(RenderGraphPassKind::ShadowCascadeMap, |this, r| {
                        r.set_viewport(cascade.viewport)?;
                        r.set_scissor(cascade.scissor)?;
                        this.set_shadow_caster_cull(Some(cascade.caster_cull));
                        super::super::passes::draw_procedural_terrain_shadow(
                            this,
                            r,
                            ctx.scene,
                            ctx.lit,
                            cascade.light_mvp,
                            &ctx.lights,
                            ctx.runtime,
                            super::super::passes::ShadowUboViewKey::directional(cascade_index),
                        )
                    })?;
            }
            return Ok(());
        }

        let _ = self.record(RenderDrawListKind::ShadowCasters, |this, r| {
            super::super::passes::draw_procedural_terrain_shadow(
                this,
                r,
                ctx.scene,
                ctx.lit,
                ctx.shadow_frame.light_mvp,
                &ctx.lights,
                ctx.runtime,
                super::super::passes::ShadowUboViewKey::directional(0),
            )
        })?;
        Ok(())
    }

    fn record_procedural_terrain_local_shadow(
        &mut self,
        ctx: &SceneExtractionCtx<'_>,
    ) -> EngineResult<()> {
        let count = ctx
            .local_shadow_frame
            .view_count
            .min(newengine_render_feature_api::MAX_LOCAL_SHADOW_VIEWS as u32)
            as usize;
        for view_index in 0..count {
            let view = ctx.local_shadow_frame.views[view_index];
            let light = ctx.local_shadow_frame.lights[view.light_slot as usize];
            let mut local_lights = ctx.lights;
            // Shadow-depth vertex shaders consume shadow_params.y as caster bias.
            // Override the directional value per local view so a 1024 point/spot
            // tile never inherits a CSM-tuned bias intended for a different depth span.
            local_lights.shadow_params[1] = light.bias.max(0.0);
            let _ = self.record_local_shadow_phase(|this, r| {
                r.set_viewport(view.viewport)?;
                r.set_scissor(view.scissor)?;
                this.set_shadow_caster_cull(Some(view.caster_cull));
                super::super::passes::draw_procedural_terrain_shadow(
                    this,
                    r,
                    ctx.scene,
                    ctx.lit,
                    view.light_mvp,
                    &local_lights,
                    ctx.runtime,
                    super::super::passes::ShadowUboViewKey::local(view_index),
                )
            })?;
        }
        Ok(())
    }

    fn record_procedural_terrain_forward(
        &mut self,
        ctx: &SceneExtractionCtx<'_>,
    ) -> EngineResult<()> {
        let _ = self.record(RenderDrawListKind::OpaqueForward, |this, r| {
            super::super::passes::draw_procedural_terrain(
                this,
                r,
                ctx.scene,
                ctx.lit,
                ctx.viewproj,
                &ctx.lights,
                ctx.shadow_frame.texture,
                ctx.local_shadow_frame.texture,
                ctx.runtime,
                ctx.camera_position,
                ctx.camera_forward,
            )
        })?;
        Ok(())
    }

    fn record_procedural_terrain_gbuffer(
        &mut self,
        ctx: &SceneExtractionCtx<'_>,
    ) -> EngineResult<()> {
        let _ = self.record_opaque_phase(RenderGraphPassKind::GBuffer, |this, r| {
            r.set_viewport(Viewport::full(ctx.viewport_extent))?;
            r.set_scissor(RectI32::new(
                0,
                0,
                ctx.viewport_extent.width as i32,
                ctx.viewport_extent.height as i32,
            ))?;
            super::super::passes::draw_procedural_terrain_gbuffer(
                this,
                r,
                ctx.scene,
                ctx.lit,
                ctx.viewproj,
                &ctx.lights,
                ctx.runtime,
                ctx.camera_position,
                ctx.camera_forward,
            )
        })?;
        Ok(())
    }

    fn record_primitive_mesh_shadow(&mut self, ctx: &SceneExtractionCtx<'_>) -> EngineResult<()> {
        let stage_started = std::time::Instant::now();
        let mut directional_body_ms = 0.0_f32;
        if ctx.shadow_frame.cascade_count > 1 {
            for cascade_index in 0..ctx.shadow_frame.cascade_count as usize {
                let cascade = ctx.shadow_frame.cascade(cascade_index);
                let sample =
                    self.record_shadow_phase(RenderGraphPassKind::ShadowCascadeMap, |this, r| {
                        r.set_viewport(cascade.viewport)?;
                        r.set_scissor(cascade.scissor)?;
                        this.set_shadow_caster_cull(Some(cascade.caster_cull));
                        super::super::passes::draw_primitives_shadow(
                            this,
                            r,
                            ctx.scene,
                            ctx.lit,
                            cascade.light_mvp,
                            &ctx.lights,
                            ctx.runtime,
                            ctx.camera_position,
                            cascade_index,
                            cascade.texel_world_size,
                            super::super::passes::ShadowUboViewKey::directional(cascade_index),
                        )
                    })?;
                if let Some(sample) = sample {
                    directional_body_ms += sample.total_ms;
                    if let Some(slot) = self
                        .primitive_stage_profile
                        .directional_cascade_ms
                        .get_mut(cascade_index)
                    {
                        *slot = sample.total_ms;
                    }
                    if let Some(slot) = self
                        .primitive_stage_profile
                        .directional_skinned_draws
                        .get_mut(cascade_index)
                    {
                        *slot = sample.skinned_draws;
                    }
                    self.primitive_stage_profile.shadow_skinned_ms += sample.skinned_ms;
                    self.primitive_stage_profile.shadow_models_ms += sample.models_ms;
                    self.primitive_stage_profile.shadow_static_ms += sample.static_ms;
                    self.primitive_stage_profile.shadow_static_body_ms += sample.static_body_ms;
                    self.primitive_stage_profile.shadow_static_scan_ms += sample.static_scan_ms;
                    self.primitive_stage_profile.shadow_static_plan_ms += sample.static_plan_ms;
                    self.primitive_stage_profile.shadow_static_upload_ms += sample.static_upload_ms;
                    self.primitive_stage_profile.shadow_static_replay_ms += sample.static_replay_ms;
                }
            }
        } else {
            let sample = self.record(RenderDrawListKind::ShadowCasters, |this, r| {
                super::super::passes::draw_primitives_shadow(
                    this,
                    r,
                    ctx.scene,
                    ctx.lit,
                    ctx.shadow_frame.light_mvp,
                    &ctx.lights,
                    ctx.runtime,
                    ctx.camera_position,
                    0,
                    0.0,
                    super::super::passes::ShadowUboViewKey::directional(0),
                )
            })?;
            if let Some(sample) = sample {
                directional_body_ms = sample.total_ms;
                self.primitive_stage_profile.directional_cascade_ms[0] = sample.total_ms;
                self.primitive_stage_profile.directional_skinned_draws[0] = sample.skinned_draws;
                self.primitive_stage_profile.shadow_skinned_ms = sample.skinned_ms;
                self.primitive_stage_profile.shadow_models_ms = sample.models_ms;
                self.primitive_stage_profile.shadow_static_ms = sample.static_ms;
                self.primitive_stage_profile.shadow_static_body_ms = sample.static_body_ms;
                self.primitive_stage_profile.shadow_static_scan_ms = sample.static_scan_ms;
                self.primitive_stage_profile.shadow_static_plan_ms = sample.static_plan_ms;
                self.primitive_stage_profile.shadow_static_upload_ms = sample.static_upload_ms;
                self.primitive_stage_profile.shadow_static_replay_ms = sample.static_replay_ms;
            }
        }
        let directional_shadow_ms = stage_started.elapsed().as_secs_f32() * 1000.0;
        self.primitive_stage_profile.sampled = true;
        self.primitive_stage_profile.directional_shadow_ms = directional_shadow_ms;
        self.primitive_stage_profile.directional_body_ms = directional_body_ms;
        self.primitive_stage_profile.directional_boundary_ms =
            (directional_shadow_ms - directional_body_ms).max(0.0);
        Ok(())
    }

    fn record_primitive_mesh_local_shadow(
        &mut self,
        ctx: &SceneExtractionCtx<'_>,
    ) -> EngineResult<()> {
        let stage_started = std::time::Instant::now();
        let count = ctx
            .local_shadow_frame
            .view_count
            .min(newengine_render_feature_api::MAX_LOCAL_SHADOW_VIEWS as u32)
            as usize;
        for view_index in 0..count {
            let view = ctx.local_shadow_frame.views[view_index];
            let light = ctx.local_shadow_frame.lights[view.light_slot as usize];
            let mut local_lights = ctx.lights;
            local_lights.shadow_params[1] = light.bias.max(0.0);
            let _ = self.record_local_shadow_phase(|this, r| {
                r.set_viewport(view.viewport)?;
                r.set_scissor(view.scissor)?;
                this.set_shadow_caster_cull(Some(view.caster_cull));
                super::super::passes::draw_primitives_shadow(
                    this,
                    r,
                    ctx.scene,
                    ctx.lit,
                    view.light_mvp,
                    &local_lights,
                    ctx.runtime,
                    ctx.camera_position,
                    0,
                    0.0,
                    super::super::passes::ShadowUboViewKey::local(view_index),
                )
            })?;
        }
        self.primitive_stage_profile.sampled = true;
        self.primitive_stage_profile.local_shadow_ms =
            stage_started.elapsed().as_secs_f32() * 1000.0;
        Ok(())
    }

    fn record_primitive_mesh_gbuffer(&mut self, ctx: &SceneExtractionCtx<'_>) -> EngineResult<()> {
        let stage_started = std::time::Instant::now();
        let _ = self.record_opaque_phase(RenderGraphPassKind::GBuffer, |this, r| {
            r.set_viewport(Viewport::full(ctx.viewport_extent))?;
            r.set_scissor(RectI32::new(
                0,
                0,
                ctx.viewport_extent.width as i32,
                ctx.viewport_extent.height as i32,
            ))?;
            super::super::passes::draw_primitives_gbuffer(
                this,
                r,
                ctx.scene,
                ctx.lit,
                ctx.viewproj,
                &ctx.lights,
                ctx.runtime,
                ctx.camera_position,
                ctx.camera_forward,
                ctx.deferred,
            )
        })?;
        self.primitive_stage_profile.sampled = true;
        self.primitive_stage_profile.gbuffer_ms = stage_started.elapsed().as_secs_f32() * 1000.0;
        Ok(())
    }

    fn record_primitive_mesh_forward(&mut self, ctx: &SceneExtractionCtx<'_>) -> EngineResult<()> {
        let stage_started = std::time::Instant::now();
        let _ = self.record(RenderDrawListKind::OpaqueForward, |this, r| {
            super::super::passes::draw_primitives(
                this,
                r,
                ctx.scene,
                ctx.lit,
                ctx.viewproj,
                &ctx.lights,
                ctx.shadow_frame.texture,
                ctx.local_shadow_frame.texture,
                ctx.runtime,
                ctx.camera_position,
                ctx.camera_forward,
                ctx.deferred,
            )
        })?;
        self.primitive_stage_profile.sampled = true;
        self.primitive_stage_profile.forward_ms = stage_started.elapsed().as_secs_f32() * 1000.0;
        Ok(())
    }

    fn record_asset_preview(
        &mut self,
        ctx: &SceneExtractionCtx<'_>,
        bundle: &newengine_model_domain_api::ModelAssetBundle,
        view: newengine_render_feature_api::AssetPreviewView,
    ) -> EngineResult<()> {
        let _ = self.record(RenderDrawListKind::OpaqueForward, |this, r| {
            super::super::passes::draw_asset_preview_bundle(
                this,
                r,
                bundle,
                ctx.lit,
                ctx.viewport_extent,
                view,
            )
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct RuntimeDrawList {
    kind: RenderDrawListKind,
}

impl RuntimeDrawList {
    #[inline]
    fn new(kind: RenderDrawListKind) -> Self {
        Self { kind }
    }
}
