use crate::catalog::{Item, Kind};
use adw::prelude::*;
use gtk::cairo::{self, Context};
use std::{cell::RefCell, collections::HashMap};

thread_local! { static TEXTURES: RefCell<HashMap<(String, i32, i32, bool), gtk::gdk::Texture>> = RefCell::default(); }
pub fn clear_cache() {
    TEXTURES.with(|c| c.borrow_mut().clear());
}

fn color(hex: &str) -> (f64, f64, f64) {
    let n = hex
        .strip_prefix('#')
        .and_then(|h| u32::from_str_radix(h, 16).ok())
        .unwrap_or(0x303038);
    (
        ((n >> 16) & 255) as f64 / 255.,
        ((n >> 8) & 255) as f64 / 255.,
        (n & 255) as f64 / 255.,
    )
}
fn rounded(cr: &Context, w: f64, h: f64) {
    let r = 10.;
    cr.new_sub_path();
    for (x, y, start) in [
        (w - r, r, -90.),
        (w - r, h - r, 0.),
        (r, h - r, 90.),
        (r, r, 180.),
    ] {
        cr.arc(
            x,
            y,
            r,
            f64::to_radians(start),
            f64::to_radians(start + 90.),
        );
    }
    cr.close_path();
    cr.clip();
}
pub fn picture(file: &str, title: &str, width: i32, height: i32) -> gtk::Picture {
    fitted_picture(file, title, width, height, false)
}
pub fn fitted_picture(
    file: &str,
    title: &str,
    width: i32,
    height: i32,
    contain: bool,
) -> gtk::Picture {
    let key = (file.to_owned(), width, height, contain);
    if let Some(texture) = TEXTURES.with(|c| c.borrow().get(&key).cloned()) {
        return picture_for_texture(&texture, title, width, height, contain);
    }
    let source = if let Some(pixels) = crate::assets::pixels(file) {
        gtk::gdk_pixbuf::Pixbuf::from_mut_slice(
            pixels.data,
            gtk::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            pixels.width,
            pixels.height,
            pixels.width * 4,
        )
    } else {
        let picture = gtk::Picture::new();
        picture.set_size_request(width, height);
        picture.add_css_class("portrait-placeholder");
        crate::home::accessible_label(&picture, &format!("Image unavailable: {title}"));
        return picture;
    };
    let scale = if contain {
        (width as f64 / source.width() as f64).min(height as f64 / source.height() as f64)
    } else {
        (width as f64 / source.width() as f64).max(height as f64 / source.height() as f64)
    };
    let scaled = source
        .scale_simple(
            (source.width() as f64 * scale).ceil() as i32,
            (source.height() as f64 * scale).ceil() as i32,
            gtk::gdk_pixbuf::InterpType::Bilinear,
        )
        .expect("scaled artwork");
    let pixels = if contain {
        scaled
    } else {
        scaled.new_subpixbuf(
            (scaled.width() - width) / 2,
            (scaled.height() - height) / 2,
            width,
            height,
        )
    };
    let texture = gtk::gdk::Texture::for_pixbuf(&pixels);
    TEXTURES.with(|c| c.borrow_mut().insert(key, texture.clone()));
    picture_for_texture(&texture, title, width, height, contain)
}
fn picture_for_texture(
    texture: &gtk::gdk::Texture,
    title: &str,
    width: i32,
    height: i32,
    contain: bool,
) -> gtk::Picture {
    let picture = gtk::Picture::for_paintable(texture);
    picture.set_content_fit(if contain {
        gtk::ContentFit::Contain
    } else {
        gtk::ContentFit::Cover
    });
    picture.set_size_request(width, height);
    picture.set_can_shrink(true);
    picture.set_halign(gtk::Align::Start);
    picture.set_valign(gtk::Align::Start);
    picture.add_css_class("artwork");
    picture.set_overflow(gtk::Overflow::Hidden);
    crate::home::accessible_label(&picture, title);
    picture
}
pub fn view(item: &Item, width: i32, progress: Option<u8>) -> gtk::Widget {
    if let Some(art) = crate::catalog::art_for(&item.id) {
        let overlay = gtk::Overlay::new();
        overlay.set_halign(gtk::Align::Start);
        overlay.set_valign(gtk::Align::Start);
        overlay.add_css_class("artwork");
        overlay.set_overflow(gtk::Overflow::Hidden);
        overlay.set_child(Some(&picture(
            crate::catalog::text(&art, "file"),
            crate::catalog::text(&art, "label"),
            width,
            width * 3 / 2,
        )));
        if let Some(value) = progress {
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(f64::from(value) / 100.0);
            bar.set_valign(gtk::Align::End);
            overlay.add_overlay(&bar);
        }
        return overlay.upcast();
    }
    let area = gtk::DrawingArea::builder()
        .content_width(width)
        .content_height(width * 3 / 2)
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .accessible_role(gtk::AccessibleRole::Presentation)
        .build();
    let item = item.clone();
    area.set_draw_func(move |_, cr, width, height| {
        let (w, h) = (width as f64, height as f64);
        rounded(cr, w, h);
        let (r, g, b) = color(&item.colors.0);
        let (lr, lg, lb) = color(&item.colors.1);
        let gradient = cairo::LinearGradient::new(0., 0., w * 0.3, h * 1.6);
        gradient.add_color_stop_rgb(0.2, r, g, b);
        gradient.add_color_stop_rgb(1., lr, lg, lb);
        let _ = cr.set_source(&gradient);
        let _ = cr.paint();
        cr.set_source_rgba(lr, lg, lb, 0.6);
        cr.set_line_width(1.);
        cr.arc(
            w * 0.705,
            h * 0.14 + w * 0.425,
            w * 0.425,
            0.,
            std::f64::consts::TAU,
        );
        let _ = cr.stroke();
        let _ = cr.save();
        cr.translate(w * 0.5, h * 0.7);
        cr.rotate(-0.56);
        let landscape = cairo::LinearGradient::new(-w, 0., w, 0.);
        landscape.add_color_stop_rgba(0., 0., 0., 0., 0.1);
        landscape.add_color_stop_rgba(1., lr, lg, lb, 0.55);
        let _ = cr.set_source(&landscape);
        cr.rectangle(-w, h * -0.2, w * 2., h);
        let _ = cr.fill();
        let _ = cr.restore();
        cr.select_font_face(
            "Georgia",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Normal,
        );
        cr.set_font_size(w * 0.175);
        cr.set_source_rgb(1., 0.96, 0.9);
        let title = if item.kind == Kind::Episode {
            item.meta.split(" · ").next().unwrap_or(&item.title)
        } else {
            &item.title
        };
        let longest = title
            .split_whitespace()
            .filter_map(|word| cr.text_extents(word).ok().map(|e| e.width()))
            .fold(0.0_f64, f64::max);
        if longest > w * 0.76 {
            cr.set_font_size(w * 0.175 * w * 0.76 / longest);
        }
        let mut line = String::new();
        let mut y = h * 0.47;
        for word in title.split_whitespace() {
            let next = if line.is_empty() {
                word.into()
            } else {
                format!("{line} {word}")
            };
            if cr.text_extents(&next).map(|e| e.width()).unwrap_or(0.) > w * 0.8 && !line.is_empty()
            {
                cr.move_to(w * 0.12, y);
                let _ = cr.show_text(&line);
                y += w * 0.185;
                line = word.into();
            } else {
                line = next;
            }
        }
        cr.move_to(w * 0.12, y);
        let _ = cr.show_text(&line);
        cr.select_font_face(
            "sans-serif",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Normal,
        );
        cr.set_font_size(6.);
        cr.set_source_rgba(1., 1., 1., 0.6);
        cr.move_to(w * 0.12, h - 18.);
        let _ = cr.show_text("POSTER NOT INCLUDED");
        if let Some(progress) = progress {
            cr.set_source_rgba(1., 1., 1., 0.2);
            cr.rectangle(0., h - 4., w, 4.);
            let _ = cr.fill();
            cr.set_source_rgb(0.91, 0.91, 0.91);
            cr.rectangle(0., h - 4., w * f64::from(progress) / 100., 4.);
            let _ = cr.fill();
        }
    });
    area.upcast()
}
