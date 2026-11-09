// .aom reading: parse the container, inflate within the declared bound,
// decode the MessagePack payload. Files are untrusted; every failure is a
// typed error, and the inflater can never run past the declared length.

use alloc::vec::Vec;

use crate::container::Header;
use crate::error::Error;
use crate::payload::AomPayload;

pub fn read(bytes: &[u8]) -> Result<AomPayload, Error> {
    let (header, payload) = Header::parse(bytes)?;
    let raw: Vec<u8>;
    let plain = if header.flags.compressed() {
        raw = miniz_oxide::inflate::decompress_to_vec_with_limit(
            payload,
            header.payload_len as usize,
        )
        .map_err(|e| Error::Inflate(alloc::format!("{e}")))?;
        if raw.len() != header.payload_len as usize {
            return Err(Error::InflateShort { declared: header.payload_len, produced: raw.len() });
        }
        raw.as_slice()
    } else {
        payload
    };
    let mut out: AomPayload =
        rmp_serde::from_slice(plain).map_err(|e| Error::Decode(alloc::format!("{e}")))?;
    // scenes from older files carry straight-line tracks; curves from here on
    for a in &mut out.animations {
        a.upgrade();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::model::{ArtMesh, Binding, Model, Parameter};
    use crate::payload::{AomPayload, Meta};

    #[test]
    fn aom_round_trips() {
        let mut model = Model::new();
        model.parameters.push(Parameter {
            id: "ParamX".into(),
            min: 0.0,
            max: 1.0,
            default: 0.5,
            repeat: false,
            decimals: 3,
            blend_shape: true,
        });
        model.art_meshes.push(ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 3,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            masks: vec![],
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            blend_shapes: vec![crate::model::BlendShape {
                parameter: 0,
                keys: vec![1.0],
                positions: vec![0.0, 3.0, 1.0, 0.0, 0.0, 1.0],
                limits: Vec::new(),
                angles: Vec::new(),
                draw_orders: Vec::new(),
                intensities: Vec::new(),
                multiply: Vec::new(),
                opacities: Vec::new(),
                origins_x: Vec::new(),
                origins_y: Vec::new(),
                scales: Vec::new(),
                screen: Vec::new(),
            }],
            clip_group: None,
            clip_inherit: false,
            multiply: Vec::new(),
            screen: Vec::new(),
            skin: None,
        });
        let payload = AomPayload {
            meta: Meta {
                name: "t".into(),
                author: String::new(),
                tool: "test".into(),
                timelapse: None,
                layer_pages: Vec::new(),
            },
            model,
            textures: vec![serde_bytes::ByteBuf::from(vec![1u8, 2, 3])],
            display: None,
            animations: Vec::new(),
            compositions: Vec::new(),
            playlists: Vec::new(),
            expressions: vec![crate::Expression {
                name: "smile".into(),
                fade_in: Some(0.5),
                fade_out: None,
                params: vec![crate::ExpressionParam {
                    id: "ParamEyeLOpen".into(),
                    value: 0.6,
                    blend: crate::ExpressionBlend::Multiply,
                }],
            }],
        };
        let bytes = crate::writer::write(&payload).unwrap();
        let back = crate::read::read(&bytes).unwrap();
        assert_eq!(back.meta.name, "t");
        assert_eq!(back.model.parameters.len(), 1);
        let m = &back.model.art_meshes[0];
        assert_eq!(m.positions, vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert_eq!(m.indices, vec![0, 1, 2]);
        assert!(back.model.parameters[0].blend_shape);
        assert_eq!(m.blend_shapes[0].keys, vec![1.0]);
        assert_eq!(m.blend_shapes[0].positions, vec![0.0, 3.0, 1.0, 0.0, 0.0, 1.0]);
        assert_eq!(back.textures[0].as_ref(), &[1u8, 2, 3]);
        assert_eq!(back.expressions, payload.expressions);
    }

    #[test]
    fn old_straight_line_tracks_become_linear_curves() {
        use crate::payload::{Animation, CurveTarget, Interp, Track};
        let old = AomPayload {
            animations: vec![Animation {
                name: "idle".into(),
                fps: 30.0,
                duration: 1.0,
                tracks: vec![Track {
                    param: "ParamX".into(),
                    points: vec![(0.0, 0.0), (1.0, 5.0)],
                }],
                ..Animation::default()
            }],
            ..AomPayload::default()
        };
        let back = crate::read::read(&crate::writer::write(&old).unwrap()).unwrap();
        let a = &back.animations[0];
        assert!(a.tracks.is_empty());
        let c = &a.curves[0];
        assert_eq!((c.target, c.id.as_str()), (CurveTarget::Parameter, "ParamX"));
        assert_eq!(c.keys.len(), 2);
        assert_eq!(c.keys[1].value, 5.0);
        assert!(c.keys.iter().all(|k| k.interp == Interp::Linear));
    }

    #[test]
    fn compositions_ride_along_and_old_files_have_none() {
        use crate::payload::{Composition, Layer, LayerSource};
        let with = AomPayload {
            compositions: vec![Composition {
                name: "intro".into(),
                fps: 30.0,
                duration: 4.0,
                layers: vec![Layer {
                    source: LayerSource::Scene("idle".into()),
                    out_point: 3.0,
                    fade_in: 0.5,
                    ..Layer::default()
                }],
                ..Composition::default()
            }],
            ..AomPayload::default()
        };
        let back = crate::read::read(&crate::writer::write(&with).unwrap()).unwrap();
        assert_eq!(back.compositions, with.compositions);
        let old =
            crate::read::read(&crate::writer::write(&AomPayload::default()).unwrap()).unwrap();
        assert!(old.compositions.is_empty());
    }

    /// A model with a bone and a mesh weighted to it.
    fn boned() -> AomPayload {
        use crate::model::{BoneData, Deformer, DeformerKind, RotationData, Skin};
        let mut model = Model::new();
        model.deformers.push(Deformer {
            id: "Bone".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            binding: Binding::default(),
            kind: DeformerKind::Bone(BoneData {
                rotation: RotationData {
                    base_angle: 0.0,
                    opacities: vec![1.0],
                    angles: vec![15.0],
                    origins_x: vec![0.5],
                    origins_y: vec![0.25],
                    scales: vec![1.0],
                    reflect_x: vec![false],
                    reflect_y: vec![false],
                },
                length: 2.5,
                scales_y: vec![0.5],
                shears_x: vec![10.0],
                shears_y: vec![-5.0],
                inherit: crate::model::Inherit::NoScale,
            }),
            multiply: Vec::new(),
            screen: Vec::new(),
            blend_shapes: Vec::new(),
        });
        // one bone, two, and five (bones past the one here are left out
        // when it plays, but the file keeps them)
        let pairs: [&[(u32, f32)]; 3] = [
            &[(0, 1.0)],
            &[(0, 0.5), (7, 0.5)],
            &[(0, 0.2), (1, 0.2), (2, 0.2), (3, 0.2), (4, 0.2)],
        ];
        let skin = Skin::from_pairs(pairs);
        model.art_meshes.push(ArtMesh {
            id: "M".into(),
            parent_part: None,
            parent_deformer: None,
            visible: true,
            enabled: true,
            texture: 0,
            flags: 0,
            blend: Default::default(),
            vertex_count: 3,
            uvs: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            indices: vec![0, 1, 2],
            masks: vec![],
            clip_group: None,
            clip_inherit: false,
            binding: Binding::default(),
            opacities: vec![1.0],
            draw_orders: vec![500.0],
            positions: vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
            blend_shapes: Vec::new(),
            multiply: Vec::new(),
            screen: Vec::new(),
            skin: Some(skin),
        });
        AomPayload { model, ..AomPayload::default() }
    }

    #[test]
    fn bones_and_weights_round_trip() {
        use crate::model::DeformerKind;
        let with = boned();
        let back = crate::read::read(&crate::writer::write(&with).unwrap()).unwrap();
        let DeformerKind::Bone(b) = &back.model.deformers[0].kind else { panic!("bone") };
        assert_eq!(b.length, 2.5);
        assert_eq!(b.inherit, crate::model::Inherit::NoScale);
        assert_eq!(
            (b.scales_y.clone(), b.shears_x.clone(), b.shears_y.clone()),
            (vec![0.5], vec![10.0], vec![-5.0])
        );
        assert_eq!(b.rotation.angles, vec![15.0]);
        assert_eq!((b.rotation.origins_x[0], b.rotation.origins_y[0]), (0.5, 0.25));
        let skin = back.model.art_meshes[0].skin.clone().unwrap();
        assert_eq!(Some(&skin), with.model.art_meshes[0].skin.as_ref());
        assert_eq!(skin.offsets, vec![0, 1, 3, 8]);
        assert!(skin.fits(3));
    }

    #[test]
    fn files_from_before_bones_load_unweighted() {
        use crate::container::{CONTAINER_VERSION, Flags, Header};
        let mut old = boned();
        old.model.deformers.clear();
        old.model.art_meshes[0].skin = None;
        let mut raw = rmp_serde::to_vec_named(&old).unwrap();
        // take the `skin` entry out of the mesh map, as a file written
        // before it existed has none: its key and nil, and one off the
        // map's length (22 fields, a map16 header)
        let entry = b"\xa4skin\xc0";
        let at = raw.windows(entry.len()).position(|w| w == entry).unwrap();
        raw.drain(at..at + entry.len());
        let head = b"\xde\x00\x16\xa2id";
        let h = raw.windows(head.len()).position(|w| w == head).unwrap();
        raw[h + 2] = 0x15;
        let header = Header {
            version: CONTAINER_VERSION,
            flags: Flags::default(),
            payload_len: raw.len() as u32,
        };
        let mut bytes = header.to_bytes().to_vec();
        bytes.extend_from_slice(&raw);
        let back = crate::read::read(&bytes).unwrap();
        let m = &back.model.art_meshes[0];
        assert!(m.skin.is_none());
        assert_eq!(m.positions, vec![0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn a_bone_from_before_shear_reads_as_unsheared() {
        use crate::model::{BoneData, RotationData};
        // the bone as first written: no y scale, no shear
        #[derive(serde::Serialize)]
        struct First {
            rotation: RotationData,
            length: f32,
        }
        let rotation = RotationData {
            base_angle: 0.0,
            opacities: vec![1.0],
            angles: vec![0.0],
            origins_x: vec![0.0],
            origins_y: vec![0.0],
            scales: vec![2.0],
            reflect_x: vec![false],
            reflect_y: vec![false],
        };
        let bytes = rmp_serde::to_vec_named(&First { rotation, length: 1.0 }).unwrap();
        let b: BoneData = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(b.length, 1.0);
        assert!(b.scales_y.is_empty() && b.shears_x.is_empty() && b.shears_y.is_empty());
        assert_eq!(b.inherit, crate::model::Inherit::Normal);
    }

    #[test]
    fn rejects_garbage() {
        assert!(crate::read::read(b"not an aom").is_err());
    }
}
