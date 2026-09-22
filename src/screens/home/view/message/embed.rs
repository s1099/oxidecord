//! Rich embeds under a message: the bordered card and the bare-media preview.
//!
//! The card mirrors Discord's layout — a coloured spine down the left edge, a
//! content column holding provider, author, title, description and fields, a
//! square thumbnail tucked into the top-right corner, then the large image and
//! the footer line beneath it all.
//!
//! Debug builds overlay a copy button on every embed that yields the raw JSON
//! Discord sent, so a mismatch against the real client can be reported with the
//! exact input that produced it.

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{ActiveTheme as _, h_flex, v_flex};

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::state::MediaKey;
use crate::ui::elevation::media_shadow;

use super::super::text::render_message_text;
use super::{MEDIA_MAX_HEIGHT, MEDIA_MAX_WIDTH, fit_within};

/// The card's width: Discord's 432px cap, or the message column when that is
/// narrower. Unlike Discord, a short embed doesn't hug its text — see the width
/// note on the card itself.
const CARD_MAX_WIDTH: f32 = 432.;
/// Padding inside the card, past the spine. Discord's is asymmetric on every
/// edge: less above the first row than below the last, and the left is short by
/// the spine's width so the text still lands 16px in from the card's edge.
const CARD_PADDING_TOP: f32 = 8.;
const CARD_PADDING_BOTTOM: f32 = 16.;
const CARD_PADDING_RIGHT: f32 = 16.;
const CARD_PADDING_LEFT: f32 = 12.;
/// The coloured spine down the card's left edge.
const SPINE_WIDTH: f32 = 4.;
/// The square the top-right thumbnail is fitted into.
const THUMBNAIL_BOX: f32 = 80.;
/// Vertical gap between the card's rows.
const ROW_GAP: f32 = 8.;
/// The large image gets twice the usual gap above it, so it reads as its own
/// block rather than another line of the text column.
const IMAGE_GAP: f32 = 16.;
/// Line heights for the card's three type sizes. gpui defaults to the golden
/// ratio, which is far looser than Discord's embed text and leaves the
/// description looking double-spaced, so every row sets its own.
const LINE_HEIGHT_TITLE: f32 = 22.;
const LINE_HEIGHT_BODY: f32 = 18.;
const LINE_HEIGHT_SMALL: f32 = 16.;

impl HomeScreen {
    /// Every embed on a message, stacked. `message_id` only seeds element ids,
    /// which have to stay unique across the whole message list.
    pub(super) fn render_embeds(
        &self,
        message_id: u64,
        embeds: &[discord::Embed],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .gap_2()
            .children(embeds.iter().enumerate().map(|(index, embed)| {
                let id = MediaKey { message_id, index };
                match embed.layout {
                    discord::EmbedLayout::Card => self.render_embed_card(id, embed, cx),
                    discord::EmbedLayout::Media => self.render_embed_media(id, embed, cx),
                }
            }))
    }

