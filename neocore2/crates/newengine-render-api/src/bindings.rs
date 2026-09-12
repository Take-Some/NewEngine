use crate::{BindGroupLayoutId, BufferId, SamplerId, TextureId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GraphTextureSemantic {
    /// Depth produced by the opaque scene path consumed by the current graph pass.
    /// Resolves to ViewportDepth in forward and GBufferDepth in deferred rendering.
    SceneDepth,
    GBufferAlbedo,
    GBufferNormal,
    GBufferMaterial,
    LitColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingKind {
    Texture2D,
    /// Texture resolved by the render backend from the current RenderGraph pass.
    /// This keeps engine-side feature recording independent of backend transient TextureIds.
    GraphTexture2D(GraphTextureSemantic),
    Sampler,
    UniformBuffer,
    StorageBuffer,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BufferBinding {
    pub buffer: BufferId,
    pub offset: u64,
    pub size: u64,
}

impl BufferBinding {
    #[inline]
    pub const fn new(buffer: BufferId, offset: u64, size: u64) -> Self {
        Self {
            buffer,
            offset,
            size,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindGroupLayoutDesc {
    pub label: Option<String>,
    pub bindings: Vec<BindingKind>,
}

impl BindGroupLayoutDesc {
    #[inline]
    pub fn new(bindings: Vec<BindingKind>) -> Self {
        Self {
            label: None,
            bindings,
        }
    }

    #[inline]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindGroupDesc {
    pub label: Option<String>,
    pub layout: BindGroupLayoutId,
    pub texture0: Option<TextureId>,
    pub texture1: Option<TextureId>,
    pub texture2: Option<TextureId>,
    pub texture3: Option<TextureId>,
    pub texture4: Option<TextureId>,
    #[serde(default)]
    pub texture5: Option<TextureId>,
    /// Safe descriptor initialization for GraphTexture2D bindings before graph replay
    /// resolves the actual pass-local texture. The backend must replace it before access.
    #[serde(default)]
    pub graph_texture_fallback: Option<TextureId>,
    pub sampler0: Option<SamplerId>,
    pub uniform0: Option<BufferBinding>,
    pub storage0: Option<BufferBinding>,
    #[serde(default)]
    pub storage1: Option<BufferBinding>,
    #[serde(default)]
    pub storage2: Option<BufferBinding>,
}

impl BindGroupDesc {
    #[inline]
    pub fn new(layout: BindGroupLayoutId) -> Self {
        Self {
            label: None,
            layout,
            texture0: None,
            texture1: None,
            texture2: None,
            texture3: None,
            texture4: None,
            texture5: None,
            graph_texture_fallback: None,
            sampler0: None,
            uniform0: None,
            storage0: None,
            storage1: None,
            storage2: None,
        }
    }

    #[inline]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[inline]
    pub fn with_texture0(mut self, tex: TextureId) -> Self {
        self.texture0 = Some(tex);
        self
    }

    #[inline]
    pub fn with_texture1(mut self, tex: TextureId) -> Self {
        self.texture1 = Some(tex);
        self
    }

    #[inline]
    pub fn with_texture2(mut self, tex: TextureId) -> Self {
        self.texture2 = Some(tex);
        self
    }

    #[inline]
    pub fn with_texture3(mut self, tex: TextureId) -> Self {
        self.texture3 = Some(tex);
        self
    }

    #[inline]
    pub fn with_texture4(mut self, tex: TextureId) -> Self {
        self.texture4 = Some(tex);
        self
    }

    #[inline]
    pub fn with_texture5(mut self, tex: TextureId) -> Self {
        self.texture5 = Some(tex);
        self
    }

    #[inline]
    pub fn with_graph_texture_fallback(mut self, tex: TextureId) -> Self {
        self.graph_texture_fallback = Some(tex);
        self
    }

    #[inline]
    pub fn texture_at(&self, index: usize) -> Option<TextureId> {
        match index {
            0 => self.texture0,
            1 => self.texture1,
            2 => self.texture2,
            3 => self.texture3,
            4 => self.texture4,
            5 => self.texture5,
            _ => None,
        }
    }

    #[inline]
    pub fn with_sampler0(mut self, sampler: SamplerId) -> Self {
        self.sampler0 = Some(sampler);
        self
    }

    #[inline]
    pub fn with_uniform0(mut self, binding: BufferBinding) -> Self {
        self.uniform0 = Some(binding);
        self
    }

    #[inline]
    pub fn with_storage0(mut self, binding: BufferBinding) -> Self {
        self.storage0 = Some(binding);
        self
    }

    #[inline]
    pub fn with_storage1(mut self, binding: BufferBinding) -> Self {
        self.storage1 = Some(binding);
        self
    }

    #[inline]
    pub fn with_storage2(mut self, binding: BufferBinding) -> Self {
        self.storage2 = Some(binding);
        self
    }

    #[inline]
    pub fn storage_at(&self, index: usize) -> Option<BufferBinding> {
        match index {
            0 => self.storage0,
            1 => self.storage1,
            2 => self.storage2,
            _ => None,
        }
    }
}
