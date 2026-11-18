// Copyright 2006 The Android Open Source Project
// Copyright 2020 Yevhenii Reizner
//
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use crate::*;

use tiny_skia_path::{PathStroker, Scalar, SCALAR_MAX};

use crate::geom::ScreenIntRect;
use crate::mask::SubMaskRef;
use crate::pipeline::{RasterPipelineBlitter, RasterPipelineBuilder};
use crate::pixmap::SubPixmapMut;
use crate::scan;

use crate::geom::IntSizeExt;
#[cfg(all(not(feature = "std"), feature = "no-std-float"))]
use tiny_skia_path::NoStdFloat;

/// A path filling rule.
#[derive(Copy, Clone, Default, PartialEq, Debug)]
pub enum FillRule {
    /// Specifies that "inside" is computed by a non-zero sum of signed edge crossings.
    #[default]
    Winding,
    /// Specifies that "inside" is computed by an odd number of edge crossings.
    EvenOdd,
}

/// Controls how a shape should be painted.
#[derive(Clone, PartialEq, Debug)]
pub struct Paint<'a> {
    /// A paint shader.
    ///
    /// Default: black color
    pub shader: Shader<'a>,

    /// Paint blending mode.
    ///
    /// Default: SourceOver
    pub blend_mode: BlendMode,

    /// Enables anti-aliased painting.
    ///
    /// Default: true
    pub anti_alias: bool,

    /// Colorspace for blending.
    ///
    /// This enables gamma correction during the blend operation.  While skia supports
    /// full color-space conversions, we only support a few (simple) cases.  Note that
    /// any color space other than Linear will force using the high-quality pipeline.
    ///
    /// Default: Linear
    pub colorspace: ColorSpace,

    /// Forces the high quality/precision rendering pipeline.
    ///
    /// `tiny-skia`, just like Skia, has two rendering pipelines:
    /// one uses `f32` and another one uses `u16`. `u16` one is usually way faster,
    /// but less precise. Which can lead to slight differences.
    ///
    /// By default, `tiny-skia` will choose the pipeline automatically,
    /// depending on a blending mode and other parameters.
    /// But you can force the high quality one using this flag.
    ///
    /// This feature is especially useful during testing.
    ///
    /// Unlike high quality pipeline, the low quality one doesn't support all
    /// rendering stages, therefore we cannot force it like hq one.
    ///
    /// Default: false
    pub force_hq_pipeline: bool,
}

impl Default for Paint<'_> {
    fn default() -> Self {
        Paint {
            shader: Shader::SolidColor(Color::BLACK),
            blend_mode: BlendMode::default(),
            anti_alias: true,
            colorspace: ColorSpace::default(),
            force_hq_pipeline: false,
        }
    }
}

impl Paint<'_> {
    /// Sets a paint source to a solid color.
    pub fn set_color(&mut self, color: Color) {
        self.shader = Shader::SolidColor(color);
    }

    /// Sets a paint source to a solid color.
    ///
    /// `self.shader = Shader::SolidColor(Color::from_rgba8(50, 127, 150, 200));` shorthand.
    pub fn set_color_rgba8(&mut self, r: u8, g: u8, b: u8, a: u8) {
        self.set_color(Color::from_rgba8(r, g, b, a))
    }

    /// Checks that the paint source is a solid color.
    pub fn is_solid_color(&self) -> bool {
        matches!(self.shader, Shader::SolidColor(_))
    }
}

impl Pixmap {
    /// Draws a filled rectangle onto the pixmap.
    ///
    /// See [`PixmapMut::fill_rect`](struct.PixmapMut.html#method.fill_rect) for details.
    pub fn fill_rect(
        &mut self,
        rect: Rect,
        paint: &Paint,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        self.as_mut().fill_rect(rect, paint, transform, mask);
    }

    /// Draws a filled path onto the pixmap.
    ///
    /// See [`PixmapMut::fill_path`](struct.PixmapMut.html#method.fill_path) for details.
    pub fn fill_path(
        &mut self,
        path: &Path,
        paint: &Paint,
        fill_rule: FillRule,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        self.as_mut()
            .fill_path(path, paint, fill_rule, transform, mask);
    }