    /// The bordered card: `rich` embeds from bots, and link and article
    /// previews.
    fn render_embed_card(
        &self,
        id: MediaKey,
        embed: &discord::Embed,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        // Everything in the left column, in Discord's order. The thumbnail sits
        // beside it rather than in it, so text wraps around the corner square.
        let column = v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(ROW_GAP))
            .children(embed.provider.clone().map(|provider| {
                div()
                    .text_xs()
                    .line_height(px(LINE_HEIGHT_SMALL))
                    .text_color(theme.foreground)
                    .child(provider)
            }))
            .children(
                embed
                    .author
                    .as_ref()
                    .map(|author| render_author(id, author, &self.image_cache, theme.link_hover)),
            )
            .children(embed.title.clone().map(|title| {
                let styled = div()
                    .text_size(px(16.))
                    .line_height(px(LINE_HEIGHT_TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title);
                // A title with a URL is the embed's primary link, so the whole
                // line is clickable and takes the theme's link colour.
                match embed.url.clone() {
                    Some(url) => div()
                        .id(id.element_id("embed-title"))
                        .cursor_pointer()
                        .text_color(theme.link)
                        .hover(|this| this.text_color(theme.link_hover))
                        .child(styled)
                        .on_click(move |_, _, cx| cx.open_url(&url))
                        .into_any_element(),
                    None => styled.into_any_element(),
                }
            }))
            .children(embed.description.as_ref().map(|description| {
                div()
                    .text_sm()
                    .line_height(px(LINE_HEIGHT_BODY))
                    .child(render_message_text(
                        id.element_id("embed-description"),
                        description,
                        theme.link,
                    ))
            }))
            .when(!embed.fields.is_empty(), |this| {
                this.child(render_fields(id, &embed.fields, theme.link))
            });

        let body = h_flex()
            .w_full()
            .min_w_0()
            .gap(px(16.))
            .items_start()
            .child(column)
            .children(
                embed
                    .thumbnail
                    .as_ref()
                    .map(|thumbnail| render_thumbnail(thumbnail, &self.image_cache)),
            );

        // The card's padding applies to every row including the media, so an
        // image is inset rather than flush — only a bare `Media` embed, which
        // has no card at all, sits edge to edge.
        let has_body = embed.has_text() || embed.thumbnail.is_some();
        let inner = v_flex()
            .min_w_0()
            .gap(px(ROW_GAP))
            .pl(px(CARD_PADDING_LEFT))
            .pr(px(CARD_PADDING_RIGHT))
            .pt(px(CARD_PADDING_TOP))
            .pb(px(CARD_PADDING_BOTTOM))
            .when(has_body, |this| this.child(body))
            .children(embed.image.as_ref().map(|image| {
                // `gap` already contributes `ROW_GAP`; top up to the image's
                // wider gap, but only when there is actually a row above it.
                // The margin has to go on the image element itself — wrapping it
                // in a plain (block) div hides its width from this column's
                // intrinsic sizing, which collapses the rows below it.
                let extra = if has_body { IMAGE_GAP - ROW_GAP } else { 0. };
                self.render_embed_image(id, embed, image, extra, cx)
            }))
            .children(
                (embed.footer.is_some() || embed.timestamp.is_some()).then(|| {
                    render_footer(
                        embed.footer.as_ref(),
                        embed.timestamp.as_deref(),
                        &self.image_cache,
                        theme.muted_foreground,
                    )
                }),
            );

        div()
            .id(id.element_id("embed"))
            .group(id.group())
            .relative()
            // An absolute width, not a percentage of the message column. Every
            // width below this point is derived from it, and a percentage that
            // resolves to one value while the card is measured and another when
            // it is painted makes the text wrap to a different number of lines
            // in each pass — leaving the card too short for what it draws. The
            // `max-width` only matters in a window too narrow for the full card.
            .w(px(CARD_MAX_WIDTH))
            .max_w(relative(1.))
            // The card is as tall as its content, never shorter. A flex item's
            // automatic minimum height is what normally guarantees that, but
            // `overflow: hidden` waives it — so a tall card would be squeezed by
            // the column above and silently clip its last fields instead.
            .flex_shrink_0()
            .rounded(px(4.))
            .shadow(media_shadow())
            .bg(theme.secondary)
            // The spine is a thick left border rather than a child, so the
            // card's rounded corners clip it for free.
            .border_l(px(SPINE_WIDTH))
            .border_color(embed.color.map_or(theme.border, |color| rgb(color).into()))
            .child(inner)
            .children(self.render_embed_debug_copy(id, embed, cx))
            .into_any_element()
    }

    /// A bare image, GIF, or video preview: no card, no spine — just the media,
    /// the way Discord renders a lone media link.
    fn render_embed_media(
        &self,
        id: MediaKey,
        embed: &discord::Embed,
        cx: &Context<Self>,
    ) -> AnyElement {
        let Some(image) = embed.image.as_ref() else {
            return div().into_any_element();
        };

        // Sized to the media rather than stretched across the message, so the
        // debug copy button anchors to the image's own corner.
        let size = fit_within(image.width, image.height, MEDIA_MAX_WIDTH, MEDIA_MAX_HEIGHT);
        div()
            .id(id.element_id("embed"))
            .group(id.group())
            .relative()
            .map(|this| match size {
                Some((width, _)) => this.w(px(width)),
                None => this.max_w(px(MEDIA_MAX_WIDTH)),
            })
            .child(self.render_embed_image(id, embed, image, 0., cx))
            .children(self.render_embed_debug_copy(id, embed, cx))
            .into_any_element()
    }

