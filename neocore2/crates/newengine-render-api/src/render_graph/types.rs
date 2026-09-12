use crate::{Extent2D, RenderTargetId, TextureFormat, TextureId};
use serde::{Deserialize, Serialize};

const fn default_sample_count() -> u8 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RenderGraphResourceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RenderGraphPassId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderGraphResourceLifetime {
    Persistent,
    TransientFrame,
    Frames(u32),
    External,
}

impl Default for RenderGraphResourceLifetime {
    #[inline]
    fn default() -> Self {
        Self::TransientFrame
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderGraphResourceUsage {
    ColorAttachment,
    DepthAttachment,
    /// Same depth image participates in fixed-function depth testing and fragment sampling.
    /// The backend must use a read-only depth attachment layout for this access.
    DepthAttachmentSampled,
    SampledTexture,
    StorageTexture,
    VertexBuffer,
    IndexBuffer,
    UniformBuffer,
    StorageBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderGraphResourceSemantic {
    Unknown,
    SurfaceColor,
    ViewportColor,
    ViewportDepth,
    ShadowMap,
    SceneHdrColor,
    GBufferAlbedo,
    GBufferNormal,
    GBufferMaterial,
    GBufferDepth,
    ParticleAccum,
    ParticleNormal,
    ParticleMaterial,
    ParticleDepth,
    FroxelFog,
    LitColor,
    BloomComposite,
    ScreenSpaceReflection,
    PostFxColor,
    UiColor,
    UiBackdropBlur,
    DebugOverlay,
    Custom,
}

impl Default for RenderGraphResourceSemantic {
    #[inline]
    fn default() -> Self {
        Self::Unknown
    }
}

impl RenderGraphResourceSemantic {
    #[inline]
    pub const fn is_depth(self) -> bool {
        matches!(
            self,
            Self::ViewportDepth | Self::ShadowMap | Self::GBufferDepth
        )
    }

    #[inline]
    pub const fn is_surface_color(self) -> bool {
        matches!(self, Self::SurfaceColor)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderGraphExternalResource {
    /// Backend-owned swapchain color surface. The backend resolves the current image.
    SwapchainColor,
    /// Runtime/backend render target created through RenderApi::create_render_target.
    RenderTarget(RenderTargetId),
    /// Backend texture imported as a graph-readable external resource.
    Texture(TextureId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderGraphResourceAccess {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RenderGraphQueueKind {
    Graphics,
    Compute,
    Transfer,
}

impl Default for RenderGraphQueueKind {
    #[inline]
    fn default() -> Self {
        Self::Graphics
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RenderGraphPassKind {
    DepthPrepass,
    /// GPU visibility/indirect-command preparation using previous-frame depth/Hi-Z.
    VisibilityCull,
    ShadowMap,
    ShadowCascadeMap,
    LocalShadowMap,
    TessellationPrepare,
    GBuffer,
    DeferredLighting,
    ForwardOpaque,
    ParticleSimulation,
    HairSimulation,
    ParticleGBuffer,
    ParticleComposite,
    Transparent,
    Water,
    FroxelFog,
    ScreenSpaceReflections,
    PostFx,
    BloomExtract,
    BloomBlur,
    TaaResolve,
    MsaaResolve,
    UiBackdropBlur,
    UiComposite,
    DebugOverlay,
    Copy,
    Custom,
}

impl Default for RenderGraphPassKind {
    #[inline]
    fn default() -> Self {
        Self::Custom
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderGraphResourceDesc {
    pub id: RenderGraphResourceId,
    pub label: Option<String>,
    #[serde(default)]
    pub semantic: RenderGraphResourceSemantic,
    pub usage: RenderGraphResourceUsage,
    pub lifetime: RenderGraphResourceLifetime,
    #[serde(default)]
    pub extent: Option<Extent2D>,
    #[serde(default)]
    pub format: Option<TextureFormat>,
    /// Physical sample count required by the logical resource. `1` is the
    /// non-MSAA default and participates in transient-allocation compatibility.
    #[serde(default = "default_sample_count")]
    pub sample_count: u8,
    /// Required allocation size for buffer resources. Texture resources derive
    /// their allocation shape from `extent` + `format` instead.
    #[serde(default)]
    pub byte_size: Option<u64>,
    #[serde(default)]
    pub external: Option<RenderGraphExternalResource>,
}

impl RenderGraphResourceDesc {
    #[inline]
    pub fn transient_texture(
        id: RenderGraphResourceId,
        label: impl Into<String>,
        usage: RenderGraphResourceUsage,
        extent: Extent2D,
        format: TextureFormat,
    ) -> Self {
        Self {
            id,
            label: Some(label.into()),
            semantic: RenderGraphResourceSemantic::Unknown,
            usage,
            lifetime: RenderGraphResourceLifetime::TransientFrame,
            extent: Some(extent),
            format: Some(format),
            sample_count: 1,
            byte_size: None,
            external: None,
        }
    }

    #[inline]
    pub fn transient_buffer(
        id: RenderGraphResourceId,
        label: impl Into<String>,
        usage: RenderGraphResourceUsage,
        byte_size: u64,
    ) -> Self {
        Self {
            id,
            label: Some(label.into()),
            semantic: RenderGraphResourceSemantic::Unknown,
            usage,
            lifetime: RenderGraphResourceLifetime::TransientFrame,
            extent: None,
            format: None,
            sample_count: 1,
            byte_size: Some(byte_size),
            external: None,
        }
    }

    #[inline]
    pub fn external(
        id: RenderGraphResourceId,
        label: impl Into<String>,
        usage: RenderGraphResourceUsage,
    ) -> Self {
        Self {
            id,
            label: Some(label.into()),
            semantic: RenderGraphResourceSemantic::Unknown,
            usage,
            lifetime: RenderGraphResourceLifetime::External,
            extent: None,
            format: None,
            sample_count: 1,
            byte_size: None,
            external: None,
        }
    }

    #[inline]
    pub fn external_swapchain(
        id: RenderGraphResourceId,
        label: impl Into<String>,
        usage: RenderGraphResourceUsage,
        extent: Extent2D,
        format: TextureFormat,
    ) -> Self {
        Self {
            id,
            label: Some(label.into()),
            semantic: RenderGraphResourceSemantic::Unknown,
            usage,
            lifetime: RenderGraphResourceLifetime::External,
            extent: Some(extent),
            format: Some(format),
            sample_count: 1,
            byte_size: None,
            external: Some(RenderGraphExternalResource::SwapchainColor),
        }
    }

    #[inline]
    pub fn external_render_target(
        id: RenderGraphResourceId,
        label: impl Into<String>,
        render_target: RenderTargetId,
        usage: RenderGraphResourceUsage,
        extent: Extent2D,
        format: TextureFormat,
    ) -> Self {
        Self {
            id,
            label: Some(label.into()),
            semantic: RenderGraphResourceSemantic::Unknown,
            usage,
            lifetime: RenderGraphResourceLifetime::External,
            extent: Some(extent),
            format: Some(format),
            sample_count: 1,
            byte_size: None,
            external: Some(RenderGraphExternalResource::RenderTarget(render_target)),
        }
    }

    #[inline]
    pub fn external_texture(
        id: RenderGraphResourceId,
        label: impl Into<String>,
        texture: TextureId,
        usage: RenderGraphResourceUsage,
    ) -> Self {
        Self {
            id,
            label: Some(label.into()),
            semantic: RenderGraphResourceSemantic::Unknown,
            usage,
            lifetime: RenderGraphResourceLifetime::External,
            extent: None,
            format: None,
            sample_count: 1,
            byte_size: None,
            external: Some(RenderGraphExternalResource::Texture(texture)),
        }
    }

    #[inline]
    pub fn with_semantic(mut self, semantic: RenderGraphResourceSemantic) -> Self {
        self.semantic = semantic;
        self
    }

    #[inline]
    pub fn with_sample_count(mut self, sample_count: u8) -> Self {
        self.sample_count = sample_count.max(1);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderGraphResourceRef {
    pub resource: RenderGraphResourceId,
    pub usage: RenderGraphResourceUsage,
    pub access: RenderGraphResourceAccess,
}

impl RenderGraphResourceRef {
    #[inline]
    pub const fn read(resource: RenderGraphResourceId, usage: RenderGraphResourceUsage) -> Self {
        Self {
            resource,
            usage,
            access: RenderGraphResourceAccess::Read,
        }
    }

    #[inline]
    pub const fn write(resource: RenderGraphResourceId, usage: RenderGraphResourceUsage) -> Self {
        Self {
            resource,
            usage,
            access: RenderGraphResourceAccess::Write,
        }
    }
}
