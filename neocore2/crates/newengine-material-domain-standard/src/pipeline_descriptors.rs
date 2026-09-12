use super::*;

#[inline]
const fn sky_depth_mode() -> PipelineDepthMode {
    PipelineDepthMode::new(true, false, PipelineDepthCompare::LessOrEqual)
}

impl PendingLitPipelineBuild {
    pub(super) fn pipeline_desc(
        &self,
        double_sided: bool,
        instanced: bool,
        sky: bool,
    ) -> MaterialDomainResult<PipelineDesc> {
        let vs = if instanced {
            required(self.instanced_vs, "instanced_vs")?
        } else {
            required(self.vs, "vs")?
        };
        let fs = if instanced {
            required(self.instanced_fs, "instanced_fs")?
        } else {
            required(self.fs, "fs")?
        };
        let bgl = required(self.bgl, "bgl")?;
        let layouts = if instanced {
            vec![primitive_vertex_layout(), instance_vertex_layout()]
        } else {
            vec![primitive_vertex_layout()]
        };
        let label = match (double_sided, instanced, sky) {
            (_, true, true) => "standard_sky_pipeline_instanced",
            (true, true, false) => "standard_lit_pipeline_instanced_double_sided",
            (false, true, false) => "standard_lit_pipeline_instanced",
            (true, false, false) => "standard_lit_pipeline_double_sided",
            _ => "standard_lit_pipeline",
        };
        let mut desc = PipelineDesc::new(vs, fs, self.profile.scene_hdr_color_format)
            .with_label(label)
            .with_cache_key(format!(
                "standard:{label}:{:?}",
                self.profile.scene_hdr_color_format
            ))
            .as_warmup()
            .with_topology(PrimitiveTopology::TriangleList)
            .with_vertex_layouts(layouts)
            .with_bind_group_layouts(vec![bgl]);
        desc = if sky {
            // Sky is replayed after terrain/world opaque batches. It must remain
            // read-only in depth, but still test against the scene depth so the
            // dome only fills pixels where no nearer world geometry was drawn.
            desc.with_depth_state(TextureFormat::Depth32Float, sky_depth_mode())
        } else {
            desc.with_depth(TextureFormat::Depth32Float)
        };
        if double_sided || sky {
            desc = desc.with_cull_mode(RasterCullMode::None);
        }
        Ok(desc)
    }

    pub(super) fn instanced_alpha_pipeline_desc(
        &self,
        double_sided: bool,
    ) -> MaterialDomainResult<PipelineDesc> {
        let label = if double_sided {
            "standard_lit_alpha_pipeline_instanced_double_sided"
        } else {
            "standard_lit_alpha_pipeline_instanced"
        };
        Ok(self
            .pipeline_desc(double_sided, true, false)?
            .with_label(label)
            .with_cache_key(format!(
                "standard:{label}:{:?}",
                self.profile.scene_hdr_color_format
            ))
            .with_depth_state(
                TextureFormat::Depth32Float,
                PipelineDepthMode::new(true, false, PipelineDepthCompare::LessOrEqual),
            )
            .with_blend_mode(PipelineBlendMode::Alpha))
    }

    pub(super) fn decal_pipeline_desc(
        &self,
        double_sided: bool,
        instanced: bool,
    ) -> MaterialDomainResult<PipelineDesc> {
        let label = if double_sided {
            "standard_lit_decal_pipeline_instanced_double_sided"
        } else {
            "standard_lit_decal_pipeline_instanced"
        };
        Ok(self
            .pipeline_desc(double_sided, instanced, false)?
            .with_label(label)
            .with_cache_key(format!(
                "standard:{label}:{:?}",
                self.profile.scene_hdr_color_format
            ))
            .with_depth_state(
                TextureFormat::Depth32Float,
                PipelineDepthMode::new(true, false, PipelineDepthCompare::LessOrEqual),
            )
            .with_blend_mode(PipelineBlendMode::Alpha)
            .with_depth_bias(-1.0, -1.0, 0.0))
    }

