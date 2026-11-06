//! A group of widgets with one opacity, optionally cut off at a size of its own, which is what fading
//! something in or morphing it needs. Iced has neither: the opacity is a patch to our iced_wgpu.

use iced_widget::core::layout::{self, Layout};
use iced_widget::core::widget::{self, Widget};
use iced_widget::core::{
    Clipboard, Element, Event, Length, Rectangle, Shell, Size, Theme, Transformation, Vector,
    mouse, overlay, renderer,
};
use iced_widget::renderer::Renderer;

/// Far bigger than anything on a screen: what a clipped group's content may take up.
const ROOM: f32 = 100_000.0;

pub struct Group<'a, Message> {
    content: Element<'a, Message, Theme, Renderer>,
    opacity: f32,
    clip: Option<Size>,
    scale: f32,
    clip_left: f32,
}

/// Wraps something, which so far looks the same.
pub fn group<'a, Message>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Group<'a, Message> {
    Group {
        content: content.into(),
        opacity: 1.0,
        clip: None,
        scale: 1.0,
        clip_left: 0.0,
    }
}

impl<Message> Group<'_, Message> {
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Draws it this much bigger or smaller around its middle, without changing what it takes up.
    pub fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    /// With a clip: also doesn't show this much of the left side.
    pub fn clip_left(mut self, left: f32) -> Self {
        self.clip_left = left;
        self
    }

    /// Takes up this size, and shows only that much of its content, which keeps its own size.
    pub fn clip(mut self, size: Size) -> Self {
        self.clip = Some(size);
        self
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Group<'_, Message> {
    fn tag(&self) -> widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> widget::tree::State {
        self.content.as_widget().state()
    }

    fn children(&self) -> Vec<widget::Tree> {
        self.content.as_widget().children()
    }

    fn diff(&self, tree: &mut widget::Tree) {
        self.content.as_widget().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        match self.clip {
            Some(size) => Size::new(Length::Fixed(size.width), Length::Fixed(size.height)),
            None => self.content.as_widget().size(),
        }
    }

    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let inner = match self.clip {
            Some(_) => layout::Limits::new(Size::ZERO, Size::new(ROOM, ROOM)),
            None => *limits,
        };
        let node = self.content.as_widget_mut().layout(tree, renderer, &inner);
        let size = self.clip.unwrap_or_else(|| node.size());
        layout::Node::with_children(size, vec![node])
    }

    fn operate(
        &mut self,
        tree: &mut widget::Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn widget::Operation,
    ) {
        self.content.as_widget_mut().operate(
            tree,
            layout.children().next().unwrap(),
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.content.as_widget_mut().update(
            tree,
            event,
            layout.children().next().unwrap(),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            tree,
            layout.children().next().unwrap(),
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if self.opacity <= 0.0 {
            return;
        }
        let bounds = layout.bounds();
        let content = layout.children().next().unwrap();
        iced_wgpu::group::multiply(self.opacity, || {
            use iced_widget::core::Renderer as _;
            if self.scale != 1.0 {
                let center = bounds.center();
                let transformation = Transformation::translate(center.x, center.y)
                    * Transformation::scale(self.scale)
                    * Transformation::translate(-center.x, -center.y);
                renderer.with_transformation(transformation, |renderer| {
                    self.content
                        .as_widget()
                        .draw(tree, renderer, theme, style, content, cursor, viewport);
                });
            } else if self.clip.is_some() {
                let bounds = Rectangle {
                    x: bounds.x + self.clip_left,
                    width: (bounds.width - self.clip_left).max(0.0),
                    ..bounds
                };
                renderer.with_layer(bounds, |renderer| {
                    self.content
                        .as_widget()
                        .draw(tree, renderer, theme, style, content, cursor, &bounds);
                });
            } else {
                self.content
                    .as_widget()
                    .draw(tree, renderer, theme, style, content, cursor, viewport);
            }
        });
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut widget::Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            tree,
            layout.children().next().unwrap(),
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<Group<'a, Message>> for Element<'a, Message, Theme, Renderer> {
    fn from(group: Group<'a, Message>) -> Self {
        Element::new(group)
    }
}
