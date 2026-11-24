//! Protótipo KTX2, despachado por plataforma:
//! - PC (`pc-ktx2-basis`): KTX2 supercompresso UASTC, embarcado via `KHR_texture_basisu`.
//! - Switch (`switch-ktx2-astc`): o mesmo encode UASTC é transcodificado para
//!   ASTC 4x4 (via `basis-shim`) e o KTX2 carrega `vkFormat` ASTC nativo, sem
//!   supercompressão e sem `KHR_texture_basisu`.
//!
//! Fluxo comum: mips RGBA8 validados -> encoder Basis (via `basis-shim`) ->
//! arquivo `.basis` -> parser próprio extrai os slices UASTC por nível ->
//! contêiner KTX2 escrito aqui. O ETC1S permanece indisponível porque um KTX2
//! desse modo exige o codebook global na seção SGD, que os bindings do encoder
//! não expõem.
//!
//! O payload KTX2 é o formato codificado (UASTC intermediário de transcodificação
//! no PC, ASTC 4x4 nativo no Switch); [`crate::model::GpuTextureInfo`] declara o
//! footprint de GPU do perfil de plataforma (UASTC no PC, ASTC 4x4 no Switch),
//! somando 16 bytes por bloco 4x4 de cada nível real registrado no levelIndex.

use basis_shim::{
    AstcTranscodeError, EncodeError, EncoderColorSpace, MipInput as EncoderMip, encode_basis,
    transcode_uastc_to_astc4x4,
};

use crate::{
    config::ConversionOptions,
    convert::texture::{fit_dimensions, resize_rgba, validate_texture},
    error::{Result, SnError},
    model::{
        ColorSpace, GpuTextureFormat, GpuTextureInfo, MipLevel, PreparedTexture,
        PreparedTexturePayload, TextureAsset, mip_chain_bytes,
    },
    platform::Platform,
};

// ---- Contêiner .basis (basist::basis_file_header / basis_slice_desc) ----

const BASIS_SIGNATURE: u16 = 0x4273; // ('B' << 8) | 's'
const BASIS_MIN_VERSION: u16 = 0x10;
const BASIS_HEADER_SIZE: usize = 77;
const SLICE_DESC_SIZE: usize = 23;
const BASIS_FLAG_ETC1S: u16 = 1;
const BASIS_FORMAT_UASTC: u8 = 1;
const UASTC_BLOCK_BYTES: usize = 16;

// ---- Contêiner KTX2 ----

const KTX2_MAGIC: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
const KTX2_HEADER_SIZE: usize = 80;
const KTX2_LEVEL_INDEX_SIZE: usize = 24;
/// Alinhamento dos dados de mip para `supercompressionScheme = 0`
/// (`lcm(4, bytes_plane_0)`; blocos UASTC têm 16 bytes).
const KTX2_MIP_ALIGNMENT: usize = 16;
const KHR_DF_MODEL_UASTC: u8 = 166;
const KHR_DF_MODEL_ASTC: u8 = 162;
const KHR_DF_PRIMARIES_BT709: u8 = 1;
const KHR_DF_TRANSFER_LINEAR: u8 = 1;
const KHR_DF_TRANSFER_SRGB: u8 = 2;
const KHR_DF_CHANNEL_UASTC_RGBA: u8 = 3;
const KHR_DF_CHANNEL_UASTC_RGB: u8 = 4;
const KHR_DF_CHANNEL_ASTC_4X4_RGBA: u8 = 0;
/// `VK_FORMAT_ASTC_4x4_UNORM_BLOCK` / `VK_FORMAT_ASTC_4x4_SRGB_BLOCK`.
const VK_FORMAT_ASTC_4X4_UNORM_BLOCK: u32 = 157;
const VK_FORMAT_ASTC_4X4_SRGB_BLOCK: u32 = 158;
/// Blocos ASTC 4x4 também têm 16 bytes.
const ASTC_BLOCK_BYTES: usize = 16;

/// Dimensões e totais da cadeia completa de mips (níveis fornecidos + cauda
/// gerada até 1x1), calculados sem alocar pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MipPlan {
    /// Cadeia completa do maior para o menor, terminando em 1x1.
    pub levels: Vec<(u32, u32)>,
    pub provided_levels: usize,
    pub total_pixels: u64,
    pub total_rgba_bytes: u64,
}

pub(crate) fn validate_ktx2_texture(
    texture: &TextureAsset,
    options: &ConversionOptions,
) -> Result<u64> {
    // Checagens existentes (RGBA8, dimensões, cadeia, orçamentos dos mips
    // fornecidos) continuam valendo; o plano estende os orçamentos para a
    // cauda gerada antes de qualquer alocação ou encoding.
    validate_texture(texture, options)?;

    let (fit_width, fit_height) =
        fit_dimensions(texture.width, texture.height, options.max_texture_dimension);
    if (fit_width, fit_height) != (texture.width, texture.height) {
        return Err(SnError::Unsupported(
            "KTX2 prototype requires textures at native resolution; max_texture_dimension resizing is not supported"
                .to_owned(),
        ));
    }
    if !texture.width.is_multiple_of(4) || !texture.height.is_multiple_of(4) {
        return Err(SnError::InvalidData {
            resource: "texture dimensions",
            reason: format!(
                "KTX2 prototype requires base dimensions multiple of four; got {}x{}",
                texture.width, texture.height
            ),
        });
    }

    let plan = plan_mip_chain((texture.width, texture.height), texture.mips.len());
    if plan.levels.len() < texture.mips.len() {
        return Err(SnError::InvalidData {
            resource: "texture mip chain",
            reason: "mip chain extends past the natural 1x1 tail".to_owned(),
        });
    }
    if plan.total_pixels > options.limits.max_texture_pixels {
        return Err(SnError::LimitExceeded {
            resource: "texture pixels",
            actual: plan.total_pixels,
            limit: options.limits.max_texture_pixels,
        });
    }
    if plan.total_rgba_bytes > options.limits.max_texture_rgba_bytes {
        return Err(SnError::LimitExceeded {
            resource: "texture RGBA bytes",
            actual: plan.total_rgba_bytes,
            limit: options.limits.max_texture_rgba_bytes,
        });
    }
    Ok(plan.total_rgba_bytes)
}