    /// Strokes a path.
    ///
    /// See [`PixmapMut::stroke_path`](struct.PixmapMut.html#method.stroke_path) for details.
    pub fn stroke_path(
        &mut self,
        path: &Path,
        paint: &Paint,
        stroke: &Stroke,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        self.as_mut()
            .stroke_path(path, paint, stroke, transform, mask);
    }

    /// Draws a `Pixmap` on top of the current `Pixmap`.
    ///
    /// See [`PixmapMut::draw_pixmap`](struct.PixmapMut.html#method.draw_pixmap) for details.
    pub fn draw_pixmap(
        &mut self,
        x: i32,
        y: i32,
        pixmap: PixmapRef,
        paint: &PixmapPaint,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        self.as_mut()
            .draw_pixmap(x, y, pixmap, paint, transform, mask);
    }

    /// Applies a masks.
    ///
    /// See [`PixmapMut::apply_mask`](struct.PixmapMut.html#method.apply_mask) for details.
    pub fn apply_mask(&mut self, mask: &Mask) {
        self.as_mut().apply_mask(mask);
    }
}

impl PixmapMut<'_> {
    // TODO: accept NonZeroRect?
    /// Draws a filled rectangle onto the pixmap.
    ///
    /// This function is usually slower than filling a rectangular path,
    /// but it produces better results. Mainly it doesn't suffer from weird
    /// clipping of horizontal/vertical edges.
    ///
    /// Used mainly to render a pixmap onto a pixmap.
    pub fn fill_rect(
        &mut self,
        rect: Rect,
        paint: &Paint,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        if transform.is_identity() && !DrawTiler::required(self.width(), self.height()) {
            // TODO: ignore rects outside the pixmap

            let clip = self.size().to_screen_int_rect(0, 0);

            let mask = mask.map(|mask| mask.as_submask());
            let mut subpix = self.as_subpixmap();
            let mut blitter = match RasterPipelineBlitter::new(paint, mask, &mut subpix) {
                Some(v) => v,
                None => return, // nothing to do, all good
            };

            if paint.anti_alias {
                scan::fill_rect_aa(&rect, &clip, &mut blitter);
            } else {
                scan::fill_rect(&rect, &clip, &mut blitter);
            }
        } else {
            let path = PathBuilder::from_rect(rect);
            self.fill_path(&path, paint, FillRule::Winding, transform, mask);
        }
    }

    /// Draws a filled path onto the pixmap.
    pub fn fill_path(
        &mut self,
        path: &Path,
        paint: &Paint,
        fill_rule: FillRule,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        if transform.is_identity() {
            // This is sort of similar to SkDraw::drawPath

            // Skip empty paths and horizontal/vertical lines.
            let path_bounds = path.bounds();
            if path_bounds.width().is_nearly_zero() || path_bounds.height().is_nearly_zero() {
                log::warn!("empty paths and horizontal/vertical lines cannot be filled");
                return;
            }

            if is_too_big_for_math(path) {
                log::warn!("path coordinates are too big");
                return;
            }

            // TODO: ignore paths outside the pixmap

            if let Some(tiler) = DrawTiler::new(self.width(), self.height()) {
                let mut path = path.clone();
                let mut paint = paint.clone();

                for tile in tiler {
                    let ts = Transform::from_translate(-(tile.x() as f32), -(tile.y() as f32));
                    path = match path.transform(ts) {
                        Some(v) => v,
                        None => {
                            log::warn!("path transformation failed");
                            return;
                        }
                    };
                    paint.shader.transform(ts);

                    let clip_rect = tile.size().to_screen_int_rect(0, 0);
                    let mut subpix = match self.subpixmap(tile.to_int_rect()) {
                        Some(v) => v,
                        None => continue, // technically unreachable
                    };

                    let submask = mask.and_then(|mask| mask.submask(tile.to_int_rect()));
                    let mut blitter = match RasterPipelineBlitter::new(&paint, submask, &mut subpix)
                    {
                        Some(v) => v,
                        None => continue, // nothing to do, all good
                    };

                    // We're ignoring "errors" here, because `fill_path` will return `None`
                    // when rendering a tile that doesn't have a path on it.
                    // Which is not an error in this case.
                    if paint.anti_alias {
                        scan::path_aa::fill_path(&path, fill_rule, &clip_rect, &mut blitter);
                    } else {
                        scan::path::fill_path(&path, fill_rule, &clip_rect, &mut blitter);
                    }

                    let ts = Transform::from_translate(tile.x() as f32, tile.y() as f32);
                    path = match path.transform(ts) {
                        Some(v) => v,
                        None => return, // technically unreachable
                    };
                    paint.shader.transform(ts);
                }
            } else {
                let clip_rect = self.size().to_screen_int_rect(0, 0);
                let submask = mask.map(|mask| mask.as_submask());
                let mut subpix = self.as_subpixmap();
                let mut blitter = match RasterPipelineBlitter::new(paint, submask, &mut subpix) {
                    Some(v) => v,
                    None => return, // nothing to do, all good
                };

                if paint.anti_alias {
                    scan::path_aa::fill_path(path, fill_rule, &clip_rect, &mut blitter);
                } else {
                    scan::path::fill_path(path, fill_rule, &clip_rect, &mut blitter);
                }
            }
        } else {
            let path = match path.clone().transform(transform) {
                Some(v) => v,
                None => {
                    log::warn!("path transformation failed");
                    return;
                }
            };

            let mut paint = paint.clone();
            paint.shader.transform(transform);

            self.fill_path(&path, &paint, fill_rule, Transform::identity(), mask)
        }
    }

    /// Strokes a path.
    ///
    /// Stroking is implemented using two separate algorithms:
    ///
    /// 1. If a stroke width is wider than 1px (after applying the transformation),
    ///    a path will be converted into a stroked path and then filled using `fill_path`.
    ///    Which means that we have to allocate a separate `Path`, that can be 2-3x larger
    ///    then the original path.
    /// 2. If a stroke width is thinner than 1px (after applying the transformation),
    ///    we will use hairline stroking, which doesn't involve a separate path allocation.
    ///
    /// Also, if a `stroke` has a dash array, then path will be converted into
    /// a dashed path first and then stroked. Which means a yet another allocation.
    pub fn stroke_path(
        &mut self,
        path: &Path,
        paint: &Paint,
        stroke: &Stroke,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        render_stroke(self, path, paint, stroke, transform, mask);
    }

    /// A stroking for paths with subpixel/hairline width.
    pub(crate) fn stroke_hairline(
        path: &Path,
        paint: &Paint,
        line_cap: LineCap,
        mask: Option<SubMaskRef>,
        pixmap: &mut SubPixmapMut,
    ) {
        let clip = pixmap.size.to_screen_int_rect(0, 0);
        let mut blitter = match RasterPipelineBlitter::new(paint, mask, pixmap) {
            Some(v) => v,
            None => return, // nothing to do, all good
        };
        if paint.anti_alias {
            scan::hairline_aa::stroke_path(path, line_cap, &clip, &mut blitter);
        } else {
            scan::hairline::stroke_path(path, line_cap, &clip, &mut blitter);
        }
    }

    /// Draws a `Pixmap` on top of the current `Pixmap`.
    ///
    /// The same as filling a rectangle with a `pixmap` pattern.
    pub fn draw_pixmap(
        &mut self,
        x: i32,
        y: i32,
        pixmap: PixmapRef,
        paint: &PixmapPaint,
        transform: Transform,
        mask: Option<&Mask>,
    ) {
        let rect = pixmap.size().to_int_rect(x, y).to_rect();

        // TODO: SkSpriteBlitter
        // TODO: partially clipped
        // TODO: clipped out

        // Translate pattern as well as bounds.
        let patt_transform = Transform::from_translate(x as f32, y as f32);

        let paint = Paint {
            shader: Pattern::new(
                pixmap,
                SpreadMode::Pad, // Pad, otherwise we will get weird borders overlap.
                paint.quality,
                paint.opacity,
                patt_transform,
            ),
            blend_mode: paint.blend_mode,
            anti_alias: false,        // Skia doesn't use it too.
            force_hq_pipeline: false, // Pattern will use hq anyway.
            colorspace: ColorSpace::default(),
        };

        self.fill_rect(rect, &paint, transform, mask);
    }

    /// Applies a masks.
    ///
    /// When a `Mask` is passed to drawing methods, it will be used to mask-out
    /// content we're about to draw.
    /// This method masks-out an already drawn content.
    /// It's not as fast, but can be useful when a mask is not available during drawing.
    ///
    /// This method is similar to filling the whole pixmap with an another,
    /// mask-like pixmap using the `DestinationOut` blend mode.
    ///
    /// `Mask` must have the same size as `Pixmap`. No transform or offset are allowed.
    pub fn apply_mask(&mut self, mask: &Mask) {
        if self.size() != mask.size() {
            log::warn!("Pixmap and Mask are expected to have the same size");
            return;
        }

        // Just a dummy.
        let pixmap_src = PixmapRef::from_bytes(&[0, 0, 0, 0], 1, 1).unwrap();

        let mut p = RasterPipelineBuilder::new();
        p.push(pipeline::Stage::LoadMaskU8);
        p.push(pipeline::Stage::LoadDestination);
        p.push(pipeline::Stage::DestinationIn);
        p.push(pipeline::Stage::Store);
        let mut p = p.compile();
        let rect = self.size().to_screen_int_rect(0, 0);
        p.run(
            &rect,
            pipeline::AAMaskCtx::default(),
            mask.as_submask().mask_ctx(),
            pixmap_src,
            &mut self.as_subpixmap(),
        );
    }
}

