use std::{borrow::Cow, io::Cursor};

use crate::{
    config::{ConversionOptions, TextureTarget},
    error::{Result, SnError},
    model::{GpuTextureInfo, PreparedTexture, PreparedTexturePayload, TextureAsset, TextureFormat},
};

pub fn prepare_texture(
    texture: &TextureAsset,
    options: &ConversionOptions,
) -> Result<PreparedTexture> {
    options.validate()?;
    ensure_supported_target(options)?;
    prevalidate_texture(texture, options)?;
    prepare_dispatched_texture(texture, options)
}

pub(crate) fn ensure_supported_target(options: &ConversionOptions) -> Result<()> {
    if options.texture_target == TextureTarget::Ktx2Basis {
        #[cfg(any(feature = "pc-ktx2-basis", feature = "switch-ktx2-astc"))]
        return Ok(());
        #[cfg(not(any(feature = "pc-ktx2-basis", feature = "switch-ktx2-astc")))]
        return Err(SnError::Unsupported(
            "KTX2 encoding requires the pc-ktx2-basis (PC/UASTC) or switch-ktx2-astc (Switch/ASTC) feature; use the portable PNG profile"
                .to_owned(),
        ));
    }
    Ok(())
}

/// Validação prévia por textura, antes de qualquer alocação/encoding: no
/// protótipo KTX2 os orçamentos incluem a cauda de mips gerada.
pub(crate) fn prevalidate_texture(
    texture: &TextureAsset,
    options: &ConversionOptions,
) -> Result<u64> {
    #[cfg(any(feature = "pc-ktx2-basis", feature = "switch-ktx2-astc"))]
    if options.texture_target == TextureTarget::Ktx2Basis {
        return crate::convert::ktx2::validate_ktx2_texture(texture, options);
    }
    validate_texture(texture, options)
}

/// Despacho de encoding por alvo; presupõe `prevalidate_texture` aprovado.
/// O alvo KTX2 despacha por plataforma dentro de `ktx2::prepare_ktx2_texture`
/// (PC→UASTC com `pc-ktx2-basis`, Switch→ASTC 4x4 com `switch-ktx2-astc`).
pub(crate) fn prepare_dispatched_texture(
    texture: &TextureAsset,
    options: &ConversionOptions,
) -> Result<PreparedTexture> {
    #[cfg(any(feature = "pc-ktx2-basis", feature = "switch-ktx2-astc"))]
    if options.texture_target == TextureTarget::Ktx2Basis {
        return crate::convert::ktx2::prepare_ktx2_texture(texture, options);
    }
    prepare_validated_texture(texture, options)
}

pub(crate) fn prepare_validated_texture(
    texture: &TextureAsset,
    options: &ConversionOptions,
) -> Result<PreparedTexture> {
    let base = &texture.mips[0];
    let (width, height) =
        fit_dimensions(texture.width, texture.height, options.max_texture_dimension);
    let rgba = resize_rgba(&base.data, texture.width, texture.height, width, height)?;
    let png_bytes = encode_png(rgba.as_ref(), width, height)?;
    // PNG é decodificado para RGBA8 na GPU: cadeia mip completa até 1x1 nas dimensões de saída.
    let gpu = GpuTextureInfo::full_chain(options.gpu_format(), width, height)?;

    Ok(PreparedTexture {
        name: texture.name.clone(),
        width,
        height,
        color_space: texture.color_space,
        payload: PreparedTexturePayload::Png(png_bytes),
        gpu: Some(gpu),
    })
}

