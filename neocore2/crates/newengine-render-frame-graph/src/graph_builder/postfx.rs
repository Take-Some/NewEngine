use newengine_render_api::{
    RenderGraphPassDesc, RenderGraphPassDomain, RenderGraphPassKind, RenderGraphResourceDesc,
    RenderGraphResourceId, RenderGraphResourceSemantic, RenderGraphResourceUsage, TextureFormat,
};

use crate::StandardRenderPhase;

use super::{
    FrameGraphBuilder, RG_BLOOM_COMPOSITE, RG_FROXEL_FOG_VOLUME, RG_GBUFFER_DEPTH,
    RG_GBUFFER_MATERIAL, RG_GBUFFER_NORMAL, RG_LIT_COLOR, RG_SCENE_HDR_COLOR, RG_SHADOW_MAP,
    RG_SSR_REFLECTION, RG_SURFACE_COLOR, RG_VIEWPORT_COLOR,
};

impl FrameGraphBuilder {
    pub fn postfx(mut self, enabled: bool) -> Self {
        if !enabled {
            return self;
        }

        let Some(input) = self.sampleable_scene_input_resource() else {
            return self;
        };
        let scene_depth = self.scene_depth_resource();
        let has_bloom_composite = self.has_resource(RG_BLOOM_COMPOSITE);
        let has_ssr = self.has_resource(RG_SSR_REFLECTION);
        let has_froxel_fog = self.has_resource(RG_FROXEL_FOG_VOLUME);
        self.add_phase_pass(StandardRenderPhase::PostFx, |pass| {
            let pass = pass
                .with_domain(RenderGraphPassDomain::PostProcess)
                .reads(input, RenderGraphResourceUsage::SampledTexture);
            let pass = if let Some(scene_depth) = scene_depth {
                pass.reads(scene_depth, RenderGraphResourceUsage::SampledTexture)
            } else {
                pass
            };
            let pass = if has_bloom_composite {
                pass.reads(RG_BLOOM_COMPOSITE, RenderGraphResourceUsage::SampledTexture)
            } else {
                pass
            };
            let pass = if has_ssr {
                pass.reads(RG_SSR_REFLECTION, RenderGraphResourceUsage::SampledTexture)
            } else {
                pass
            };
            let pass = if has_froxel_fog {
                pass.reads(RG_FROXEL_FOG_VOLUME, RenderGraphResourceUsage::SampledTexture)
            } else {
                pass
            };
            pass.writes(RG_VIEWPORT_COLOR, RenderGraphResourceUsage::ColorAttachment)
        });
        self
    }

    /// Materializes a logical 3D froxel volume in a packed 2D RGBA16F atlas.
    ///
    /// The logical grid is `(ceil(viewport/tile), depth_slices)`. Physical storage is
    /// deliberately provider-neutral here: the Vulkan backend can later swap the atlas
    /// for a native 3D image without changing world/post-FX authoring contracts.
    pub fn froxel_fog(mut self, tile_size_px: u32, depth_slices: u32) -> Self {
        let Some(scene_depth) = self.scene_depth_resource() else {
            return self;
        };
        let tile_size_px = tile_size_px.clamp(8, 64);
        let depth_slices = depth_slices.clamp(16, 128);
        let atlas_extent = froxel_atlas_extent(self.target.viewport_extent, tile_size_px, depth_slices);
        if !self.has_resource(RG_FROXEL_FOG_VOLUME) {
            self.graph.resources.push(
                RenderGraphResourceDesc::transient_texture(
                    RG_FROXEL_FOG_VOLUME,
                    "froxel_fog_volume",
                    RenderGraphResourceUsage::ColorAttachment,
                    atlas_extent,
                    TextureFormat::Rgba16Float,
                )
                .with_semantic(RenderGraphResourceSemantic::FroxelFog),
            );
        }
        // A cached directional shadow atlas has no writer in this frame but is still an
        // authoritative external input. Materialize it as an external graph resource so
        // volumetric lighting keeps shadowing stable across shadow-cache hits.
        if !self.has_resource(RG_SHADOW_MAP) {
            if let Some(rt) = self.target.shadow_render_target {
                self.graph.resources.push(
                    RenderGraphResourceDesc::external_render_target(
                        RG_SHADOW_MAP,
                        "shadow_cascade_atlas_cached",
                        rt,
                        RenderGraphResourceUsage::SampledTexture,
                        self.target.shadow_extent,
                        TextureFormat::R32Float,
                    )
                    .with_semantic(RenderGraphResourceSemantic::ShadowMap),
                );
            }
        }
        let has_shadow_map = self.has_resource(RG_SHADOW_MAP);
        self.add_phase_pass(StandardRenderPhase::FroxelFog, |pass| {
            let pass = pass
                .with_domain(RenderGraphPassDomain::PostProcess)
                .reads(scene_depth, RenderGraphResourceUsage::SampledTexture);
            let pass = if has_shadow_map {
                pass.reads(RG_SHADOW_MAP, RenderGraphResourceUsage::SampledTexture)
            } else {
                pass
            };
            pass.writes(
                RG_FROXEL_FOG_VOLUME,
                RenderGraphResourceUsage::ColorAttachment,
            )
        });
        self
    }

