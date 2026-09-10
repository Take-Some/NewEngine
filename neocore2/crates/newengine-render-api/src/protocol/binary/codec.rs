use crate::{IndexFormat, RenderDrawListKind, RenderGraphPassKind};

#[inline]
pub(super) fn put_len(out: &mut Vec<u8>, len: usize, what: &str) -> Result<(), String> {
    let len = u32::try_from(len)
        .map_err(|_| format!("{what} is too large for binary render command packet"))?;
    put_u32(out, len);
    Ok(())
}

#[inline]
pub(super) fn put_bytes(out: &mut Vec<u8>, bytes: &[u8], what: &str) -> Result<(), String> {
    put_len(out, bytes.len(), what)?;
    out.extend_from_slice(bytes);
    Ok(())
}

#[inline]
pub(super) fn put_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}
#[inline]
pub(super) fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
#[inline]
pub(super) fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}
#[inline]
pub(super) fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
#[inline]
pub(super) fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_le_bytes());
}
#[inline]
pub(super) fn put_optional_render_graph_pass_kind(
    out: &mut Vec<u8>,
    phase: Option<RenderGraphPassKind>,
) {
    match phase {
        Some(phase) => {
            put_u8(out, 1);
            put_u8(out, render_graph_pass_kind_tag(phase));
        }
        None => put_u8(out, 0),
    }
}

#[inline]
pub(super) fn put_optional_render_draw_list_kind(
    out: &mut Vec<u8>,
    kind: Option<RenderDrawListKind>,
) {
    match kind {
        Some(kind) => {
            put_u8(out, 1);
            put_u8(out, render_draw_list_kind_tag(kind));
        }
        None => put_u8(out, 0),
    }
}

#[inline]
pub(super) fn render_graph_pass_kind_tag(kind: RenderGraphPassKind) -> u8 {
    match kind {
        RenderGraphPassKind::DepthPrepass => 1,
        RenderGraphPassKind::VisibilityCull => 25,
        RenderGraphPassKind::ShadowMap => 2,
        RenderGraphPassKind::ShadowCascadeMap => 3,
        RenderGraphPassKind::LocalShadowMap => 20,
        RenderGraphPassKind::TessellationPrepare => 4,
        RenderGraphPassKind::GBuffer => 5,
        RenderGraphPassKind::DeferredLighting => 6,
        RenderGraphPassKind::ForwardOpaque => 7,
        RenderGraphPassKind::ParticleSimulation => 21,
        RenderGraphPassKind::HairSimulation => 22,
        RenderGraphPassKind::ParticleGBuffer => 23,
        RenderGraphPassKind::ParticleComposite => 24,
        RenderGraphPassKind::Transparent => 8,
        RenderGraphPassKind::Water => 9,
        RenderGraphPassKind::PostFx => 10,
        RenderGraphPassKind::BloomExtract => 11,
        RenderGraphPassKind::BloomBlur => 12,
        RenderGraphPassKind::TaaResolve => 13,
        RenderGraphPassKind::MsaaResolve => 14,
        RenderGraphPassKind::UiComposite => 15,
        RenderGraphPassKind::UiBackdropBlur => 19,
        RenderGraphPassKind::DebugOverlay => 16,
        RenderGraphPassKind::Copy => 17,
        RenderGraphPassKind::Custom => 18,
    }
}

