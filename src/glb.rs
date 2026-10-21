//! GLB → flat, world-space mesh data for rendering, picking and the map windows.

use glam::{Mat3, Mat4, Vec2, Vec3};
use std::path::Path;

pub struct Mesh {
    /// glTF node name = Blender object name (used to map a region back to Blender).
    pub name: String,
    pub positions: Vec<Vec3>,
    /// Empty when the file has none; the shader then uses flat normals.
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
    pub material: Option<usize>,
}

impl Mesh {
    pub fn tri(&self, t: usize) -> [Vec3; 3] {
        let i = &self.indices[t * 3..t * 3 + 3];
        [self.positions[i[0] as usize], self.positions[i[1] as usize], self.positions[i[2] as usize]]
    }
    pub fn tri_count(&self) -> usize {
        self.indices.len() / 3
    }
}

pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapKind {
    BaseColor,
    Normal,
    MetalRough,
    Occlusion,
    Emissive,
    Uv,
}

impl MapKind {
    pub const ALL: [MapKind; 6] = [MapKind::BaseColor, MapKind::Normal, MapKind::MetalRough, MapKind::Occlusion, MapKind::Emissive, MapKind::Uv];
    pub fn label(self) -> &'static str {
        match self {
            MapKind::BaseColor => "Base Color",
            MapKind::Normal => "Normal",
            MapKind::MetalRough => "Metallic / Roughness",
            MapKind::Occlusion => "Ambient Occlusion",
            MapKind::Emissive => "Emissive",
            MapKind::Uv => "UV Layout",
        }
    }
}

pub struct Material {
    pub name: String,
    pub base_color: [f32; 4],
    /// Image index per MapKind (Uv shows the base color image under the layout).
    pub maps: [Option<usize>; 5],
}

impl Material {
    pub fn map(&self, k: MapKind) -> Option<usize> {
        match k {
            MapKind::Uv => self.maps[0],
            k => self.maps[k as usize],
        }
    }
}

pub struct Model {
    pub meshes: Vec<Mesh>,
    pub materials: Vec<Material>,
    pub images: Vec<Image>,
    pub min: Vec3,
    pub max: Vec3,
}

pub struct Stats {
    pub tris: usize,
    pub verts: usize,
    pub objects: usize,
    pub materials: usize,
    pub size: Vec3,
}

impl Model {
    pub fn load(path: &Path) -> Result<Model, String> {
        let (doc, buffers, images) = gltf::import(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut m = Model {
            meshes: vec![],
            materials: doc.materials().map(material).collect(),
            images: images.into_iter().map(to_rgba).collect(),
            min: Vec3::splat(f32::MAX),
            max: Vec3::splat(f32::MIN),
        };
        if let Some(scene) = doc.default_scene().or_else(|| doc.scenes().next()) {
            for node in scene.nodes() {
                m.add_node(&node, Mat4::IDENTITY, &buffers);
            }
        }
        if m.meshes.is_empty() {
            m.min = Vec3::ZERO;
            m.max = Vec3::ZERO;
        }
        Ok(m)
    }

    fn add_node(&mut self, node: &gltf::Node, parent: Mat4, buffers: &[gltf::buffer::Data]) {
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            let nmat = Mat3::from_mat4(world).inverse().transpose();
            let name = node.name().or(mesh.name()).unwrap_or("unnamed").to_string();
            for prim in mesh.primitives().filter(|p| p.mode() == gltf::mesh::Mode::Triangles) {
                let r = prim.reader(|b| Some(&buffers[b.index()]));
                let Some(pos) = r.read_positions() else { continue };
                let positions: Vec<Vec3> = pos.map(|p| world.transform_point3(p.into())).collect();
                for p in &positions {
                    self.min = self.min.min(*p);
                    self.max = self.max.max(*p);
                }
                let indices = match r.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                self.meshes.push(Mesh {
                    name: name.clone(),
                    normals: r.read_normals().map(|n| n.map(|n| (nmat * Vec3::from(n)).normalize_or_zero()).collect()).unwrap_or_default(),
                    uvs: r.read_tex_coords(0).map(|t| t.into_f32().map(Vec2::from).collect()).unwrap_or_default(),
                    positions,
                    indices,
                    material: prim.material().index(),
                });
            }
        }
        for child in node.children() {
            self.add_node(&child, world, buffers);
        }
    }

    pub fn stats(&self) -> Stats {
        let mut names: Vec<&str> = self.meshes.iter().map(|m| m.name.as_str()).collect();
        names.dedup();
        Stats {
            tris: self.meshes.iter().map(Mesh::tri_count).sum(),
            verts: self.meshes.iter().map(|m| m.positions.len()).sum(),
            objects: names.len(),
            materials: self.materials.len(),
            size: self.max - self.min,
        }
    }
}

fn material(m: gltf::Material) -> Material {
    let pbr = m.pbr_metallic_roughness();
    Material {
        name: m.name().unwrap_or("(unnamed)").to_string(),
        base_color: pbr.base_color_factor(),
        maps: [
            pbr.base_color_texture().map(|t| t.texture().source().index()),
            m.normal_texture().map(|t| t.texture().source().index()),
            pbr.metallic_roughness_texture().map(|t| t.texture().source().index()),
            m.occlusion_texture().map(|t| t.texture().source().index()),
            m.emissive_texture().map(|t| t.texture().source().index()),
        ],
    }
}

fn to_rgba(img: gltf::image::Data) -> Image {
    use gltf::image::Format::*;
    let px = img.pixels;
    let rgba = match img.format {
        R8G8B8A8 => px,
        R8G8B8 => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        R8G8 => px.chunks(2).flat_map(|c| [c[0], c[1], 0, 255]).collect(),
        R8 => px.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        // 16-bit / float formats: keep the high byte of each channel.
        R16G16B16A16 => px.chunks(8).flat_map(|c| [c[1], c[3], c[5], c[7]]).collect(),
        R16G16B16 => px.chunks(6).flat_map(|c| [c[1], c[3], c[5], 255]).collect(),
        _ => vec![255; (img.width * img.height * 4) as usize],
    };
    Image { width: img.width, height: img.height, rgba }
}
