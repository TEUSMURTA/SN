use std::io::Write;

use crate::error::{Result, SnError};

mod asset;

pub use asset::{GlbDocument, build_glb_document, write_prepared_glb};

const GLB_MAGIC: u32 = 0x4654_6c67;
const GLB_VERSION: u32 = 2;
const JSON_CHUNK_TYPE: u32 = 0x4e4f_534a;
const BIN_CHUNK_TYPE: u32 = 0x004e_4942;
const GLB_HEADER_LEN: u64 = 12;
const CHUNK_HEADER_LEN: u64 = 8;

#[derive(Clone, Copy, Debug)]
struct GlbLayout {
    total_len: u32,
    json_padded_len: u64,
    binary_padded_len: Option<u64>,
}

/// Writes a GLB 2.0 container with one JSON chunk and an optional binary chunk.
///
/// The complete size is checked before any bytes are written. JSON padding uses
/// spaces and binary padding uses zeroes, as required by the GLB specification.
pub fn write_glb<W: Write + ?Sized>(
    writer: &mut W,
    json: &[u8],
    binary: Option<&[u8]>,
    max_bytes: u64,
) -> Result<()> {
    validate_json(json)?;
    write_glb_container(writer, json, binary, max_bytes).map(|_| ())
}

pub(crate) fn write_glb_container<W: Write + ?Sized>(
    writer: &mut W,
    json: &[u8],
    binary: Option<&[u8]>,
    max_bytes: u64,
) -> Result<usize> {
    let json_len = u64::try_from(json.len()).map_err(|_| SnError::SizeOverflow {
        resource: "GLB JSON chunk",
    })?;
    let binary_len = binary
        .map(|bytes| {
            u64::try_from(bytes.len()).map_err(|_| SnError::SizeOverflow {
                resource: "GLB binary chunk",
            })
        })
        .transpose()?;
    let layout = checked_layout(json_len, binary_len, max_bytes)?;

    write_all(writer, &GLB_MAGIC.to_le_bytes())?;
    write_all(writer, &GLB_VERSION.to_le_bytes())?;
    write_all(writer, &layout.total_len.to_le_bytes())?;
    write_chunk(writer, JSON_CHUNK_TYPE, layout.json_padded_len, json, b' ')?;
    if let (Some(bytes), Some(padded_len)) = (binary, layout.binary_padded_len) {
        write_chunk(writer, BIN_CHUNK_TYPE, padded_len, bytes, 0)?;
    }
    usize::try_from(layout.total_len).map_err(|_| SnError::SizeOverflow {
        resource: "GLB size",
    })
}

/// Validates the GLB 2.0 header, chunk boundaries, alignment, and JSON payload.
///
/// Unknown chunks after the required JSON chunk are accepted so producers can
/// extend the container. A binary chunk is only valid in the second position.
pub fn validate_glb(bytes: &[u8], max_bytes: u64) -> Result<()> {
    let actual_len = u64::try_from(bytes.len()).map_err(|_| SnError::SizeOverflow {
        resource: "GLB input",
    })?;
    if actual_len > max_bytes {
        return Err(SnError::LimitExceeded {
            resource: "GLB input",
            actual: actual_len,
            limit: max_bytes,
        });
    }

    let mut cursor = Cursor::new(bytes);
    let magic = cursor.read_u32()?;
    let version = cursor.read_u32()?;
    let declared_len = cursor.read_u32()?;
    if magic != GLB_MAGIC {
        return Err(invalid("GLB header", "incorrect magic"));
    }
    if version != GLB_VERSION {
        return Err(invalid("GLB header", "only GLB version 2 is supported"));
    }

    let declared_len_usize = usize::try_from(declared_len).map_err(|_| SnError::SizeOverflow {
        resource: "GLB declared length",
    })?;
    if declared_len_usize > bytes.len() {
        return Err(SnError::UnexpectedEof {
            offset: actual_len,
            requested: declared_len_usize - bytes.len(),
        });
    }
    if declared_len_usize < bytes.len() {
        return Err(invalid(
            "GLB length",
            "declared length is smaller than the input; trailing bytes are not allowed",
        ));
    }
    if declared_len % 4 != 0 {
        return Err(invalid(
            "GLB length",
            "total length must be aligned to 4 bytes",
        ));
    }

    let mut chunk_index = 0_usize;
    while cursor.offset < declared_len_usize {
        let chunk_len = cursor.read_u32()?;
        let chunk_type = cursor.read_u32()?;
        if chunk_len % 4 != 0 {
            return Err(invalid(
                "GLB chunk",
                "chunk length must be aligned to 4 bytes",
            ));
        }
        let chunk_len = usize::try_from(chunk_len).map_err(|_| SnError::SizeOverflow {
            resource: "GLB chunk length",
        })?;
        let payload = cursor.take(chunk_len)?;

        match (chunk_index, chunk_type) {
            (0, JSON_CHUNK_TYPE) => validate_json(payload)?,
            (0, _) => {
                return Err(invalid(
                    "GLB chunks",
                    "the first chunk must contain glTF JSON",
                ));
            }
            (_, JSON_CHUNK_TYPE) => {
                return Err(invalid("GLB chunks", "duplicate JSON chunk"));
            }
            (1, BIN_CHUNK_TYPE) => {}
            (_, BIN_CHUNK_TYPE) => {
                return Err(invalid(
                    "GLB chunks",
                    "the binary chunk must be the second and only binary chunk",
                ));
            }
            _ => {}
        }
        chunk_index = chunk_index.checked_add(1).ok_or(SnError::SizeOverflow {
            resource: "GLB chunk count",
        })?;
    }

    if chunk_index == 0 {
        return Err(SnError::UnexpectedEof {
            offset: actual_len,
            requested: 8,
        });
    }
    Ok(())
}