pub(crate) fn prepare_ktx2_texture(
    texture: &TextureAsset,
    options: &ConversionOptions,
) -> Result<PreparedTexture> {
    validate_ktx2_texture(texture, options)?;
    if options.basis_mode == crate::config::BasisMode::Etc1s {
        return Err(SnError::Unsupported(
            "ETC1S KTX2 requires the global codebook (SGD), which the encoder bindings do not expose; use BasisMode::Uastc"
                .to_owned(),
        ));
    }

    match options.platform {
        Platform::Pc => prepare_pc_ktx2_texture(texture, options),
        Platform::Switch => prepare_switch_ktx2_texture(texture, options),
    }
}

/// Codifica a cadeia completa (fornecida + cauda gerada) em `.basis` UASTC e
/// extrai os slices por nível, conferidos contra o plano de mips.
fn encode_uastc_levels(
    texture: &TextureAsset,
) -> Result<(MipPlan, basis_shim::EncodedBasis, Vec<Vec<u8>>)> {
    let plan = plan_mip_chain((texture.width, texture.height), texture.mips.len());
    let provided = texture.mips.len();
    let tail = generate_mip_tail(&texture.mips[provided - 1], &plan)?;

    let mut encoder_mips = Vec::with_capacity(plan.levels.len());
    for mip in &texture.mips {
        encoder_mips.push(EncoderMip {
            width: mip.width,
            height: mip.height,
            rgba: &mip.data,
        });
    }
    for (index, generated) in tail.iter().enumerate() {
        let (width, height) = plan.levels[provided + index];
        encoder_mips.push(EncoderMip {
            width,
            height,
            rgba: generated,
        });
    }

    let encoded = encode_basis(
        basis_shim::BasisMode::Uastc,
        encoder_color_space(texture.color_space),
        &encoder_mips,
    )
    .map_err(map_encode_error)?;

    let level_slices = extract_uastc_levels(&encoded.bytes, &plan)?;
    let levels: Vec<Vec<u8>> = level_slices.iter().map(|slice| slice.to_vec()).collect();
    Ok((plan, encoded, levels))
}

#[cfg(feature = "pc-ktx2-basis")]
fn prepare_pc_ktx2_texture(
    texture: &TextureAsset,
    options: &ConversionOptions,
) -> Result<PreparedTexture> {
    let (_plan, encoded, levels) = encode_uastc_levels(texture)?;
    let level_refs: Vec<&[u8]> = levels.iter().map(Vec::as_slice).collect();

    let ktx2_bytes = write_ktx2(
        Ktx2Codec::Uastc {
            srgb: texture.color_space == ColorSpace::Srgb,
            has_alpha: encoded.has_alpha,
        },
        texture.width,
        texture.height,
        &level_refs,
    )?;

    // Footprint de GPU pelo levelIndex real: cada nível registrado contribui
    // com os bytes do seu slice (16 por bloco 4x4), no formato do perfil.
    let gpu = gpu_info_from_levels(
        options.gpu_format(),
        texture.width,
        texture.height,
        &level_refs,
    )?;

    Ok(PreparedTexture {
        name: texture.name.clone(),
        width: texture.width,
        height: texture.height,
        color_space: texture.color_space,
        payload: PreparedTexturePayload::Ktx2Basis(ktx2_bytes),
        gpu: Some(gpu),
    })
}

#[cfg(not(feature = "pc-ktx2-basis"))]
fn prepare_pc_ktx2_texture(
    _texture: &TextureAsset,
    _options: &ConversionOptions,
) -> Result<PreparedTexture> {
    Err(SnError::Unsupported(
        "PC KTX2/Basis encoding requires the pc-ktx2-basis feature".to_owned(),
    ))
}

#[cfg(feature = "switch-ktx2-astc")]
fn prepare_switch_ktx2_texture(
    texture: &TextureAsset,
    options: &ConversionOptions,
) -> Result<PreparedTexture> {
    let (plan, encoded, _uastc_levels) = encode_uastc_levels(texture)?;

    // UASTC é o intermediário de transcodificação: cada nível vira blocos
    // ASTC 4x4 (16 bytes por bloco), conferidos contra o plano de mips.
    let astc_levels =
        transcode_uastc_to_astc4x4(&encoded.bytes, plan.levels.len()).map_err(map_astc_error)?;
    for (index, level) in astc_levels.iter().enumerate() {
        let (width, height) = plan.levels[index];
        let (blocks_x, blocks_y) = block_dims(width, height);
        let expected = blocks_x as usize * blocks_y as usize * ASTC_BLOCK_BYTES;
        if level.len() != expected {
            return Err(SnError::InvalidData {
                resource: "ASTC mip level",
                reason: format!(
                    "mip {index} has {} bytes, expected {expected} for {width}x{height}",
                    level.len()
                ),
            });
        }
    }
    let level_refs: Vec<&[u8]> = astc_levels.iter().map(Vec::as_slice).collect();

    let ktx2_bytes = write_ktx2(
        Ktx2Codec::Astc4x4 {
            srgb: texture.color_space == ColorSpace::Srgb,
        },
        texture.width,
        texture.height,
        &level_refs,
    )?;

    // No perfil Switch o footprint de GPU é ASTC 4x4; a soma dos níveis
    // registrados no levelIndex é conferida contra a cadeia mip planejada.
    let gpu = gpu_info_from_levels(
        options.gpu_format(),
        texture.width,
        texture.height,
        &level_refs,
    )?;

    Ok(PreparedTexture {
        name: texture.name.clone(),
        width: texture.width,
        height: texture.height,
        color_space: texture.color_space,
        payload: PreparedTexturePayload::Ktx2Astc(ktx2_bytes),
        gpu: Some(gpu),
    })
}

