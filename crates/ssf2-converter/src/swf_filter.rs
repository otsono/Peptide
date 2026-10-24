//! Flash display filters, applied to rasterized art at convert time.
//!
//! A placement can carry a filter chain, and the filter is part of how the art LOOKS: fourside's
//! moon is ringed by a glow that lives nowhere in its shapes, so dropping the chain drops the halo.
//! Fraymakers has no runtime filters, and the art it loads is a sprite sheet, so the same rule as
//! blend modes applies -- the filter has to happen before the art ships.
//!
//! Flash filters an object AFTER it composites, so a chain on a clip covers the whole subtree at
//! once. Here art arrives one shape at a time, so the chain runs per shape. The two agree except
//! where shapes under one filtered clip OVERLAP, and there the difference is confined to the seam.
//!
//! Blur, glow and drop shadow are built on one operation (blur the source, then read it as colour,
//! as alpha, or as both), which is why they are together here. The rest of Flash's filters are not
//! implemented and pass the art through untouched.

use image::RgbaImage;
use swf::{Filter, Fixed16};

/// Premultiplied linear pixels, so a blur never drags colour out of transparent ground.
type Buf = Vec<[f32; 4]>;

fn to_buf(img: &RgbaImage) -> Buf {
    img.pixels().map(|p| {
        let a = p[3] as f32 / 255.0;
        [p[0] as f32 / 255.0 * a, p[1] as f32 / 255.0 * a, p[2] as f32 / 255.0 * a, a]
    }).collect()
}

fn to_img(buf: &Buf, w: u32, h: u32) -> RgbaImage {
    let mut out = RgbaImage::new(w, h);
    for (i, p) in buf.iter().enumerate() {
        let a = p[3].clamp(0.0, 1.0);
        let un = |c: f32| if a > 0.0 { (c / a).clamp(0.0, 1.0) } else { 0.0 };
        out.put_pixel(i as u32 % w, i as u32 / w,
            image::Rgba([(un(p[0]) * 255.0).round() as u8, (un(p[1]) * 255.0).round() as u8,
                         (un(p[2]) * 255.0).round() as u8, (a * 255.0).round() as u8]));
    }
    out
}

/// Pad on every side so the blur has room to spread into.
fn pad(buf: &Buf, w: usize, h: usize, px: usize, py: usize) -> (Buf, usize, usize) {
    let (nw, nh) = (w + px * 2, h + py * 2);
    let mut out = vec![[0.0f32; 4]; nw * nh];
    for y in 0..h {
        out[(y + py) * nw + px..(y + py) * nw + px + w].copy_from_slice(&buf[y * w..y * w + w]);
    }
    (out, nw, nh)
}

/// One separable box blur pass over a channel-agnostic premultiplied buffer.
fn box_pass(buf: &Buf, w: usize, h: usize, rx: usize, ry: usize) -> Buf {
    let mut mid = vec![[0.0f32; 4]; w * h];
    if rx > 0 {
        let n = (rx * 2 + 1) as f32;
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0.0f32; 4];
                for k in x.saturating_sub(rx)..=(x + rx).min(w - 1) {
                    let p = buf[y * w + k];
                    for c in 0..4 { acc[c] += p[c]; }
                }
                for c in 0..4 { mid[y * w + x][c] = acc[c] / n; }
            }
        }
    } else { mid.copy_from_slice(buf); }
    if ry == 0 { return mid; }
    let mut out = vec![[0.0f32; 4]; w * h];
    let n = (ry * 2 + 1) as f32;
    for x in 0..w {
        for y in 0..h {
            let mut acc = [0.0f32; 4];
            for k in y.saturating_sub(ry)..=(y + ry).min(h - 1) {
                let p = mid[k * w + x];
                for c in 0..4 { acc[c] += p[c]; }
            }
            for c in 0..4 { out[y * w + x][c] = acc[c] / n; }
        }
    }
    out
}

fn blur(buf: &Buf, w: usize, h: usize, rx: usize, ry: usize, passes: u8) -> Buf {
    let mut cur = buf.clone();
    for _ in 0..passes.max(1) { cur = box_pass(&cur, w, h, rx, ry); }
    cur
}

