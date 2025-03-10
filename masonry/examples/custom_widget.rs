// Copyright 2019 the Xilem Authors and the Druid Authors
// SPDX-License-Identifier: Apache-2.0

//! An example of a custom drawing widget.
//! We draw an image, some text, a shape, and a curve.

// On Windows platform, don't show a console when opening the app.
#![windows_subsystem = "windows"]
#![expect(elided_lifetimes_in_paths, reason = "Deferred: Noisy")]
#![expect(clippy::shadow_unrelated, reason = "Deferred: Noisy")]
#![expect(clippy::cast_possible_truncation, reason = "Deferred: Noisy")]

use accesskit::{Node, Role};
use masonry::app::{AppDriver, DriverCtx};
use masonry::core::{
    AccessCtx, AccessEvent, Action, BoxConstraints, EventCtx, LayoutCtx, ObjectFit, PaintCtx,
    PointerEvent, PropertiesMut, PropertiesRef, QueryCtx, RegisterCtx, TextEvent, Widget, WidgetId,
};
use masonry::kurbo::{Affine, BezPath, Point, Rect, Size, Stroke};
use masonry::palette;
use masonry::peniko::Color;
use masonry::widgets::RootWidget;
use parley::layout::{Alignment, AlignmentOptions};
use parley::style::{FontFamily, FontStack, StyleProperty};
use smallvec::SmallVec;
use tracing::{Span, trace_span};
use vello::Scene;
use vello::peniko::{Fill, Image, ImageFormat};
use winit::window::Window;

struct Driver;

impl AppDriver for Driver {
    fn on_action(&mut self, _ctx: &mut DriverCtx<'_>, _widget_id: WidgetId, _action: Action) {}
}

struct CustomWidget(String);

impl Widget for CustomWidget {
    fn on_pointer_event(
        &mut self,
        _ctx: &mut EventCtx,
        _props: &mut PropertiesMut<'_>,
        _event: &PointerEvent,
    ) {
    }

    fn on_text_event(
        &mut self,
        _ctx: &mut EventCtx,
        _props: &mut PropertiesMut<'_>,
        _event: &TextEvent,
    ) {
    }

    fn on_access_event(
        &mut self,
        _ctx: &mut EventCtx,
        _props: &mut PropertiesMut<'_>,
        _event: &AccessEvent,
    ) {
    }

    fn register_children(&mut self, _ctx: &mut RegisterCtx) {}

    fn layout(
        &mut self,
        _layout_ctx: &mut LayoutCtx,
        _props: &mut PropertiesMut<'_>,
        bc: &BoxConstraints,
    ) -> Size {
        // BoxConstraints are passed by the parent widget.
        // This method can return any Size within those constraints:
        // bc.constrain(my_size)
        //
        // To check if a dimension is infinite or not (e.g. scrolling):
        // bc.is_width_bounded() / bc.is_height_bounded()
        //
        // bx.max() returns the maximum size of the widget. Be careful
        // using this, since always make sure the widget is bounded.
        // If bx.max() is used in a scrolling widget things will probably
        // not work correctly.
        if bc.is_width_bounded() && bc.is_height_bounded() {
            bc.max()
        } else {
            let size = Size::new(100.0, 100.0);
            bc.constrain(size)
        }
    }

    // The paint method gets called last, after an event flow.
    // It goes event -> update -> layout -> paint, and each method can influence the next.
    // Basically, anything that changes the appearance of a widget causes a paint.
    fn paint(&mut self, ctx: &mut PaintCtx, _props: &PropertiesRef<'_>, scene: &mut Scene) {
        // Clear the whole widget with the color of your choice
        // (ctx.size() returns the size of the layout rect we're painting in)
        // Note: ctx also has a `clear` method, but that clears the whole context,
        // and we only want to clear this widget's area.
        let size = ctx.size();
        let rect = size.to_rect();
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette::css::WHITE,
            None,
            &rect,
        );