/// A surface that can be stroked into.
///
/// Both the color [`PixmapMut`] and the coverage [`crate::Mask`] implement this
/// trait, so that the whole stroking pipeline — dashing, hairline detection,
/// resolution scaling and large-pixmap tiling — is shared between them. This
/// guarantees that a thin stroke produces the same coverage in a mask as it
/// does in the alpha channel of a color pixmap.
pub(crate) trait StrokeSurface {
    fn surface_width(&self) -> u32;
    fn surface_height(&self) -> u32;

    /// Rasterizes a hairline (a single modulated scan line) into one tile.
    ///
    /// `source_alpha` is the full effective source coverage (0..=255), already
    /// combining the paint alpha with the hairline pre-scale. Color pixmaps
    /// ignore it because their paint carries the same value; masks use it so
    /// their stored coverage matches the color path's alpha byte.
    fn stroke_hairline_tile(
        &mut self,
        path: &Path,
        paint: &Paint,
        line_cap: LineCap,
        clip_mask: Option<SubMaskRef>,
        source_alpha: u8,
    );

    /// Fills an already-stroked outline path (used for strokes wider than a hairline).
    fn fill_stroke_outline(
        &mut self,
        path: &Path,
        paint: &Paint,
        transform: Transform,
        clip_mask: Option<&Mask>,
    );
}