pub(crate) fn validate_texture(texture: &TextureAsset, options: &ConversionOptions) -> Result<u64> {
    if texture.width == 0 || texture.height == 0 {
        return Err(SnError::InvalidData {
            resource: "texture dimensions",
            reason: "width and height must be greater than zero".to_owned(),
        });
    }
    if texture.format != TextureFormat::Rgba8 {
        return Err(SnError::Unsupported(
            "only RGBA8 textures are implemented".to_owned(),
        ));
    }
    let base = texture.mips.first().ok_or_else(|| SnError::InvalidData {
        resource: "texture mip chain",
        reason: "a base mip level is required".to_owned(),
    })?;
    if base.width != texture.width || base.height != texture.height {
        return Err(SnError::InvalidData {
            resource: "texture mip chain",
            reason: "base mip dimensions do not match the texture".to_owned(),
        });
    }

    let mut total_pixels = 0_u64;
    let mut total_bytes = 0_u64;
    let mut previous_width = texture.width;
    let mut previous_height = texture.height;
    for (index, mip) in texture.mips.iter().enumerate() {
        if index > 0 {
            let expected_width = (previous_width / 2).max(1);
            let expected_height = (previous_height / 2).max(1);
            if mip.width != expected_width || mip.height != expected_height {
                return Err(SnError::InvalidData {
                    resource: "texture mip chain",
                    reason: format!("unexpected dimensions at mip {index}"),
                });
            }
        }
        let pixels = u64::from(mip.width)
            .checked_mul(u64::from(mip.height))
            .ok_or(SnError::SizeOverflow {
                resource: "texture dimensions",
            })?;
        let expected_bytes = pixels.checked_mul(4).ok_or(SnError::SizeOverflow {
            resource: "texture byte length",
        })?;
        total_pixels = total_pixels
            .checked_add(pixels)
            .ok_or(SnError::SizeOverflow {
                resource: "texture pixel budget",
            })?;
        total_bytes = total_bytes
            .checked_add(expected_bytes)
            .ok_or(SnError::SizeOverflow {
                resource: "texture byte budget",
            })?;
        if total_pixels > options.limits.max_texture_pixels {
            return Err(SnError::LimitExceeded {
                resource: "texture pixels",
                actual: total_pixels,
                limit: options.limits.max_texture_pixels,
            });
        }
        if total_bytes > options.limits.max_texture_rgba_bytes {
            return Err(SnError::LimitExceeded {
                resource: "texture RGBA bytes",
                actual: total_bytes,
                limit: options.limits.max_texture_rgba_bytes,
            });
        }
        if mip.data.len() as u64 != expected_bytes {
            return Err(SnError::InvalidData {
                resource: "texture data",
                reason: format!(
                    "mip {index} has {} bytes, expected {expected_bytes}",
                    mip.data.len()
                ),
            });
        }
        previous_width = mip.width;
        previous_height = mip.height;
    }
    Ok(total_bytes)
}

pub(crate) fn fit_dimensions(width: u32, height: u32, maximum: Option<u32>) -> (u32, u32) {
    let Some(maximum) = maximum else {
        return (width, height);
    };
    if width <= maximum && height <= maximum {
        return (width, height);
    }
    if width >= height {
        let fitted_height = (u64::from(height) * u64::from(maximum) / u64::from(width)).max(1);
        (maximum, fitted_height as u32)
    } else {
        let fitted_width = (u64::from(width) * u64::from(maximum) / u64::from(height)).max(1);
        (fitted_width as u32, maximum)
    }
}

