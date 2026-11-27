//! A view as tools and overlays see it, whichever view it is: where it is
//! on screen, how points of the model map onto it, and what is under a
//! position (exactly, from the CPU engine).

use llevant::vec3::dot;
use llevant::{Camera, Model, Vec3, Visibility};

use crate::pick::{pick_plane_point, pick_ray, Picked, SURFACE_REACH};
use crate::slice::PlaneView;

/// Which of the two views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKind {
    ThreeD,
    Slice,
}

/// How model points map onto a view.
#[derive(Clone, Debug)]
pub enum Projection {
    /// The 3D view's camera (its `width × height` pixels fill the view).
    Camera(Camera),
    Plane(PlaneView),
}

/// Where a model point shows in a view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnScreen {
    pub pos: egui::Pos2,
    /// In the slice: how far off the plane the point is, in screen points
    /// (0 in 3D).
    pub off_plane: f32,
}

/// A view this frame.
pub struct ViewPort {
    pub kind: ViewKind,
    /// The pointer's interaction with the view (its `rect` is the view).
    pub response: egui::Response,
    pub projection: Projection,
}

impl ViewPort {
    pub fn rect(&self) -> egui::Rect {
        self.response.rect
    }

    /// Where `p` shows; `None` behind the 3D camera.
    pub fn to_screen(&self, p: Vec3) -> Option<OnScreen> {
        let rect = self.rect();
        match &self.projection {
            Projection::Camera(c) => {
                let [f, r, u] = c.basis();
                let v = [0, 1, 2].map(|k| p[k] - c.eye[k]);
                let z = dot(v, f);
                if !(z > 0.0) {
                    return None;
                }
                // Half the view's height where the point is.
                let half = match c.ortho {
                    Some(h) => 0.5 * h,
                    None => z * (0.5 * c.fov_deg).to_radians().tan(),
                };
                let aspect = c.width as f64 / c.height as f64;
                let x = dot(v, r) / (half * aspect);
                let y = dot(v, u) / half;
                let pos = rect.center()
                    + egui::vec2(
                        (x * 0.5 * rect.width() as f64) as f32,
                        (-y * 0.5 * rect.height() as f64) as f32,
                    );
                Some(OnScreen {
                    pos,
                    off_plane: 0.0,
                })
            }
            Projection::Plane(plane) => {
                let (pos, off) = plane.to_screen(rect, p);
                Some(OnScreen {
                    pos,
                    off_plane: (off / plane.cm_per_point(rect)) as f32,
                })
            }
        }
    }

    /// The camera pixel under `pos` (3D view only).
    pub fn pixel(&self, pos: egui::Pos2) -> Option<(usize, usize)> {
        let Projection::Camera(c) = &self.projection else {
            return None;
        };
        let rect = self.rect();
        let x = (pos.x - rect.min.x) as f64 / rect.width() as f64 * c.width as f64;
        let y = (pos.y - rect.min.y) as f64 / rect.height() as f64 * c.height as f64;
        Some((
            (x.max(0.0) as usize).min(c.width - 1),
            (y.max(0.0) as usize).min(c.height - 1),
        ))
    }

    /// [`ViewPort::pick`] at `pos`, to run on another thread.
    pub fn picker(
        &self,
        pos: egui::Pos2,
    ) -> impl FnOnce(&Model, &Visibility) -> Option<Picked> + Send + 'static {
        let vp = Detached {
            projection: self.projection.clone(),
            rect: self.rect(),
            pixel: self.pixel(pos),
        };
        move |m: &Model, vis: &Visibility| vp.pick(m, vis, pos)
    }

    /// What is at `pos`: in 3D the first visible point along its ray (`None`
    /// over the background), in the slice the point of the plane; with the
    /// surface there (in the slice, the nearest within
    /// [`SURFACE_REACH`] screen points).
    pub fn pick(&self, m: &Model, vis: &Visibility, pos: egui::Pos2) -> Option<Picked> {
        Detached {
            projection: self.projection.clone(),
            rect: self.rect(),
            pixel: self.pixel(pos),
        }
        .pick(m, vis, pos)
    }
}

/// What a pick needs of a [`ViewPort`] (which holds egui's response).
struct Detached {
    projection: Projection,
    rect: egui::Rect,
    /// The camera pixel under the position picked.
    pixel: Option<(usize, usize)>,
}

impl Detached {
    fn pick(&self, m: &Model, vis: &Visibility, pos: egui::Pos2) -> Option<Picked> {
        match &self.projection {
            Projection::Camera(c) => {
                let (i, j) = self.pixel?;
                pick_ray(m, vis, c, i, j)
            }
            Projection::Plane(plane) => {
                let rect = self.rect;
                Some(pick_plane_point(
                    m,
                    plane.from_screen(rect, pos),
                    plane.frame.u,
                    plane.frame.v,
                    SURFACE_REACH * plane.cm_per_point(rect),
                    vis.level,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llevant::Aabb;

    #[test]
    fn camera_projection_inverts_pixel_rays() {
        let ctx = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(300.0, 200.0));
        for ortho in [None, Some(5.0)] {
            let mut cam = Camera::fit(
                &Aabb {
                    lo: [-1.0; 3],
                    hi: [2.0, 1.0, 3.0],
                },
                [0.3, 1.0, -0.4],
                600,
                400,
            );
            cam.ortho = ortho;
            let mut vp = None;
            // The frame's textures are not uploaded: dropped on purpose.
            ctx.run_ui(Default::default(), |ui| {
                let response = ui.interact(rect, egui::Id::new("v"), egui::Sense::hover());
                vp = Some(ViewPort {
                    kind: ViewKind::ThreeD,
                    response,
                    projection: Projection::Camera(cam.clone()),
                });
            })
            .textures_delta
            .clear();
            let vp = vp.unwrap();
            for (i, j) in [(0, 0), (599, 399), (300, 17), (45, 321)] {
                let p = cam.point(i, j, 7.0);
                let s = vp.to_screen(p).unwrap();
                assert_eq!(vp.pixel(s.pos), Some((i, j)), "{ortho:?}: {i} {j}");
                let centre = rect.min + egui::vec2((i as f32 + 0.5) / 2.0, (j as f32 + 0.5) / 2.0);
                assert!((s.pos - centre).length() < 1e-3, "{:?} {centre:?}", s.pos);
            }
            // Behind the eye.
            let f = cam.basis()[0];
            assert!(vp.to_screen([0, 1, 2].map(|k| cam.eye[k] - f[k])).is_none());
        }
    }
}