        // Create an arbitrary bezier path
        let mut path = BezPath::new();
        path.move_to(Point::ORIGIN);
        path.quad_to((60.0, 120.0), (size.width, size.height));
        // Create a color
        let stroke_color = Color::from_rgb8(0, 128, 0);
        // Stroke the path with thickness 5.0
        scene.stroke(
            &Stroke::new(5.0),
            Affine::IDENTITY,
            stroke_color,
            None,
            &path,
        );

        // Rectangles: the path for practical people
        let rect = Rect::from_origin_size((10.0, 10.0), (100.0, 100.0));
        // Note the Color:from_rgba8 which includes an alpha channel (7F in this case)
        let fill_color = Color::from_rgba8(0x00, 0x00, 0x00, 0x7F);
        scene.fill(Fill::NonZero, Affine::IDENTITY, fill_color, None, &rect);

        // To render text, we first create a LayoutBuilder and set the text properties.
        let mut lcx = parley::LayoutContext::new();
        let mut text_layout_builder = lcx.ranged_builder(ctx.text_contexts().0, &self.0, 1.0);

        text_layout_builder.push_default(StyleProperty::FontStack(FontStack::Single(
            FontFamily::Generic(parley::style::GenericFamily::Serif),
        )));
        text_layout_builder.push_default(StyleProperty::FontSize(24.0));

        let mut text_layout = text_layout_builder.build(&self.0);
        text_layout.break_all_lines(None);
        text_layout.align(None, Alignment::Start, AlignmentOptions::default());

        // We can pass a transform matrix to rotate the text we render
        masonry::core::render_text(
            scene,
            Affine::rotate(std::f64::consts::FRAC_PI_4).then_translate((80.0, 40.0).into()),
            &text_layout,
            &[fill_color.into()],
            true,
        );

        // Let's burn some CPU to make a (partially transparent) image buffer
        let image_data = make_image_data(256, 256);
        let image_data = Image::new(image_data.into(), ImageFormat::Rgba8, 256, 256);
        let transform = ObjectFit::Fill.affine_to_fill(ctx.size(), size);
        scene.draw_image(&image_data, transform);
    }

    fn accessibility_role(&self) -> Role {
        Role::Window
    }

    fn accessibility(&mut self, _ctx: &mut AccessCtx, _props: &PropertiesRef<'_>, node: &mut Node) {
        let text = &self.0;
        node.set_label(
            format!("This is a demo of the Masonry Widget trait. Masonry has accessibility tree support. The demo shows colored shapes with the text '{text}'."),
        );
    }

    fn children_ids(&self) -> SmallVec<[WidgetId; 16]> {
        SmallVec::new()
    }

    fn make_trace_span(&self, ctx: &QueryCtx<'_>) -> Span {
        trace_span!("CustomWidget", id = ctx.widget_id().trace())
    }
}

fn main() {
    let my_string = "Masonry + Vello".to_string();
    let window_attributes = Window::default_attributes().with_title("Fancy colors");

    masonry::app::run(
        masonry::app::EventLoop::with_user_event(),
        window_attributes,
        RootWidget::new(CustomWidget(my_string)),
        Driver,
    )
    .unwrap();
}

fn make_image_data(width: usize, height: usize) -> Vec<u8> {
    let mut result = vec![0; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            let ix = (y * width + x) * 4;
            result[ix] = x as u8;
            result[ix + 1] = y as u8;
            result[ix + 2] = !(x as u8);
            result[ix + 3] = 127;
        }
    }
    result
}

// --- MARK: TESTS ---
#[cfg(test)]
mod tests {
    use insta::assert_debug_snapshot;
    use masonry::assert_render_snapshot;
    use masonry::testing::TestHarness;

    use super::*;

    #[test]
    fn screenshot_test() {
        let my_string = "Masonry + Vello".to_string();

        let mut harness = TestHarness::create(CustomWidget(my_string));
        assert_debug_snapshot!(harness.root_widget());
        assert_render_snapshot!(harness, "initial_screenshot");
    }
}
