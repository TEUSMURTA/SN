use std::{
    error::Error,
    fs,
    io::{Error as IoError, ErrorKind},
    ops::Range,
    path::Path,
};

use scarlet_nexus_assets::{
    config::ConversionOptions,
    glb::{validate_glb, write_prepared_glb},
    io::ue_texture::read_umodel_texture,
    model::{
        Asset, ColorSpace, ConversionReport, GpuTextureFormat, Material, MeshAsset, MeshLod,
        MipLevel, Primitive, TextureAsset, TextureFormat, VertexAttributes, full_mip_count,
        mip_chain_bytes,
    },
    platform::{self, SWITCH_TARGET_ID},
    prepare_asset,
};

const FIXTURE_DIRECTORY: &str = "tests/fixtures/ref_umodel/Game/Environment/Prop/10_Props/\
                                 Rubble/pr_Industrial_Decor_Pack";

fn invalid_data(reason: impl Into<String>) -> IoError {
    IoError::new(ErrorKind::InvalidData, reason.into())
}

fn main() -> Result<(), Box<dyn Error>> {
    let fixture_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_DIRECTORY);
    let mesh_path = fixture_directory.join("SM_pr_Rubble_Concrete_Chunk_03.gltf");
    let texture_path = fixture_directory.join("T_pr_Rubble_Concrete_Chunk01_ALB.tga");
    for path in [&mesh_path, &texture_path] {
        if !path.is_file() {
            return Err(invalid_data(format!(
                "required local UModel fixture is absent: {}",
                path.display()
            ))
            .into());
        }
    }

    println!(
        "Real UModel asset (glTF + BIN + TGA; no PSK/PSKX fixture is present):\n  {}\n  {}",
        mesh_path.display(),
        texture_path.display()
    );
    let asset = load_real_asset(&mesh_path, &texture_path)?;
    let output_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/real_asset_profiles");
    fs::create_dir_all(&output_directory)?;

    let pc = export_profile(
        "PC",
        "rubble_concrete_pc.glb",
        &asset,
        platform::pc::conversion_options(),
        &output_directory,
    )?;
    let switch = export_profile(
        &format!("Switch ({SWITCH_TARGET_ID})"),
        "rubble_concrete_switch.glb",
        &asset,
        platform::switch::conversion_options(),
        &output_directory,
    )?;
    print_vram_reduction(
        "Switch vs PC (PNG)",
        pc.report.gpu_texture_bytes,
        switch.report.gpu_texture_bytes,
    );
    #[cfg(feature = "pc-ktx2-basis")]
    {
        let ktx2 = export_profile(
            "PC (KTX2 UASTC)",
            "rubble_concrete_pc_ktx2.glb",
            &asset,
            platform::pc::ktx2_conversion_options(scarlet_nexus_assets::config::BasisMode::Uastc),
            &output_directory,
        )?;
        print_vram_reduction(
            "PC KTX2 UASTC vs PC PNG",
            pc.report.gpu_texture_bytes,
            ktx2.report.gpu_texture_bytes,
        );
    }
    #[cfg(feature = "switch-ktx2-astc")]
    {
        let astc = export_profile(
            &format!("Switch KTX2 ASTC 4x4 ({SWITCH_TARGET_ID})"),
            "rubble_concrete_switch_ktx2_astc.glb",
            &asset,
            platform::switch::ktx2_astc_conversion_options(),
            &output_directory,
        )?;
        print_vram_reduction(
            "Switch KTX2 ASTC 4x4 vs Switch PNG",
            switch.report.gpu_texture_bytes,
            astc.report.gpu_texture_bytes,
        );
        print_vram_reduction(
            "Switch KTX2 ASTC 4x4 vs PC PNG",
            pc.report.gpu_texture_bytes,
            astc.report.gpu_texture_bytes,
        );
    }
    benchmark_texture_sizes(&asset, &output_directory)?;
    Ok(())
}

fn print_vram_reduction(label: &str, before: u64, after: u64) {
    if before == 0 {
        println!("{label}: footprint de GPU indisponível para comparação (0 bytes antes)");
        return;
    }
    let saved = before.saturating_sub(after);
    let percent = saved as f64 * 100.0 / before as f64;
    println!(
        "{label}: VRAM de texturas {before} -> {after} bytes (redução de {percent:.1}%, -{saved} bytes)"
    );
}