/// Nearest-neighbor resizing avoids mixing sRGB and linear channels or normal-map directions.
/// Returns the source slice unchanged when no resize is needed.
pub(crate) fn resize_rgba<'a>(
    source: &'a [u8],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
) -> Result<Cow<'a, [u8]>> {
    if source_width == width && source_height == height {
        return Ok(Cow::Borrowed(source));
    }
    let output_len = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(SnError::SizeOverflow {
            resource: "resized texture",
        })?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_len as usize)
        .map_err(|_| SnError::AllocationFailed {
            resource: "resized texture",
        })?;

    for y in 0..height {
        let source_y = u64::from(y) * u64::from(source_height) / u64::from(height);
        for x in 0..width {
            let source_x = u64::from(x) * u64::from(source_width) / u64::from(width);
            let source_index = ((source_y * u64::from(source_width) + source_x) * 4) as usize;
            let pixel =
                source
                    .get(source_index..source_index + 4)
                    .ok_or_else(|| SnError::InvalidData {
                        resource: "texture data",
                        reason: "resizing sampled outside the source buffer".to_owned(),
                    })?;
            output.extend_from_slice(pixel);
        }
    }
    Ok(Cow::Owned(output))
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(Cursor::new(&mut encoded), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColorSpace, GpuTextureFormat, MipLevel};

    fn texture() -> TextureAsset {
        TextureAsset {
            name: "base".to_owned(),
            width: 2,
            height: 2,
            format: TextureFormat::Rgba8,
            color_space: ColorSpace::Srgb,
            mips: vec![MipLevel {
                width: 2,
                height: 2,
                data: vec![
                    255, 0, 0, 255, // red
                    0, 255, 0, 255, // green
                    0, 0, 255, 255, // blue
                    255, 255, 255, 255, // white
                ],
            }],
        }
    }

    #[test]
    fn borrows_source_rgba_and_encodes_png_without_resize() {
        let texture = texture();
        let source = texture.mips[0].data.as_slice();
        let rgba = resize_rgba(source, 2, 2, 2, 2).expect("matching dimensions must not resize");

        assert!(matches!(&rgba, Cow::Borrowed(_)));
        assert_eq!(rgba.as_ptr(), source.as_ptr());
        let png = encode_png(rgba.as_ref(), 2, 2).expect("borrowed RGBA must encode");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn resizes_with_deterministic_nearest_sampling() {
        let texture = texture();
        let resized =
            resize_rgba(&texture.mips[0].data, 2, 2, 1, 1).expect("2x2 texture must resize");
        assert!(matches!(&resized, Cow::Owned(_)));
        assert_eq!(resized.as_ref(), [255, 0, 0, 255]);
    }

    #[test]
    fn prepares_portable_png_and_respects_dimension_cap() {
        let options = ConversionOptions {
            max_texture_dimension: Some(1),
            ..ConversionOptions::default()
        };
        let prepared = prepare_texture(&texture(), &options).expect("texture must encode");
        assert_eq!((prepared.width, prepared.height), (1, 1));
        assert!(matches!(prepared.payload, PreparedTexturePayload::Png(_)));
        assert!(
            prepared
                .payload
                .encoded_bytes()
                .starts_with(b"\x89PNG\r\n\x1a\n")
        );
        assert_eq!(
            prepared.payload.encoded_len(),
            prepared.payload.encoded_bytes().len()
        );
        // Saída 1x1: cadeia completa com um único nível de 4 bytes.
        let gpu = prepared.gpu.expect("PNG must declare its GPU footprint");
        assert_eq!(gpu.format, GpuTextureFormat::Rgba8);
        assert_eq!((gpu.mip_count, gpu.byte_len), (1, 4));
    }

    #[test]
    fn png_declares_full_mip_chain_at_output_dimensions() {
        let prepared = prepare_texture(&texture(), &ConversionOptions::default())
            .expect("texture must encode");
        assert_eq!((prepared.width, prepared.height), (2, 2));
        let gpu = prepared.gpu.expect("PNG must declare its GPU footprint");
        assert_eq!(gpu.format, GpuTextureFormat::Rgba8);
        // 2x2 + 1x1 pixels * 4 bytes por pixel.
        assert_eq!(gpu.mip_count, 2);
        assert_eq!(gpu.byte_len, (4 + 1) * 4);
    }

    #[test]
    fn rejects_truncated_texture_data() {
        let mut texture = texture();
        texture.mips[0].data.pop();
        let error = prepare_texture(&texture, &ConversionOptions::default())
            .expect_err("truncated texture must fail");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "texture data",
                ..
            }
        ));
    }

    #[test]
    fn enforces_texture_pixel_budget() {
        let options = ConversionOptions {
            limits: crate::config::Limits {
                max_texture_pixels: 3,
                ..crate::config::Limits::default()
            },
            ..ConversionOptions::default()
        };
        let error = prepare_texture(&texture(), &options).expect_err("pixel budget must fail");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "texture pixels",
                actual: 4,
                limit: 3
            }
        ));
    }

    #[cfg(not(any(feature = "pc-ktx2-basis", feature = "switch-ktx2-astc")))]
    #[test]
    fn rejects_unimplemented_ktx2_instead_of_emitting_png() {
        let options = ConversionOptions {
            texture_target: TextureTarget::Ktx2Basis,
            ..ConversionOptions::default()
        };
        assert!(matches!(
            prepare_texture(&texture(), &options),
            Err(SnError::Unsupported(_))
        ));
    }
}
