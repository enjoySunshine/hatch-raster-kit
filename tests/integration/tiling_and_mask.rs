use tiny_skia::*;

fn pixel(p: &Pixmap, x: u32, y: u32) -> u32 {
    let c = p.pixel(x, y).unwrap();
    ((c.red() as u32) << 24)
        | ((c.green() as u32) << 16)
        | ((c.blue() as u32) << 8)
        | (c.alpha() as u32)
}

fn hline_path(x0: f32, x1: f32, y: f32) -> Path {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y);
    pb.line_to(x1, y);
    pb.finish().unwrap()
}

fn standard_paint() -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(50, 127, 150, 200);
    paint
}

// A dashed horizontal line rendered into a wide pixmap must be the same as
// rendering the same line translated into a tile-sized pixmap: dashing is
// expanded on the whole path before tiling, so the gap at the tile boundary
// stays in the same place and the second tile is byte-identical to the small
// pixmap.
#[test]
fn dash_is_expanded_before_tiling() {
    fn render(width: u32, shift: f32) -> Pixmap {
        let mut pixmap = Pixmap::new(width, 48).unwrap();
        let path = hline_path(8000.0 - shift, 8400.0 - shift, 24.0);

        let mut stroke = Stroke::default();
        stroke.width = 4.0;
        stroke.line_cap = LineCap::Butt;
        stroke.dash = Some(StrokeDash::new(vec![10.0, 6.0], 0.0).unwrap());

        pixmap.stroke_path(&path, &standard_paint(), &stroke, Transform::identity(), None);
        pixmap
    }

    let big = render(9000, 0.0);
    let small = render(809, 8191.0);

    // Ink/gap positions on the first tile.
    assert_eq!(pixel(&big, 8000, 24), 0x2764_76c8);
    assert_eq!(pixel(&big, 8010, 24), 0);
    assert_eq!(pixel(&big, 8016, 24), 0x2764_76c8);
    // The gap continues onto the second tile.
    assert_eq!(pixel(&big, 8191, 24), 0);
    assert_eq!(pixel(&big, 8200, 24), 0x2764_76c8);

    // The whole second tile must be byte-identical to the translated small pixmap.
    let tile_w = (9000 - 8191) as usize;
    for y in 0..48usize {
        let big_row = &big.data()[(y * 9000 + 8191) * 4..(y * 9000 + 8191 + tile_w) * 4];
        let small_row = &small.data()[y * 809 * 4..(y * 809 + tile_w) * 4];
        assert_eq!(big_row, small_row, "row {y} must match across the tile seam");
    }
}

// Solid strokes of various (sub-pixel) widths keep their coverage across tiles.
#[test]
fn solid_stroke_across_tile() {
    fn render(width: f32) -> Pixmap {
        let mut pixmap = Pixmap::new(9000, 48).unwrap();
        let path = hline_path(8000.0, 8400.0, 24.0);
        let mut stroke = Stroke::default();
        stroke.width = width;
        stroke.line_cap = LineCap::Butt;
        pixmap.stroke_path(&path, &standard_paint(), &stroke, Transform::identity(), None);
        pixmap
    }

    assert_eq!(pixel(&render(3.0), 8191, 24), 0x2764_76c8);
    assert_eq!(pixel(&render(0.4), 8191, 24), 0x0814_1828);
}