fn load_real_asset(mesh_path: &Path, texture_path: &Path) -> Result<Asset, Box<dyn Error>> {
    let json = fs::read(mesh_path)?;
    let document = gltf::Gltf::from_slice(&json)?;
    if document.buffers().count() != 1 {
        return Err(invalid_data("the real fixture must have exactly one binary buffer").into());
    }
    if document.materials().count() != 1 {
        return Err(invalid_data("the real fixture must have exactly one material").into());
    }

    // The UModel fixture is already glTF rather than PSK. Its adjacent BIN contains the geometry.
    let binary = fs::read(mesh_path.with_extension("bin"))?;
    let source_mesh = document
        .meshes()
        .next()
        .ok_or_else(|| invalid_data("the real fixture has no mesh"))?;
    let mesh_name = mesh_path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rubble_concrete".to_owned());
    let mut primitives = Vec::new();
    for primitive in source_mesh.primitives() {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            return Err(invalid_data("the real fixture must contain triangle primitives").into());
        }

        let positions = primitive
            .get(&gltf::mesh::Semantic::Positions)
            .ok_or_else(|| invalid_data("the real fixture has no POSITION accessor"))?;
        let positions = read_f32_array::<3>(&positions, &binary, gltf::accessor::Dimensions::Vec3)?
            .into_iter()
            .map(unreal_position_from_gltf)
            .collect();
        let normals = primitive
            .get(&gltf::mesh::Semantic::Normals)
            .map(|accessor| {
                read_f32_array::<3>(&accessor, &binary, gltf::accessor::Dimensions::Vec3).map(
                    |values| {
                        values
                            .into_iter()
                            .map(unreal_position_from_gltf)
                            .collect::<Vec<_>>()
                    },
                )
            })
            .transpose()?;
        let tangents = primitive
            .get(&gltf::mesh::Semantic::Tangents)
            .map(|accessor| {
                read_f32_array::<4>(&accessor, &binary, gltf::accessor::Dimensions::Vec4).map(
                    |values| {
                        values
                            .into_iter()
                            .map(|tangent| {
                                let xyz =
                                    unreal_position_from_gltf([tangent[0], tangent[1], tangent[2]]);
                                [xyz[0], xyz[1], xyz[2], -tangent[3]]
                            })
                            .collect::<Vec<_>>()
                    },
                )
            })
            .transpose()?;
        let texcoords = primitive
            .get(&gltf::mesh::Semantic::TexCoords(0))
            .map(|accessor| {
                read_f32_array::<2>(&accessor, &binary, gltf::accessor::Dimensions::Vec2)
            })
            .transpose()?;
        let indices_accessor = primitive
            .indices()
            .ok_or_else(|| invalid_data("the real fixture has no index accessor"))?;
        let mut indices = read_indices(&indices_accessor, &binary)?;
        if indices.is_empty() || !indices.len().is_multiple_of(3) {
            return Err(invalid_data("the real fixture has an incomplete triangle list").into());
        }

        // prepare_asset expects Unreal coordinates and winding. Invert both so its output
        // preserves the already-converted geometry from the real UModel glTF fixture.
        for triangle in indices.as_chunks_mut::<3>().0 {
            triangle.swap(1, 2);
        }
        primitives.push(Primitive {
            attributes: VertexAttributes {
                positions,
                normals,
                tangents,
                texcoords,
                joints: None,
                weights: None,
            },
            indices,
            material: primitive.material().index(),
        });
    }

    let source_material = document
        .materials()
        .next()
        .ok_or_else(|| invalid_data("the real fixture has no material"))?;
    let pbr = source_material.pbr_metallic_roughness();
    let texture = read_umodel_texture(
        texture_path,
        ColorSpace::Srgb,
        &ConversionOptions::default().limits,
    )?;
    Ok(Asset {
        mesh: MeshAsset {
            name: mesh_name,
            lods: vec![MeshLod { primitives }],
            skeleton: None,
        },
        textures: vec![texture],
        materials: vec![Material {
            name: "MI_en_pr_Rubble_Concrete_Chunk01".to_owned(),
            base_color_texture: Some(0),
            normal_texture: None,
            metallic_roughness_texture: None,
            base_color_factor: pbr.base_color_factor(),
            metallic_factor: pbr.metallic_factor(),
            roughness_factor: pbr.roughness_factor(),
        }],
    })
}

