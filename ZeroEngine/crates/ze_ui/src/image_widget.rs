use yakui::{
	Color, Constraints, Rect, Response, TextureId, Vec2,
	paint::PaintRect,
	widget::{LayoutContext, PaintContext, Widget},
};

/// Draws a texture cropped to `uv_rect` (a normalized `[x, y, w, h]` sub-rect,
/// `Rect::ONE` for the whole texture), tinted by `color`.
///
/// yakui's stock `yakui::widgets::Image` always samples the full `0..1` UV
/// range (`ImageWidget::paint` hardcodes `Rect::ONE`), so it can't display a
/// single `TextureSheet` cell out of a larger sheet texture. The underlying
/// primitive (`PaintRect::texture`) already supports an arbitrary UV rect --
/// yakui's own glyph-atlas and nine-slice rendering rely on exactly this --
/// so this widget just exposes that instead of re-deriving pixel math.
///
/// Named `TexturedQuad` rather than `UiImage`/`Image` to keep it visually
/// distinct from the `ze_ui::UIImage` ECS component it renders.
#[derive(Debug, Clone)]
pub struct TexturedQuad {
	pub image: Option<TextureId>,
	pub size: Vec2,
	pub color: Color,
	pub uv_rect: Rect,
}

impl TexturedQuad {
	pub const fn new(image: Option<TextureId>, size: Vec2, color: Color, uv_rect: Rect) -> Self {
		Self {
			image,
			size,
			color,
			uv_rect,
		}
	}

	#[track_caller]
	pub fn show(self) -> Response<()> { yakui::util::widget::<TexturedQuadWidget>(self) }
}

#[derive(Debug)]
pub struct TexturedQuadWidget {
	props: TexturedQuad,
}

impl Widget for TexturedQuadWidget {
	type Props<'a> = TexturedQuad;
	type Response = ();

	fn new() -> Self {
		Self {
			props: TexturedQuad::new(None, Vec2::ZERO, Color::WHITE, Rect::ONE),
		}
	}

	fn update(&mut self, props: Self::Props<'_>) -> Self::Response { self.props = props; }

	fn layout(&self, _ctx: LayoutContext<'_>, input: Constraints) -> Vec2 { input.constrain(self.props.size) }

	fn paint(&self, ctx: PaintContext<'_>) {
		let Some(layout_node) = ctx.layout.get(ctx.dom.current()) else {
			return;
		};

		if let Some(image) = self.props.image {
			let mut rect = PaintRect::new(layout_node.rect);
			rect.color = self.props.color;
			rect.texture = Some((image, self.props.uv_rect));
			rect.add(ctx.paint);
		}
	}
}
