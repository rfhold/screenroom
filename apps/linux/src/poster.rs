use crate::catalog::{Item, Kind};
use adw::prelude::*;
use gtk::cairo::{self, Context};
use std::cell::RefCell;

#[path = "poster_progress.rs"]
mod progress;

thread_local! { static TEXTURES: RefCell<crate::cache::Cache<(String, i32, i32, bool), gtk::gdk::Texture>> = RefCell::new(crate::cache::Cache::new(crate::cache::TEXTURE_ENTRIES, crate::cache::TEXTURE_BYTES)); }
type TextureKey = (String, i32, i32, bool);
thread_local! { static LIVE_TEXTURES: RefCell<crate::cache::LiveIndex<TextureKey, gtk::glib::WeakRef<gtk::gdk::Texture>>> = RefCell::default(); }
type PendingPicture = (
    gtk::glib::WeakRef<gtk::Picture>,
    String,
    String,
    i32,
    i32,
    bool,
);
thread_local! { static PENDING: RefCell<Vec<PendingPicture>> = RefCell::default(); }
pub fn refresh_pending() {
    LIVE_TEXTURES.with(|textures| {
        textures
            .borrow_mut()
            .prune(|texture| texture.upgrade().is_some())
    });
    let pending = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (weak, file, title, width, height, contain) in pending {
        if let Some(picture) = weak.upgrade() {
            if crate::assets::contains(&file) {
                let loaded = fitted_picture(&file, &title, width, height, contain);
                picture.set_paintable(loaded.paintable().as_ref());
                picture.set_content_fit(if contain {
                    gtk::ContentFit::Contain
                } else {
                    gtk::ContentFit::Cover
                });
                crate::home::accessible_label(&picture, &title);
            } else {
                PENDING.with(|p| {
                    p.borrow_mut()
                        .push((weak, file, title, width, height, contain))
                });
            }
        }
    }
}
pub fn needs_artwork(file: &str) -> bool {
    PENDING.with(|pending| {
        pending
            .borrow()
            .iter()
            .any(|(widget, url, ..)| url == file && widget.upgrade().is_some())
    })
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
    if let Some(texture) = TEXTURES.with(|c| c.borrow_mut().get(&key).cloned()) {
        return picture_for_texture(&texture, title, width, height, contain);
    }
    if let Some(texture) =
        LIVE_TEXTURES.with(|textures| textures.borrow().get(&key, gtk::glib::WeakRef::upgrade))
    {
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
        PENDING.with(|p| {
            p.borrow_mut().push((
                picture.downgrade(),
                file.into(),
                title.into(),
                width,
                height,
                contain,
            ))
        });
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
        scaled
            .new_subpixbuf(
                (scaled.width() - width) / 2,
                (scaled.height() - height) / 2,
                width,
                height,
            )
            .copy()
            .expect("cropped artwork")
    };
    let texture = gtk::gdk::Texture::for_pixbuf(&pixels);
    LIVE_TEXTURES.with(|textures| {
        textures
            .borrow_mut()
            .insert(key.clone(), texture.downgrade())
    });
    let bytes = (texture.width() as usize)
        .saturating_mul(texture.height() as usize)
        .saturating_mul(4);
    TEXTURES.with(|c| c.borrow_mut().insert(key, texture.clone(), bytes));
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
pub fn view(item: &Item, width: i32, progress: Option<u8>, announce: bool) -> gtk::Widget {
    view_context(item, width, progress, announce, None)
}
pub fn view_context(
    item: &Item,
    width: i32,
    progress: Option<u8>,
    announce: bool,
    heading: Option<String>,
) -> gtk::Widget {
    let overlay = gtk::Overlay::builder()
        .halign(gtk::Align::Start)
        .valign(gtk::Align::Start)
        .build();
    let poster_title = if item.kind == Kind::Episode {
        format!("{}, {}", heading.clone().unwrap_or_else(|| crate::presentation::episode_labels(&item.meta, false).heading()), item.title)
    } else {
        item.title.clone()
    };
    let artwork = artwork_view(item, width, progress, heading);
    overlay.set_child(Some(&artwork));
    if announce {
        crate::home::accessible_label(
            &overlay,
            &format!("{}, Poster{}", poster_title, {
                let state = item.playback_accessibility(progress);
                if state.is_empty() {
                    state
                } else {
                    format!(", {state}")
                }
            }),
        );
    }
    if item.is_watched() {
        let badge = gtk::DrawingArea::builder()
            .content_width(26)
            .content_height(26)
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(8)
            .margin_end(8)
            .accessible_role(gtk::AccessibleRole::Presentation)
            .can_target(false)
            .build();
        badge.set_draw_func(|_, cr, _, _| {
            cr.set_source_rgba(22. / 255., 37. / 255., 31. / 255., 0.93);
            cr.arc(13., 13., 13., 0., std::f64::consts::TAU);
            let _ = cr.fill();
            cr.set_source_rgb(227. / 255., 244. / 255., 233. / 255.);
            cr.set_line_width(2.);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_join(cairo::LineJoin::Round);
            cr.move_to(7.5, 13.3);
            cr.line_to(11.4, 17.2);
            cr.line_to(19., 9.1);
            let _ = cr.stroke();
        });
        overlay.add_overlay(&badge);
    }
    overlay.upcast()
}
fn artwork_view(
    item: &Item,
    width: i32,
    progress: Option<u8>,
    heading_override: Option<String>,
) -> gtk::Widget {
    if let Some(art) = crate::catalog::art_for(&item.id) {
        let title = if item.kind == Kind::Episode {
            format!(
                "{}, {}",
                heading_override.clone().unwrap_or_else(|| {
                    crate::presentation::episode_labels(&item.meta, false).heading()
                }),
                item.title
            )
        } else {
            crate::catalog::text(&art, "label").into()
        };
        let overlay = gtk::Overlay::new();
        overlay.set_halign(gtk::Align::Start);
        overlay.set_valign(gtk::Align::Start);
        overlay.add_css_class("artwork");
        overlay.set_overflow(gtk::Overflow::Hidden);
        overlay.set_child(Some(&picture(
            crate::catalog::text(&art, "file"),
            &title,
            width,
            width * 3 / 2,
        )));
        if let Some(value) = progress {
            let bar = gtk::DrawingArea::builder()
                .content_height(progress::HEIGHT)
                .halign(gtk::Align::Fill)
                .valign(gtk::Align::End)
                .accessible_role(gtk::AccessibleRole::Presentation)
                .can_target(false)
                .build();
            bar.set_draw_func(move |_, cr, width, height| {
                progress::paint(cr, f64::from(width), f64::from(height), Some(value));
            });
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
        let heading;
        let title = if item.kind == Kind::Episode {
            heading = heading_override.clone().unwrap_or_else(|| {
                crate::presentation::episode_labels(&item.meta, false).heading()
            });
            &heading
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
        progress::paint(cr, w, h, progress);
    });
    area.upcast()
}