#[inline]
pub(super) fn render_graph_pass_kind_from_tag(tag: u8) -> Result<RenderGraphPassKind, String> {
    match tag {
        1 => Ok(RenderGraphPassKind::DepthPrepass),
        25 => Ok(RenderGraphPassKind::VisibilityCull),
        2 => Ok(RenderGraphPassKind::ShadowMap),
        3 => Ok(RenderGraphPassKind::ShadowCascadeMap),
        20 => Ok(RenderGraphPassKind::LocalShadowMap),
        4 => Ok(RenderGraphPassKind::TessellationPrepare),
        5 => Ok(RenderGraphPassKind::GBuffer),
        6 => Ok(RenderGraphPassKind::DeferredLighting),
        7 => Ok(RenderGraphPassKind::ForwardOpaque),
        21 => Ok(RenderGraphPassKind::ParticleSimulation),
        22 => Ok(RenderGraphPassKind::HairSimulation),
        23 => Ok(RenderGraphPassKind::ParticleGBuffer),
        24 => Ok(RenderGraphPassKind::ParticleComposite),
        8 => Ok(RenderGraphPassKind::Transparent),
        9 => Ok(RenderGraphPassKind::Water),
        10 => Ok(RenderGraphPassKind::PostFx),
        11 => Ok(RenderGraphPassKind::BloomExtract),
        12 => Ok(RenderGraphPassKind::BloomBlur),
        13 => Ok(RenderGraphPassKind::TaaResolve),
        14 => Ok(RenderGraphPassKind::MsaaResolve),
        15 => Ok(RenderGraphPassKind::UiComposite),
        19 => Ok(RenderGraphPassKind::UiBackdropBlur),
        16 => Ok(RenderGraphPassKind::DebugOverlay),
        17 => Ok(RenderGraphPassKind::Copy),
        18 => Ok(RenderGraphPassKind::Custom),
        _ => Err(format!("invalid render graph pass kind tag {tag}")),
    }
}

#[inline]
pub(super) fn render_draw_list_kind_tag(kind: RenderDrawListKind) -> u8 {
    match kind {
        RenderDrawListKind::ShadowCasters => 1,
        RenderDrawListKind::LocalShadowCasters => 6,
        RenderDrawListKind::OpaqueForward => 2,
        RenderDrawListKind::ParticleGBuffer => 7,
        RenderDrawListKind::Transparent => 3,
        RenderDrawListKind::Debug => 5,
    }
}

#[inline]
pub(super) fn render_draw_list_kind_from_tag(tag: u8) -> Result<RenderDrawListKind, String> {
    match tag {
        1 => Ok(RenderDrawListKind::ShadowCasters),
        6 => Ok(RenderDrawListKind::LocalShadowCasters),
        2 => Ok(RenderDrawListKind::OpaqueForward),
        7 => Ok(RenderDrawListKind::ParticleGBuffer),
        3 => Ok(RenderDrawListKind::Transparent),
        5 => Ok(RenderDrawListKind::Debug),
        _ => Err(format!("invalid render draw-list kind tag {tag}")),
    }
}

#[inline]
pub(super) fn put_index_format(out: &mut Vec<u8>, format: IndexFormat) {
    out.push(match format {
        IndexFormat::U16 => 16,
        IndexFormat::U32 => 32,
    });
}
#[inline]
pub(super) fn get_index_format(v: u8) -> Result<IndexFormat, String> {
    match v {
        16 => Ok(IndexFormat::U16),
        32 => Ok(IndexFormat::U32),
        _ => Err(format!("invalid index format tag {v}")),
    }
}

pub(super) struct BinReader<'a>(newengine_ui_draw::binary_codec::ReadCursor<'a>);

impl<'a> BinReader<'a> {
    #[inline]
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self(newengine_ui_draw::binary_codec::ReadCursor::new(
            bytes,
            "render command batch binary packet",
        ))
    }

    pub(super) fn string(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes_vec()?)
            .map_err(|e| format!("invalid UTF-8 string in render binary packet: {e}"))
    }

    pub(super) fn optional_render_graph_pass_kind(
        &mut self,
    ) -> Result<Option<RenderGraphPassKind>, String> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(render_graph_pass_kind_from_tag(self.u8()?)?)),
            tag => Err(format!(
                "invalid optional render graph pass kind presence tag {tag}"
            )),
        }
    }

    pub(super) fn optional_render_draw_list_kind(
        &mut self,
    ) -> Result<Option<RenderDrawListKind>, String> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(render_draw_list_kind_from_tag(self.u8()?)?)),
            tag => Err(format!(
                "invalid optional render draw-list kind presence tag {tag}"
            )),
        }
    }
}

impl<'a> core::ops::Deref for BinReader<'a> {
    type Target = newengine_ui_draw::binary_codec::ReadCursor<'a>;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl core::ops::DerefMut for BinReader<'_> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