// A gradient must move together with the path when the canvas is split into
// tiles, and the translated small pixmap must match the second tile byte for byte.
#[test]
fn gradient_follows_path_translation() {
    fn render(canvas_w: u32, shift: f32) -> Pixmap {
        let mut pixmap = Pixmap::new(canvas_w, 48).unwrap();
        let rect = Rect::from_xywh(8000.0 - shift, 0.0, 400.0, 16.0).unwrap();
        let path = PathBuilder::from_rect(rect);
        let shader = LinearGradient::new(
            Point::from_xy(8000.0 - shift, 0.0),
            Point::from_xy(8400.0 - shift, 0.0),
            vec![
                GradientStop::new(0.0, Color::from_rgba8(255, 0, 0, 255)),
                GradientStop::new(1.0, Color::from_rgba8(0, 0, 255, 255)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        )
        .unwrap();
        let paint = Paint {
            shader,
            ..Paint::default()
        };
        pixmap.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
        pixmap
    }

    let big = render(9000, 0.0);
    let small = render(809, 8191.0);

    assert_eq!(pixel(&big, 8191, 8), 0x8500_7aff);
    assert_eq!(pixel(&big, 8200, 8), 0x7f00_80ff);

    let tile_w = (9000 - 8191) as usize;
    for y in 0..16usize {
        let big_row = &big.data()[(y * 9000 + 8191) * 4..(y * 9000 + 8191 + tile_w) * 4];
        let small_row = &small.data()[y * 809 * 4..(y * 809 + tile_w) * 4];
        assert_eq!(big_row, small_row, "gradient row {y} must match the tile");
    }
}

// A hairline stroke writes the same coverage into a mask as it writes into the
// color pixmap's alpha channel, instead of approximating it with an expanded outline.
#[test]
fn mask_hairline_matches_color_coverage() {
    let path = hline_path(20.0, 180.0, 30.0);

    let mut stroke = Stroke::default();
    stroke.width = 0.4;
    stroke.line_cap = LineCap::Butt;

    let mut color = Pixmap::new(200, 60).unwrap();
    color.stroke_path(&path, &standard_paint(), &stroke, Transform::identity(), None);
    assert_eq!(pixel(&color, 100, 30), 0x0814_1828);

    let mut mask = Mask::new(200, 60).unwrap();
    mask.stroke_path(&path, &stroke, true, 200, Transform::identity());
    assert_eq!(mask.data()[30 * 200 + 100], color.pixel(100, 30).unwrap().alpha());
    assert_eq!(mask.data()[30 * 200 + 100], 40);
}

// An opaque, wider stroke keeps producing full coverage in both surfaces.
#[test]
fn mask_opaque_wide_stroke_stays_aligned() {
    let path = hline_path(20.0, 180.0, 30.0);

    let mut stroke = Stroke::default();
    stroke.width = 4.0;
    stroke.line_cap = LineCap::Butt;

    let mut color = Pixmap::new(200, 60).unwrap();
    let mut opaque = standard_paint();
    opaque.set_color_rgba8(50, 127, 150, 255);
    color.stroke_path(&path, &opaque, &stroke, Transform::identity(), None);

    let mut mask = Mask::new(200, 60).unwrap();
    mask.stroke_path(&path, &stroke, true, 255, Transform::identity());

    assert_eq!(mask.data()[30 * 200 + 100], color.pixel(100, 30).unwrap().alpha());
    assert_eq!(mask.data()[30 * 200 + 100], 255);
}

// With an anisotropic scale where one axis still maps to a sub-pixel width,
// the stroke stays a hairline and its coverage follows the thin transformed axis.
#[test]
fn anisotropic_scale_keeps_hairline() {
    let mut pixmap = Pixmap::new(200, 80).unwrap();
    let path = hline_path(10.0, 90.0, 40.0);

    let mut stroke = Stroke::default();
    stroke.width = 0.8;
    stroke.line_cap = LineCap::Butt;

    let ts = Transform::from_row(2.0, 0.0, 0.0, 0.5, 0.0, 0.0);
    pixmap.stroke_path(&path, &standard_paint(), &stroke, ts, None);

    assert_eq!(pixel(&pixmap, 50, 20), 0x0917_1b2d);
    assert_eq!(pixmap.pixels().iter().filter(|p| p.alpha() != 0).count(), 320);
}

// A regular 2px wide circle outline is unaffected by the hairline changes.
#[test]
fn circle_outline_unchanged() {
    let mut pixmap = Pixmap::new(200, 200).unwrap();
    let path = PathBuilder::from_circle(100.0, 100.0, 50.0).unwrap();

    let mut stroke = Stroke::default();
    stroke.width = 2.0;

    pixmap.stroke_path(&path, &standard_paint(), &stroke, Transform::identity(), None);

    assert_eq!(pixel(&pixmap, 50, 100), 0x2764_76c8);
    assert_eq!(pixel(&pixmap, 48, 100), 0);
    assert_eq!(pixmap.pixels().iter().filter(|p| p.alpha() != 0).count(), 920);
}

// A solid filled circle keeps identical coverage between mask and color pixmap.
#[test]
fn solid_circle_mask_and_color_agree() {
    let path = PathBuilder::from_circle(40.0, 40.0, 18.0).unwrap();

    let mut mask = Mask::new(80, 80).unwrap();
    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());

    let mut color = Pixmap::new(80, 80).unwrap();
    let mut opaque = standard_paint();
    opaque.set_color_rgba8(50, 127, 150, 255);
    color.fill_path(&path, &opaque, FillRule::Winding, Transform::identity(), None);

    for y in 0..80u32 {
        for x in 0..80u32 {
            assert_eq!(
                mask.data()[(y as usize) * 80 + x as usize],
                color.pixel(x, y).unwrap().alpha(),
                "mismatch at ({x},{y})"
            );
        }
    }
}
