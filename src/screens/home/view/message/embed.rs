//! Rich embeds under a message: the bordered card and the bare-media preview.
//!
//! Debug builds overlay a copy button on each embed that yields the raw JSON Discord sent,
//! so a mismatch against the real client can be reported with its exact input.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::data::gif::gif_source;
use crate::screens::home::data::image_viewer::ViewerImage;
use crate::screens::home::media::MediaKey;
use crate::screens::home::selection::{EmbedPiece, TextPart};
use crate::ui::depth::media_shadow;

use super::super::markdown::MarkdownOptions;
use super::{MEDIA_MAX_HEIGHT, MEDIA_MAX_WIDTH, fit_within};

/// Discord's 516px cap, spine included, or the message column when narrower. Any line that
/// wraps reports the full wrap width, so a card with one is always at the cap, as in Discord.
const CARD_MAX_WIDTH: f32 = 516.;
/// Discord's is asymmetric: less above the first row than below the last, and the left is
/// short by the spine's width so text lands 16px in.
const CARD_PADDING_TOP: f32 = 8.;
const CARD_PADDING_BOTTOM: f32 = 16.;
const CARD_PADDING_RIGHT: f32 = 16.;
const CARD_PADDING_LEFT: f32 = 12.;
const SPINE_WIDTH: f32 = 4.;
const THUMBNAIL_BOX: f32 = 80.;
const ROW_GAP: f32 = 8.;
/// Twice the usual gap, so the large image reads as its own block.
const IMAGE_GAP: f32 = 16.;
/// gpui's default golden-ratio line height is far looser than Discord's embed text, so
/// every row sets its own.
const LINE_HEIGHT_TITLE: f32 = 19.;
const LINE_HEIGHT_BODY: f32 = 17.;
const LINE_HEIGHT_SMALL: f32 = 14.;
/// Discord's embed sizes scaled to this app's 14px message text, as Discord's are to its 16px:
/// the title matches the message, the rest sit a step and two below it.
const TEXT_TITLE: Rems = Rems(0.875);
const TEXT_BODY: Rems = Rems(0.8125);
const TEXT_SMALL: Rems = Rems(0.6875);

impl HomeScreen {
    /// `message_id` only seeds element ids, which must be unique across the message list.
    pub(super) fn render_embeds(
        &self,
        message_id: u64,
        embeds: &[discord::Embed],
        mentions: &[discord::MentionedUser],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            // Not stretched, so each card is as wide as its content, as in Discord.
            .items_start()
            .gap_2()
            .children(embeds.iter().enumerate().map(|(index, embed)| {
                let id = MediaKey {
                    message_id,
                    embed: true,
                    index,
                };
                match embed.layout {
                    discord::EmbedLayout::Card => self.render_embed_card(id, embed, mentions, cx),
                    discord::EmbedLayout::Media => self.render_embed_media(id, embed, cx),
                }
            }))
    }