/// Flash states a blur radius in the object's OWN pixels; the tile is already at its placed size,
/// so the radius scales with it.
fn radius(v: Fixed16, scale: f64) -> usize {
    ((f64::from(v) * scale).abs() / 2.0).round().max(0.0) as usize
}

/// The number of box passes the filter asks for (Flash calls it quality).
fn passes(flags: u8) -> u8 { (flags & 0b11111).clamp(1, 3) }

/// Draw `src` over `dst` in place, both premultiplied.
fn over(dst: &mut Buf, src: &Buf) {
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        let ia = 1.0 - s[3];
        for c in 0..4 { d[c] = s[c] + d[c] * ia; }
    }
}

/// Read a blurred buffer's ALPHA as the coverage of a solid colour: the shape of a glow or shadow.
fn tint(blurred: &Buf, color: swf::Color, strength: f32) -> Buf {
    let (r, g, b) = (color.r as f32 / 255.0, color.g as f32 / 255.0, color.b as f32 / 255.0);
    let ca = color.a as f32 / 255.0;
    blurred.iter().map(|p| {
        let a = (p[3] * strength).clamp(0.0, 1.0) * ca;
        [r * a, g * a, b * a, a]
    }).collect()
}

/// Apply a filter chain to a rasterized tile.
///
/// `scale_x`/`scale_y` carry the tile's size relative to the art the filter was authored against.
/// Returns the new image and how far its top-left moved from the input's, which is what the caller
/// adds to the tile's placement so the art stays put while the filter grows around it.
pub fn apply(img: &RgbaImage, filters: &[Filter], scale_x: f64, scale_y: f64)
    -> (RgbaImage, i64, i64)
{
    let (mut w, mut h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 { return (img.clone(), 0, 0); }
    let mut buf = to_buf(img);
    let (mut dx, mut dy) = (0i64, 0i64);
    for f in filters {
        let (rx, ry, n) = match f {
            Filter::BlurFilter(b) => (radius(b.blur_x, scale_x), radius(b.blur_y, scale_y), passes(b.flags.bits())),
            Filter::GlowFilter(g) => (radius(g.blur_x, scale_x), radius(g.blur_y, scale_y), passes(g.flags.bits())),
            Filter::DropShadowFilter(d) => (radius(d.blur_x, scale_x), radius(d.blur_y, scale_y), passes(d.flags.bits())),
            // every other filter passes the art through as it stands
            _ => continue,
        };
        let (px, py) = (rx * n as usize + 1, ry * n as usize + 1);
        let (padded, nw, nh) = pad(&buf, w, h, px, py);
        dx -= px as i64; dy -= py as i64;
        w = nw; h = nh;
        let blurred = blur(&padded, w, h, rx, ry, n);
        buf = match f {
            Filter::BlurFilter(_) => blurred,
            Filter::GlowFilter(g) => {
                let strength = f32::from(g.strength);
                if g.is_inner() {
                    // inside the shape: the glow is where the blur FALLS OFF, clipped to the source
                    let inv: Buf = blurred.iter().map(|p| [0.0, 0.0, 0.0, 1.0 - p[3]]).collect();
                    let mut lit = tint(&inv, g.color, strength);
                    for (l, s) in lit.iter_mut().zip(padded.iter()) {
                        for c in l.iter_mut() { *c *= s[3]; }
                    }
                    let mut out = if g.is_knockout() { vec![[0.0f32; 4]; w * h] } else { padded.clone() };
                    over(&mut out, &lit);
                    out
                } else {
                    let mut out = tint(&blurred, g.color, strength);
                    if !g.is_knockout() { over(&mut out, &padded); }
                    out
                }
            }
            Filter::DropShadowFilter(d) => {
                let strength = f32::from(d.strength);
                let dist = f64::from(d.distance) * scale_x;
                let ang = f64::from(d.angle);
                let (ox, oy) = ((ang.cos() * dist).round() as i64, (ang.sin() * dist).round() as i64);
                let lit = tint(&blurred, d.color, strength);
                let mut shifted = vec![[0.0f32; 4]; w * h];
                for y in 0..h as i64 {
                    for x in 0..w as i64 {
                        let (sx, sy) = (x - ox, y - oy);
                        if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                            shifted[y as usize * w + x as usize] = lit[sy as usize * w + sx as usize];
                        }
                    }
                }
                let mut out = shifted;
                if !d.is_knockout() { over(&mut out, &padded); }
                out
            }
            _ => unreachable!(),
        };
    }
    (to_img(&buf, w as u32, h as u32), dx, dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use swf::{BlurFilter, BlurFilterFlags, Color, GlowFilter, GlowFilterFlags};

    /// A 10x10 opaque white square on nothing.
    fn square() -> RgbaImage {
        let mut img = RgbaImage::new(10, 10);
        for p in img.pixels_mut() { *p = image::Rgba([255, 255, 255, 255]); }
        img
    }

    fn glow(blur: f64, strength: f64, flags: GlowFilterFlags) -> Filter {
        Filter::GlowFilter(Box::new(GlowFilter {
            color: Color { r: 255, g: 0, b: 0, a: 255 },
            blur_x: Fixed16::from_f64(blur), blur_y: Fixed16::from_f64(blur),
            strength: swf::Fixed8::from_f64(strength),
            flags: flags | GlowFilterFlags::from_passes(1),
        }))
    }

    #[test]
    fn a_glow_grows_the_tile_and_moves_its_corner() {
        let (out, dx, dy) = apply(&square(), &[glow(8.0, 1.0, GlowFilterFlags::COMPOSITE_SOURCE)], 1.0, 1.0);
        assert!(out.width() > 10 && out.height() > 10, "the glow spreads outside the shape");
        assert_eq!(dx, -(out.width() as i64 - 10) / 2, "the corner moves by half the growth");
        assert_eq!(dy, -(out.height() as i64 - 10) / 2);
    }

    #[test]
    fn a_glow_paints_its_own_colour_outside_the_shape_and_leaves_the_shape_alone() {
        let (out, dx, dy) = apply(&square(), &[glow(8.0, 1.0, GlowFilterFlags::COMPOSITE_SOURCE)], 1.0, 1.0);
        let just_outside = out.get_pixel((-dx) as u32 - 1, (-dy) as u32 + 5);
        assert!(just_outside[3] > 0, "the ring outside the shape is painted");
        assert!(just_outside[0] > just_outside[1], "and it is the glow's red, not the shape's white");
        let inside = out.get_pixel((-dx) as u32 + 5, (-dy) as u32 + 5);
        assert_eq!(inside[3], 255, "the shape itself still covers");
        assert_eq!((inside[0], inside[1], inside[2]), (255, 255, 255));
    }

    #[test]
    fn a_blur_softens_the_edge_instead_of_ringing_it() {
        let f = Filter::BlurFilter(Box::new(BlurFilter {
            blur_x: Fixed16::from_f64(8.0), blur_y: Fixed16::from_f64(8.0),
            flags: BlurFilterFlags::from_passes(1),
        }));
        let (out, dx, dy) = apply(&square(), &[f], 1.0, 1.0);
        let edge = out.get_pixel((-dx) as u32, (-dy) as u32 + 5);
        assert!(edge[3] > 0 && edge[3] < 255, "the edge is partly covered, not hard");
        assert_eq!((edge[0], edge[1], edge[2]), (255, 255, 255), "and keeps its colour");
    }

    #[test]
    fn a_filter_we_do_not_render_leaves_the_art_as_it_stands() {
        let f = Filter::ConvolutionFilter(Box::new(swf::ConvolutionFilter {
            num_matrix_rows: 1, num_matrix_cols: 1, matrix: vec![1.0],
            divisor: 1.0, bias: 0.0,
            default_color: Color { r: 0, g: 0, b: 0, a: 0 }, flags: swf::ConvolutionFilterFlags::empty(),
        }));
        let (out, dx, dy) = apply(&square(), &[f], 1.0, 1.0);
        assert_eq!((out.width(), out.height(), dx, dy), (10, 10, 0, 0));
    }
}
