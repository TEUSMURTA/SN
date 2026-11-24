use crate::error::{Result, SnError};

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
    /// KTX2 supercompresso Basis (UASTC/ETC1S) — embarcado via `KHR_texture_basisu`.
    Ktx2Basis(Vec<u8>),
    /// KTX2 ASTC 4x4 nativo (vkFormat preenchido, sem supercompressão) — embarcado
    /// como `image/ktx2` comum, sem `KHR_texture_basisu`.
    Ktx2Astc(Vec<u8>),
}

impl PreparedTexturePayload {
    #[must_use]
    pub fn encoded_bytes(&self) -> &[u8] {
        match self {
            Self::Png(bytes) | Self::Ktx2Basis(bytes) | Self::Ktx2Astc(bytes) => bytes,
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
    Uastc,
}

impl GpuTextureFormat {
    /// Bytes por unidade mínima do formato: 4 por pixel (RGBA8) ou 16 por bloco 4x4 (BC7/ASTC/UASTC).
    #[must_use]
    pub fn bytes_per_unit(&self) -> u64 {
        match self {
            Self::Rgba8 => 4,
            Self::Bc7 | Self::Astc4x4 | Self::Uastc => 16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuTextureInfo {
    pub format: GpuTextureFormat,
    pub mip_count: u32,
    pub byte_len: u64,
}

impl GpuTextureInfo {
    /// Cadeia mip completa (até 1x1) do formato de GPU para as dimensões da textura.
    pub fn full_chain(format: GpuTextureFormat, width: u32, height: u32) -> Result<Self> {
        let mip_count = full_mip_count(width, height)?;
        Self::with_mip_count(format, width, height, mip_count)
    }

    /// Primeiros `mip_count` níveis da cadeia a partir das dimensões base.
    pub fn with_mip_count(
        format: GpuTextureFormat,
        width: u32,
        height: u32,
        mip_count: u32,
    ) -> Result<Self> {
        let byte_len = mip_chain_bytes(format, width, height, mip_count)?;
        Ok(Self {
            format,
            mip_count,
            byte_len,
        })
    }
}

/// Blocos 4x4 que cobrem uma dimensão, arredondando para cima sem overflow.
fn block_count(dim: u32) -> u32 {
    dim / 4 + u32::from(!dim.is_multiple_of(4))
}

fn invalid_gpu_dims(width: u32, height: u32) -> SnError {
    SnError::InvalidData {
        resource: "gpu texture dimensions",
        reason: format!("dimensions {width}x{height} must be non-zero"),
    }
}

/// Níveis mip de `width`x`height` até 1x1; falha se alguma dimensão for 0.
pub fn full_mip_count(width: u32, height: u32) -> Result<u32> {
    if width == 0 || height == 0 {
        return Err(invalid_gpu_dims(width, height));
    }
    // largest >= 1, então checked_ilog2 é Some e exp + 1 não estoura (exp <= 31).
    Ok(width.max(height).checked_ilog2().unwrap_or(0) + 1)
}

/// Bytes de um único nível mip do formato de GPU para as dimensões dadas.
pub fn level_bytes(format: GpuTextureFormat, width: u32, height: u32) -> Result<u64> {
    if width == 0 || height == 0 {
        return Err(invalid_gpu_dims(width, height));
    }
    let units = match format {
        GpuTextureFormat::Rgba8 => {
            u64::from(width)
                .checked_mul(u64::from(height))
                .ok_or(SnError::SizeOverflow {
                    resource: "gpu mip level pixels",
                })?
        }
        GpuTextureFormat::Bc7 | GpuTextureFormat::Astc4x4 | GpuTextureFormat::Uastc => {
            u64::from(block_count(width))
                .checked_mul(u64::from(block_count(height)))
                .ok_or(SnError::SizeOverflow {
                    resource: "gpu mip level blocks",
                })?
        }
    };
    units
        .checked_mul(format.bytes_per_unit())
        .ok_or(SnError::SizeOverflow {
            resource: "gpu mip level bytes",
        })
}

/// Soma dos bytes dos `mip_count` primeiros níveis da cadeia a partir das dimensões base.
pub fn mip_chain_bytes(
    format: GpuTextureFormat,
    width: u32,
    height: u32,
    mip_count: u32,
) -> Result<u64> {
    let full = full_mip_count(width, height)?;
    if mip_count == 0 || mip_count > full {
        return Err(SnError::InvalidData {
            resource: "gpu texture mip_count",
            reason: format!("mip_count {mip_count} fora de 1..={full} para {width}x{height}"),
        });
    }
    let mut total = 0u64;
    for level in 0..mip_count {
        let level_width = width.checked_shr(level).unwrap_or(1).max(1);
        let level_height = height.checked_shr(level).unwrap_or(1).max(1);
        let bytes = level_bytes(format, level_width, level_height)?;
        total = total.checked_add(bytes).ok_or(SnError::SizeOverflow {
            resource: "gpu mip chain bytes",
        })?;
    }
    Ok(total)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_mip_count_pot_e_npot() {
        assert_eq!(full_mip_count(1, 1).unwrap(), 1);
        assert_eq!(full_mip_count(8, 4).unwrap(), 4);
        assert_eq!(full_mip_count(1024, 1024).unwrap(), 11);
        assert_eq!(full_mip_count(1000, 500).unwrap(), 10);
    }

    #[test]
    fn full_mip_count_rejeita_dimimensao_zero() {
        assert!(full_mip_count(0, 4).is_err());
        assert!(full_mip_count(4, 0).is_err());
        assert!(full_mip_count(0, 0).is_err());
    }

    #[test]
    fn level_bytes_rgba8_e_blocos_4x4() {
        // RGBA8: 4 bytes por pixel.
        assert_eq!(
            level_bytes(GpuTextureFormat::Rgba8, 8, 4).unwrap(),
            8 * 4 * 4
        );
        // UASTC: 16 bytes por bloco 4x4; 8x4 cobre 2 blocos.
        assert_eq!(level_bytes(GpuTextureFormat::Uastc, 8, 4).unwrap(), 2 * 16);
        // NPOT: 5x3 cobre ceil(5/4) x ceil(3/4) = 2 blocos; 1x1 ainda ocupa um bloco.
        assert_eq!(level_bytes(GpuTextureFormat::Uastc, 5, 3).unwrap(), 2 * 16);
        assert_eq!(level_bytes(GpuTextureFormat::Uastc, 1, 1).unwrap(), 16);
        assert_eq!(level_bytes(GpuTextureFormat::Astc4x4, 4, 4).unwrap(), 16);
        assert!(level_bytes(GpuTextureFormat::Uastc, 0, 4).is_err());
        assert!(level_bytes(GpuTextureFormat::Rgba8, u32::MAX, u32::MAX).is_err());
    }

    #[test]
    fn mip_chain_bytes_soma_os_niveis() {
        // RGBA8 4x4 cadeia completa: 16 + 4 + 1 pixels * 4 bytes = 84.
        assert_eq!(
            mip_chain_bytes(GpuTextureFormat::Rgba8, 4, 4, 3).unwrap(),
            84
        );
        // UASTC 8x4 cadeia completa: 2 + 1 + 1 + 1 blocos * 16 bytes = 80.
        assert_eq!(
            mip_chain_bytes(GpuTextureFormat::Uastc, 8, 4, 4).unwrap(),
            5 * 16
        );
        // Cadeia parcial conta apenas os níveis pedidos.
        assert_eq!(
            mip_chain_bytes(GpuTextureFormat::Uastc, 8, 4, 1).unwrap(),
            2 * 16
        );
        assert!(mip_chain_bytes(GpuTextureFormat::Uastc, 8, 4, 0).is_err());
        assert!(mip_chain_bytes(GpuTextureFormat::Uastc, 8, 4, 5).is_err());
        assert!(mip_chain_bytes(GpuTextureFormat::Uastc, 0, 4, 1).is_err());
    }

    #[test]
    fn gpu_texture_info_full_chain_e_with_mip_count() {
        let info = GpuTextureInfo::full_chain(GpuTextureFormat::Uastc, 8, 4).unwrap();
        assert_eq!(info.format, GpuTextureFormat::Uastc);
        assert_eq!(info.mip_count, 4);
        assert_eq!(info.byte_len, 80);

        let partial = GpuTextureInfo::with_mip_count(GpuTextureFormat::Rgba8, 4, 4, 2).unwrap();
        assert_eq!(partial.mip_count, 2);
        assert_eq!(partial.byte_len, 64 + 16);

        assert!(GpuTextureInfo::with_mip_count(GpuTextureFormat::Rgba8, 4, 4, 0).is_err());
        assert!(GpuTextureInfo::with_mip_count(GpuTextureFormat::Rgba8, 4, 4, 4).is_err());
        assert!(GpuTextureInfo::full_chain(GpuTextureFormat::Uastc, 0, 8).is_err());
    }
}