    fn render_embed_card(
        &self,
        id: MediaKey,
        embed: &discord::Embed,
        mentions: &[discord::MentionedUser],
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let scope = |part: &str| format!("embed-{part}-{}-{}", id.message_id, id.index);
        let part = |piece| TextPart::Embed {
            embed: id.index,
            piece,
        };

        // The thumbnail sits beside this column, not in it, so text wraps around the corner square.
        let column = v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(ROW_GAP))
            .children(embed.provider.clone().map(|provider| {
                div()
                    .text_size(TEXT_SMALL)
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
            .children(embed.title.as_ref().map(|title| {
                // A linked title is clickable as a whole, so nothing inside it takes the click.
                let options = MarkdownOptions::new(scope("title"), mentions)
                    .interactive(embed.url.is_none())
                    .selectable(id.message_id, part(EmbedPiece::Title));
                let styled = div()
                    .text_size(TEXT_TITLE)
                    .line_height(px(LINE_HEIGHT_TITLE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.render_inline_markdown(title, options, cx));
                match embed.url.clone() {
                    Some(url) => div()
                        .id(id.element_id("embed-title"))
                        .cursor_pointer()
                        .text_color(theme.link)
                        .hover(|this| this.text_color(theme.link_hover))
                        .child(styled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            // A drag that selected the title wasn't a click.
                            if !this.has_text_selection() {
                                cx.open_url(&url);
                            }
                        }))
                        .into_any_element(),
                    None => styled.into_any_element(),
                }
            }))
            .children(embed.description.as_ref().map(|description| {
                div()
                    .text_size(TEXT_BODY)
                    .line_height(px(LINE_HEIGHT_BODY))
                    .child(
                        self.render_markdown(
                            description,
                            MarkdownOptions::new(scope("description"), mentions)
                                .selectable(id.message_id, part(EmbedPiece::Description)),
                            cx,
                        ),
                    )
            }))
            .when(!embed.fields.is_empty(), |this| {
                this.child(self.render_fields(id, &embed.fields, mentions, cx))
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

        // Padding applies to the media too, so an image is inset; only a bare `Media` embed is
        // edge to edge.
        let has_body = embed.has_text() || embed.thumbnail.is_some();
        let inner = v_flex()
            .min_w_0()
            .gap(px(ROW_GAP))
            .pl(px(CARD_PADDING_LEFT))
            .pr(px(CARD_PADDING_RIGHT))
            .pt(px(CARD_PADDING_TOP))
            .pb(px(CARD_PADDING_BOTTOM))
            .when(has_body, |this| this.child(body))
            .map(|this| {
                // `gap` already adds `ROW_GAP`; top up to the wider gap only when a row is above.
                // The margin goes on the media itself: a block div wrapper hides its width from
                // this column's intrinsic sizing and collapses the rows below.
                let extra = if has_body { IMAGE_GAP - ROW_GAP } else { 0. };
                match (&embed.video, &embed.image) {
                    (Some(video), _) => this.child(
                        div()
                            .flex()
                            .mt(px(extra))
                            .child(self.render_embed_video(id, video, cx)),
                    ),
                    (None, Some(image)) => {
                        this.child(self.render_embed_image(id, embed, image, extra, cx))
                    }
                    (None, None) => this,
                }
            })
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
            // Fits its content up to the cap; the parent's width bounds it in a narrow window.
            .max_w(px(CARD_MAX_WIDTH))
            // Never shorter than its content: `overflow: hidden` waives a flex item's automatic
            // minimum height, so a tall card would be squeezed by the column above and clip its
            // fields.
            .flex_shrink_0()
            .rounded(px(4.))
            .shadow(media_shadow())
            .bg(theme.secondary)
            // A thick left border rather than a child, so the rounded corners clip it for free.
            .border_l(px(SPINE_WIDTH))
            .border_color(embed.color.map_or(theme.border, |color| rgb(color).into()))
            .child(inner)
            .children(self.render_embed_debug_copy(id, embed, cx))
            .into_any_element()
    }

    fn render_embed_media(
        &self,
        id: MediaKey,
        embed: &discord::Embed,
        cx: &Context<Self>,
    ) -> AnyElement {
        if let Some(video) = &embed.video {
            return div()
                .id(id.element_id("embed"))
                .group(id.group())
                .relative()
                .child(self.render_embed_video(id, video, cx))
                .children(self.render_embed_debug_copy(id, embed, cx))
                .into_any_element();
        }
        let Some(image) = embed.image.as_ref() else {
            return div().into_any_element();
        };

        // Sized to the media so the debug copy button anchors to the image's own corner.
        let size = media_size(embed, image);
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

    fn render_embed_video(
        &self,
        id: MediaKey,
        video: &discord::EmbedVideo,
        cx: &Context<Self>,
    ) -> AnyElement {
        self.render_video_player(
            &video.url,
            &video.poster_url,
            (video.width, video.height),
            id,
            cx,
        )
    }

    /// Video embeds get a play badge and open the source link instead of the frame.
    fn render_embed_image(
        &self,
        id: MediaKey,
        embed: &discord::Embed,
        image: &discord::EmbedMedia,
        top_gap: f32,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let size = media_size(embed, image);

        // Named on the element, as in `attachment::render_image`: `list` renders during prepaint,
        // which an ancestor `image_cache(..)` would miss.
        let mut media = img(image.url.clone())
            .image_cache(&self.image_cache)
            .rounded(px(4.))
            // Only bare media is lifted; inside a card a second shadow would make the image float
            // above its own embed.
            .when(
                matches!(embed.layout, discord::EmbedLayout::Media),
                |this| this.shadow(media_shadow()),
            )
            .max_w(px(MEDIA_MAX_WIDTH));
        match size {
            Some((width, height)) => media = media.w(px(width)).h(px(height)),
            None => media = media.max_h(px(MEDIA_MAX_HEIGHT)),
        }
        // An id gives gpui somewhere to keep an animated image's frame index; without one a GIF
        // stays on frame 0.
        let media = media.id(id.element_id("embed-media"));

        // A GIF served as video loops over its poster, which stays underneath until the clip
        // loads (and for good if it fails).
        let media = match (&embed.gif, size) {
            (Some(gif), Some((width, height))) => div()
                .relative()
                .w(px(width))
                .h(px(height))
                .child(media)
                .child(
                    img(gif_source(
                        gif.url.clone(),
                        (width, height),
                        self.gifs.clone(),
                    ))
                    .id(id.element_id("embed-gif"))
                    .absolute()
                    .inset_0()
                    .size_full()
                    .rounded(px(4.)),
                )
                .into_any_element(),
            _ => media.into_any_element(),
        };

        // Video opens the source page, so a YouTube preview lands on YouTube; anything else opens
        // the image viewer. A looping GIF opens nothing, as in Discord.
        let video_url = embed.url.clone().filter(|_| embed.has_video);
        let viewed = (!embed.has_video && embed.gif.is_none()).then(|| ViewerImage::from(image));

        div()
            .id(id.element_id("embed-image"))
            .relative()
            .flex()
            // Only as wide as the image, so the space beside it isn't a link.
            .self_start()
            .mt(px(top_gap))
            .when_some(video_url, |this, url| {
                this.cursor_pointer()
                    .hover(|this| this.opacity(0.9))
                    .on_click(move |_, _, cx| cx.open_url(&url))
            })
            .when_some(viewed, |this, viewed| {
                this.cursor_pointer()
                    .hover(|this| this.opacity(0.9))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_image_viewer(viewed.clone(), window, cx);
                    }))
            })
            .child(media)
            .when(embed.has_video, |this| {
                this.child(
                    // Centred without knowing its size: a full overlay that centres its child.
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

    /// Discord lays fields over twelve columns: inline runs pack up to three per row, other
    /// fields take a row. Two split in half; one or three get a third each, so a leftover
    /// inline field stays narrow.
    fn render_fields(
        &self,
        id: MediaKey,
        fields: &[discord::EmbedField],
        mentions: &[discord::MentionedUser],
        cx: &Context<Self>,
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
                // A one-field row is a plain block, not a flex row: `flex_1`'s percentage basis
                // resolves
                // differently when measured than painted, wrapping to different line counts and
                // leaving
                // the card too short.
                if !row[0].1.inline {
                    let (index, field) = row[0];
                    return div()
                        .w_full()
                        .child(self.render_field(id, index, field, mentions, cx))
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
                            .child(self.render_field(id, index, field, mentions, cx))
                    }))
                    .into_any_element()
            }))
    }