fn read_f32_array<const N: usize>(
    accessor: &gltf::Accessor<'_>,
    binary: &[u8],
    dimensions: gltf::accessor::Dimensions,
) -> Result<Vec<[f32; N]>, Box<dyn Error>> {
    if accessor.data_type() != gltf::accessor::DataType::F32 || accessor.dimensions() != dimensions
    {
        return Err(invalid_data("unexpected real fixture floating-point accessor").into());
    }

    let element_bytes = N
        .checked_mul(4)
        .ok_or_else(|| invalid_data("floating-point accessor size overflow"))?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(accessor.count())
        .map_err(|_| invalid_data("cannot allocate real fixture attributes"))?;
    for index in 0..accessor.count() {
        let range = element_range(accessor, index, element_bytes)?;
        let bytes = binary
            .get(range)
            .ok_or_else(|| invalid_data("real fixture attribute lies outside its BIN"))?;
        let mut value = [0.0_f32; N];
        for (component, output) in value.iter_mut().enumerate() {
            let offset = component
                .checked_mul(4)
                .ok_or_else(|| invalid_data("attribute component offset overflow"))?;
            let raw: [u8; 4] = bytes
                .get(offset..offset + 4)
                .and_then(|slice| slice.try_into().ok())
                .ok_or_else(|| invalid_data("truncated real fixture attribute"))?;
            *output = f32::from_le_bytes(raw);
        }
        values.push(value);
    }
    Ok(values)
}

fn read_indices(accessor: &gltf::Accessor<'_>, binary: &[u8]) -> Result<Vec<u32>, Box<dyn Error>> {
    if accessor.dimensions() != gltf::accessor::Dimensions::Scalar {
        return Err(invalid_data("real fixture indices must be scalar").into());
    }
    let component_bytes = match accessor.data_type() {
        gltf::accessor::DataType::U8 => 1,
        gltf::accessor::DataType::U16 => 2,
        gltf::accessor::DataType::U32 => 4,
        _ => return Err(invalid_data("unsupported real fixture index component type").into()),
    };

    let mut indices = Vec::new();
    indices
        .try_reserve_exact(accessor.count())
        .map_err(|_| invalid_data("cannot allocate real fixture indices"))?;
    for index in 0..accessor.count() {
        let range = element_range(accessor, index, component_bytes)?;
        let bytes = binary
            .get(range)
            .ok_or_else(|| invalid_data("real fixture index lies outside its BIN"))?;
        let value = match accessor.data_type() {
            gltf::accessor::DataType::U8 => u32::from(
                *bytes
                    .first()
                    .ok_or_else(|| invalid_data("truncated u8 index"))?,
            ),
            gltf::accessor::DataType::U16 => {
                let raw: [u8; 2] = bytes
                    .try_into()
                    .map_err(|_| invalid_data("truncated u16 index"))?;
                u32::from(u16::from_le_bytes(raw))
            }
            gltf::accessor::DataType::U32 => {
                let raw: [u8; 4] = bytes
                    .try_into()
                    .map_err(|_| invalid_data("truncated u32 index"))?;
                u32::from_le_bytes(raw)
            }
            _ => return Err(invalid_data("unsupported real fixture index type").into()),
        };
        indices.push(value);
    }
    Ok(indices)
}

fn element_range(
    accessor: &gltf::Accessor<'_>,
    index: usize,
    element_bytes: usize,
) -> Result<Range<usize>, IoError> {
    let view = accessor
        .view()
        .ok_or_else(|| invalid_data("sparse real fixture accessors are not supported"))?;
    let stride = view.stride().unwrap_or(element_bytes);
    if stride < element_bytes {
        return Err(invalid_data("real fixture accessor stride is too small"));
    }
    let relative_offset = index
        .checked_mul(stride)
        .ok_or_else(|| invalid_data("real fixture accessor offset overflow"))?;
    let start = view
        .offset()
        .checked_add(accessor.offset())
        .and_then(|offset| offset.checked_add(relative_offset))
        .ok_or_else(|| invalid_data("real fixture accessor offset overflow"))?;
    let end = start
        .checked_add(element_bytes)
        .ok_or_else(|| invalid_data("real fixture accessor length overflow"))?;
    Ok(start..end)
}

