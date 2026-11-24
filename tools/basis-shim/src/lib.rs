//! Front-end seguro para o encoder C++ basis-universal.
//!
//! Este crate é a única fronteira FFI do protótipo KTX2/Basis de PC. O crate
//! `basis-universal` 0.3.1 expõe `Compressor::init` e `Compressor::process`
//! como `unsafe fn` (parâmetros inválidos podem causar comportamento
//! indefinido dentro do encoder C++), de modo que o crate principal
//! (`scarlet-nexus-assets`, com `unsafe_code = "forbid"`) não pode chamá-lo
//! diretamente. Aqui cada chamada `unsafe` é precedida de validação estrita
//! dos mips e contida neste módulo; nenhum outro código do workspace escreve
//! blocos `unsafe`.

pub use basis_universal::{ColorSpace as EncoderColorSpace, CompressorErrorCode};

use basis_universal::{BasisTextureFormat, Compressor, CompressorParams};
use thiserror::Error;

/// Modo de compressão Basis aceito pelo encoder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BasisMode {
    Etc1s,
    Uastc,
}

/// Um nível de mip RGBA8 pronto para o encoder.
#[derive(Clone, Copy, Debug)]
pub struct MipInput<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error("the mip chain is empty")]
    EmptyMipChain,
    #[error("mip {level}: expected {expected} RGBA bytes for {width}x{height}, found {actual}")]
    InvalidMip {
        level: usize,
        width: u32,
        height: u32,
        expected: u64,
        actual: u64,
    },
    #[error("mip {level}: {width}x{height} does not halve from {previous_width}x{previous_height}")]
    InvalidMipChain {
        level: usize,
        width: u32,
        height: u32,
        previous_width: u32,
        previous_height: u32,
    },
    #[error("the encoder rejected the validated parameters")]
    InitFailed,
    #[error("basis encoding failed: {0:?}")]
    Encode(CompressorErrorCode),
}

/// Resultado do encoder: arquivo `.basis` completo e indicador de alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedBasis {
    pub bytes: Vec<u8>,
    pub has_alpha: bool,
}