#[cfg(not(feature = "switch-ktx2-astc"))]
fn prepare_switch_ktx2_texture(
    _texture: &TextureAsset,
    _options: &ConversionOptions,
) -> Result<PreparedTexture> {
    Err(SnError::Unsupported(
        "Switch KTX2/ASTC encoding requires the switch-ktx2-astc feature; use the portable PNG profile"
            .to_owned(),
    ))
}

fn map_astc_error(error: AstcTranscodeError) -> SnError {
    match error {
        AstcTranscodeError::EmptyMipChain => SnError::InvalidData {
            resource: "texture mip chain",
            reason: "a base mip level is required".to_owned(),
        },
        AstcTranscodeError::NotUastc(_) => {
            SnError::Unsupported("ASTC 4x4 transcoding requires a UASTC .basis file".to_owned())
        }
        AstcTranscodeError::PrepareFailed => SnError::Unsupported(
            "basis transcoding failed to initialize on the encoded file".to_owned(),
        ),
        AstcTranscodeError::LevelCountMismatch { expected, actual } => SnError::InvalidData {
            resource: "texture mip chain",
            reason: format!("transcoder reports {actual} levels, planned {expected}"),
        },
        AstcTranscodeError::LevelNotFound { level } => SnError::InvalidData {
            resource: "texture mip chain",
            reason: format!("mip {level} is missing from the encoded file"),
        },
        AstcTranscodeError::UnexpectedSize {
            level,
            expected,
            actual,
            ..
        } => SnError::InvalidData {
            resource: "ASTC mip level",
            reason: format!("mip {level} has {actual} bytes, expected {expected}"),
        },
        AstcTranscodeError::Failed { level, .. } => {
            SnError::Unsupported(format!("ASTC 4x4 transcode of mip {level} failed"))
        }
    }
}

/// Monta o [`GpuTextureInfo`] a partir dos níveis reais do levelIndex: cada
/// fatia UASTC contribui com seus próprios bytes (16 por bloco 4x4) e a soma é
/// conferida contra a cadeia mip planejada para as dimensões da textura.
fn gpu_info_from_levels(
    format: GpuTextureFormat,
    width: u32,
    height: u32,
    level_slices: &[&[u8]],
) -> Result<GpuTextureInfo> {
    let mip_count = u32::try_from(level_slices.len()).map_err(|_| SnError::SizeOverflow {
        resource: "gpu texture mip_count",
    })?;
    let mut byte_len = 0_u64;
    for slice in level_slices {
        byte_len = byte_len
            .checked_add(slice.len() as u64)
            .ok_or(SnError::SizeOverflow {
                resource: "gpu mip chain bytes",
            })?;
    }
    let expected = mip_chain_bytes(format, width, height, mip_count)?;
    if byte_len != expected {
        return Err(SnError::InvalidData {
            resource: "gpu texture footprint",
            reason: format!("levelIndex sums to {byte_len} bytes, planned chain is {expected}"),
        });
    }
    Ok(GpuTextureInfo {
        format,
        mip_count,
        byte_len,
    })
}

fn encoder_color_space(color_space: ColorSpace) -> EncoderColorSpace {
    match color_space {
        ColorSpace::Srgb => EncoderColorSpace::Srgb,
        ColorSpace::Linear => EncoderColorSpace::Linear,
    }
}

fn map_encode_error(error: EncodeError) -> SnError {
    match error {
        EncodeError::InvalidMip {
            level,
            width,
            height,
            expected,
            actual,
        } => SnError::InvalidData {
            resource: "texture data",
            reason: format!(
                "mip {level} has {actual} bytes, expected {expected} for {width}x{height}"
            ),
        },
        EncodeError::InvalidMipChain { level, .. } => SnError::InvalidData {
            resource: "texture mip chain",
            reason: format!("mip {level} breaks the halving chain"),
        },
        EncodeError::EmptyMipChain => SnError::InvalidData {
            resource: "texture mip chain",
            reason: "a base mip level is required".to_owned(),
        },
        other => SnError::Unsupported(format!("basis encoding failed: {other}")),
    }
}

/// Planeja a cadeia completa até 1x1 e totaliza pixels/bytes sem alocar dados.
pub(crate) fn plan_mip_chain(base: (u32, u32), provided: usize) -> MipPlan {
    let mut levels = vec![base];
    let (mut width, mut height) = base;
    while (width, height) != (1, 1) {
        width = (width / 2).max(1);
        height = (height / 2).max(1);
        levels.push((width, height));
    }

    let mut total_pixels = 0_u64;
    let mut total_rgba_bytes = 0_u64;
    for &(width, height) in &levels {
        let pixels = u64::from(width) * u64::from(height);
        total_pixels += pixels;
        total_rgba_bytes += pixels * 4;
    }

    MipPlan {
        levels,
        provided_levels: provided,
        total_pixels,
        total_rgba_bytes,
    }
}