fn checked_layout(json_len: u64, binary_len: Option<u64>, max_bytes: u64) -> Result<GlbLayout> {
    let json_padded_len = padded_len(json_len, "GLB JSON chunk")?;
    let binary_padded_len = binary_len
        .map(|len| padded_len(len, "GLB binary chunk"))
        .transpose()?;
    let mut total_len = GLB_HEADER_LEN
        .checked_add(CHUNK_HEADER_LEN)
        .and_then(|len| len.checked_add(json_padded_len))
        .ok_or(SnError::SizeOverflow {
            resource: "GLB size",
        })?;
    if let Some(padded_len) = binary_padded_len {
        total_len = total_len
            .checked_add(CHUNK_HEADER_LEN)
            .and_then(|len| len.checked_add(padded_len))
            .ok_or(SnError::SizeOverflow {
                resource: "GLB size",
            })?;
    }
    if total_len > max_bytes {
        return Err(SnError::LimitExceeded {
            resource: "GLB size",
            actual: total_len,
            limit: max_bytes,
        });
    }
    if total_len > u64::from(u32::MAX) {
        return Err(SnError::LimitExceeded {
            resource: "GLB size",
            actual: total_len,
            limit: u64::from(u32::MAX),
        });
    }

    Ok(GlbLayout {
        total_len: u32::try_from(total_len).map_err(|_| SnError::SizeOverflow {
            resource: "GLB size",
        })?,
        json_padded_len,
        binary_padded_len,
    })
}

fn padded_len(len: u64, resource: &'static str) -> Result<u64> {
    let padding = (4 - (len % 4)) % 4;
    len.checked_add(padding)
        .ok_or(SnError::SizeOverflow { resource })
}

fn write_chunk<W: Write + ?Sized>(
    writer: &mut W,
    chunk_type: u32,
    padded_len: u64,
    data: &[u8],
    padding_byte: u8,
) -> Result<()> {
    let chunk_len = u32::try_from(padded_len).map_err(|_| SnError::SizeOverflow {
        resource: "GLB chunk length",
    })?;
    let data_len = u64::try_from(data.len()).map_err(|_| SnError::SizeOverflow {
        resource: "GLB chunk",
    })?;
    let padding = padded_len
        .checked_sub(data_len)
        .ok_or(SnError::SizeOverflow {
            resource: "GLB chunk padding",
        })?;
    let padding = usize::try_from(padding).map_err(|_| SnError::SizeOverflow {
        resource: "GLB chunk padding",
    })?;

    write_all(writer, &chunk_len.to_le_bytes())?;
    write_all(writer, &chunk_type.to_le_bytes())?;
    write_all(writer, data)?;
    write_all(writer, &[padding_byte; 3][..padding])
}

fn write_all<W: Write + ?Sized>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    writer
        .write_all(bytes)
        .map_err(|error| SnError::io("writing GLB", error))
}

fn validate_json(json: &[u8]) -> Result<()> {
    let value: serde_json::Value = serde_json::from_slice(json).map_err(|error| {
        invalid(
            "GLB JSON chunk",
            &format!("invalid UTF-8 JSON payload: {error}"),
        )
    })?;
    if !value.is_object() {
        return Err(invalid(
            "GLB JSON chunk",
            "the glTF JSON root must be an object",
        ));
    }
    Ok(())
}