    /// The large image, scaled into the 400x300 box and clickable. Video embeds
    /// get a play badge over it and open the source link instead of the frame.
    fn render_embed_image(
        &self,
        id: MediaKey,
        embed: &discord::Embed,
        image: &discord::EmbedMedia,
        top_gap: f32,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let size = fit_within(image.width, image.height, MEDIA_MAX_WIDTH, MEDIA_MAX_HEIGHT);

        // The cache has to be named on the element itself: `list` renders its
        // items during prepaint, so an ancestor `image_cache(..)` — which only
        // pushes during layout and paint — would be missed entirely. See
        // `attachment::render_image`.
        let mut media = img(image.url.clone())
            .image_cache(&self.image_cache)
            .rounded(px(4.))
            // Only bare media is lifted off the background. Inside a card the
            // card already casts the shadow, and a second one under the image
            // would read as the picture floating above its own embed.
            .when(
                matches!(embed.layout, discord::EmbedLayout::Media),
                |this| this.shadow(media_shadow()),
            )
            .max_w(px(MEDIA_MAX_WIDTH));
        match size {
            Some((width, height)) => media = media.w(px(width)).h(px(height)),
            None => media = media.max_h(px(MEDIA_MAX_HEIGHT)),
        }

        // Clicking opens the source page for a video and the image itself
        // otherwise, so a YouTube preview lands on YouTube rather than on a
        // still frame.
        let target = if embed.has_video {
            embed.url.clone()
        } else {
            Some(image.url.clone())
        };

        div()
            .id(id.element_id("embed-image"))
            .relative()
            .flex()
            .mt(px(top_gap))
            .when_some(target, |this, url| {
                this.cursor_pointer()
                    .hover(|this| this.opacity(0.9))
                    .on_click(move |_, _, cx| cx.open_url(&url))
            })
            .child(media)
            .when(embed.has_video, |this| {
                this.child(
                    // Centred over the frame without knowing its size: a full
                    // overlay that centres its own child.
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(px(48.))
                                .rounded_full()
                                .bg(black().opacity(0.6))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    div()
                                        .text_size(px(20.))
                                        .text_color(theme.primary_foreground)
                                        .child("▶"),
                                ),
                        ),
                )
            })
    }

    /// The debug-only button that copies the embed's raw JSON, revealed while
    /// the embed is hovered. Release builds don't carry the JSON, so there is
    /// nothing to render.
    #[cfg(debug_assertions)]
    fn render_embed_debug_copy(
        &self,
        id: MediaKey,
        embed: &discord::Embed,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        use gpui_component::clipboard::Clipboard;

        let theme = cx.theme();
        Some(
            div()
                .absolute()
                .top(px(4.))
                .right(px(4.))
                .invisible()
                .group_hover(id.group(), |this| this.visible())
                .p(px(2.))
                .rounded(px(6.))
                .bg(theme.popover)
                .border_1()
                .border_color(theme.border)
                .shadow_sm()
                .child(Clipboard::new(id.element_id("embed-copy-raw")).value(embed.raw.clone())),
        )
    }

    #[cfg(not(debug_assertions))]
    fn render_embed_debug_copy(
        &self,
        _id: MediaKey,
        _embed: &discord::Embed,
        _cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        None::<Div>
    }
}

/// The author row: a small round icon and the name. Unlike the title, an author
/// link keeps the body text colour and only underlines on hover, the way
/// Discord draws it.
fn render_author(
    id: MediaKey,
    author: &discord::EmbedAuthor,
    cache: &Entity<RetainAllImageCache>,
    hover_color: Hsla,
) -> impl IntoElement {
    let name = div()
        .text_sm()
        .line_height(px(LINE_HEIGHT_BODY))
        .font_weight(FontWeight::SEMIBOLD)
        .child(author.name.clone());

    h_flex()
        .gap(px(8.))
        .items_center()
        .children(author.icon_url.clone().map(|url| {
            img(url)
                .image_cache(cache)
                .size(px(24.))
                .rounded_full()
                .flex_shrink_0()
        }))
        .child(match author.url.clone() {
            Some(url) => div()
                .id(id.element_id("embed-author"))
                .cursor_pointer()
                .hover(|this| this.text_color(hover_color).underline())
                .child(name)
                .on_click(move |_, _, cx| cx.open_url(&url))
                .into_any_element(),
            None => name.into_any_element(),
        })
}