fn unreal_position_from_gltf(position: [f32; 3]) -> [f32; 3] {
    [position[2], -position[0], position[1]]
}

struct TextureFootprint {
    name: String,
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    format: GpuTextureFormat,
    mip_count: u32,
    byte_len: u64,
    encoded_bytes: usize,
}

struct ProfileOutput {
    report: ConversionReport,
    textures: Vec<TextureFootprint>,
}

fn export_profile(
    label: &str,
    file_name: &str,
    asset: &Asset,
    options: ConversionOptions,
    output_directory: &Path,
) -> Result<ProfileOutput, Box<dyn Error>> {
    let prepared = prepare_asset(asset, &options)?;
    let mut textures = Vec::new();
    for (source, prepared_texture) in asset.textures.iter().zip(&prepared.textures) {
        let gpu = prepared_texture.gpu.ok_or_else(|| {
            invalid_data(format!(
                "{label}: texture '{}' must declare a GPU footprint",
                prepared_texture.name
            ))
        })?;
        let expected_mips = full_mip_count(prepared_texture.width, prepared_texture.height)?;
        if gpu.mip_count != expected_mips {
            return Err(invalid_data(format!(
                "{label}: texture '{}' declares {} mips, expected the full chain of {expected_mips}",
                prepared_texture.name, gpu.mip_count
            ))
            .into());
        }
        let expected_bytes = mip_chain_bytes(
            gpu.format,
            prepared_texture.width,
            prepared_texture.height,
            gpu.mip_count,
        )?;
        if gpu.byte_len != expected_bytes {
            return Err(invalid_data(format!(
                "{label}: texture '{}' declares {} GPU bytes, expected {expected_bytes}",
                prepared_texture.name, gpu.byte_len
            ))
            .into());
        }
        textures.push(TextureFootprint {
            name: prepared_texture.name.clone(),
            source_width: source.width,
            source_height: source.height,
            width: prepared_texture.width,
            height: prepared_texture.height,
            format: gpu.format,
            mip_count: gpu.mip_count,
            byte_len: gpu.byte_len,
            encoded_bytes: prepared_texture.payload.encoded_len(),
        });
    }
    let output_path = output_directory.join(file_name);
    let report = {
        let mut output = fs::File::create(&output_path)?;
        write_prepared_glb(&mut output, &prepared, &options.limits)?
    };
    let glb = fs::read(&output_path)?;
    validate_glb(&glb, options.limits.max_asset_bytes)?;
    if report.gpu_texture_bytes == 0 {
        return Err(invalid_data(format!(
            "{label} must declare a positive GPU texture footprint"
        ))
        .into());
    }
    println!(
        "{label}\n{report:#?}\nGLB: {}\nVRAM de texturas: {} bytes",
        output_path.display(),
        report.gpu_texture_bytes
    );
    for texture in &textures {
        println!(
            "  textura '{}': {}x{} -> {}x{}, {} mips, GPU {:?} com {} bytes, {} bytes codificados",
            texture.name,
            texture.source_width,
            texture.source_height,
            texture.width,
            texture.height,
            texture.mip_count,
            texture.format,
            texture.byte_len,
            texture.encoded_bytes
        );
    }
    Ok(ProfileOutput { report, textures })
}

const SIZE_1024_TEXTURE: &str = "tests/fixtures/ref_umodel/Game/Character/om/om0427/99_Resource/\
                                 T_om0427_ALB.tga";
const SIZE_2048_TEXTURE: &str =
    "tests/fixtures/ref_umodel/Game/Environment/Sky/Sky01/T_Sky_Blue2.tga";