    pub(super) fn terrain_pipeline_desc(&self) -> MaterialDomainResult<PipelineDesc> {
        let label = "standard_terrain_surface_pipeline";
        Ok(PipelineDesc::new(
            required(self.vs, "vs")?,
            required(self.terrain_fs, "terrain_fs")?,
            self.profile.scene_hdr_color_format,
        )
        .with_label(label)
        .with_cache_key(format!(
            "standard:{label}:{:?}",
            self.profile.scene_hdr_color_format
        ))
        .as_warmup()
        .with_topology(PrimitiveTopology::TriangleList)
        .with_vertex_layouts(vec![primitive_vertex_layout()])
        .with_bind_group_layouts(vec![required(self.bgl, "bgl")?])
        .with_depth(TextureFormat::Depth32Float))
    }

    pub(super) fn gbuffer_terrain_pipeline_desc(&self) -> MaterialDomainResult<PipelineDesc> {
        let label = "standard_gbuffer_terrain_pipeline";
        Ok(PipelineDesc::new(
            required(self.vs, "vs")?,
            required(self.gbuffer_terrain_fs, "gbuffer_terrain_fs")?,
            TextureFormat::Rgba8Unorm,
        )
        .with_label(label)
        .with_cache_key(format!("standard:{label}:gbuffer"))
        .as_warmup()
        .with_topology(PrimitiveTopology::TriangleList)
        .with_vertex_layouts(vec![primitive_vertex_layout()])
        .with_bind_group_layouts(vec![required(self.bgl, "bgl")?])
        .with_color_formats(gbuffer_color_formats())
        .with_depth(TextureFormat::Depth32Float))
    }

    pub(super) fn gbuffer_pipeline_desc(
        &self,
        double_sided: bool,
        instanced: bool,
    ) -> MaterialDomainResult<PipelineDesc> {
        let label = match (double_sided, instanced) {
            (true, true) => "standard_gbuffer_lit_pipeline_instanced_double_sided",
            (false, true) => "standard_gbuffer_lit_pipeline_instanced",
            (true, false) => "standard_gbuffer_lit_pipeline_double_sided",
            _ => "standard_gbuffer_lit_pipeline",
        };
        let vs = if instanced {
            required(self.instanced_vs, "instanced_vs")?
        } else {
            required(self.vs, "vs")?
        };
        let layouts = if instanced {
            vec![primitive_vertex_layout(), instance_vertex_layout()]
        } else {
            vec![primitive_vertex_layout()]
        };
        let mut desc = PipelineDesc::new(
            vs,
            required(self.gbuffer_fs, "gbuffer_fs")?,
            TextureFormat::Rgba8Unorm,
        )
        .with_label(label)
        .with_cache_key(format!("standard:{label}:gbuffer"))
        .as_warmup()
        .with_topology(PrimitiveTopology::TriangleList)
        .with_vertex_layouts(layouts)
        .with_bind_group_layouts(vec![required(self.bgl, "bgl")?])
        .with_color_formats(gbuffer_color_formats())
        .with_depth(TextureFormat::Depth32Float);
        if double_sided {
            desc = desc.with_cull_mode(RasterCullMode::None);
        }
        Ok(desc)
    }

