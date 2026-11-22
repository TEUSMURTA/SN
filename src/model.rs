#[derive(Clone, Debug, Default, PartialEq)]
pub struct VertexAttributes {
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 4]>>,
    pub texcoords: Option<Vec<[f32; 2]>>,
    pub joints: Option<Vec<[u16; 4]>>,
    pub weights: Option<Vec<[f32; 4]>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Primitive {
    pub attributes: VertexAttributes,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshLod {
    pub primitives: Vec<Primitive>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MeshAsset {
    pub name: String,
    pub lods: Vec<MeshLod>,
    pub skeleton: Option<Skeleton>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    pub joints: Vec<Joint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: String,
    pub base_color_texture: Option<usize>,
    pub normal_texture: Option<usize>,
    pub metallic_roughness_texture: Option<usize>,
    pub base_color_factor: [f32; 4],
    pub metallic_factor: f32,
    pub roughness_factor: f32,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            name: String::new(),
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            base_color_factor: [1.0; 4],
            metallic_factor: 0.0,
            roughness_factor: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureFormat {
    Rgba8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Srgb,
    Linear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureAsset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub color_space: ColorSpace,
    pub mips: Vec<MipLevel>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    pub mesh: MeshAsset,
    pub textures: Vec<TextureAsset>,
    pub materials: Vec<Material>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedAsset {
    pub name: String,
    pub mesh: MeshLod,
    pub textures: Vec<PreparedTexture>,
    pub materials: Vec<Material>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedTexturePayload {
    Png(Vec<u8>),
    Ktx2Basis(Vec<u8>),
}

impl PreparedTexturePayload {
    #[must_use]
    pub fn encoded_bytes(&self) -> &[u8] {
        match self {
            Self::Png(bytes) | Self::Ktx2Basis(bytes) => bytes,
        }
    }

    #[must_use]
    pub fn encoded_len(&self) -> usize {
        self.encoded_bytes().len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuTextureFormat {
    Rgba8,
    Bc7,
    Astc4x4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTextureInfo {
    pub format: GpuTextureFormat,
    pub mip_count: u32,
    pub byte_len: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedTexture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub color_space: ColorSpace,
    pub payload: PreparedTexturePayload,
    /// Runtime GPU layout, when validated; unrelated to the encoded file length.
    pub gpu: Option<GpuTextureInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConversionReport {
    pub glb_bytes: usize,
    pub primitive_count: usize,
    pub vertex_count: usize,
    pub index_count: usize,
    pub texture_count: usize,
    /// Encoded PNG/KTX2 bytes stored in the GLB.
    pub encoded_texture_bytes: usize,
    /// Sum of declared GPU footprints; textures without GPU metadata contribute zero.
    pub gpu_texture_bytes: u64,
}