/// The square image in the card's top-right corner, fitted into an 80x80 box
/// with its aspect ratio preserved.
fn render_thumbnail(
    thumbnail: &discord::EmbedMedia,
    cache: &Entity<RetainAllImageCache>,
) -> impl IntoElement {
    let mut element = img(thumbnail.url.clone())
        .image_cache(cache)
        .rounded(px(4.))
        .flex_shrink_0();
    match fit_within(
        thumbnail.width,
        thumbnail.height,
        THUMBNAIL_BOX,
        THUMBNAIL_BOX,
    ) {
        Some((width, height)) => element = element.w(px(width)).h(px(height)),
        None => element = element.size(px(THUMBNAIL_BOX)),
    }
    element
}

/// The field grid. Discord lays fields over twelve columns: a run of `inline`
/// fields packs up to three to a row, and anything not inline takes a row of its
/// own. A row of two splits in half; rows of one or three give each field a
/// third of the width, so a leftover inline field stays narrow rather than
/// stretching across the card.
fn render_fields(
    id: MediaKey,
    fields: &[discord::EmbedField],
    link_color: Hsla,
) -> impl IntoElement {
    let mut rows: Vec<Vec<(usize, &discord::EmbedField)>> = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        let extends_run = field.inline
            && rows
                .last()
                .is_some_and(|row| row.len() < 3 && row[0].1.inline);
        if extends_run {
            rows.last_mut().expect("row exists").push((index, field));
        } else {
            rows.push(vec![(index, field)]);
        }
    }

    v_flex()
        .w_full()
        .min_w_0()
        .gap(px(ROW_GAP))
        .children(rows.into_iter().map(|row| {
            // A row holding one full-width field is emitted as a plain block
            // rather than a flex row. `flex_1` would give it a percentage
            // flex-basis, and a percentage that resolves differently while the
            // row is being measured than when it is painted makes the text wrap
            // to a different number of lines in each pass — so the card ends up
            // shorter than the text it paints.
            if !row[0].1.inline {
                let (index, field) = row[0];
                return div()
                    .w_full()
                    .child(render_field(id, index, field, link_color))
                    .into_any_element();
            }

            let share = if row.len() == 2 { 0.5 } else { 1. / 3. };
            h_flex()
                .w_full()
                .min_w_0()
                .gap(px(8.))
                .items_start()
                .children(row.into_iter().map(|(index, field)| {
                    div()
                        .w(relative(share))
                        .min_w_0()
                        .child(render_field(id, index, field, link_color))
                }))
                .into_any_element()
        }))
}

/// One field: its bold name over its value.
fn render_field(
    id: MediaKey,
    index: usize,
    field: &discord::EmbedField,
    link_color: Hsla,
) -> impl IntoElement {
    // Both halves need ids unique across the message list, and the field index
    // is the only thing distinguishing them within an embed.
    let value_id = ElementId::NamedInteger(
        SharedString::from(format!("embed-field-{}-{}", id.message_id, id.index)),
        index as u64,
    );

    v_flex()
        .min_w_0()
        .gap(px(2.))
        .child(
            div()
                .text_sm()
                .line_height(px(LINE_HEIGHT_BODY))
                .font_weight(FontWeight::SEMIBOLD)
                .child(field.name.clone()),
        )
        .child(
            div()
                .text_sm()
                .line_height(px(LINE_HEIGHT_BODY))
                .child(render_message_text(value_id, &field.value, link_color)),
        )
}

/// The footer line: a small round icon, the footer text, and the timestamp,
/// separated by Discord's bullet.
fn render_footer(
    footer: Option<&discord::EmbedFooter>,
    timestamp: Option<&str>,
    cache: &Entity<RetainAllImageCache>,
    muted: Hsla,
) -> impl IntoElement {
    let parts: Vec<String> = footer
        .map(|footer| footer.text.clone())
        .into_iter()
        .chain(timestamp.map(str::to_string))
        .collect();

    h_flex()
        .w_full()
        .gap(px(8.))
        .items_center()
        .text_xs()
        .line_height(px(LINE_HEIGHT_SMALL))
        .text_color(muted)
        .children(
            footer
                .and_then(|footer| footer.icon_url.clone())
                .map(|url| {
                    img(url)
                        .image_cache(cache)
                        .size(px(20.))
                        .rounded_full()
                        .flex_shrink_0()
                }),
        )
        // Deliberately no `min_w_0`: the min-content floor is what stops a
        // squeezed row from wrapping the timestamp one character per line.
        .child(div().child(parts.join(" • ")))
}