    #[inline]
    pub fn screen_space_reflections(mut self) -> Self {
        if !(self.has_resource(RG_GBUFFER_DEPTH)
            && self.has_resource(RG_GBUFFER_NORMAL)
            && self.has_resource(RG_GBUFFER_MATERIAL))
        {
            return self;
        }
        let Some(scene_color) = self.sampleable_scene_input_resource() else {
            return self;
        };
        if !self.has_resource(RG_SSR_REFLECTION) {
            self.graph.resources.push(
                RenderGraphResourceDesc::transient_texture(
                    RG_SSR_REFLECTION,
                    "ssr_reflection",
                    RenderGraphResourceUsage::ColorAttachment,
                    self.target.viewport_extent,
                    TextureFormat::Rgba16Float,
                )
                .with_semantic(RenderGraphResourceSemantic::ScreenSpaceReflection),
            );
        }
        self.add_phase_pass(StandardRenderPhase::ScreenSpaceReflections, |pass| {
            pass.with_domain(RenderGraphPassDomain::PostProcess)
                .reads(scene_color, RenderGraphResourceUsage::SampledTexture)
                .reads(RG_GBUFFER_DEPTH, RenderGraphResourceUsage::SampledTexture)
                .reads(RG_GBUFFER_NORMAL, RenderGraphResourceUsage::SampledTexture)
                .reads(RG_GBUFFER_MATERIAL, RenderGraphResourceUsage::SampledTexture)
                .writes(RG_SSR_REFLECTION, RenderGraphResourceUsage::ColorAttachment)
        });
        self
    }

    pub fn bloom_extract(mut self) -> Self {
        let Some(input) = self.sampleable_scene_input_resource() else {
            return self;
        };
        if !self.has_resource(RG_BLOOM_COMPOSITE) {
            self.graph.resources.push(
                RenderGraphResourceDesc::transient_texture(
                    RG_BLOOM_COMPOSITE,
                    "bloom_composite",
                    RenderGraphResourceUsage::ColorAttachment,
                    self.target.viewport_extent,
                    TextureFormat::Rgba16Float,
                )
                .with_semantic(RenderGraphResourceSemantic::BloomComposite),
            );
        }
        self.add_phase_pass(StandardRenderPhase::BloomExtract, |pass| {
            pass.with_domain(RenderGraphPassDomain::PostProcess)
                .reads(input, RenderGraphResourceUsage::SampledTexture)
                .writes(RG_BLOOM_COMPOSITE, RenderGraphResourceUsage::ColorAttachment)
        });
        self
    }

    #[inline]
    pub fn bloom_blur(mut self) -> Self {
        if !self.has_resource(RG_BLOOM_COMPOSITE) {
            return self;
        }
        self.add_phase_pass(StandardRenderPhase::BloomBlur, |pass| {
            pass.with_domain(RenderGraphPassDomain::PostProcess)
                .reads(RG_BLOOM_COMPOSITE, RenderGraphResourceUsage::SampledTexture)
                .writes(RG_BLOOM_COMPOSITE, RenderGraphResourceUsage::ColorAttachment)
        });
        self
    }

