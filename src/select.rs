//! Region capture: which triangles fall inside a screen-space box/lasso and are visible.

use crate::glb::Mesh;
use glam::{Mat4, Vec2, Vec3};

pub struct View {
    pub view_proj: Mat4,
    pub eye: Vec3,
    pub near: f32,
    pub far: f32,
    pub size: Vec2,
}

impl View {
    /// World point -> (screen pixel, NDC depth 0..1). None if behind the camera.
    pub fn project(&self, p: Vec3) -> Option<(Vec2, f32)> {
        let c = self.view_proj * p.extend(1.0);
        if c.w <= 0.0 {
            return None;
        }
        let n = c.truncate() / c.w;
        Some((Vec2::new((n.x + 1.0) * 0.5 * self.size.x, (1.0 - n.y) * 0.5 * self.size.y), n.z))
    }

    /// NDC depth -> view distance (inverse of glam's perspective_rh 0..1 mapping).
    fn linear(&self, z: f32) -> f32 {
        self.far * self.near / (self.far - z * (self.far - self.near))
    }
}

/// Triangles per mesh whose centroid is inside `poly`, faces the camera and isn't hidden.
/// `depth(px)` returns the depth-buffer NDC value at that pixel (None = skip occlusion test).
pub fn pick(meshes: &[Mesh], view: &View, poly: &[Vec2], depth: &dyn Fn(Vec2) -> Option<f32>) -> Vec<Vec<u32>> {
    meshes
        .iter()
        .map(|m| {
            (0..m.tri_count() as u32)
                .filter(|&t| {
                    let [a, b, c] = m.tri(t as usize);
                    let centre = (a + b + c) / 3.0;
                    if (b - a).cross(c - a).dot(view.eye - centre) <= 0.0 {
                        return false;
                    }
                    let Some((px, z)) = view.project(centre) else { return false };
                    if !inside(px, poly) {
                        return false;
                    }
                    depth(px).is_none_or(|d| {
                        let (seen, mine) = (view.linear(d), view.linear(z));
                        seen >= mine * 0.99 - 1e-4
                    })
                })
                .collect()
        })
        .collect()
}

/// Even-odd point-in-polygon.
pub fn inside(p: Vec2, poly: &[Vec2]) -> bool {
    let mut hit = false;
    let mut j = poly.len().wrapping_sub(1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            hit = !hit;
        }
        j = i;
    }
    hit
}

pub fn rect(a: Vec2, b: Vec2) -> Vec<Vec2> {
    vec![a, Vec2::new(b.x, a.y), b, Vec2::new(a.x, b.y)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2x2x2 cube, outward CCW faces, 12 triangles; tris 0,1 are the +Z face.
    fn cube() -> Mesh {
        let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let faces = [
            [p(-1., -1., 1.), p(1., -1., 1.), p(1., 1., 1.), p(-1., 1., 1.)],     // +Z
            [p(1., -1., -1.), p(-1., -1., -1.), p(-1., 1., -1.), p(1., 1., -1.)], // -Z
            [p(1., -1., 1.), p(1., -1., -1.), p(1., 1., -1.), p(1., 1., 1.)],     // +X
            [p(-1., -1., -1.), p(-1., -1., 1.), p(-1., 1., 1.), p(-1., 1., -1.)], // -X
            [p(-1., 1., 1.), p(1., 1., 1.), p(1., 1., -1.), p(-1., 1., -1.)],     // +Y
            [p(-1., -1., -1.), p(1., -1., -1.), p(1., -1., 1.), p(-1., -1., 1.)], // -Y
        ];
        let mut m = Mesh { name: "Box".into(), positions: vec![], normals: vec![], uvs: vec![], indices: vec![], material: None };
        for f in faces {
            let b = m.positions.len() as u32;
            m.positions.extend(f);
            m.indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
        }
        m
    }

    fn front_view() -> View {
        let eye = Vec3::new(0., 0., 5.);
        let (near, far) = (0.1, 100.);
        let vp = Mat4::perspective_rh(1.0, 1.0, near, far) * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        View { view_proj: vp, eye, near, far, size: Vec2::new(400., 400.) }
    }

    #[test]
    fn box_over_front_face_picks_its_two_triangles() {
        let (m, v) = (cube(), front_view());
        let front_depth = v.project(Vec3::new(0., 0., 1.)).unwrap().1;
        let sel = pick(&[m], &v, &rect(Vec2::new(100., 100.), Vec2::new(300., 300.)), &|_| Some(front_depth));
        assert_eq!(sel[0], vec![0, 1]);
    }

    #[test]
    fn occluded_triangles_are_skipped() {
        let (m, v) = (cube(), front_view());
        // Something nearer than the cube everywhere.
        let near_depth = v.project(Vec3::new(0., 0., 3.)).unwrap().1;
        let sel = pick(&[m], &v, &rect(Vec2::ZERO, Vec2::new(400., 400.)), &|_| Some(near_depth));
        assert!(sel[0].is_empty());
    }

    #[test]
    fn lasso_outside_model_picks_nothing() {
        let sel = pick(&[cube()], &front_view(), &rect(Vec2::new(0., 0.), Vec2::new(20., 20.)), &|_| None);
        assert!(sel[0].is_empty());
    }

    #[test]
    fn point_in_polygon() {
        let tri = [Vec2::new(0., 0.), Vec2::new(10., 0.), Vec2::new(0., 10.)];
        assert!(inside(Vec2::new(2., 2.), &tri));
        assert!(!inside(Vec2::new(8., 8.), &tri));
    }
}