fn invalid(resource: &'static str, reason: &str) -> SnError {
    SnError::InvalidData {
        resource,
        reason: reason.to_owned(),
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        let mut array = [0_u8; 4];
        array.copy_from_slice(bytes);
        Ok(u32::from_le_bytes(array))
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(len).ok_or(SnError::SizeOverflow {
            resource: "GLB input offset",
        })?;
        let offset = u64::try_from(self.offset).map_err(|_| SnError::SizeOverflow {
            resource: "GLB input offset",
        })?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(SnError::UnexpectedEof {
                offset,
                requested: len,
            })?;
        self.offset = end;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        let mut array = [0_u8; 4];
        array.copy_from_slice(&bytes[offset..offset + 4]);
        u32::from_le_bytes(array)
    }

    #[test]
    fn writes_aligned_json_and_binary_chunks() {
        let mut output = Vec::new();
        write_glb(&mut output, b"{}", Some(&[1, 2, 3]), 1024).expect("GLB must be written");

        assert_eq!(output.len(), 36);
        assert_eq!(u32_at(&output, 0), GLB_MAGIC);
        assert_eq!(u32_at(&output, 4), GLB_VERSION);
        assert_eq!(u32_at(&output, 8), 36);
        assert_eq!(u32_at(&output, 12), 4);
        assert_eq!(u32_at(&output, 16), JSON_CHUNK_TYPE);
        assert_eq!(&output[20..24], b"{}  ");
        assert_eq!(u32_at(&output, 24), 4);
        assert_eq!(u32_at(&output, 28), BIN_CHUNK_TYPE);
        assert_eq!(&output[32..36], &[1, 2, 3, 0]);
        validate_glb(&output, 36).expect("written GLB must validate");
    }

    #[test]
    fn writes_json_only_glb_at_the_minimum_limit() {
        let mut output = Vec::new();
        write_glb(&mut output, b"{}", None, 24).expect("24-byte GLB must fit its limit");
        assert_eq!(output.len(), 24);
        assert_eq!(u32_at(&output, 8), 24);
        validate_glb(&output, 24).expect("JSON-only GLB must validate");
    }

    #[test]
    fn enforces_writer_limit_before_writing() {
        let mut output = Vec::new();
        let error = write_glb(&mut output, b"{}", None, 23)
            .expect_err("one byte below the required size must fail");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "GLB size",
                actual: 24,
                limit: 23
            }
        ));
        assert!(output.is_empty());
    }

    #[test]
    fn enforces_validation_limit() {
        let mut output = Vec::new();
        write_glb(&mut output, b"{}", None, 24).expect("GLB must be written");
        let error = validate_glb(&output, 23).expect_err("input must exceed its limit");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "GLB input",
                actual: 24,
                limit: 23
            }
        ));
    }

    #[test]
    fn reports_eof_in_a_truncated_header() {
        let error = validate_glb(&[0_u8; 11], 1024).expect_err("header must be truncated");
        assert!(matches!(
            error,
            SnError::UnexpectedEof {
                offset: 8,
                requested: 4
            }
        ));
    }

    #[test]
    fn reports_eof_in_a_truncated_chunk() {
        let mut output = Vec::new();
        write_glb(&mut output, b"{}", Some(&[1, 2, 3]), 1024).expect("GLB must be written");
        output.pop();
        let error = validate_glb(&output, 1024).expect_err("last padding byte must be missing");
        assert!(matches!(
            error,
            SnError::UnexpectedEof {
                offset: 35,
                requested: 1
            }
        ));
    }

    #[test]
    fn detects_total_size_overflow() {
        let error = checked_layout(u64::MAX - 7, None, u64::MAX)
            .expect_err("header plus chunk size must overflow");
        assert!(matches!(
            error,
            SnError::SizeOverflow {
                resource: "GLB size"
            }
        ));
    }

    #[test]
    fn detects_alignment_overflow() {
        let error =
            padded_len(u64::MAX, "GLB JSON chunk").expect_err("rounding u64::MAX up must overflow");
        assert!(matches!(
            error,
            SnError::SizeOverflow {
                resource: "GLB JSON chunk"
            }
        ));
    }

    #[test]
    fn rejects_sizes_beyond_the_u32_container_limit() {
        let error = checked_layout(u64::from(u32::MAX), None, u64::MAX)
            .expect_err("GLB length field cannot represent this size");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                resource: "GLB size",
                limit,
                ..
            } if limit == u64::from(u32::MAX)
        ));
    }

    #[test]
    fn rejects_trailing_bytes_and_misaligned_chunks() {
        let mut output = Vec::new();
        write_glb(&mut output, b"{}", None, 24).expect("GLB must be written");

        let mut trailing = output.clone();
        trailing.push(0);
        assert!(matches!(
            validate_glb(&trailing, 1024),
            Err(SnError::InvalidData {
                resource: "GLB length",
                ..
            })
        ));

        output[12] = 2;
        assert!(matches!(
            validate_glb(&output, 1024),
            Err(SnError::InvalidData {
                resource: "GLB chunk",
                ..
            })
        ));
    }

    #[test]
    fn rejects_invalid_json_without_writing() {
        let mut output = Vec::new();
        let error = write_glb(&mut output, b"not json", None, 1024)
            .expect_err("invalid JSON must be rejected");
        assert!(matches!(
            error,
            SnError::InvalidData {
                resource: "GLB JSON chunk",
                ..
            }
        ));
        assert!(output.is_empty());
    }
}
