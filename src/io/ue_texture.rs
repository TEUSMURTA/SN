use std::path::Path;

use crate::{
    config::Limits,
    error::{Result, SnError},
    io::reader::{AssetReader, checked_byte_len, read_file_bounded},
    model::{ColorSpace, MipLevel, TextureAsset, TextureFormat},
};

/// Reads an uncompressed or RLE-compressed TGA exported by UModel.
pub fn read_umodel_texture(
    path: impl AsRef<Path>,
    color_space: ColorSpace,
    limits: &Limits,
) -> Result<TextureAsset> {
    let path = path.as_ref();
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tga"))
    {
        return Err(SnError::Unsupported(
            "only TGA texture exports are currently supported".to_owned(),
        ));
    }
    let bytes = read_file_bounded(
        path,
        limits.max_input_bytes,
        "reading UModel texture",
        "TGA texture",
    )?;
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "texture".to_owned());
    parse_tga(&name, &bytes, color_space, limits)
}

/// Decodes TGA pixels into top-left-origin RGBA8, as expected by the PNG encoder.
pub fn parse_tga(
    name: &str,
    input: &[u8],
    color_space: ColorSpace,
    limits: &Limits,
) -> Result<TextureAsset> {
    let input_len = u64::try_from(input.len()).map_err(|_| SnError::SizeOverflow {
        resource: "TGA texture",
    })?;
    if input_len > limits.max_input_bytes {
        return Err(SnError::LimitExceeded {
            resource: "TGA texture",
            actual: input_len,
            limit: limits.max_input_bytes,
        });
    }

    let mut reader = AssetReader::new(input, limits.max_input_bytes);
    let header = reader.read_array::<18>()?;
    if header[1] != 0 {
        return Err(SnError::Unsupported(
            "color-mapped TGA textures are not supported".to_owned(),
        ));
    }
    let image_type = header[2];
    let width = u16::from_le_bytes([header[12], header[13]]);
    let height = u16::from_le_bytes([header[14], header[15]]);
    let pixel_depth = header[16];
    let descriptor = header[17];
    if descriptor & 0xc0 != 0 {
        return Err(SnError::Unsupported(
            "interleaved TGA textures are not supported".to_owned(),
        ));
    }
    let format = match (image_type, pixel_depth) {
        (2 | 10, 16) => PixelFormat::Bgr5551,
        (2 | 10, 24) => PixelFormat::Bgr24,
        (2 | 10, 32) => PixelFormat::Bgra32,
        (3 | 11, 8) => PixelFormat::Gray8,
        (2 | 3 | 10 | 11, _) => {
            return Err(SnError::Unsupported(format!(
                "TGA pixel depth {pixel_depth} for image type {image_type}"
            )));
        }
        _ => {
            return Err(SnError::Unsupported(format!("TGA image type {image_type}")));
        }
    };

    let rgba_len = checked_rgba_len(u32::from(width), u32::from(height), limits)?;
    let pixel_count = rgba_len / 4;
    let id_length = usize::from(header[0]);
    reader.read_bytes(id_length, "TGA image ID")?;
    let right_to_left = descriptor & 0x10 != 0;
    let top_to_bottom = descriptor & 0x20 != 0;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(rgba_len)
        .map_err(|_| SnError::AllocationFailed {
            resource: "decoded TGA pixels",
        })?;
    rgba.resize(rgba_len, 0);

    if matches!(image_type, 2 | 3) {
        let pixel_count = u64::try_from(pixel_count).map_err(|_| SnError::SizeOverflow {
            resource: "TGA pixel data",
        })?;
        let source_len = checked_byte_len(
            pixel_count,
            u64::try_from(format.bytes_per_pixel()).map_err(|_| SnError::SizeOverflow {
                resource: "TGA pixel data",
            })?,
            limits.max_input_bytes,
            "TGA pixel data",
        )?;
        let source = reader.read_bytes(source_len, "TGA pixel data")?;
        for (index, pixel) in source.chunks_exact(format.bytes_per_pixel()).enumerate() {
            write_pixel(
                &mut rgba,
                index,
                usize::from(width),
                right_to_left,
                top_to_bottom,
                decode_pixel(format, pixel),
            )?;
        }
    } else {
        decode_rle(
            &mut reader,
            &mut rgba,
            pixel_count,
            usize::from(width),
            format,
            right_to_left,
            top_to_bottom,
        )?;
    }

    let width = u32::from(width);
    let height = u32::from(height);
    Ok(TextureAsset {
        name: name.to_owned(),
        width,
        height,
        format: TextureFormat::Rgba8,
        color_space,
        mips: vec![MipLevel {
            width,
            height,
            data: rgba,
        }],
    })
}

