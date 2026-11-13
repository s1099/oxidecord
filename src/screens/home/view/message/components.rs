//! A message's components: an app's rows of buttons and select menus, and the
//! containers, sections, text and media a components-v2 message is built from.

use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord::{self, ButtonAction, ButtonStyle, Component};
use crate::screens::home::HomeScreen;
use crate::screens::home::data::image_viewer::ViewerImage;
use crate::screens::home::media::MediaKey;
use crate::screens::home::selection::TextPart;
use crate::screens::home::view::components::{SelectTrigger, choice_emoji, picks_label};
use crate::ui::button::Button;

use super::super::markdown::MarkdownOptions;
use super::{MEDIA_MAX_HEIGHT, MEDIA_MAX_WIDTH};

/// Discord's cap on a container's width.
const CONTAINER_MAX_WIDTH: f32 = 600.;
const CONTAINER_PADDING: f32 = 16.;
const ACCENT_WIDTH: f32 = 4.;
/// A section's thumbnail accessory.
const THUMBNAIL_SIZE: f32 = 85.;
const SELECT_WIDTH: f32 = 400.;
const GALLERY_GAP: f32 = 4.;

/// Threaded through the walk, so every element id is unique in the message.
struct Walk {
    message_id: Id<MessageMarker>,
    /// Interacting needs the server to know the message.
    live: bool,
    next: usize,
}

impl Walk {
    fn id(&mut self, part: &str) -> ElementId {
        self.next += 1;
        ElementId::Name(format!("component-{part}-{}-{}", self.message_id, self.next).into())
    }
}

impl HomeScreen {
    pub(super) fn render_components(
        &self,
        message: &discord::Message,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut walk = Walk {
            message_id: message.id,
            live: message.delivery == discord::Delivery::Sent,
            next: 0,
        };
        let failed = self.components.failed.get(&message.id).cloned();
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .children(
                message
                    .components
                    .iter()
                    .filter(|component| component.is_visible())
                    .map(|component| self.render_component(component, &mut walk, cx))
                    .collect::<Vec<_>>(),
            )
            .when_some(failed, |this, failed| {
                this.child(div().text_xs().text_color(cx.theme().danger).child(failed))
            })
            .into_any_element()
    }

    fn render_component(
        &self,
        component: &Component,
        walk: &mut Walk,
        cx: &Context<Self>,
    ) -> AnyElement {
        match component {
            Component::ActionRow(children) => h_flex()
                .flex_wrap()
                .gap_2()
                .children(
                    children
                        .iter()
                        .map(|child| self.render_component(child, walk, cx))
                        .collect::<Vec<_>>(),
                )
                .into_any_element(),
            Component::Button(button) => self.render_component_button(button, walk, cx),
            Component::Select(select) => self.render_component_select(select, walk, cx),
            Component::Section {
                components,
                accessory,
            } => h_flex()
                .w_full()
                .min_w_0()
                .gap_3()
                .items_start()
                .child(
                    v_flex().flex_1().min_w_0().gap_2().children(
                        components
                            .iter()
                            .map(|child| self.render_component(child, walk, cx))
                            .collect::<Vec<_>>(),
                    ),
                )
                .children(
                    accessory
                        .as_ref()
                        .map(|accessory| self.render_component(accessory, walk, cx)),
                )
                .into_any_element(),
            Component::Text(text) => self.render_markdown(
                &text.markdown,
                MarkdownOptions::new(
                    format!("component-text-{}-{}", walk.message_id, text.index),
                    &[],
                )
                .selectable(walk.message_id.get(), TextPart::Component(text.index)),
                cx,
            ),
            Component::Thumbnail(item) => self.render_thumbnail(item, walk, cx),
            Component::Gallery(items) => self.render_gallery(items, walk, cx),
            Component::File(item) => self.render_file(
                &item.file,
                MediaKey::attachment(walk.message_id.get(), item.index),
                cx,
            ),
            Component::Separator { divider, large } => {
                let space = if *large { 16. } else { 8. };
                div()
                    .w_full()
                    .h(px(space))
                    .flex()
                    .items_center()
                    .when(*divider, |this| {
                        this.child(div().w_full().h(px(1.)).bg(cx.theme().border))
                    })
                    .into_any_element()
            }
            Component::Container {
                accent,
                spoiler,
                components,
            } => self.render_container(*accent, *spoiler, components, walk, cx),
        }
    }