impl<'b> StrokeSurface for PixmapMut<'b> {
    fn surface_width(&self) -> u32 {
        self.width()
    }

    fn surface_height(&self) -> u32 {
        self.height()
    }

    fn stroke_hairline_tile(
        &mut self,
        path: &Path,
        paint: &Paint,
        line_cap: LineCap,
        clip_mask: Option<SubMaskRef>,
        _source_alpha: u8,
    ) {
        let mut subpix = self.as_subpixmap();
        PixmapMut::stroke_hairline(path, paint, line_cap, clip_mask, &mut subpix);
    }

    fn fill_stroke_outline(
        &mut self,
        path: &Path,
        paint: &Paint,
        transform: Transform,
        clip_mask: Option<&Mask>,
    ) {
        self.fill_path(path, paint, FillRule::Winding, transform, clip_mask);
    }
}

/// Shared stroking pipeline used by both color pixmaps and coverage masks.
pub(crate) fn render_stroke<S: StrokeSurface>(
    surface: &mut S,
    path: &Path,
    paint: &Paint,
    stroke: &Stroke,
    transform: Transform,
    clip_mask: Option<&Mask>,
) {
    if stroke.width < 0.0 {
        log::warn!("negative stroke width isn't allowed");
        return;
    }

    // Dashing is expanded on the whole path before any tiling happens, so the
    // ink/gap pattern is continuous across tile boundaries.
    let res_scale = PathStroker::compute_resolution_scale(&transform);

    let dash_path;
    let path = if let Some(ref dash) = stroke.dash {
        dash_path = match path.dash(dash, res_scale) {
            Some(v) => v,
            None => {
                log::warn!("path dashing failed");
                return;
            }
        };
        &dash_path
    } else {
        path
    };

    // Hairline vs. outlined-stroke decision is made on the original transform,
    // together with dash expansion and the resolution scale, before tiling.
    if let Some(coverage) = treat_as_hairline(paint, stroke, transform) {
        // Effective source coverage for a coverage mask: the solid paint's own
        // alpha combined with the hairline sub-pixel coverage. Non-solid shaders
        // have no single scalar alpha, so they stay fully opaque in a mask.
        let source_alpha = match paint.shader {
            Shader::SolidColor(ref c) => {
                let paint_alpha = (c.alpha() * 255.0).round() as u8;
                if coverage >= 1.0 {
                    paint_alpha
                } else {
                    // Mirror the integer pre-scale applied to the color paint.
                    let scale = (coverage * 256.0) as i32;
                    let prescaled = ((255 * scale) >> 8) as u8;
                    crate::color::premultiply_u8(paint_alpha, prescaled)
                }
            }
            _ => 255,
        };

        let mut paint = paint.clone();
        if coverage == 1.0 {
            // No changes to the `paint`.
        } else if paint.blend_mode.should_pre_scale_coverage() {
            // This is the old technique, which we preserve for now so
            // we don't change previous results (testing)
            // the new way seems fine, its just (a tiny bit) different.
            let scale = (coverage * 256.0) as i32;
            let new_alpha = (255 * scale) >> 8;
            paint.shader.apply_opacity(new_alpha as f32 / 255.0);
        }

        if let Some(tiler) = DrawTiler::new(surface.surface_width(), surface.surface_height()) {
            let mut path = path.clone();
            let mut paint = paint.clone();

            if !transform.is_identity() {
                paint.shader.transform(transform);
                path = match path.transform(transform) {
                    Some(v) => v,
                    None => {
                        log::warn!("path transformation failed");
                        return;
                    }
                };
            }

            for tile in tiler {
                let ts = Transform::from_translate(-(tile.x() as f32), -(tile.y() as f32));
                path = match path.transform(ts) {
                    Some(v) => v,
                    None => {
                        log::warn!("path transformation failed");
                        return;
                    }
                };
                paint.shader.transform(ts);

                let submask = clip_mask.and_then(|mask| mask.submask(tile.to_int_rect()));
                surface.stroke_hairline_tile(&path, &paint, stroke.line_cap, submask, source_alpha);

                let ts = Transform::from_translate(tile.x() as f32, tile.y() as f32);
                path = match path.transform(ts) {
                    Some(v) => v,
                    None => return,
                };
                paint.shader.transform(ts);
            }
        } else if !transform.is_identity() {
            paint.shader.transform(transform);
            let path = match path.clone().transform(transform) {
                Some(v) => v,
                None => {
                    log::warn!("path transformation failed");
                    return;
                }
            };

            let submask = clip_mask.map(|mask| mask.as_submask());
            surface.stroke_hairline_tile(&path, &paint, stroke.line_cap, submask, source_alpha);
        } else {
            let submask = clip_mask.map(|mask| mask.as_submask());
            surface.stroke_hairline_tile(path, &paint, stroke.line_cap, submask, source_alpha);
        }
    } else {
        let path = match path.stroke(stroke, res_scale) {
            Some(v) => v,
            None => {
                log::warn!("path stroking failed");
                return;
            }
        };

        surface.fill_stroke_outline(&path, paint, transform, clip_mask);
    }
}