/// Codifica uma cadeia de mips RGBA8 em um arquivo `.basis`.
///
/// A cadeia deve começar na resolução base e dividir as dimensões por dois
/// até 1x1 (ou terminar antes); cada nível precisa de exatamente
/// `width * height * 4` bytes. A validação é executada antes de qualquer
/// chamada ao encoder C++.
pub fn encode_basis(
    mode: BasisMode,
    color_space: EncoderColorSpace,
    mips: &[MipInput<'_>],
) -> Result<EncodedBasis, EncodeError> {
    let first = mips.first().ok_or(EncodeError::EmptyMipChain)?;
    let mut previous = (first.width, first.height);
    for (level, mip) in mips.iter().enumerate() {
        let expected = u64::from(mip.width)
            .checked_mul(u64::from(mip.height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(EncodeError::InvalidMip {
                level,
                width: mip.width,
                height: mip.height,
                expected: u64::MAX,
                actual: mip.rgba.len() as u64,
            })?;
        if mip.rgba.len() as u64 != expected {
            return Err(EncodeError::InvalidMip {
                level,
                width: mip.width,
                height: mip.height,
                expected,
                actual: mip.rgba.len() as u64,
            });
        }
        if level > 0 {
            let (expected_width, expected_height) = halve(previous);
            if mip.width != expected_width || mip.height != expected_height {
                return Err(EncodeError::InvalidMipChain {
                    level,
                    width: mip.width,
                    height: mip.height,
                    previous_width: previous.0,
                    previous_height: previous.1,
                });
            }
        }
        previous = (mip.width, mip.height);
    }

    let mut params = CompressorParams::new();
    params.set_print_status_to_stdout(false);
    params.set_basis_format(match mode {
        BasisMode::Etc1s => BasisTextureFormat::ETC1S,
        BasisMode::Uastc => BasisTextureFormat::UASTC4x4,
    });
    params.set_color_space(color_space);
    params.set_mip_color_space(color_space);
    // A cadeia completa é fornecida explicitamente; o encoder não gera mips.
    params.set_generate_mipmaps(false);
    if mips.len() > 1 {
        params.resize_source_mipmap_image_list(1);
    }
    for (level, mip) in mips.iter().enumerate() {
        if level == 0 {
            params
                .source_image_mut(0)
                .init(mip.rgba, mip.width, mip.height, 4);
        } else {
            params
                .source_mipmap_image_mut(0, (level - 1) as u32)
                .init(mip.rgba, mip.width, mip.height, 4);
        }
    }

    // `Compressor::default` usa um único thread de job pool e executa
    // `encoder_init` internamente; a saída fica determinística.
    let mut compressor = Compressor::default();
    // SAFETY: os parâmetros foram validados acima — cada mip tem exatamente
    // `width * height * 4` bytes RGBA e a cadeia obedece às divisões por dois.
    // O encoder C++ só apresenta comportamento indefinido com parâmetros
    // inválidos, e os parâmetros não mudam entre a validação e aqui.
    if !unsafe { compressor.init(&params) } {
        return Err(EncodeError::InitFailed);
    }
    // SAFETY: mesmas garantias de `init`; estado interno consistente após o
    // `init` bem-sucedido.
    unsafe { compressor.process() }.map_err(EncodeError::Encode)?;

    Ok(EncodedBasis {
        bytes: compressor.basis_file().to_vec(),
        has_alpha: compressor.any_source_image_has_alpha(),
    })
}

fn halve(dims: (u32, u32)) -> (u32, u32) {
    ((dims.0 / 2).max(1), (dims.1 / 2).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOUR_BY_FOUR_RED: [u8; 64] = [
        255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0,
        255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255,
        0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
    ];

    fn base_mips() -> [MipInput<'static>; 1] {
        [MipInput {
            width: 4,
            height: 4,
            rgba: &FOUR_BY_FOUR_RED,
        }]
    }

    #[test]
    fn encodes_uastc_basis_file() {
        let encoded = encode_basis(BasisMode::Uastc, EncoderColorSpace::Srgb, &base_mips())
            .expect("4x4 UASTC must encode");
        assert_eq!(
            u16::from_le_bytes([encoded.bytes[0], encoded.bytes[1]]),
            0x4273
        );
        assert!(!encoded.has_alpha);
    }

    #[test]
    fn encodes_etc1s_basis_file() {
        let encoded = encode_basis(BasisMode::Etc1s, EncoderColorSpace::Linear, &base_mips())
            .expect("4x4 ETC1S must encode");
        assert_eq!(
            u16::from_le_bytes([encoded.bytes[0], encoded.bytes[1]]),
            0x4273
        );
    }

    #[test]
    fn rejects_mismatched_mip_bytes() {
        let mips = [MipInput {
            width: 4,
            height: 4,
            rgba: &FOUR_BY_FOUR_RED[..63],
        }];
        let error = encode_basis(BasisMode::Uastc, EncoderColorSpace::Srgb, &mips)
            .expect_err("truncated mip must fail");
        assert!(matches!(error, EncodeError::InvalidMip { level: 0, .. }));
    }

    #[test]
    fn rejects_broken_halving() {
        let second = [0u8; 16];
        let mips = [
            MipInput {
                width: 4,
                height: 4,
                rgba: &FOUR_BY_FOUR_RED,
            },
            MipInput {
                width: 1,
                height: 4,
                rgba: &second,
            },
        ];
        let error = encode_basis(BasisMode::Uastc, EncoderColorSpace::Srgb, &mips)
            .expect_err("1x4 must not halve from 4x4");
        assert!(matches!(
            error,
            EncodeError::InvalidMipChain { level: 1, .. }
        ));
    }

    #[test]
    fn rejects_empty_chain() {
        let error = encode_basis(BasisMode::Uastc, EncoderColorSpace::Srgb, &[])
            .expect_err("empty chain must fail");
        assert!(matches!(error, EncodeError::EmptyMipChain));
    }
}
