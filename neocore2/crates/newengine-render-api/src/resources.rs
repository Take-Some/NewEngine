use crate::Extent2D;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BufferUsage {
    Vertex,
    Index,
    Uniform,
    Storage,
    /// GPU-generated draw command/count buffer. Concrete backends should make this writable from
    /// compute as well as consumable by their indirect draw stage.
    Indirect,
    Staging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryHint {
    GpuOnly,
    CpuToGpu,
    GpuToCpu,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferDesc {
    pub label: Option<String>,
    pub size: u64,
    pub usage: BufferUsage,
    pub memory: MemoryHint,
}

impl BufferDesc {
    #[inline]
    pub fn new(size: u64, usage: BufferUsage, memory: MemoryHint) -> Self {
        Self {
            label: None,
            size,
            usage,
            memory,
        }
    }

    #[inline]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureFormat {
    Rgba8Unorm,
    Rgba8Srgb,
    Bgra8Unorm,
    Bgra8Srgb,
    Rgba16Float,
    /// Single-channel 32-bit float render/sample format.
    /// Used for color-packed shadow depth when backend depth-sampling is not available.
    R32Float,
    Bc1RgbaUnorm,
    Bc1RgbaSrgb,
    Bc3RgbaUnorm,
    Bc3RgbaSrgb,
    Bc5RgUnorm,
    Bc7RgbaUnorm,
    Bc7RgbaSrgb,
    Depth24Stencil8,
    Depth32Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureMipDataDesc {
    pub level: u32,
    pub width: u32,
    pub height: u32,
    pub offset: u64,
    pub byte_len: u64,
}

impl TextureMipDataDesc {
    #[inline]
    pub fn new(level: u32, width: u32, height: u32, offset: u64, byte_len: u64) -> Self {
        Self {
            level,
            width,
            height,
            offset,
            byte_len,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureUsage {
    Sampled,
    RenderTarget,
    DepthStencil,
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextureDataPolicy {
    /// Data may be uploaded immediately. Useful for tiny fallback resources and startup bootstrap.
    Immediate,
    /// Data should be queued or staged by higher-level render-base systems before it reaches the backend.
    Deferred,
    /// No initial data is expected; the texture will be used as an attachment/storage target.
    Empty,
}

impl Default for TextureDataPolicy {
    #[inline]
    fn default() -> Self {
        Self::Immediate
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextureDesc {
    pub label: Option<String>,
    pub extent: Extent2D,
    pub format: TextureFormat,
    pub usage: TextureUsage,
    pub mip_levels: NonZeroU32,
    pub data: Option<Vec<u8>>,
    /// Optional byte layout for payloads that already contain a complete runtime mip chain.
    /// Empty layout means base-level upload followed by backend mip generation.
    #[serde(default)]
    pub mip_data: Vec<TextureMipDataDesc>,
    #[serde(default)]
    pub data_policy: TextureDataPolicy,
}

impl TextureDesc {
    #[inline]
    pub fn new(extent: Extent2D, format: TextureFormat, usage: TextureUsage) -> Self {
        Self {
            label: None,
            extent,
            format,
            usage,
            mip_levels: NonZeroU32::new(1).expect("mip_levels must be non-zero"),
            data: None,
            mip_data: Vec::new(),
            data_policy: TextureDataPolicy::Immediate,
        }
    }

    #[inline]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[inline]
    pub fn with_mips(mut self, mip_levels: NonZeroU32) -> Self {
        self.mip_levels = mip_levels;
        self
    }

    #[inline]
    pub fn with_data(mut self, data: Vec<u8>) -> Self {
        self.data = Some(data);
        self.data_policy = TextureDataPolicy::Immediate;
        self
    }

    #[inline]
    pub fn with_deferred_data(mut self, data: Vec<u8>) -> Self {
        self.data = Some(data);
        self.data_policy = TextureDataPolicy::Deferred;
        self
    }

    #[inline]
    pub fn with_deferred_mip_data(
        mut self,
        mip_data: Vec<TextureMipDataDesc>,
        data: Vec<u8>,
    ) -> Self {
        self.data = Some(data);
        self.mip_data = mip_data;
        self.data_policy = TextureDataPolicy::Deferred;
        self
    }

    #[inline]
    pub fn with_data_policy(mut self, policy: TextureDataPolicy) -> Self {
        self.data_policy = policy;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode {
    Nearest,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressMode {
    ClampToEdge,
    Repeat,
    MirroredRepeat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamplerDesc {
    pub label: Option<String>,
    pub min_filter: FilterMode,
    pub mag_filter: FilterMode,
    pub mip_filter: FilterMode,
    pub address_u: AddressMode,
    pub address_v: AddressMode,
    pub address_w: AddressMode,
}

impl Default for SamplerDesc {
    #[inline]
    fn default() -> Self {
        Self {
            label: None,
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            mip_filter: FilterMode::Linear,
            address_u: AddressMode::ClampToEdge,
            address_v: AddressMode::ClampToEdge,
            address_w: AddressMode::ClampToEdge,
        }
    }
}

impl SamplerDesc {
    #[inline]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[inline]
    pub fn with_min_filter(mut self, value: FilterMode) -> Self {
        self.min_filter = value;
        self
    }

    #[inline]
    pub fn with_mag_filter(mut self, value: FilterMode) -> Self {
        self.mag_filter = value;
        self
    }

    #[inline]
    pub fn with_mip_filter(mut self, value: FilterMode) -> Self {
        self.mip_filter = value;
        self
    }

    #[inline]
    pub fn with_address_u(mut self, value: AddressMode) -> Self {
        self.address_u = value;
        self
    }

    #[inline]
    pub fn with_address_v(mut self, value: AddressMode) -> Self {
        self.address_v = value;
        self
    }

    #[inline]
    pub fn with_address_w(mut self, value: AddressMode) -> Self {
        self.address_w = value;
        self
    }

    #[inline]
    pub fn with_repeat(mut self) -> Self {
        self.address_u = AddressMode::Repeat;
        self.address_v = AddressMode::Repeat;
        self.address_w = AddressMode::Repeat;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShaderStage {
    Vertex,
    Fragment,
    Compute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShaderSourceKind {
    Glsl,
    Hlsl,
    Wgsl,
    Spirv,
}

impl ShaderSourceKind {
    #[inline]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Glsl => "glsl",
            Self::Hlsl => "hlsl",
            Self::Wgsl => "wgsl",
            Self::Spirv => "spirv",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderDefine {
    pub name: String,
    pub value: String,
}

impl ShaderDefine {
    #[inline]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderAssetDesc {
    pub logical_path: String,
    pub source_kind: ShaderSourceKind,
    #[serde(default = "default_shader_entry")]
    pub entry: String,
    #[serde(default)]
    pub variant_id: String,
    #[serde(default)]
    pub defines: Vec<ShaderDefine>,
    #[serde(default)]
    pub optional: bool,
}

impl ShaderAssetDesc {
    #[inline]
    pub fn new(logical_path: impl Into<String>, source_kind: ShaderSourceKind) -> Self {
        Self {
            logical_path: logical_path.into(),
            source_kind,
            entry: default_shader_entry(),
            variant_id: "default".to_owned(),
            defines: Vec::new(),
            optional: false,
        }
    }

    #[inline]
    pub fn with_entry(mut self, entry: impl Into<String>) -> Self {
        self.entry = entry.into();
        self
    }

    #[inline]
    pub fn with_variant(mut self, variant_id: impl Into<String>) -> Self {
        self.variant_id = variant_id.into();
        self
    }

    #[inline]
    pub fn with_define(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.defines.push(ShaderDefine::new(name, value));
        self
    }

    #[inline]
    pub fn optional(mut self) -> Self {
        self.optional = true;
        self
    }
}

fn default_shader_entry() -> String {
    "main".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShaderDesc {
    pub label: Option<String>,
    pub stage: ShaderStage,
    pub entry: String,
    #[serde(default)]
    pub spirv: Vec<u32>,
    #[serde(default)]
    pub asset: Option<ShaderAssetDesc>,
}

impl ShaderDesc {
    #[inline]
    pub fn new(stage: ShaderStage, entry: impl Into<String>, spirv: Vec<u32>) -> Self {
        Self {
            label: None,
            stage,
            entry: entry.into(),
            spirv,
            asset: None,
        }
    }

    #[inline]
    pub fn from_asset(
        stage: ShaderStage,
        entry: impl Into<String>,
        logical_path: impl Into<String>,
        source_kind: ShaderSourceKind,
    ) -> Self {
        let entry = entry.into();
        Self {
            label: None,
            stage,
            entry: entry.clone(),
            spirv: Vec::new(),
            asset: Some(ShaderAssetDesc::new(logical_path, source_kind).with_entry(entry)),
        }
    }

    #[inline]
    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    #[inline]
    pub fn with_asset(mut self, asset: ShaderAssetDesc) -> Self {
        self.asset = Some(asset);
        self
    }

    #[inline]
    pub fn is_inline_spirv(&self) -> bool {
        !self.spirv.is_empty()
    }
}