fn treat_as_hairline(paint: &Paint, stroke: &Stroke, mut ts: Transform) -> Option<f32> {
    fn fast_len(p: Point) -> f32 {
        let mut x = p.x.abs();
        let mut y = p.y.abs();
        if x < y {
            core::mem::swap(&mut x, &mut y);
        }

        x + y.half()
    }

    debug_assert!(stroke.width >= 0.0);

    if stroke.width == 0.0 {
        return Some(1.0);
    }

    if !paint.anti_alias {
        return None;
    }

    // We don't care about translate.
    ts.tx = 0.0;
    ts.ty = 0.0;

    // We need to try to fake a thick-stroke with a modulated hairline.
    //
    // A hairline is kept as long as the transformed width stays at or below one
    // pixel on *at least one* axis. When an anisotropic scale stretches the
    // stroke in one direction while it stays thin in the other, the stroke is
    // still a single scan line; only when the stroke is wider than one pixel on
    // both axes do we fall back to a regular outlined (filled) stroke.
    let mut points = [
        Point::from_xy(stroke.width, 0.0),
        Point::from_xy(0.0, stroke.width),
    ];
    ts.map_points(&mut points);

    let len0 = fast_len(points[0]);
    let len1 = fast_len(points[1]);

    if len0 <= 1.0 || len1 <= 1.0 {
        let thin = len0.min(len1);
        let wide = len0.max(len1);
        // The coverage is driven by the thin axis (the one that still behaves
        // like a hairline), with a small extra term from the part of the wide
        // axis that exceeds a full pixel. For a uniform transform both axes are
        // equal so `wide <= 1` and this reduces to the original `thin` coverage;
        // the `/ 12` extra term only changes the anisotropic case (the wide axis
        // only reaches it once it is already over one pixel wide).
        return Some((thin + (wide - 1.0).max(0.0) / 12.0).min(1.0));
    }

    None
}