    #[inline]
    pub fn taa_resolve(mut self) -> Self {
        self.add_phase_pass(StandardRenderPhase::TaaResolve, |pass| {
            pass.with_domain(RenderGraphPassDomain::PostProcess)
                .reads(RG_SURFACE_COLOR, RenderGraphResourceUsage::SampledTexture)
                .writes(RG_SURFACE_COLOR, RenderGraphResourceUsage::ColorAttachment)
        });
        self
    }

    #[inline]
    pub fn msaa_resolve(mut self) -> Self {
        self.add_phase_pass(StandardRenderPhase::MsaaResolve, |pass| {
            pass.with_domain(RenderGraphPassDomain::PostProcess)
                .reads(RG_SURFACE_COLOR, RenderGraphResourceUsage::SampledTexture)
                .writes(RG_SURFACE_COLOR, RenderGraphResourceUsage::ColorAttachment)
        });
        self
    }

    pub(super) fn finalize_surface_output(&mut self) {
        if !self.target.hdr_scene_enabled || self.has_viewport_color_writer() {
            return;
        }

        let Some(input) = self.sampleable_scene_input_resource() else {
            return;
        };

        let id = newengine_render_api::RenderGraphPassId(self.next_custom_pass);
        self.next_custom_pass = self.next_custom_pass.saturating_add(1);
        let pass = RenderGraphPassDesc::new(
            id,
            "hdr_scene_resolve_to_surface",
            RenderGraphPassKind::Copy,
        )
        .with_domain(RenderGraphPassDomain::PostProcess)
        .reads(input, RenderGraphResourceUsage::SampledTexture)
        .writes(RG_VIEWPORT_COLOR, RenderGraphResourceUsage::ColorAttachment);
        self.graph.passes.push(pass);
    }

    #[inline]
    fn has_viewport_color_writer(&self) -> bool {
        self.graph.passes.iter().any(|pass| {
            pass.writes
                .iter()
                .any(|write| write.resource == RG_VIEWPORT_COLOR)
        })
    }

    #[inline]
    pub(super) fn sampleable_scene_input_resource(&self) -> Option<RenderGraphResourceId> {
        if self.has_resource(RG_LIT_COLOR) {
            return Some(RG_LIT_COLOR);
        }
        if self.has_resource(RG_SCENE_HDR_COLOR) {
            return Some(RG_SCENE_HDR_COLOR);
        }
        None
    }
}

#[inline]
fn froxel_atlas_extent(
    viewport: newengine_render_api::Extent2D,
    tile_size_px: u32,
    depth_slices: u32,
) -> newengine_render_api::Extent2D {
    let tile = tile_size_px.clamp(8, 64);
    let slices = depth_slices.clamp(16, 128);
    let froxel_x = viewport.width.max(1).div_ceil(tile);
    let froxel_y = viewport.height.max(1).div_ceil(tile);
    let columns = ceil_sqrt_u32(slices).max(1);
    let rows = slices.div_ceil(columns).max(1);
    newengine_render_api::Extent2D::new(
        froxel_x.saturating_mul(columns).max(1),
        froxel_y.saturating_mul(rows).max(1),
    )
}

#[inline]
fn ceil_sqrt_u32(value: u32) -> u32 {
    let value = value.max(1);
    let mut root = 1u32;
    while root.saturating_mul(root) < value {
        root = root.saturating_add(1);
    }
    root
}

#[cfg(test)]
mod froxel_layout_tests {
    use super::*;

    #[test]
    fn sixty_four_slices_pack_into_eight_by_eight_tiles() {
        let extent = froxel_atlas_extent(newengine_render_api::Extent2D::new(1920, 1080), 16, 64);
        assert_eq!(extent.width, 120 * 8);
        assert_eq!(extent.height, 68 * 8);
    }

    #[test]
    fn non_square_slice_count_uses_bounded_rectangular_tail() {
        let extent = froxel_atlas_extent(newengine_render_api::Extent2D::new(1280, 720), 16, 96);
        assert_eq!(extent.width, 80 * 10);
        assert_eq!(extent.height, 45 * 10);
    }
}