/// Compara os 4 perfis (PC PNG, PC KTX2 UASTC, Switch PNG, Switch KTX2 ASTC 4x4)
/// sobre texturas reais de 1024x1024 e 2048x2048; na ausência do fixture, gera
/// casos determinísticos com as mesmas dimensões.
fn benchmark_texture_sizes(
    base_asset: &Asset,
    output_directory: &Path,
) -> Result<(), Box<dyn Error>> {
    let cases = [
        ("1024x1024", SIZE_1024_TEXTURE, 1024_u32),
        ("2048x2048", SIZE_2048_TEXTURE, 2048_u32),
    ];
    for (label, relative, fallback_dimension) in cases {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        let texture = if path.is_file() {
            println!("\nCaso {label}: textura real {}", path.display());
            read_umodel_texture(
                &path,
                ColorSpace::Srgb,
                &ConversionOptions::default().limits,
            )?
        } else {
            println!(
                "\nCaso {label}: fixture ausente ({relative}); textura determinística {fallback_dimension}x{fallback_dimension}"
            );
            synthetic_texture(
                &format!("synthetic_{fallback_dimension}x{fallback_dimension}"),
                fallback_dimension,
                fallback_dimension,
            )?
        };
        let mut asset = base_asset.clone();
        asset.textures = vec![texture];
        benchmark_profiles(label, &asset, output_directory)?;
    }
    Ok(())
}

fn benchmark_profiles(
    label: &str,
    asset: &Asset,
    output_directory: &Path,
) -> Result<(), Box<dyn Error>> {
    let source = asset
        .textures
        .first()
        .ok_or_else(|| invalid_data("benchmark case needs exactly one texture"))?;
    println!(
        "Benchmark {label}: entrada {}x{}, {} mip(s) fornecido(s)",
        source.width,
        source.height,
        source.mips.len()
    );

    let pc = export_profile(
        "PC (PNG)",
        &format!("bench_{label}_pc.glb"),
        asset,
        platform::pc::conversion_options(),
        output_directory,
    )?;
    let switch_options = platform::switch::conversion_options();
    let switch = export_profile(
        &format!("Switch PNG ({SWITCH_TARGET_ID})"),
        &format!("bench_{label}_switch.glb"),
        asset,
        switch_options.clone(),
        output_directory,
    )?;
    if let Some(cap) = switch_options.max_texture_dimension {
        for texture in &switch.textures {
            if texture.width > cap || texture.height > cap {
                return Err(invalid_data(format!(
                    "Switch profile must cap the output at {cap}, got {}x{}",
                    texture.width, texture.height
                ))
                .into());
            }
        }
    }
    print_vram_reduction(
        "Switch PNG vs PC PNG",
        pc.report.gpu_texture_bytes,
        switch.report.gpu_texture_bytes,
    );

    #[cfg(feature = "pc-ktx2-basis")]
    {
        let pc_ktx2 = export_profile(
            "PC (KTX2 UASTC)",
            &format!("bench_{label}_pc_ktx2.glb"),
            asset,
            platform::pc::ktx2_conversion_options(scarlet_nexus_assets::config::BasisMode::Uastc),
            output_directory,
        )?;
        print_vram_reduction(
            "PC KTX2 UASTC vs PC PNG",
            pc.report.gpu_texture_bytes,
            pc_ktx2.report.gpu_texture_bytes,
        );
    }
    #[cfg(feature = "switch-ktx2-astc")]
    {
        let astc_options = platform::switch::ktx2_astc_conversion_options();
        // O protótipo KTX2 exige resolução nativa: o cap de 1024 do perfil Switch é
        // aplicado na entrada, antes do encode UASTC -> transcode ASTC 4x4.
        let mut astc_asset = asset.clone();
        if let Some(cap) = astc_options.max_texture_dimension {
            astc_asset.textures = vec![cap_texture_dimension(source, cap)?];
        }
        let astc = export_profile(
            &format!("Switch KTX2 ASTC 4x4 ({SWITCH_TARGET_ID})"),
            &format!("bench_{label}_switch_ktx2_astc.glb"),
            &astc_asset,
            astc_options,
            output_directory,
        )?;
        print_vram_reduction(
            "Switch KTX2 ASTC 4x4 vs Switch PNG",
            switch.report.gpu_texture_bytes,
            astc.report.gpu_texture_bytes,
        );
        print_vram_reduction(
            "Switch KTX2 ASTC 4x4 vs PC PNG",
            pc.report.gpu_texture_bytes,
            astc.report.gpu_texture_bytes,
        );
    }
    Ok(())
}