/// Sometimes in the drawing pipeline, we have to perform math on path coordinates, even after
/// the path is in device-coordinates. Tessellation and clipping are two examples. Usually this
/// is pretty modest, but it can involve subtracting/adding coordinates, or multiplying by
/// small constants (e.g. 2,3,4). To try to preflight issues where these optionations could turn
/// finite path values into infinities (or NaNs), we allow the upper drawing code to reject
/// the path if its bounds (in device coordinates) is too close to max float.
pub(crate) fn is_too_big_for_math(path: &Path) -> bool {
    // This value is just a guess. smaller is safer, but we don't want to reject largish paths
    // that we don't have to.
    const SCALE_DOWN_TO_ALLOW_FOR_SMALL_MULTIPLIES: f32 = 0.25;
    const MAX: f32 = SCALAR_MAX * SCALE_DOWN_TO_ALLOW_FOR_SMALL_MULTIPLIES;

    let b = path.bounds();

    // use ! expression so we return true if bounds contains NaN
    !(b.left() >= -MAX && b.top() >= -MAX && b.right() <= MAX && b.bottom() <= MAX)
}

/// Splits the target pixmap into a list of tiles.
///
/// Skia/tiny-skia uses a lot of fixed-point math during path rendering.
/// Probably more for precision than performance.
/// And our fixed-point types are limited by 8192 and 32768.
/// Which means that we cannot render a path larger than 8192 onto a pixmap.
/// When pixmap is smaller than 8192, the path will be automatically clipped anyway,
/// but for large pixmaps we have to render in tiles.
pub(crate) struct DrawTiler {
    image_width: u32,
    image_height: u32,
    x_offset: u32,
    y_offset: u32,
    finished: bool,
}

impl DrawTiler {
    // 8K is 1 too big, since 8K << supersample == 32768 which is too big for Fixed.
    const MAX_DIMENSIONS: u32 = 8192 - 1;

    fn required(image_width: u32, image_height: u32) -> bool {
        image_width > Self::MAX_DIMENSIONS || image_height > Self::MAX_DIMENSIONS
    }

    pub(crate) fn new(image_width: u32, image_height: u32) -> Option<Self> {
        if Self::required(image_width, image_height) {
            Some(DrawTiler {
                image_width,
                image_height,
                x_offset: 0,
                y_offset: 0,
                finished: false,
            })
        } else {
            None
        }
    }
}