    pub(super) fn shadow_pipeline_desc(
        &self,
        double_sided: bool,
        instanced: bool,
    ) -> MaterialDomainResult<PipelineDesc> {
        let label = match (double_sided, instanced) {
            (true, true) => "standard_sun_shadow_depth_pipeline_instanced_double_sided",
            (false, true) => "standard_sun_shadow_depth_pipeline_instanced",
            (true, false) => "standard_sun_shadow_depth_pipeline_double_sided",
            _ => "standard_sun_shadow_depth_pipeline",
        };
        let vs = if instanced {
            required(self.shadow_instanced_vs, "shadow_instanced_vs")?
        } else {
            required(self.shadow_vs, "shadow_vs")?
        };
        let layouts = if instanced {
            vec![primitive_vertex_layout(), instance_vertex_layout()]
        } else {
            vec![primitive_vertex_layout()]
        };
        let mut desc = PipelineDesc::new(
            vs,
            required(self.shadow_fs, "shadow_fs")?,
            self.profile.shadow_map_color_format,
        )
        .with_label(label)
        .with_cache_key(format!(
            "standard:{label}:{:?}",
            self.profile.shadow_map_color_format
        ))
        .as_warmup()
        .with_topology(PrimitiveTopology::TriangleList)
        .with_vertex_layouts(layouts)
        .with_bind_group_layouts(vec![required(self.bgl, "bgl")?])
        .with_depth(TextureFormat::Depth32Float);
        if double_sided {
            desc = desc.with_cull_mode(RasterCullMode::None);
        }
        Ok(desc)
    }

    pub(super) fn skinned_pipeline_desc(
        &self,
        double_sided: bool,
        gbuffer: bool,
    ) -> MaterialDomainResult<PipelineDesc> {
        let label = match (gbuffer, double_sided) {
            (true, true) => "standard_gbuffer_lit_pipeline_skinned_double_sided",
            (true, false) => "standard_gbuffer_lit_pipeline_skinned",
            (false, true) => "standard_lit_pipeline_skinned_double_sided",
            (false, false) => "standard_lit_pipeline_skinned",
        };
        let color_format = if gbuffer {
            TextureFormat::Rgba8Unorm
        } else {
            self.profile.scene_hdr_color_format
        };
        let fs = if gbuffer {
            required(self.gbuffer_fs, "gbuffer_fs")?
        } else {
            required(self.fs, "fs")?
        };
        let mut desc =
            PipelineDesc::new(required(self.skinned_vs, "skinned_vs")?, fs, color_format)
                .with_label(label)
                .with_cache_key(format!("standard:{label}:{:?}", color_format))
                .as_warmup()
                .with_topology(PrimitiveTopology::TriangleList)
                .with_vertex_layouts(vec![primitive_vertex_layout(), skin_vertex_layout()])
                .with_bind_group_layouts(vec![
                    required(self.bgl, "bgl")?,
                    required(self.skin_bgl, "skin_bgl")?,
                ])
                .with_depth(TextureFormat::Depth32Float);
        if gbuffer {
            desc = desc.with_color_formats(gbuffer_color_formats());
        }
        if double_sided {
            // Diagnostic only: when selected through the skinned debug gate, front-cull
            // distinguishes globally reversed character winding from a genuine need for
            // two-sided rasterization. Production keeps authored double-sided semantics.
            let cull = if std::env::var_os("NEWENGINE_DEBUG_SKIN_CULL_FRONT").is_some() {
                RasterCullMode::Front
            } else {
                RasterCullMode::None
            };
            desc = desc.with_cull_mode(cull);
        }
        Ok(desc)
    }

    pub(super) fn skinned_alpha_pipeline_desc(
        &self,
        double_sided: bool,
    ) -> MaterialDomainResult<PipelineDesc> {
        let label = if double_sided {
            "standard_lit_alpha_pipeline_skinned_double_sided"
        } else {
            "standard_lit_alpha_pipeline_skinned"
        };
        Ok(self
            .skinned_pipeline_desc(double_sided, false)?
            .with_label(label)
            .with_cache_key(format!(
                "standard:{label}:{:?}",
                self.profile.scene_hdr_color_format
            ))
            .with_depth_state(
                TextureFormat::Depth32Float,
                PipelineDepthMode::new(true, false, PipelineDepthCompare::LessOrEqual),
            )
            .with_blend_mode(PipelineBlendMode::Alpha))
    }