/// Gera a cauda ausente da cadeia a partir do último mip fornecido,
/// reutilizando o reamostrador nearest-neighbor do caminho PNG.
fn generate_mip_tail(last_provided: &MipLevel, plan: &MipPlan) -> Result<Vec<Vec<u8>>> {
    let mut tail: Vec<Vec<u8>> = Vec::new();
    let (mut previous_width, mut previous_height) = (last_provided.width, last_provided.height);
    for &(width, height) in plan.levels[plan.provided_levels..].iter() {
        let source: &[u8] = match tail.last() {
            Some(previous) => previous.as_slice(),
            None => last_provided.data.as_slice(),
        };
        let resized = resize_rgba(source, previous_width, previous_height, width, height)?;
        tail.push(resized.into_owned());
        previous_width = width;
        previous_height = height;
    }
    Ok(tail)
}

// ---- Parser do contêiner .basis ----

#[derive(Clone, Copy, Debug)]
struct SliceDesc {
    image_index: u32,
    level_index: u8,
    flags: u8,
    orig_width: u16,
    orig_height: u16,
    num_blocks_x: u16,
    num_blocks_y: u16,
    file_ofs: u32,
    file_size: u32,
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| invalid_basis("truncated header"))?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u24(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset + 3)
        .ok_or_else(|| invalid_basis("truncated header"))?;
    Ok(u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_basis("truncated header"))?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn invalid_basis(reason: &str) -> SnError {
    SnError::InvalidData {
        resource: "basis file",
        reason: reason.to_owned(),
    }
}

/// Extrai os dados UASTC por nível do arquivo `.basis` produzido pelo encoder.
fn extract_uastc_levels<'a>(basis: &'a [u8], plan: &MipPlan) -> Result<Vec<&'a [u8]>> {
    if basis.len() < BASIS_HEADER_SIZE {
        return Err(invalid_basis("file is shorter than the fixed header"));
    }
    if read_u16(basis, 0)? != BASIS_SIGNATURE {
        return Err(invalid_basis("bad file signature"));
    }
    if read_u16(basis, 2)? < BASIS_MIN_VERSION {
        return Err(invalid_basis("unsupported file version"));
    }
    if read_u16(basis, 4)? as usize != BASIS_HEADER_SIZE {
        return Err(invalid_basis("unexpected header size"));
    }
    let tex_format = basis[20];
    if tex_format != BASIS_FORMAT_UASTC || read_u16(basis, 21)? & BASIS_FLAG_ETC1S != 0 {
        return Err(SnError::Unsupported(
            "only UASTC .basis files are supported by the KTX2 prototype".to_owned(),
        ));
    }
    let total_slices = read_u24(basis, 14)?;
    let total_images = read_u24(basis, 17)?;
    if total_images == 0 {
        return Err(invalid_basis("file contains no images"));
    }
    let slice_desc_ofs = read_u32(basis, 65)? as usize;
    let desc_table_end = slice_desc_ofs
        .checked_add(total_slices as usize * SLICE_DESC_SIZE)
        .ok_or_else(|| invalid_basis("slice descriptor table overflows"))?;
    if desc_table_end > basis.len() {
        return Err(invalid_basis("slice descriptor table is truncated"));
    }

    let mut slices = Vec::new();
    for index in 0..total_slices {
        let base = slice_desc_ofs + (index as usize) * SLICE_DESC_SIZE;
        let desc = SliceDesc {
            image_index: read_u24(basis, base)?,
            level_index: basis[base + 3],
            flags: basis[base + 4],
            orig_width: read_u16(basis, base + 5)?,
            orig_height: read_u16(basis, base + 7)?,
            num_blocks_x: read_u16(basis, base + 9)?,
            num_blocks_y: read_u16(basis, base + 11)?,
            file_ofs: read_u32(basis, base + 13)?,
            file_size: read_u32(basis, base + 17)?,
        };
        if desc.image_index == 0 {
            slices.push(desc);
        }
    }
    if slices.len() != plan.levels.len() {
        return Err(invalid_basis(
            "image 0 does not cover the planned mip chain",
        ));
    }
    slices.sort_by_key(|desc| desc.level_index);

    let mut levels = Vec::with_capacity(slices.len());
    for (index, desc) in slices.iter().enumerate() {
        if desc.level_index as usize != index {
            return Err(invalid_basis("mip levels are not contiguous from zero"));
        }
        if desc.flags & 1 != 0 {
            return Err(invalid_basis(
                "unexpected separate alpha slice in a UASTC file",
            ));
        }
        let (width, height) = plan.levels[index];
        let (blocks_x, blocks_y) = block_dims(width, height);
        if u32::from(desc.orig_width) != width || u32::from(desc.orig_height) != height {
            return Err(invalid_basis("slice dimensions do not match the mip plan"));
        }
        if u32::from(desc.num_blocks_x) != blocks_x || u32::from(desc.num_blocks_y) != blocks_y {
            return Err(invalid_basis(
                "slice block dimensions do not match the mip plan",
            ));
        }
        let expected_size = blocks_x as usize * blocks_y as usize * UASTC_BLOCK_BYTES;
        if desc.file_size as usize != expected_size {
            return Err(invalid_basis("slice byte length does not match its blocks"));
        }
        let start = desc.file_ofs as usize;
        let end = start
            .checked_add(desc.file_size as usize)
            .ok_or_else(|| invalid_basis("slice range overflows"))?;
        let bytes = basis
            .get(start..end)
            .ok_or_else(|| invalid_basis("slice data lies outside the file"))?;
        levels.push(bytes);
    }
    Ok(levels)
}