    fn render_field(
        &self,
        id: MediaKey,
        index: usize,
        field: &discord::EmbedField,
        mentions: &[discord::MentionedUser],
        cx: &Context<Self>,
    ) -> impl IntoElement {
        // Ids must be unique across the message list; the field index distinguishes the halves.
        let scope =
            |part: &str| format!("embed-field-{part}-{}-{}-{index}", id.message_id, id.index);
        let part = |piece| TextPart::Embed {
            embed: id.index,
            piece,
        };

        v_flex()
            .min_w_0()
            .gap(px(2.))
            .child(
                div()
                    .text_size(TEXT_BODY)
                    .line_height(px(LINE_HEIGHT_BODY))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(
                        self.render_inline_markdown(
                            &field.name,
                            MarkdownOptions::new(scope("name"), mentions)
                                .selectable(id.message_id, part(EmbedPiece::FieldName(index))),
                            cx,
                        ),
                    ),
            )
            .child(
                div()
                    .text_size(TEXT_BODY)
                    .line_height(px(LINE_HEIGHT_BODY))
                    .child(
                        self.render_markdown(
                            &field.value,
                            MarkdownOptions::new(scope("value"), mentions)
                                .selectable(id.message_id, part(EmbedPiece::FieldValue(index))),
                            cx,
                        ),
                    ),
            )
    }

    /// Debug-only; release builds don't carry the raw JSON.
    #[cfg(debug_assertions)]
    fn render_embed_debug_copy(
        &self,
        id: MediaKey,
        embed: &discord::Embed,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        use gpui_kit::component::clipboard::Clipboard;

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

/// A GIF's poster is the clip's first frame, so the clip's dimensions stand in when the
/// poster's are missing.
fn media_size(embed: &discord::Embed, image: &discord::EmbedMedia) -> Option<(f32, f32)> {
    let (width, height) = match (&embed.gif, image.width.zip(image.height)) {
        (Some(gif), None) => (gif.width, gif.height),
        _ => (image.width, image.height),
    };
    fit_within(width, height, MEDIA_MAX_WIDTH, MEDIA_MAX_HEIGHT)
}

/// Unlike the title, an author link keeps the body colour and underlines only on hover.
fn render_author(
    id: MediaKey,
    author: &discord::EmbedAuthor,
    cache: &Entity<RetainAllImageCache>,
    hover_color: Hsla,
) -> impl IntoElement {
    let name = div()
        .text_size(TEXT_BODY)
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
        .text_size(TEXT_SMALL)
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
