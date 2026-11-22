use std::{fs::File, io::Read, path::Path};

use crate::error::{Result, SnError};

/// A bounded little-endian reader for untrusted asset data.
pub struct AssetReader<R> {
    inner: R,
    offset: u64,
    max_bytes: u64,
}

impl<R: Read> AssetReader<R> {
    #[must_use]
    pub fn new(inner: R, max_bytes: u64) -> Self {
        Self {
            inner,
            offset: 0,
            max_bytes,
        }
    }

    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_array::<1>()?[0])
    }

    pub fn read_u16_le(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read_array::<2>()?))
    }

    pub fn read_u32_le(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    pub fn read_f32_le(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.read_array::<4>()?))
    }

    pub fn read_count(
        &mut self,
        element_size: u64,
        max_count: u64,
        max_allocation: u64,
        resource: &'static str,
    ) -> Result<usize> {
        let count = u64::from(self.read_u32_le()?);
        if count > max_count {
            return Err(SnError::LimitExceeded {
                resource,
                actual: count,
                limit: max_count,
            });
        }
        checked_byte_len(count, element_size, max_allocation, resource)?;
        usize::try_from(count).map_err(|_| SnError::SizeOverflow { resource })
    }

    pub fn read_bytes(&mut self, len: usize, resource: &'static str) -> Result<Vec<u8>> {
        self.check_budget(len, resource)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(len)
            .map_err(|_| SnError::AllocationFailed { resource })?;
        bytes.resize(len, 0);
        self.read_into(&mut bytes, resource)?;
        Ok(bytes)
    }

    pub(crate) fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.check_budget(N, "binary input")?;
        let mut bytes = [0; N];
        self.read_into(&mut bytes, "binary input")?;
        Ok(bytes)
    }

    fn check_budget(&self, len: usize, resource: &'static str) -> Result<()> {
        let requested = u64::try_from(len).map_err(|_| SnError::SizeOverflow { resource })?;
        let end = self
            .offset
            .checked_add(requested)
            .ok_or(SnError::SizeOverflow { resource })?;
        if end > self.max_bytes {
            return Err(SnError::LimitExceeded {
                resource,
                actual: end,
                limit: self.max_bytes,
            });
        }
        Ok(())
    }

    pub(crate) fn read_into(&mut self, buffer: &mut [u8], resource: &'static str) -> Result<()> {
        let requested = buffer.len();
        match self.inner.read_exact(buffer) {
            Ok(()) => {
                let read =
                    u64::try_from(requested).map_err(|_| SnError::SizeOverflow { resource })?;
                self.offset = self
                    .offset
                    .checked_add(read)
                    .ok_or(SnError::SizeOverflow { resource })?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                Err(SnError::UnexpectedEof {
                    offset: self.offset,
                    requested,
                })
            }
            Err(error) => Err(SnError::io("reading asset data", error)),
        }
    }
}

pub(crate) fn read_file_bounded(
    path: &Path,
    max_bytes: u64,
    operation: &'static str,
    resource: &'static str,
) -> Result<Vec<u8>> {
    let metadata = path
        .metadata()
        .map_err(|error| SnError::io(operation, error))?;
    if !metadata.is_file() {
        return Err(SnError::InvalidData {
            resource,
            reason: format!("{} is not a regular file", path.display()),
        });
    }
    let len = metadata.len();
    if len > max_bytes {
        return Err(SnError::LimitExceeded {
            resource,
            actual: len,
            limit: max_bytes,
        });
    }
    let len = usize::try_from(len).map_err(|_| SnError::SizeOverflow { resource })?;
    let file = File::open(path).map_err(|error| SnError::io(operation, error))?;
    AssetReader::new(file, max_bytes).read_bytes(len, resource)
}

pub fn checked_byte_len(
    count: u64,
    element_size: u64,
    max_bytes: u64,
    resource: &'static str,
) -> Result<usize> {
    let bytes = count
        .checked_mul(element_size)
        .ok_or(SnError::SizeOverflow { resource })?;
    if bytes > max_bytes {
        return Err(SnError::LimitExceeded {
            resource,
            actual: bytes,
            limit: max_bytes,
        });
    }
    usize::try_from(bytes).map_err(|_| SnError::SizeOverflow { resource })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_truncated_input_as_eof() {
        let mut reader = AssetReader::new(&[1_u8, 2, 3][..], 4);
        let error = reader.read_u32_le().expect_err("input must be truncated");
        assert!(matches!(
            error,
            SnError::UnexpectedEof {
                offset: 0,
                requested: 4
            }
        ));
    }

    #[test]
    fn rejects_read_beyond_budget_before_touching_input() {
        let mut reader = AssetReader::new(&[0_u8; 8][..], 3);
        let error = reader.read_u32_le().expect_err("budget must be enforced");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                actual: 4,
                limit: 3,
                ..
            }
        ));
    }

    #[test]
    fn detects_collection_size_overflow() {
        let error = checked_byte_len(u64::MAX, 2, u64::MAX, "vertices")
            .expect_err("multiplication must overflow");
        assert!(matches!(
            error,
            SnError::SizeOverflow {
                resource: "vertices"
            }
        ));
    }

    #[test]
    fn rejects_excessive_element_count_before_allocation() {
        let bytes = 3_u32.to_le_bytes();
        let mut reader = AssetReader::new(&bytes[..], 4);
        let error = reader
            .read_count(12, 2, 1024, "vertices")
            .expect_err("count must exceed its limit");
        assert!(matches!(
            error,
            SnError::LimitExceeded {
                actual: 3,
                limit: 2,
                ..
            }
        ));
    }

    #[test]
    fn tracks_offsets_and_reads_bounded_bytes() {
        let mut reader = AssetReader::new(&[1_u8, 2, 3, 4][..], 4);
        assert_eq!(reader.read_u8().expect("u8 must be readable"), 1);
        assert_eq!(reader.offset(), 1);
        assert_eq!(
            reader.read_bytes(3, "payload").expect("bytes must fit"),
            [2, 3, 4]
        );
        assert_eq!(reader.offset(), 4);
    }
}
