use gpui::{Bounds, Pixels, RenderImage, Rgba, Window, point, px, size};
use image::{Frame, RgbaImage};
use std::sync::Arc;

#[derive(Default)]
pub struct CornerCache {
    key: Option<(Rgba, Rgba, f32)>,
    images: Vec<Arc<RenderImage>>,
}

impl CornerCache {
    pub fn images(
        &mut self,
        outer: Rgba,
        shell: Rgba,
        window: &mut Window,
    ) -> Vec<Arc<RenderImage>> {
        let scale = window.scale_factor();
        let key = (outer, shell, scale);
        if self.key != Some(key) {
            for image in self.images.drain(..) {
                if let Err(error) = window.drop_image(image) {
                    eprintln!("Could not release shell corner image: {error}");
                }
            }
            let side = (crate::SHELL_RADIUS * scale).ceil() as u32;
            for (flip_x, flip_y) in [(false, false), (true, false), (false, true), (true, true)] {
                let pixels = RgbaImage::from_fn(side, side, |column, row| {
                    let x = if flip_x { side - 1 - column } else { column };
                    let y = if flip_y { side - 1 - row } else { row };
                    let distance = ((x as f32 + 0.5 - side as f32).powi(2)
                        + (y as f32 + 0.5 - side as f32).powi(2))
                    .sqrt();
                    let coverage = (0.5 + side as f32 - distance).clamp(0.0, 1.0);
                    let color = mix_materials(outer, shell, coverage);
                    let byte = |value: f32| (value * 255.0).round() as u8;
                    // GPUI's image atlas expects straight-alpha BGRA bytes.
                    image::Rgba([byte(color.b), byte(color.g), byte(color.r), byte(color.a)])
                });
                self.images
                    .push(Arc::new(RenderImage::new(vec![Frame::new(pixels)])));
            }
            self.key = Some(key);
        }
        self.images.clone()
    }
}

// Composite the two mutually exclusive coverages before painting. Painting two
// antialiased edges separately darkens their shared pixels with source-over RGB.
fn mix_materials(outer: Rgba, shell: Rgba, coverage: f32) -> Rgba {
    if coverage == 0.0 {
        return outer;
    }
    if coverage == 1.0 {
        return shell;
    }
    let outer_weight = outer.a * (1.0 - coverage);
    let shell_weight = shell.a * coverage;
    let alpha = outer_weight + shell_weight;
    if alpha == 0.0 {
        return Rgba::default();
    }
    let channel = |outer, shell| (outer * outer_weight + shell * shell_weight) / alpha;
    Rgba {
        r: channel(outer.r, shell.r),
        g: channel(outer.g, shell.g),
        b: channel(outer.b, shell.b),
        a: alpha,
    }
}

pub fn paint(
    bounds: Bounds<Pixels>,
    surface: Bounds<Pixels>,
    outer: Rgba,
    shell: Rgba,
    corners: &[Arc<RenderImage>],
    window: &mut Window,
) {
    let radius = px((crate::SHELL_RADIUS * window.scale_factor()).ceil() / window.scale_factor());
    let left = surface.left();
    let right = surface.right();
    let top = surface.top();
    let bottom = surface.bottom();
    let mut fill = |x, y, width, height, color| {
        window.paint_quad(gpui::fill(
            Bounds::new(point(x, y), size(width, height)),
            color,
        ));
    };
    // Rectangular strips and the four corner tiles form a disjoint partition.
    fill(
        bounds.left(),
        bounds.top(),
        bounds.size.width,
        top - bounds.top(),
        outer,
    );
    fill(
        bounds.left(),
        bottom,
        bounds.size.width,
        bounds.bottom() - bottom,
        outer,
    );
    fill(
        bounds.left(),
        top,
        left - bounds.left(),
        surface.size.height,
        outer,
    );
    fill(
        right,
        top,
        bounds.right() - right,
        surface.size.height,
        outer,
    );
    fill(
        left,
        top + radius,
        surface.size.width,
        surface.size.height - radius * 2.0,
        shell,
    );
    fill(
        left + radius,
        top,
        surface.size.width - radius * 2.0,
        radius,
        shell,
    );
    fill(
        left + radius,
        bottom - radius,
        surface.size.width - radius * 2.0,
        radius,
        shell,
    );
    for (origin, image) in [
        point(left, top),
        point(right - radius, top),
        point(left, bottom - radius),
        point(right - radius, bottom - radius),
    ]
    .into_iter()
    .zip(corners)
    {
        if let Err(error) = window.paint_image(
            Bounds::new(origin, size(radius, radius)),
            px(0.0).into(),
            image.clone(),
            0,
            false,
        ) {
            eprintln!("Could not paint shell corner: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::rgba;

    #[test]
    fn opaque_corner_transition_has_no_dark_seam() {
        let outer = rgba(0xeaeaedff);
        let shell = rgba(0xfbfbfbff);
        let mixed = mix_materials(outer, shell, 0.5);
        assert_eq!(mixed.a, 1.0);
        assert!(mixed.r >= outer.r && mixed.r <= shell.r);
        assert!(mixed.g >= outer.g && mixed.g <= shell.g);
        assert!(mixed.b >= outer.b && mixed.b <= shell.b);
    }

    #[test]
    fn corner_transition_preserves_premultiplied_material_and_alpha() {
        let outer = rgba(0x3c3d4eb8);
        let shell = rgba(0x1a1a1a99);
        assert_eq!(mix_materials(outer, shell, 0.0), outer);
        assert_eq!(mix_materials(outer, shell, 1.0), shell);
        let mixed = mix_materials(outer, shell, 0.5);
        assert!((mixed.a - (outer.a + shell.a) * 0.5).abs() < 0.00001);
        assert!(
            (mixed.r * mixed.a - (outer.r * outer.a + shell.r * shell.a) * 0.5).abs() < 0.00001
        );
        assert_eq!(mix_materials(outer, Rgba::default(), 1.0), Rgba::default());
    }
}