    pub(super) fn skinned_shadow_pipeline_desc(
        &self,
        double_sided: bool,
    ) -> MaterialDomainResult<PipelineDesc> {
        let label = if double_sided {
            "standard_sun_shadow_depth_pipeline_skinned_double_sided"
        } else {
            "standard_sun_shadow_depth_pipeline_skinned"
        };
        let mut desc = PipelineDesc::new(
            required(self.shadow_skinned_vs, "shadow_skinned_vs")?,
            required(self.shadow_fs, "shadow_fs")?,
            self.profile.shadow_map_color_format,
        )
        .with_label(label)
        .with_cache_key(format!(
            "standard:{label}:{:?}",
            self.profile.shadow_map_color_format
        ))
        .as_warmup()
        .with_topology(PrimitiveTopology::TriangleList)
        .with_vertex_layouts(vec![primitive_vertex_layout(), skin_vertex_layout()])
        .with_bind_group_layouts(vec![
            required(self.bgl, "bgl")?,
            required(self.skin_bgl, "skin_bgl")?,
        ])
        .with_depth(TextureFormat::Depth32Float);
        if double_sided {
            desc = desc.with_cull_mode(RasterCullMode::None);
        }
        Ok(desc)
    }
}

#[inline]
pub(super) fn required<T: Copy>(value: Option<T>, name: &str) -> MaterialDomainResult<T> {
    value.ok_or_else(|| {
        MaterialDomainError::other(format!("pipeline warmup invariant missing '{name}'"))
    })
}

fn primitive_vertex_layout() -> VertexLayout {
    let stride = std::mem::size_of::<PrimitiveVertex>() as u32;
    VertexLayout::new(
        stride,
        vec![
            VertexAttribute::new(0, 0, VertexFormat::Float32x3),
            VertexAttribute::new(1, 12, VertexFormat::Float32x3),
            VertexAttribute::new(2, 24, VertexFormat::Float32x2),
        ],
    )
}

fn skin_vertex_layout() -> VertexLayout {
    VertexLayout::new(
        48,
        vec![
            VertexAttribute::new(3, 0, VertexFormat::Uint16x4),
            VertexAttribute::new(4, 8, VertexFormat::Float32x4),
            VertexAttribute::new(5, 24, VertexFormat::Uint16x4),
            VertexAttribute::new(6, 32, VertexFormat::Float32x4),
        ],
    )
}

fn instance_vertex_layout() -> VertexLayout {
    VertexLayout::new(
        LIT_INSTANCE_VERTEX_STRIDE,
        vec![
            VertexAttribute::new(5, 0, VertexFormat::Float32x4),
            VertexAttribute::new(6, 16, VertexFormat::Float32x4),
            VertexAttribute::new(7, 32, VertexFormat::Float32x4),
            VertexAttribute::new(8, 48, VertexFormat::Float32x4),
            VertexAttribute::new(9, 64, VertexFormat::Float32x4),
            VertexAttribute::new(10, 80, VertexFormat::Float32x4),
            VertexAttribute::new(11, 96, VertexFormat::Float32x4),
            VertexAttribute::new(12, 112, VertexFormat::Float32x4),
            VertexAttribute::new(13, 128, VertexFormat::Float32x4),
            VertexAttribute::new(14, 144, VertexFormat::Float32x4),
            VertexAttribute::new(15, 160, VertexFormat::Float32x4),
            VertexAttribute::new(16, 176, VertexFormat::Float32x4),
        ],
    )
    .per_instance()
}

fn gbuffer_color_formats() -> Vec<TextureFormat> {
    vec![
        TextureFormat::Rgba8Unorm,
        TextureFormat::Rgba16Float,
        TextureFormat::Rgba8Unorm,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sky_depth_is_read_only_and_occlusion_aware() {
        let depth = sky_depth_mode();
        assert!(
            depth.test,
            "sky must respect depth written by world geometry"
        );
        assert!(!depth.write, "sky must never overwrite scene depth");
        assert_eq!(depth.compare, PipelineDepthCompare::LessOrEqual);
    }
}