/// Dimensões em blocos 4x4 com divisão de teto; níveis de cauda 2x2/1x1
/// ocupam um bloco, sem exigir múltiplos de 4 nos níveis finais.
fn block_dims(width: u32, height: u32) -> (u32, u32) {
    (width.div_ceil(4), height.div_ceil(4))
}

// ---- Writer do contêiner KTX2 (UASTC, sem supercompressão) ----

#[derive(Clone, Copy)]
struct LevelIndexEntry {
    byte_offset: u64,
    byte_length: u64,
    uncompressed_byte_length: u64,
}

/// DFD UASTC4x4 de 44 bytes, espelhando `g_ktx2_uastc_{alpha,nonalpha}_dfd`
/// de `basisu_comp.cpp`: totalSize=44, version=2, blockSize=40, model=166
/// (UASTC), primaries=1 (BT709), transfer patched, blockDim 3,3,0,0,
/// bytesPlane0=16, sample0 bitLength=127, channel RGBA/RGB.
fn uastc_dfd(has_alpha: bool, srgb: bool) -> [u8; 44] {
    let mut dfd = [0_u8; 44];
    dfd[0..4].copy_from_slice(&44_u32.to_le_bytes());
    dfd[8..10].copy_from_slice(&2_u16.to_le_bytes());
    dfd[10..12].copy_from_slice(&40_u16.to_le_bytes());
    dfd[12] = KHR_DF_MODEL_UASTC;
    dfd[13] = KHR_DF_PRIMARIES_BT709;
    dfd[14] = if srgb {
        KHR_DF_TRANSFER_SRGB
    } else {
        KHR_DF_TRANSFER_LINEAR
    };
    dfd[16] = 3;
    dfd[17] = 3;
    dfd[20] = UASTC_BLOCK_BYTES as u8;
    dfd[30] = 127;
    dfd[31] = if has_alpha {
        KHR_DF_CHANNEL_UASTC_RGBA
    } else {
        KHR_DF_CHANNEL_UASTC_RGB
    };
    dfd[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    dfd
}

/// DFD ASTC 4x4 de 44 bytes, no mesmo layout do DFD UASTC: totalSize=44,
/// version=2, blockSize=40, model=162 (KHR_DF_MODEL_ASTC), primaries=1
/// (BT709), transfer conforme o espaço de cor, blockDim 3,3,0,0 (4x4),
/// bytesPlane0=16, sample0 bitLength=127, channel 0
/// (KHR_DF_CHANNEL_ASTC_4x4_RGBA — alfa incluso no bloco).
fn astc_dfd(srgb: bool) -> [u8; 44] {
    let mut dfd = [0_u8; 44];
    dfd[0..4].copy_from_slice(&44_u32.to_le_bytes());
    dfd[8..10].copy_from_slice(&2_u16.to_le_bytes());
    dfd[10..12].copy_from_slice(&40_u16.to_le_bytes());
    dfd[12] = KHR_DF_MODEL_ASTC;
    dfd[13] = KHR_DF_PRIMARIES_BT709;
    dfd[14] = if srgb {
        KHR_DF_TRANSFER_SRGB
    } else {
        KHR_DF_TRANSFER_LINEAR
    };
    dfd[16] = 3;
    dfd[17] = 3;
    dfd[20] = ASTC_BLOCK_BYTES as u8;
    dfd[30] = 127;
    dfd[31] = KHR_DF_CHANNEL_ASTC_4X4_RGBA;
    dfd[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    dfd
}

/// Codec do payload KTX2: UASTC supercompresso (vkFormat indefinido) ou
/// ASTC 4x4 nativo (vkFormat ASTC, supercompressionScheme = NONE).
#[derive(Clone, Copy)]
enum Ktx2Codec {
    Uastc { srgb: bool, has_alpha: bool },
    Astc4x4 { srgb: bool },
}

fn write_ktx2(codec: Ktx2Codec, width: u32, height: u32, levels: &[&[u8]]) -> Result<Vec<u8>> {
    let (vk_format, dfd, writer_tag) = match codec {
        Ktx2Codec::Uastc { srgb, has_alpha } => {
            (0_u32, uastc_dfd(has_alpha, srgb), "pc-ktx2-basis")
        }
        Ktx2Codec::Astc4x4 { srgb } => (
            if srgb {
                VK_FORMAT_ASTC_4X4_SRGB_BLOCK
            } else {
                VK_FORMAT_ASTC_4X4_UNORM_BLOCK
            },
            astc_dfd(srgb),
            "switch-ktx2-astc",
        ),
    };
    let level_count = levels.len();
    let dfd_offset = (KTX2_HEADER_SIZE + KTX2_LEVEL_INDEX_SIZE * level_count) as u32;

    let writer_key = b"KTXwriter\0";
    let writer_value = format!(
        "scarlet-nexus-assets {} {writer_tag}\0",
        env!("CARGO_PKG_VERSION")
    )
    .into_bytes();
    let kvd_entry_bytes = 4 + writer_key.len() + writer_value.len();
    let kvd_length = kvd_entry_bytes.div_ceil(4) * 4;
    let kvd_offset = dfd_offset + dfd.len() as u32;
    let data_offset =
        (kvd_offset as usize + kvd_length).div_ceil(KTX2_MIP_ALIGNMENT) * KTX2_MIP_ALIGNMENT;

    let total = data_offset + levels.iter().map(|level| level.len()).sum::<usize>();
    let mut out = Vec::new();
    out.try_reserve_exact(total)
        .map_err(|_| SnError::AllocationFailed {
            resource: "KTX2 container",
        })?;

    out.extend_from_slice(&KTX2_MAGIC);
    out.extend_from_slice(&vk_format.to_le_bytes()); // 0 = VK_FORMAT_UNDEFINED (UASTC supercompresso)
    out.extend_from_slice(&1_u32.to_le_bytes()); // typeSize
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes()); // pixelDepth
    out.extend_from_slice(&0_u32.to_le_bytes()); // layerCount
    out.extend_from_slice(&1_u32.to_le_bytes()); // faceCount
    out.extend_from_slice(&(level_count as u32).to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes()); // supercompressionScheme = NONE
    out.extend_from_slice(&dfd_offset.to_le_bytes());
    out.extend_from_slice(&(dfd.len() as u32).to_le_bytes());
    out.extend_from_slice(&kvd_offset.to_le_bytes());
    out.extend_from_slice(&(kvd_length as u32).to_le_bytes());
    out.extend_from_slice(&0_u64.to_le_bytes()); // sgdByteOffset
    out.extend_from_slice(&0_u64.to_le_bytes()); // sgdByteLength

    let index_start = out.len();
    for _ in 0..level_count {
        out.extend_from_slice(&[0_u8; KTX2_LEVEL_INDEX_SIZE]);
    }

    debug_assert_eq!(out.len(), dfd_offset as usize);
    out.extend_from_slice(&dfd);

    debug_assert_eq!(out.len(), kvd_offset as usize);
    out.extend_from_slice(&(kvd_entry_bytes as u32).to_le_bytes());
    out.extend_from_slice(writer_key);
    out.extend_from_slice(&writer_value);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    while out.len() % KTX2_MIP_ALIGNMENT != 0 {
        out.push(0);
    }

    // Dados de mip do menor para o maior, espelhando o writer upstream;
    // os offsets reais são registrados no levelIndex ao final.
    for (mip_index, level) in levels.iter().enumerate().rev() {
        let entry = LevelIndexEntry {
            byte_offset: out.len() as u64,
            byte_length: level.len() as u64,
            uncompressed_byte_length: level.len() as u64,
        };
        out.extend_from_slice(level);

        let at = index_start + mip_index * KTX2_LEVEL_INDEX_SIZE;
        out[at..at + 8].copy_from_slice(&entry.byte_offset.to_le_bytes());
        out[at + 8..at + 16].copy_from_slice(&entry.byte_length.to_le_bytes());
        out[at + 16..at + 24].copy_from_slice(&entry.uncompressed_byte_length.to_le_bytes());
    }

    debug_assert_eq!(out.len(), total);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{BasisMode, Limits},
        model::TextureFormat,
        platform::Platform,
    };

    fn texture(width: u32, height: u32, mips: Vec<MipLevel>) -> TextureAsset {
        TextureAsset {
            name: "ktx2_proto".to_owned(),
            width,
            height,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            mips,
        }
    }

    fn solid_mip(width: u32, height: u32) -> MipLevel {
        MipLevel {
            width,
            height,
            data: [255u8, 0, 0, 255].repeat((width * height) as usize),
        }
    }

    fn uastc_options() -> ConversionOptions {
        ConversionOptions {
            texture_target: crate::config::TextureTarget::Ktx2Basis,
            basis_mode: BasisMode::Uastc,
            ..ConversionOptions::default()
        }
    }

    #[test]
    fn plans_full_chain_and_totals() {
        let plan = plan_mip_chain((4, 4), 1);
        assert_eq!(plan.levels, vec![(4, 4), (2, 2), (1, 1)]);
        assert_eq!(plan.provided_levels, 1);
        assert_eq!(plan.total_pixels, 21);
        assert_eq!(plan.total_rgba_bytes, 84);

        let plan = plan_mip_chain((8, 4), 4);
        assert_eq!(plan.levels, vec![(8, 4), (4, 2), (2, 1), (1, 1)]);
        assert_eq!(plan.provided_levels, 4);
        assert_eq!(plan.total_pixels, 32 + 8 + 2 + 1);
    }

    #[test]
    fn validates_base_multiple_of_four() {
        for (width, height) in [(5, 4), (4, 5), (6, 8), (4, 2)] {
            let texture = texture(width, height, vec![solid_mip(width, height)]);
            let error = validate_ktx2_texture(&texture, &uastc_options())
                .expect_err("base dimensions must be multiples of four");
            assert!(matches!(
                error,
                SnError::InvalidData {
                    resource: "texture dimensions",
                    ..
                }
            ));
        }
    }

    #[test]
    fn rejects_dimension_resize() {
        let texture = texture(8, 8, vec![solid_mip(8, 8)]);
        let options = ConversionOptions {
            max_texture_dimension: Some(4),
            ..uastc_options()
        };
        let error = validate_ktx2_texture(&texture, &options)
            .expect_err("resized base must be rejected in the prototype");
        assert!(matches!(error, SnError::Unsupported(_)));
    }

    #[test]
    fn rejects_chain_past_natural_tail() {
        // validate_texture aceita 1x1 repetido; o plano KTX2 rejeita.
        let texture = texture(
            4,
            4,
            vec![
                solid_mip(4, 4),
                solid_mip(2, 2),
                solid_mip(1, 1),
                solid_mip(1, 1),
            ],
        );
        let error = validate_ktx2_texture(&texture, &uastc_options())
            .expect_err("mips beyond 1x1 must be rejected");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "texture mip chain",
                ..
            }
        ));
    }

    #[test]
    fn enforces_budgets_over_generated_tail() {
        // 4x4 fornecido (16 pixels) cabe em 20 pixels; a cadeia completa
        // (21 pixels) estoura o orçamento antes de qualquer alocação.
        let texture = texture(4, 4, vec![solid_mip(4, 4)]);
        let options = ConversionOptions {
            limits: Limits {
                max_texture_pixels: 20,
                max_texture_rgba_bytes: 256,
                ..Limits::default()
            },
            ..uastc_options()
        };
        let error = validate_ktx2_texture(&texture, &options)
            .expect_err("generated tail must count toward the pixel budget");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "texture pixels",
                actual: 21,
                limit: 20,
            }
        ));
    }

    #[cfg(feature = "pc-ktx2-basis")]
    #[test]
    fn prepares_uastc_ktx2_with_gpu_footprint_from_level_index() {
        let texture = texture(4, 4, vec![solid_mip(4, 4)]);
        let prepared =
            prepare_ktx2_texture(&texture, &uastc_options()).expect("4x4 UASTC must convert");
        assert!(matches!(
            prepared.payload,
            PreparedTexturePayload::Ktx2Basis(_)
        ));
        assert_eq!((prepared.width, prepared.height), (4, 4));
        // PC + KTX2 -> UASTC; 4x4, 2x2 e 1x1 = 1 bloco de 16 bytes cada.
        let gpu = prepared.gpu.expect("KTX2 must declare its GPU footprint");
        assert_eq!(gpu.format, GpuTextureFormat::Uastc);
        assert_eq!(gpu.mip_count, 3);
        assert_eq!(gpu.byte_len, 3 * UASTC_BLOCK_BYTES as u64);

        let bytes = prepared.payload.encoded_bytes();
        assert_eq!(&bytes[..12], &KTX2_MAGIC);

        let reader = ktx2::Reader::new(bytes).expect("independent parser must accept the KTX2");
        let header = reader.header();
        assert_eq!(header.pixel_width, 4);
        assert_eq!(header.pixel_height, 4);
        assert_eq!(header.level_count, 3);
        assert_eq!(header.face_count, 1);
        assert!(header.format.is_none());
        assert!(header.supercompression_scheme.is_none());
        assert_eq!(header.type_size, 1);
        assert_eq!(reader.color_model(), Some(ktx2::ColorModel::UASTC));
        assert_eq!(
            reader.transfer_function(),
            Some(ktx2::TransferFunction::SRGB)
        );

        let levels: Vec<_> = reader.levels().collect();
        assert_eq!(levels.len(), 3);
        let expected = [(1, 16), (1, 16), (1, 16)];
        for (level, (blocks, bytes)) in levels.iter().zip(expected) {
            assert_eq!(level.data.len(), blocks * bytes);
            assert_eq!(level.uncompressed_byte_length, level.data.len() as u64);
        }
    }

    #[cfg(feature = "pc-ktx2-basis")]
    #[test]
    fn prepares_8x4_chain_with_tail_from_base_only() {
        let texture = texture(8, 4, vec![solid_mip(8, 4)]);
        let prepared = prepare_ktx2_texture(&texture, &uastc_options())
            .expect("8x4 UASTC must convert with generated tail");
        let reader = ktx2::Reader::new(prepared.payload.encoded_bytes())
            .expect("independent parser must accept the KTX2");
        assert_eq!(reader.header().level_count, 4);
        let levels: Vec<_> = reader.levels().collect();
        // 8x4 = 2x1 blocos; 4x2, 2x1 e 1x1 = 1 bloco cada.
        let expected_blocks = [2, 1, 1, 1];
        for (level, blocks) in levels.iter().zip(expected_blocks) {
            assert_eq!(level.data.len(), blocks * UASTC_BLOCK_BYTES);
        }
        let gpu = prepared.gpu.expect("KTX2 must declare its GPU footprint");
        assert_eq!(gpu.format, GpuTextureFormat::Uastc);
        assert_eq!(gpu.mip_count, 4);
        assert_eq!(gpu.byte_len, 5 * UASTC_BLOCK_BYTES as u64);
    }

    #[cfg(feature = "switch-ktx2-astc")]
    #[test]
    fn switch_profile_transcodes_astc4x4_with_header_and_gpu_footprint() {
        let mut texture = texture(4, 4, vec![solid_mip(4, 4)]);
        texture.color_space = ColorSpace::Srgb;
        let options = ConversionOptions {
            platform: Platform::Switch,
            ..uastc_options()
        };
        let prepared =
            prepare_ktx2_texture(&texture, &options).expect("4x4 Switch ASTC must convert");
        assert!(matches!(
            prepared.payload,
            PreparedTexturePayload::Ktx2Astc(_)
        ));

        // GPU do perfil Switch: ASTC 4x4, soma dos níveis conferida contra a cadeia.
        let gpu = prepared.gpu.expect("KTX2 must declare its GPU footprint");
        assert_eq!(gpu.format, GpuTextureFormat::Astc4x4);
        assert_eq!(gpu.mip_count, 3);
        assert_eq!(gpu.byte_len, 3 * ASTC_BLOCK_BYTES as u64);
        assert_eq!(
            gpu.byte_len,
            mip_chain_bytes(GpuTextureFormat::Astc4x4, 4, 4, 3).expect("planned chain")
        );

        let bytes = prepared.payload.encoded_bytes();
        assert_eq!(&bytes[..12], &KTX2_MAGIC);
        let reader = ktx2::Reader::new(bytes).expect("independent parser must accept the KTX2");
        let header = reader.header();
        assert_eq!(header.pixel_width, 4);
        assert_eq!(header.pixel_height, 4);
        assert_eq!(header.level_count, 3);
        assert_eq!(header.face_count, 1);
        assert_eq!(header.type_size, 1);
        assert_eq!(
            header.format,
            Some(ktx2::Format::ASTC_4x4_SRGB_BLOCK),
            "ASTC sRGB deve usar vkFormat 158"
        );
        assert!(header.supercompression_scheme.is_none());
        assert_eq!(reader.color_model(), Some(ktx2::ColorModel::ASTC));
        assert_eq!(
            reader.transfer_function(),
            Some(ktx2::TransferFunction::SRGB)
        );

        let levels: Vec<_> = reader.levels().collect();
        assert_eq!(levels.len(), 3);
        for level in &levels {
            assert_eq!(level.data.len(), ASTC_BLOCK_BYTES);
            assert_eq!(level.uncompressed_byte_length, level.data.len() as u64);
        }
    }

    #[cfg(feature = "switch-ktx2-astc")]
    #[test]
    fn switch_astc4x4_pot_8x8_full_chain() {
        let texture = texture(8, 8, vec![solid_mip(8, 8)]);
        let options = ConversionOptions {
            platform: Platform::Switch,
            ..uastc_options()
        };
        let prepared =
            prepare_ktx2_texture(&texture, &options).expect("8x8 Switch ASTC must convert");
        let reader = ktx2::Reader::new(prepared.payload.encoded_bytes())
            .expect("independent parser must accept the KTX2");
        assert_eq!(reader.header().level_count, 4); // 8x8, 4x4, 2x2, 1x1
        let expected_blocks = [4, 1, 1, 1]; // blocos 4x4 por nível
        for (level, blocks) in reader.levels().zip(expected_blocks) {
            assert_eq!(level.data.len(), blocks * ASTC_BLOCK_BYTES);
        }
        let gpu = prepared.gpu.expect("KTX2 must declare its GPU footprint");
        assert_eq!(gpu.format, GpuTextureFormat::Astc4x4);
        assert_eq!((gpu.mip_count, gpu.byte_len), (4, 7 * 16));
    }

    #[cfg(feature = "switch-ktx2-astc")]
    #[test]
    fn switch_astc4x4_npot_12x4_block_ceil() {
        // 12x4 não é potência de dois, mas é múltiplo de 4 (exigido pelo protótipo).
        let texture = texture(12, 4, vec![solid_mip(12, 4)]);
        let options = ConversionOptions {
            platform: Platform::Switch,
            ..uastc_options()
        };
        let prepared =
            prepare_ktx2_texture(&texture, &options).expect("12x4 Switch ASTC must convert");
        let reader = ktx2::Reader::new(prepared.payload.encoded_bytes())
            .expect("independent parser must accept the KTX2");
        assert_eq!(reader.header().level_count, 4); // 12x4, 6x2, 3x1, 1x1
        let expected_blocks = [3, 2, 1, 1]; // ceil(12/4)x1, ceil(6/4)x1, ...
        for (level, blocks) in reader.levels().zip(expected_blocks) {
            assert_eq!(level.data.len(), blocks * ASTC_BLOCK_BYTES);
        }
        let gpu = prepared.gpu.expect("KTX2 must declare its GPU footprint");
        assert_eq!(gpu.format, GpuTextureFormat::Astc4x4);
        assert_eq!(gpu.byte_len, 7 * 16);
        assert_eq!(
            gpu.byte_len,
            mip_chain_bytes(GpuTextureFormat::Astc4x4, 12, 4, 4).expect("planned chain")
        );
    }

    #[cfg(feature = "switch-ktx2-astc")]
    #[test]
    fn switch_astc4x4_linear_uses_unorm_vk_format() {
        let mut texture = texture(4, 4, vec![solid_mip(4, 4)]);
        texture.color_space = ColorSpace::Linear;
        let options = ConversionOptions {
            platform: Platform::Switch,
            ..uastc_options()
        };
        let prepared =
            prepare_ktx2_texture(&texture, &options).expect("linear Switch ASTC must convert");
        let reader = ktx2::Reader::new(prepared.payload.encoded_bytes())
            .expect("independent parser must accept the KTX2");
        assert_eq!(
            reader.header().format,
            Some(ktx2::Format::ASTC_4x4_UNORM_BLOCK),
            "ASTC linear deve usar vkFormat 157"
        );
        assert_eq!(reader.color_model(), Some(ktx2::ColorModel::ASTC));
        assert_eq!(
            reader.transfer_function(),
            Some(ktx2::TransferFunction::Linear)
        );
        let gpu = prepared.gpu.expect("KTX2 must declare its GPU footprint");
        assert_eq!(gpu.format, GpuTextureFormat::Astc4x4);
    }

    #[test]
    fn rejects_etc1s_before_encoding() {
        let texture = texture(4, 4, vec![solid_mip(4, 4)]);
        let options = ConversionOptions {
            basis_mode: BasisMode::Etc1s,
            ..uastc_options()
        };
        let error = prepare_ktx2_texture(&texture, &options)
            .expect_err("ETC1S requires SGD support not exposed by the bindings");
        assert!(matches!(error, SnError::Unsupported(_)));
    }

    #[test]
    fn rejects_truncated_basis_files() {
        let error = extract_uastc_levels(&[], &plan_mip_chain((4, 4), 1))
            .expect_err("empty basis must fail");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "basis file",
                ..
            }
        ));
    }

    #[test]
    fn rejects_bad_basis_signature() {
        let mut header = vec![0_u8; BASIS_HEADER_SIZE];
        header[0..2].copy_from_slice(&0x1234_u16.to_le_bytes());
        let error = extract_uastc_levels(&header, &plan_mip_chain((4, 4), 1))
            .expect_err("bad signature must fail");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "basis file",
                reason
            } if reason.contains("signature")
        ));
    }
}