#[derive(Clone, Copy)]
enum PixelFormat {
    Gray8,
    Bgr5551,
    Bgr24,
    Bgra32,
}

impl PixelFormat {
    fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Gray8 => 1,
            Self::Bgr5551 => 2,
            Self::Bgr24 => 3,
            Self::Bgra32 => 4,
        }
    }
}

fn checked_rgba_len(width: u32, height: u32, limits: &Limits) -> Result<usize> {
    if width == 0 || height == 0 {
        return Err(SnError::InvalidData {
            resource: "texture dimensions",
            reason: "width and height must be greater than zero".to_owned(),
        });
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(SnError::SizeOverflow {
            resource: "texture dimensions",
        })?;
    if pixels > limits.max_texture_pixels {
        return Err(SnError::LimitExceeded {
            resource: "texture pixels",
            actual: pixels,
            limit: limits.max_texture_pixels,
        });
    }
    let bytes = pixels.checked_mul(4).ok_or(SnError::SizeOverflow {
        resource: "decoded TGA pixels",
    })?;
    if bytes > limits.max_texture_rgba_bytes {
        return Err(SnError::LimitExceeded {
            resource: "texture RGBA bytes",
            actual: bytes,
            limit: limits.max_texture_rgba_bytes,
        });
    }
    usize::try_from(bytes).map_err(|_| SnError::SizeOverflow {
        resource: "decoded TGA pixels",
    })
}

#[allow(clippy::too_many_arguments)]
fn decode_rle(
    reader: &mut AssetReader<&[u8]>,
    output: &mut [u8],
    pixel_count: usize,
    width: usize,
    format: PixelFormat,
    right_to_left: bool,
    top_to_bottom: bool,
) -> Result<()> {
    let mut decoded = 0_usize;
    while decoded < pixel_count {
        let packet = reader.read_u8()?;
        let packet_len = usize::from(packet & 0x7f) + 1;
        let packet_end = decoded
            .checked_add(packet_len)
            .ok_or(SnError::SizeOverflow {
                resource: "TGA RLE packet",
            })?;
        if packet_end > pixel_count {
            return Err(SnError::InvalidData {
                resource: "TGA RLE packet",
                reason: "packet extends beyond the declared image dimensions".to_owned(),
            });
        }

        if packet & 0x80 != 0 {
            let mut bytes = [0_u8; 4];
            let bytes_per_pixel = format.bytes_per_pixel();
            reader.read_into(&mut bytes[..bytes_per_pixel], "TGA RLE pixel")?;
            let pixel = decode_pixel(format, &bytes[..bytes_per_pixel]);
            for index in decoded..packet_end {
                write_pixel(output, index, width, right_to_left, top_to_bottom, pixel)?;
            }
        } else {
            for index in decoded..packet_end {
                let mut bytes = [0_u8; 4];
                let bytes_per_pixel = format.bytes_per_pixel();
                reader.read_into(&mut bytes[..bytes_per_pixel], "TGA raw RLE pixel")?;
                let pixel = decode_pixel(format, &bytes[..bytes_per_pixel]);
                write_pixel(output, index, width, right_to_left, top_to_bottom, pixel)?;
            }
        }
        decoded = packet_end;
    }
    Ok(())
}

fn decode_pixel(format: PixelFormat, source: &[u8]) -> [u8; 4] {
    match format {
        PixelFormat::Gray8 => [source[0], source[0], source[0], 255],
        PixelFormat::Bgr24 => [source[2], source[1], source[0], 255],
        PixelFormat::Bgra32 => [source[2], source[1], source[0], source[3]],
        PixelFormat::Bgr5551 => {
            let value = u16::from_le_bytes([source[0], source[1]]);
            let expand = |component| ((component * 255 + 15) / 31) as u8;
            [
                expand((value >> 10) & 0x1f),
                expand((value >> 5) & 0x1f),
                expand(value & 0x1f),
                if value & 0x8000 == 0 { 0 } else { 255 },
            ]
        }
    }
}

fn write_pixel(
    output: &mut [u8],
    source_index: usize,
    width: usize,
    right_to_left: bool,
    top_to_bottom: bool,
    pixel: [u8; 4],
) -> Result<()> {
    let source_x = source_index % width;
    let source_y = source_index / width;
    let x = if right_to_left {
        width - 1 - source_x
    } else {
        source_x
    };
    let y = if top_to_bottom {
        source_y
    } else {
        output.len() / 4 / width - 1 - source_y
    };
    let offset = (y * width + x) * 4;
    let destination = output
        .get_mut(offset..offset + 4)
        .ok_or_else(|| SnError::InvalidData {
            resource: "TGA pixel destination",
            reason: "pixel lies outside the decoded image".to_owned(),
        })?;
    destination.copy_from_slice(&pixel);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u16, height: u16, depth: u8, image_type: u8, descriptor: u8) -> Vec<u8> {
        let mut bytes = vec![0_u8; 18];
        bytes[2] = image_type;
        bytes[12..14].copy_from_slice(&width.to_le_bytes());
        bytes[14..16].copy_from_slice(&height.to_le_bytes());
        bytes[16] = depth;
        bytes[17] = descriptor;
        bytes
    }

    #[test]
    fn decodes_bottom_origin_bgr_to_top_origin_rgba() {
        let mut bytes = header(1, 2, 24, 2, 0);
        bytes.extend_from_slice(&[255, 0, 0, 0, 255, 0]);
        let texture = parse_tga("two", &bytes, ColorSpace::Srgb, &Limits::default())
            .expect("valid TGA must decode");
        assert_eq!(texture.mips[0].data, [0, 255, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn decodes_top_origin_rle_and_alpha() {
        let mut bytes = header(2, 1, 32, 10, 0x20);
        bytes.extend_from_slice(&[0x81, 30, 20, 10, 40]);
        let texture = parse_tga("rle", &bytes, ColorSpace::Linear, &Limits::default())
            .expect("valid RLE TGA must decode");
        assert_eq!(texture.mips[0].data, [10, 20, 30, 40, 10, 20, 30, 40]);
        assert_eq!(texture.color_space, ColorSpace::Linear);
    }

    #[test]
    fn reports_truncated_pixel_data_as_eof() {
        let mut bytes = header(2, 1, 24, 2, 0x20);
        bytes.extend_from_slice(&[0, 0, 255]);
        let error = parse_tga("short", &bytes, ColorSpace::Srgb, &Limits::default())
            .expect_err("missing pixel must fail");
        assert!(matches!(
            error,
            SnError::UnexpectedEof {
                offset: 18,
                requested: 6
            }
        ));
    }

    #[test]
    fn rejects_rle_packet_beyond_image_bounds() {
        let mut bytes = header(2, 1, 24, 10, 0x20);
        bytes.extend_from_slice(&[0x82, 0, 0, 255]);
        let error = parse_tga("bad-rle", &bytes, ColorSpace::Srgb, &Limits::default())
            .expect_err("oversized packet must fail");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "TGA RLE packet",
                ..
            }
        ));
    }

    #[test]
    fn enforces_pixel_limit_before_decoding() {
        let mut bytes = header(2, 1, 24, 2, 0x20);
        bytes.extend_from_slice(&[0; 6]);
        let limits = Limits {
            max_texture_pixels: 1,
            ..Limits::default()
        };
        let error = parse_tga("large", &bytes, ColorSpace::Srgb, &limits)
            .expect_err("pixel limit must fail");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "texture pixels",
                actual: 2,
                limit: 1
            }
        ));
    }

    #[test]
    fn detects_decoded_size_overflow() {
        let limits = Limits {
            max_texture_pixels: u64::MAX,
            max_texture_rgba_bytes: u64::MAX,
            ..Limits::default()
        };
        let error = checked_rgba_len(u32::MAX, u32::MAX, &limits)
            .expect_err("RGBA byte length must overflow");
        assert!(matches!(
            error,
            SnError::SizeOverflow {
                resource: "decoded TGA pixels"
            }
        ));
    }

    #[test]
    fn reads_umodel_fixture_when_present() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
            "tests/fixtures/ref_umodel/Game/Environment/Prop/10_Props/Rubble/\
             pr_Industrial_Decor_Pack/T_pr_Rubble_Concrete_Chunk01_ALB.tga",
        );
        if !path.exists() {
            eprintln!("optional UModel fixture is absent: {}", path.display());
            return;
        }
        let texture = read_umodel_texture(path, ColorSpace::Srgb, &Limits::default())
            .expect("UModel TGA fixture must decode");
        assert_eq!((texture.width, texture.height), (256, 256));
        assert_eq!(texture.mips[0].data.len(), 256 * 256 * 4);
        assert_eq!(&texture.mips[0].data[..4], &[134, 132, 118, 255]);
    }
}
