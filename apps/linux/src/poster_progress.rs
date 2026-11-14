use super::cairo::Context;

pub const HEIGHT: i32 = 4;

pub fn paint(cr: &Context, width: f64, height: f64, progress: Option<u8>) {
    let Some(value) = progress else { return };
    cr.set_source_rgba(1., 1., 1., 53. / 255.);
    cr.rectangle(0., height - f64::from(HEIGHT), width, f64::from(HEIGHT));
    let _ = cr.fill();
    cr.set_source_rgb(233. / 255., 233. / 255., 233. / 255.);
    cr.rectangle(
        0.,
        height - f64::from(HEIGHT),
        width * f64::from(value.min(100)) / 100.,
        f64::from(HEIGHT),
    );
    let _ = cr.fill();
}

#[cfg(test)]
mod tests {
    use super::super::cairo::{Context, Format, ImageSurface};
    use super::*;

    fn pixels(progress: Option<u8>, scale: i32) -> Vec<u32> {
        let width = 100 * scale;
        let height = 12 * scale;
        let mut surface = ImageSurface::create(Format::ARgb32, width, height).unwrap();
        surface.set_device_scale(f64::from(scale), f64::from(scale));
        let cr = Context::new(&surface).unwrap();
        paint(&cr, 100., 12., progress);
        drop(cr);
        let stride = surface.stride() as usize;
        let data = surface.data().unwrap();
        (0..height as usize)
            .flat_map(|y| {
                let row = &data[y * stride..y * stride + width as usize * 4];
                row.chunks_exact(4)
                    .map(|pixel| u32::from_ne_bytes(pixel.try_into().unwrap()))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn unknown_progress_is_hidden_but_known_zero_keeps_track() {
        assert!(pixels(None, 1).iter().all(|&pixel| pixel == 0));
        let pixels = pixels(Some(0), 1);
        assert!(pixels[..800].iter().all(|&pixel| pixel == 0));
        assert!(pixels[800..].iter().all(|&pixel| pixel == 0x35353535));
    }

    #[test]
    fn fill_has_exact_color_bounds_and_four_logical_pixel_height() {
        for scale in [1, 2] {
            let pixels = pixels(Some(37), scale);
            let width = 100 * scale as usize;
            for (y, row) in pixels.chunks_exact(width).enumerate() {
                if y < 8 * scale as usize {
                    assert!(row.iter().all(|&pixel| pixel == 0));
                } else {
                    let end = 37 * scale as usize;
                    assert!(row[..end].iter().all(|&pixel| pixel == 0xffe9e9e9));
                    assert!(row[end..].iter().all(|&pixel| pixel == 0x35353535));
                }
            }
        }
    }

    #[test]
    fn full_and_out_of_range_progress_fill_only_the_track() {
        for value in [100, 101, 255] {
            let pixels = pixels(Some(value), 1);
            assert!(pixels[..800].iter().all(|&pixel| pixel == 0));
            assert!(pixels[800..].iter().all(|&pixel| pixel == 0xffe9e9e9));
        }
    }
}
