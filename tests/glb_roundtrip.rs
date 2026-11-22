use scarlet_nexus_assets::{
    config::ConversionOptions,
    convert::{mesh::convert_mesh, texture::prepare_texture},
    glb::write_prepared_glb,
    model::{
        ColorSpace, Material, MeshAsset, MeshLod, MipLevel, PreparedAsset, Primitive, TextureAsset,
        TextureFormat, VertexAttributes,
    },
};

fn read_vec3(bytes: &[u8], offset: usize) -> [f32; 3] {
    let mut result = [0.0_f32; 3];
    for (component, raw) in bytes[offset..offset + 12]
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
    {
        result[component] = f32::from_le_bytes(*raw);
    }
    result
}

#[test]
fn exports_mesh_and_png_as_a_gltf_document() {
    let options = ConversionOptions::default();
    let source = MeshAsset {
        name: "triangle".to_owned(),
        lods: vec![MeshLod {
            primitives: vec![Primitive {
                attributes: VertexAttributes {
                    positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    normals: Some(vec![[0.0, 0.0, 1.0]; 3]),
                    tangents: None,
                    texcoords: Some(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]),
                    joints: None,
                    weights: None,
                },
                indices: vec![0, 1, 2],
                material: Some(0),
            }],
        }],
        skeleton: None,
    };
    let texture = TextureAsset {
        name: "albedo".to_owned(),
        width: 1,
        height: 1,
        format: TextureFormat::Rgba8,
        color_space: ColorSpace::Srgb,
        mips: vec![MipLevel {
            width: 1,
            height: 1,
            data: vec![255, 0, 0, 255],
        }],
    };

    let prepared = PreparedAsset {
        name: "triangle".to_owned(),
        mesh: convert_mesh(&source, &options).expect("mesh must convert"),
        textures: vec![prepare_texture(&texture, &options).expect("PNG must encode")],
        materials: vec![Material {
            name: "painted_triangle".to_owned(),
            base_color_texture: Some(0),
            ..Material::default()
        }],
    };
    let texture_bytes = prepared.textures[0].png_bytes.len();
    let mut output = Vec::new();
    let report = write_prepared_glb(&mut output, &prepared, &options.limits)
        .expect("prepared asset must export");

    assert_eq!(report.primitive_count, 1);
    assert_eq!(report.vertex_count, 3);
    assert_eq!(report.index_count, 3);
    assert_eq!(report.texture_count, 1);
    assert_eq!(report.texture_bytes, texture_bytes);
    assert_eq!(report.glb_bytes, output.len());

    let document = gltf::Gltf::from_slice(&output).expect("gltf crate must validate the document");
    assert_eq!(document.scenes().count(), 1);
    assert_eq!(document.nodes().count(), 1);
    assert_eq!(document.meshes().count(), 1);
    assert_eq!(document.images().count(), 1);
    assert_eq!(document.textures().count(), 1);

    let mesh = document.meshes().next().expect("one mesh");
    let primitive = mesh.primitives().next().expect("one primitive");
    assert_eq!(primitive.mode(), gltf::mesh::Mode::Triangles);

    let positions = primitive
        .get(&gltf::mesh::Semantic::Positions)
        .expect("POSITION accessor");
    assert_eq!(positions.count(), 3);
    assert_eq!(positions.data_type(), gltf::accessor::DataType::F32);
    let position_view = positions.view().expect("position buffer view");
    let position_offset = position_view.offset() + positions.offset();
    let position_stride = position_view.stride().unwrap_or(12);
    let blob = document.blob.as_ref().expect("embedded BIN chunk");
    assert_eq!(read_vec3(blob, position_offset), [0.0, 0.0, 0.0]);
    assert_eq!(
        read_vec3(blob, position_offset + position_stride),
        [0.0, 0.0, 1.0]
    );
    assert_eq!(
        read_vec3(blob, position_offset + 2 * position_stride),
        [-1.0, 0.0, 0.0]
    );

    let indices = primitive.indices().expect("index accessor");
    assert_eq!(indices.count(), 3);
    assert_eq!(indices.data_type(), gltf::accessor::DataType::U8);
    let index_view = indices.view().expect("index buffer view");
    let index_offset = index_view.offset() + indices.offset();
    assert_eq!(&blob[index_offset..index_offset + 3], &[0, 2, 1]);

    let base_color = primitive
        .material()
        .pbr_metallic_roughness()
        .base_color_texture()
        .expect("base color texture");
    assert_eq!(base_color.texture().source().index(), 0);

    let image = document.images().next().expect("embedded image");
    match image.source() {
        gltf::image::Source::View { view, mime_type } => {
            assert_eq!(mime_type, "image/png");
            let start = view.offset();
            let end = start + view.length();
            assert_eq!(&blob[start..end], &prepared.textures[0].png_bytes);
        }
        other => panic!("expected buffer-view image, got {other:?}"),
    }
}