/// Downscale nearest-neighbor até `cap` na maior dimensão, preservando a proporção.
fn cap_texture_dimension(texture: &TextureAsset, cap: u32) -> Result<TextureAsset, Box<dyn Error>> {
    if cap == 0 {
        return Err(invalid_data("dimension cap must be greater than zero").into());
    }
    let (width, height) = if texture.width <= cap && texture.height <= cap {
        (texture.width, texture.height)
    } else if texture.width >= texture.height {
        let fitted = u64::from(texture.height)
            .checked_mul(u64::from(cap))
            .and_then(|value| value.checked_div(u64::from(texture.width)))
            .ok_or_else(|| invalid_data("dimension fit overflow"))?
            .max(1);
        (cap, u32::try_from(fitted)?)
    } else {
        let fitted = u64::from(texture.width)
            .checked_mul(u64::from(cap))
            .and_then(|value| value.checked_div(u64::from(texture.height)))
            .ok_or_else(|| invalid_data("dimension fit overflow"))?
            .max(1);
        (u32::try_from(fitted)?, cap)
    };
    if (width, height) == (texture.width, texture.height) {
        return Ok(texture.clone());
    }
    let base = texture
        .mips
        .first()
        .ok_or_else(|| invalid_data("texture has no base mip"))?;
    let output_len = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| invalid_data("resized texture size overflow"))?;
    let output_len = usize::try_from(output_len)?;
    let mut data = Vec::new();
    data.try_reserve_exact(output_len)
        .map_err(|_| invalid_data("cannot allocate resized texture"))?;
    for y in 0..height {
        let source_y = u64::from(y)
            .checked_mul(u64::from(texture.height))
            .and_then(|value| value.checked_div(u64::from(height)))
            .ok_or_else(|| invalid_data("resize source row overflow"))?;
        for x in 0..width {
            let source_x = u64::from(x)
                .checked_mul(u64::from(texture.width))
                .and_then(|value| value.checked_div(u64::from(width)))
                .ok_or_else(|| invalid_data("resize source column overflow"))?;
            let source_index = usize::try_from(
                source_y
                    .checked_mul(u64::from(texture.width))
                    .and_then(|row| row.checked_add(source_x))
                    .and_then(|pixel| pixel.checked_mul(4))
                    .ok_or_else(|| invalid_data("resize source index overflow"))?,
            )?;
            let pixel = base
                .data
                .get(source_index..source_index + 4)
                .ok_or_else(|| invalid_data("resize sampled outside the source buffer"))?;
            data.extend_from_slice(pixel);
        }
    }
    Ok(TextureAsset {
        name: texture.name.clone(),
        width,
        height,
        format: texture.format,
        color_space: texture.color_space,
        mips: vec![MipLevel {
            width,
            height,
            data,
        }],
    })
}

/// Textura RGBA8 determinística para o caso de fallback quando o fixture real
/// não está presente no checkout.
fn synthetic_texture(name: &str, width: u32, height: u32) -> Result<TextureAsset, Box<dyn Error>> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| invalid_data("synthetic texture dimensions overflow"))?;
    let len = usize::try_from(
        pixels
            .checked_mul(4)
            .ok_or_else(|| invalid_data("synthetic texture size overflow"))?,
    )?;
    let mut data = Vec::new();
    data.try_reserve_exact(len)
        .map_err(|_| invalid_data("cannot allocate synthetic texture"))?;
    for index in 0..pixels {
        let x = index % u64::from(width);
        let y = index / u64::from(width);
        data.extend_from_slice(&[
            (x ^ (y << 3)) as u8,
            (x.wrapping_add(y) & 0xff) as u8,
            (x.wrapping_mul(3).wrapping_add(y.wrapping_mul(7)) & 0xff) as u8,
            255,
        ]);
    }
    Ok(TextureAsset {
        name: name.to_owned(),
        width,
        height,
        format: TextureFormat::Rgba8,
        color_space: ColorSpace::Srgb,
        mips: vec![MipLevel {
            width,
            height,
            data,
        }],
    })
}