    fn render_component_button(
        &self,
        button: &discord::Button,
        walk: &mut Walk,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = walk.id("button");
        let message_id = walk.message_id;
        let mut element = Button::new(id);
        element = match button.style {
            ButtonStyle::Primary => element.primary(),
            ButtonStyle::Success => element.success(),
            ButtonStyle::Danger => element.danger(),
            ButtonStyle::Secondary | ButtonStyle::Link | ButtonStyle::Premium => element,
        };
        if let Some(emoji) = &button.emoji {
            element = element.glyph(choice_emoji(emoji, px(18.), cx));
        }
        if let Some(label) = &button.label {
            element = element.label(label.clone());
        }
        let element = match &button.action {
            ButtonAction::Interaction(custom_id) => {
                let custom_id = custom_id.clone();
                let pending = self.component_pending(message_id, &custom_id);
                element
                    .loading(pending)
                    .disabled(
                        button.disabled
                            || !walk.live
                            || (!pending && self.components_busy(message_id)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.press_component_button(message_id, custom_id.clone(), cx);
                    }))
            }
            ButtonAction::Link(url) => {
                let url = url.clone();
                element
                    .icon(IconName::ExternalLink)
                    .tooltip(url.clone())
                    .disabled(button.disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // A message link jumps in the app, as one in text does.
                        match discord::MessageLink::parse(&url) {
                            Some(link) => {
                                if !this.open_message_link(link, window, cx) {
                                    cx.open_url(&link.web_url());
                                }
                            }
                            None => cx.open_url(&url),
                        }
                    }))
            }
            ButtonAction::Premium => element
                .disabled(true)
                .tooltip("Purchases aren't supported here"),
        };
        element.into_any_element()
    }

    fn render_component_select(
        &self,
        select: &discord::Select,
        walk: &mut Walk,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = walk.id("select");
        let message_id = walk.message_id;
        let picks = self.select_picks(message_id, select);
        let pending = self.component_pending(message_id, &select.custom_id);
        let busy = self.components_busy(message_id);
        let placeholder = select
            .placeholder
            .clone()
            .unwrap_or_else(|| "Make a selection".into());
        let trigger = SelectTrigger::new(id.clone())
            .label(picks_label(&picks))
            .placeholder(placeholder)
            .loading(pending)
            .disabled(select.disabled || !walk.live || busy);
        if select.disabled || !walk.live || busy {
            return div()
                .w_full()
                .max_w(px(SELECT_WIDTH))
                .child(trigger)
                .into_any_element();
        }
        let picker = self
            .components
            .open_select
            .as_ref()
            .filter(|open| open.message_id == message_id && open.custom_id == select.custom_id)
            .map(|open| open.picker.clone());
        let select = select.clone();
        let trigger_style = StyleRefinement::default()
            .w_full()
            .min_w_0()
            .max_w(px(SELECT_WIDTH));
        gpui_kit::component::popover::Popover::new(id)
            .appearance(false)
            .open(picker.is_some())
            .trigger_style(trigger_style)
            .on_open_change(cx.listener(move |this, open: &bool, window, cx| {
                if *open {
                    this.open_component_select(message_id, select.clone(), window, cx);
                } else {
                    this.close_component_select(cx);
                }
            }))
            .trigger(trigger)
            .content(move |_, _, _| div().children(picker.clone()))
            .into_any_element()
    }

    fn render_thumbnail(
        &self,
        item: &discord::MediaItem,
        walk: &mut Walk,
        cx: &Context<Self>,
    ) -> AnyElement {
        let id = walk.id("thumbnail");
        match &item.media {
            discord::Media::Image(image) => {
                let viewed = ViewerImage::from(image);
                div()
                    .id(id.clone())
                    .flex_shrink_0()
                    .size(px(THUMBNAIL_SIZE))
                    .cursor_pointer()
                    .hover(|this| this.opacity(0.9))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_image_viewer(viewed.clone(), window, cx);
                    }))
                    .child(
                        img(image.url.clone())
                            .image_cache(&self.image_cache)
                            .id(walk.id("thumbnail-media"))
                            .size_full()
                            .rounded(px(8.))
                            .object_fit(ObjectFit::Cover),
                    )
                    .into_any_element()
            }
            discord::Media::Video(video) => self.render_video(
                video,
                MediaKey::attachment(walk.message_id.get(), item.index),
                cx,
            ),
        }
    }

    /// One item at its own shape; more as a grid of cropped tiles, two or three
    /// across as in Discord.
    fn render_gallery(
        &self,
        items: &[discord::MediaItem],
        walk: &mut Walk,
        cx: &Context<Self>,
    ) -> AnyElement {
        let message_id = walk.message_id.get();
        if let [item] = items {
            let key = MediaKey::attachment(message_id, item.index);
            return match &item.media {
                discord::Media::Image(image) => self.render_image(image, key, cx),
                discord::Media::Video(video) => self.render_video(video, key, cx),
            };
        }
        let columns = if items.len() == 2 || items.len() == 4 {
            2.
        } else {
            3.
        };
        let tile = (MEDIA_MAX_WIDTH - GALLERY_GAP * (columns - 1.)) / columns;
        h_flex()
            .w(px(MEDIA_MAX_WIDTH))
            .max_w_full()
            .flex_wrap()
            .gap(px(GALLERY_GAP))
            .children(
                items
                    .iter()
                    .map(|item| {
                        let key = MediaKey::attachment(message_id, item.index);
                        match &item.media {
                            discord::Media::Image(image) if image.spoiler.is_none() => {
                                let viewed = ViewerImage::from(image);
                                div()
                                    .id(walk.id("gallery"))
                                    .size(px(tile.min(MEDIA_MAX_HEIGHT)))
                                    .cursor_pointer()
                                    .hover(|this| this.opacity(0.9))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_image_viewer(viewed.clone(), window, cx);
                                    }))
                                    .child(
                                        img(image.url.clone())
                                            .image_cache(&self.image_cache)
                                            .id(walk.id("gallery-media"))
                                            .size_full()
                                            .rounded(px(4.))
                                            .object_fit(ObjectFit::Cover),
                                    )
                                    .into_any_element()
                            }
                            // Spoilers and videos keep their own covers and players.
                            discord::Media::Image(image) => self.render_image(image, key, cx),
                            discord::Media::Video(video) => self.render_video(video, key, cx),
                        }
                    })
                    .collect::<Vec<_>>(),
            )
            .into_any_element()
    }

    fn render_container(
        &self,
        accent: Option<u32>,
        spoiler: bool,
        components: &[Component],
        walk: &mut Walk,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let id = walk.id("container");
        let reveal: SharedString = format!("container-{}-{}", walk.message_id, walk.next).into();
        let hidden = spoiler && !self.revealed_spoilers.contains(&reveal);
        let content: Vec<_> = components
            .iter()
            .filter(|component| component.is_visible())
            .map(|component| self.render_component(component, walk, cx))
            .collect();
        div()
            .id(id)
            .relative()
            .w_full()
            .max_w(px(CONTAINER_MAX_WIDTH))
            .flex_shrink_0()
            .rounded(px(8.))
            .overflow_hidden()
            .bg(theme.secondary)
            .border_1()
            .border_color(theme.border)
            .when_some(accent, |this, accent| {
                this.border_l(px(ACCENT_WIDTH)).border_color(rgb(accent))
            })
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .p(px(CONTAINER_PADDING))
                    .children(content)
                    .when(hidden, |this| this.invisible()),
            )
            .when(hidden, |this| {
                this.cursor_pointer()
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .bg(theme.muted)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .px_3()
                                    .py_1()
                                    .rounded_full()
                                    .bg(black().opacity(0.6))
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(white())
                                    .child("SPOILER"),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.revealed_spoilers.insert(reveal.clone());
                        cx.notify();
                    }))
            })
            .into_any_element()
    }
}