impl Iterator for DrawTiler {
    type Item = ScreenIntRect;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        if self.x_offset < self.image_width && self.y_offset < self.image_height {
            let h = if self.y_offset < self.image_height {
                (self.image_height - self.y_offset).min(Self::MAX_DIMENSIONS)
            } else {
                self.image_height
            };

            let r = ScreenIntRect::from_xywh(
                self.x_offset,
                self.y_offset,
                (self.image_width - self.x_offset).min(Self::MAX_DIMENSIONS),
                h,
            );

            self.x_offset += Self::MAX_DIMENSIONS;
            if self.x_offset >= self.image_width {
                self.x_offset = 0;
                self.y_offset += Self::MAX_DIMENSIONS;
            }

            return r;
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const MAX_DIM: u32 = DrawTiler::MAX_DIMENSIONS;

    #[test]
    fn mask_hairline_matches_color_coverage() {
        use crate::Mask;
        let mut pb = PathBuilder::new();
        pb.move_to(20.0, 30.0);
        pb.line_to(180.0, 30.0);
        let path = pb.finish().unwrap();
        let mut stroke = Stroke::default();
        stroke.width = 0.4;
        stroke.line_cap = LineCap::Butt;
        let mut mask = Mask::new(200, 60).unwrap();
        mask.stroke_path(&path, &stroke, true, 200, Transform::identity());
        // Same alpha byte the color pixmap writes for this hairline.
        assert_eq!(mask.data()[30 * 200 + 100], 40);
    }

    #[test]
    fn hairline_coverage_identity_thin() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(50, 127, 150, 200);
        let mut stroke = Stroke::default();
        stroke.width = 0.4;
        let cov = treat_as_hairline(&paint, &stroke, Transform::identity()).unwrap();
        assert!((cov - 0.4).abs() < 1e-6, "cov={cov}");

        let scale = (cov * 256.0) as i32;
        let prescaled = ((255 * scale) >> 8) as u8;
        let source = crate::color::premultiply_u8(200, prescaled);
        assert_eq!(prescaled, 101);
        assert_eq!(source, 79);
    }

    #[test]
    fn hairline_coverage_anisotropic() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(50, 127, 150, 200);
        let mut stroke = Stroke::default();
        stroke.width = 0.8;
        // x2 horizontally, /2 vertically: the x axis becomes 1.6px wide while the
        // y axis stays 0.4px, so the stroke remains a (modulated) hairline.
        let ts = Transform::from_row(2.0, 0.0, 0.0, 0.5, 0.0, 0.0);
        let cov = treat_as_hairline(&paint, &stroke, ts).unwrap();
        assert!((cov - 0.45).abs() < 1e-6, "cov={cov}");
    }

    #[test]
    fn hairline_outline_when_both_axes_wide() {
        let mut paint = Paint::default();
        paint.set_color_rgba8(50, 127, 150, 200);
        let mut stroke = Stroke::default();
        stroke.width = 2.0;
        // Both axes map the stroke well above one pixel: not a hairline.
        assert!(treat_as_hairline(&paint, &stroke, Transform::identity()).is_none());
    }

    #[test]
    fn skip() {
        assert!(DrawTiler::new(100, 500).is_none());
    }

    #[test]
    fn horizontal() {
        let mut iter = DrawTiler::new(10000, 500).unwrap();
        assert_eq!(iter.next(), ScreenIntRect::from_xywh(0, 0, MAX_DIM, 500));
        assert_eq!(
            iter.next(),
            ScreenIntRect::from_xywh(MAX_DIM, 0, 10000 - MAX_DIM, 500)
        );
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn vertical() {
        let mut iter = DrawTiler::new(500, 10000).unwrap();
        assert_eq!(iter.next(), ScreenIntRect::from_xywh(0, 0, 500, MAX_DIM));
        assert_eq!(
            iter.next(),
            ScreenIntRect::from_xywh(0, MAX_DIM, 500, 10000 - MAX_DIM)
        );
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn rect() {
        let mut iter = DrawTiler::new(10000, 10000).unwrap();
        // Row 1
        assert_eq!(
            iter.next(),
            ScreenIntRect::from_xywh(0, 0, MAX_DIM, MAX_DIM)
        );
        assert_eq!(
            iter.next(),
            ScreenIntRect::from_xywh(MAX_DIM, 0, 10000 - MAX_DIM, MAX_DIM)
        );
        // Row 2
        assert_eq!(
            iter.next(),
            ScreenIntRect::from_xywh(0, MAX_DIM, MAX_DIM, 10000 - MAX_DIM)
        );
        assert_eq!(
            iter.next(),
            ScreenIntRect::from_xywh(MAX_DIM, MAX_DIM, 10000 - MAX_DIM, 10000 - MAX_DIM)
        );
        assert_eq!(iter.next(), None);
    }
}
