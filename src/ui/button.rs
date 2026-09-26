//! The app's button, lit from above like everything else (see [`depth`]).
//!
//! gpui-component's `Button` sets its own hover and active styles last, so a
//! pressed shadow or a sheen can't be layered onto it from outside — hence a
//! button of our own. Its builder mirrors that one, so it reads the same at the
//! call site.
//!
//! Clicked things go up: every filled variant is faintly raised, and is pushed
//! in — shadow gone, a light inner shadow on — while held. Ghost and link
//! buttons stay flat. A selected button stays pushed in, which is how a
//! toggle that's on reads.

use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Disableable, Icon, Selectable, Sizable, Size, StyledExt as _,
    menu::DropdownMenu, spinner::Spinner,
};

use crate::ui::depth::{self, Finish, Level, Lit as _, radius};
use crate::ui::tooltip;

/// Secondary unless a builder method picks another.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    Primary,
    Secondary,
    Outline,
    Ghost,
    Danger,
    Link,
}

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Button {
    base: Stateful<Div>,
    style: StyleRefinement,
    variant: Variant,
    size: Size,
    icon: Option<Icon>,
    label: Option<SharedString>,
    tooltip: Option<SharedString>,
    selected: bool,
    disabled: bool,
    loading: bool,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            base: div().id(id),
            style: StyleRefinement::default(),
            variant: Variant::Secondary,
            size: Size::Medium,
            icon: None,
            label: None,
            tooltip: None,
            selected: false,
            disabled: false,
            loading: false,
            on_click: None,
        }
    }

    pub fn primary(mut self) -> Self {
        self.variant = Variant::Primary;
        self
    }

    pub fn outline(mut self) -> Self {
        self.variant = Variant::Outline;
        self
    }

    pub fn ghost(mut self) -> Self {
        self.variant = Variant::Ghost;
        self
    }

    pub fn danger(mut self) -> Self {
        self.variant = Variant::Danger;
        self
    }

    pub fn link(mut self) -> Self {
        self.variant = Variant::Link;
        self
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl Sizable for Button {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Selectable for Button {
    fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.selected
    }
}

impl Disableable for Button {
    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl Styled for Button {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl InteractiveElement for Button {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl DropdownMenu for Button {}

/// Height, padding, and type for one size, after the toolkit's scale.
struct Metrics {
    height: Pixels,
    padding: Pixels,
    gap: Pixels,
    icon: Pixels,
    text: Pixels,
    radius: Pixels,
}

impl Metrics {
    fn of(size: Size) -> Self {
        match size {
            Size::XSmall => Self {
                height: px(24.),
                padding: px(8.),
                gap: px(4.),
                icon: px(12.),
                text: px(12.),
                radius: radius::ITEM,
            },
            Size::Small => Self {
                height: px(28.),
                padding: px(10.),
                gap: px(4.),
                icon: px(14.),
                text: px(12.8),
                radius: radius::ITEM,
            },
            Size::Large => Self {
                height: px(36.),
                padding: px(10.),
                gap: px(6.),
                icon: px(16.),
                text: px(14.),
                radius: radius::CONTROL,
            },
            Size::Medium => Self {
                height: px(32.),
                padding: px(10.),
                gap: px(6.),
                icon: px(16.),
                text: px(14.),
                radius: radius::CONTROL,
            },
            Size::Size(height) => Self {
                height,
                padding: height * 0.3,
                gap: px(6.),
                icon: height * 0.5,
                text: px(14.),
                radius: radius::CONTROL.min(height * 0.3),
            },
        }
    }
}

/// How one variant is coloured and lit.
struct Look {
    bg: Hsla,
    hover: Hsla,
    fg: Hsla,
    border: Option<Hsla>,
    level: Option<Level>,
    finish: Finish,
}

impl Variant {
    fn look(self, cx: &App) -> Look {
        let theme = cx.theme();
        let dark = theme.mode.is_dark();
        let transparent = theme.transparent;

        match self {
            Variant::Primary => Look {
                bg: theme.primary,
                hover: theme.primary.opacity(0.9),
                fg: theme.primary_foreground,
                border: None,
                level: Some(Level::RaisedStrong),
                finish: Finish::Subtle,
            },
            Variant::Secondary => Look {
                bg: theme.secondary,
                hover: theme.secondary.blend(theme.foreground.opacity(0.05)),
                fg: theme.secondary_foreground,
                border: None,
                level: Some(Level::Raised),
                finish: Finish::Subtle,
            },
            Variant::Outline => Look {
                bg: if dark {
                    theme.input.opacity(0.3)
                } else {
                    theme.background
                },
                hover: if dark {
                    theme.input.opacity(0.5)
                } else {
                    theme.muted
                },
                fg: theme.foreground,
                border: Some(if dark { theme.input } else { theme.border }),
                level: Some(Level::Raised),
                finish: Finish::Subtle,
            },
            Variant::Danger => Look {
                bg: theme.danger.opacity(if dark { 0.2 } else { 0.1 }),
                hover: theme.danger.opacity(if dark { 0.3 } else { 0.2 }),
                fg: theme.danger,
                border: None,
                level: Some(Level::Raised),
                finish: Finish::Subtle,
            },
            Variant::Ghost => Look {
                bg: transparent,
                hover: if dark {
                    theme.muted.opacity(0.5)
                } else {
                    theme.muted
                },
                fg: theme.foreground,
                border: None,
                level: None,
                finish: Finish::Matte,
            },
            Variant::Link => Look {
                bg: transparent,
                hover: transparent,
                fg: theme.primary,
                border: None,
                level: None,
                finish: Finish::Matte,
            },
        }
    }
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let metrics = Metrics::of(self.size);
        let look = self.variant.look(cx);
        let icon_only = self.label.is_none();
        let interactive = !self.disabled && !self.loading;
        let is_link = self.variant == Variant::Link;
        // Everything but a link can be pushed in; a flat button has nothing to
        // lift, but still shades in when pressed so the click registers.
        let pressable = interactive && !is_link;
        let inner_radius = if look.border.is_some() {
            metrics.radius - px(1.)
        } else {
            metrics.radius
        };
        let selected_bg = if self.variant == Variant::Ghost {
            look.hover
        } else {
            look.bg
        };

        self.base
            .group(depth::PRESS_GROUP)
            .relative()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .h(metrics.height)
            .map(|this| {
                if icon_only {
                    this.w(metrics.height)
                } else {
                    this.px(metrics.padding)
                }
            })
            .gap(metrics.gap)
            .text_size(metrics.text)
            .font_weight(FontWeight::MEDIUM)
            .text_color(look.fg)
            .bg(if self.selected { selected_bg } else { look.bg })
            .when_some(look.border, |this, border| {
                this.border_1().border_color(border)
            })
            .map(|this| match look.level {
                Some(_) if self.selected => this.lit(
                    Level::Pressed,
                    look.finish,
                    metrics.height,
                    inner_radius,
                    cx,
                ),
                Some(level) => this.lit(level, look.finish, metrics.height, inner_radius, cx),
                None if self.selected => this.lit(
                    Level::Pressed,
                    Finish::Matte,
                    metrics.height,
                    inner_radius,
                    cx,
                ),
                None => this.rounded(metrics.radius),
            })
            .when(pressable && !self.selected, |this| {
                this.child(depth::press_layer(metrics.height, inner_radius, cx))
                    .active(|style| style.shadow(Vec::new()))
            })
            .when(interactive, |this| {
                this.cursor_pointer().hover(|style| {
                    if is_link {
                        style.underline()
                    } else {
                        style.bg(look.hover)
                    }
                })
            })
            .when(self.disabled, |this| this.opacity(0.5))
            .when_some(self.icon.filter(|_| !self.loading), |this, icon| {
                this.child(icon.with_size(Size::Size(metrics.icon)))
            })
            .when(self.loading, |this| {
                this.child(Spinner::new().with_size(Size::Size(metrics.icon)))
            })
            .when_some(self.label, |this, label| {
                this.child(div().flex_none().line_height(relative(1.)).child(label))
            })
            .refine_style(&self.style)
            .when_some(self.tooltip, |this, text| this.tooltip(tooltip::text(text)))
            .when_some(self.on_click, |this, on_click| {
                this.on_click(move |event, window, cx| {
                    if interactive {
                        on_click(event, window, cx);
                    }
                })
            })
            // A disabled button still swallows the click, so it can't fall
            // through to whatever the button sits on — or open its menu.
            .when(!interactive, |this| {
                this.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            })
    }
}
