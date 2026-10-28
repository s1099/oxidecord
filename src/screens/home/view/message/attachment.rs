//! Inline previews for a message's image and video attachments, and cards
//! for the files that aren't either.

use std::time::Duration;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, progress::Progress, slider::Slider,
    spinner::Spinner, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use twilight_model::id::{Id, marker::MessageMarker};

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::state::{MediaKey, PlaybackState, VideoPlayback};
use crate::ui::button::Button;
use crate::ui::depth::media_shadow;

use super::super::super::data::attachments::format_size;
use super::super::super::data::image_viewer::ViewerImage;
use super::{MEDIA_MAX_HEIGHT, MEDIA_MAX_WIDTH, fit_within};

/// Fallback video card shape when Discord reports no dimensions; about 16:9.
const FALLBACK_VIDEO_SIZE: (f32, f32) = (MEDIA_MAX_WIDTH, MEDIA_MAX_WIDTH * 9. / 16.);

/// A spoiler's real shape is unknown until revealed, so an unsized cover is just a card.
const FALLBACK_COVER_SIZE: (f32, f32) = (MEDIA_MAX_WIDTH, MEDIA_MAX_WIDTH * 9. / 16.);

impl HomeScreen {
    pub(super) fn render_image(
        &self,
        image: &discord::ImageAttachment,
        key: MediaKey,
        cx: &Context<Self>,
    ) -> AnyElement {
        let size = fit_within(image.width, image.height, MEDIA_MAX_WIDTH, MEDIA_MAX_HEIGHT);

        if let Some(cover) = self.unrevealed_spoiler(image.spoiler.as_ref(), key, "image") {
            let (width, height) = size.unwrap_or(FALLBACK_COVER_SIZE);
            return self.render_spoiler_cover(cover, key, "image", width, height, cx);
        }

        // Named on the element: `list` renders items during prepaint, before an ancestor
        // `image_cache(..)` is on the stack, so images would fall back to gpui's global
        // cache, which never evicts.
        let element = img(image.url.clone())
            .image_cache(&self.image_cache)
            .rounded(px(8.))
            .shadow(media_shadow())
            .max_w(px(MEDIA_MAX_WIDTH));
        // Intrinsic dimensions give the exact scaled box, so the message doesn't reflow on load.
        let element = match size {
            Some((width, height)) => element.w(px(width)).h(px(height)),
            None => element.max_h(px(MEDIA_MAX_HEIGHT)),
        };
        // An id gives gpui somewhere to keep an animated image's frame index; without one a GIF
        // stays on frame 0.
        let element = element.id(key.element_id("image-frame"));

        let viewed = ViewerImage::from(image);
        div()
            .id(key.element_id("image"))
            .flex()
            // Otherwise the column stretches it, making the whole row width clickable.
            .self_start()
            .cursor_pointer()
            .child(element)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_image_viewer(viewed.clone(), window, cx);
            }))
            .into_any_element()
    }

    fn unrevealed_spoiler<'a>(
        &self,
        spoiler: Option<&'a discord::SpoilerCover>,
        key: MediaKey,
        role: &str,
    ) -> Option<&'a discord::SpoilerCover> {
        spoiler.filter(|_| !self.revealed_spoilers.contains(&key.spoiler_key(role)))
    }

    /// The attachment isn't fetched until clicked.
    fn render_spoiler_cover(
        &self,
        cover: &discord::SpoilerCover,
        key: MediaKey,
        role: &str,
        width: f32,
        height: f32,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        // The reveal key doubles as the hover group, being unique per cover.
        let reveal = key.spoiler_key(role);

        // gpui packs its atlas without padding, so the tiny picture's edge pixels would sample a
        // neighbouring sprite; drawing half a pixel larger all round pushes that fringe outside the
        // clip.
        // Size is estimated the way the proxy was asked for it.
        let scale = discord::SpoilerCover::SIZE as f32 / width.max(height);
        let texel_width = width / (width * scale).round().max(1.);
        let texel_height = height / (height * scale).round().max(1.);
        let bleed = texel_width.max(texel_height) / 2.;

        div()
            .id(key.element_id(&format!("{role}-spoiler")))
            .group(reveal.clone())
            .relative()
            .w(px(width))
            .h(px(height))
            .rounded(px(8.))
            .shadow(media_shadow())
            .overflow_hidden()
            .bg(theme.muted)
            .cursor_pointer()
            .child(
                img(cover.url.clone())
                    .image_cache(&self.image_cache)
                    .absolute()
                    .top(px(-bleed))
                    .left(px(-bleed))
                    .w(px(width + 2. * bleed))
                    .h(px(height + 2. * bleed))
                    .object_fit(ObjectFit::Cover),
            )
            // A scrim, not a theme colour: the label must read over any blur.
            .child(div().absolute().inset_0().bg(black().opacity(0.45)))
            .child(centered(
                div()
                    .px_3()
                    .py_1()
                    .rounded_full()
                    .bg(black().opacity(0.6))
                    .group_hover(reveal.clone(), |this| this.bg(black().opacity(0.85)))
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(white())
                    .child("SPOILER"),
            ))
            .on_click(cx.listener(move |this, _, _window, cx| {
                this.revealed_spoilers.insert(reveal.clone());
                cx.notify();
            }))
            .into_any_element()
    }

    /// Nothing decodes until play is pressed, and only one clip decodes at a time.
    pub(super) fn render_video(
        &self,
        video: &discord::VideoAttachment,
        key: MediaKey,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        let playback = self.video.as_ref().filter(|playback| playback.key == key);

        // Once a frame arrives the card takes its shape: Discord's dimensions can disagree with
        // the file (anamorphic pixels) or be missing, and sizing to the frame avoids letterboxing.
        let (width, height) = playback
            .and_then(|playback| playback.frame.as_ref())
            .map_or_else(
                || video_size(video),
                |frame| {
                    let size = frame.size(0);
                    fit_preview(f64::from(size.width.0), f64::from(size.height.0))
                },
            );
        let caption = div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("{} · {}", video.filename, format_size(video.size)));

        // Covers the play button too, so a spoiler clip can't start unrevealed.
        if let Some(cover) = self.unrevealed_spoiler(video.spoiler.as_ref(), key, "video") {
            return v_flex()
                .gap(px(2.))
                .child(self.render_spoiler_cover(cover, key, "video", width, height, cx))
                .child(caption)
                .into_any_element();
        }

        let surface = div()
            .id(key.element_id("video"))
            .relative()
            .w(px(width))
            .h(px(height))
            .rounded(px(8.))
            .shadow(media_shadow())
            .overflow_hidden()
            // Black, not themed: it's the letterbox, which reads as part of the video.
            .bg(black())
            .border_1()
            .border_color(theme.border);

        let surface = match playback.and_then(|playback| playback.frame.clone()) {
            // Raw pixels skip the image cache; `ImageSource::Render` goes straight to the renderer.
            Some(frame) => surface.child(img(frame).size_full()),
            None => surface.child(
                img(video.poster_url.clone())
                    .image_cache(&self.image_cache)
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            ),
        };

        // A finished or failed clip has no decoder (the thread exits, releasing the device and
        // temp file), so the card offers play again from the start. Only live playback gets
        // controls.
        let finished = playback.is_some_and(|playback| {
            matches!(
                playback.state,
                PlaybackState::Ended | PlaybackState::Failed(_)
            )
        });
        // The whole card is the hit target, as in Discord: it starts an idle clip and pauses or
        // resumes a playing one. The control bar swallows its own presses.
        let surface = match playback.filter(|_| !finished) {
            Some(playback) => {
                let loading = playback.state == PlaybackState::Loading;
                surface
                    .when(!loading, |this| {
                        this.cursor_pointer().on_click(
                            cx.listener(|this, _, _window, cx| this.toggle_video_paused(cx)),
                        )
                    })
                    .when(loading, |this| {
                        this.child(centered(Spinner::new().large().color(white())))
                    })
                    .when(!loading && playback.player.is_paused(), |this| {
                        this.child(centered(play_badge()))
                    })
                    .child(self.video_controls(playback, key, cx))
            }
            None => {
                let url = video.url.clone();
                surface
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        // The whole preview box, not the card's guessed size: the decoder fits the
                        // true shape inside it. Physical pixels, since decoding at logical size is
                        // soft above 1x.
                        let scale = window.scale_factor();
                        let target = (
                            (MEDIA_MAX_WIDTH * scale).round().max(1.) as u32,
                            (MEDIA_MAX_HEIGHT * scale).round().max(1.) as u32,
                        );
                        this.play_video(key, url.clone(), target, window, cx);
                    }))
                    .when_some(
                        playback.and_then(|playback| match &playback.state {
                            PlaybackState::Failed(error) => Some(error.clone()),
                            _ => None,
                        }),
                        |this, error| {
                            this.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .right_0()
                                    .p_2()
                                    .bg(black().opacity(0.55))
                                    .text_xs()
                                    .text_center()
                                    .text_color(white())
                                    .child(error),
                            )
                        },
                    )
                    .child(centered(play_badge()))
            }
        };

        v_flex()
            .gap(px(2.))
            .child(surface)
            .child(caption)
            .into_any_element()
    }

    /// Name and download button both open the URL in the browser, which downloads it.
    /// A pending message's files on their way up, with how far along they are.
    pub(super) fn render_upload(
        &self,
        message_id: Id<MessageMarker>,
        upload: &discord::Upload,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let name = match upload.filenames.as_slice() {
            [filename] => filename.clone(),
            filenames => format!("{} files", filenames.len()),
        };
        let (sent, total) = (upload.progress.sent(), upload.progress.total());
        let percent = if total == 0 {
            100.
        } else {
            sent as f32 / total as f32 * 100.
        };

        h_flex()
            .w_full()
            .max_w(px(MEDIA_MAX_WIDTH))
            .p_3()
            .gap_3()
            .items_center()
            .rounded(px(8.))
            .bg(theme.muted.opacity(0.5))
            .border_1()
            .border_color(theme.border)
            .child(
                Icon::default()
                    .path("icons/file.svg")
                    .size_8()
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().truncate().text_sm().child(name))
                    .child(
                        Progress::new(("upload-progress", message_id.get()))
                            .small()
                            .value(percent)
                            .accessibility_label("Upload progress"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{} of {}", format_size(sent), format_size(total))),
                    ),
            )
            .child(
                Button::new(("upload-cancel", message_id.get()))
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .tooltip("Cancel upload")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cancel_upload(message_id, cx);
                    })),
            )
            .into_any_element()
    }

    pub(super) fn render_file(
        &self,
        file: &discord::FileAttachment,
        key: MediaKey,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let open = {
            let url = file.url.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| cx.open_url(&url)
        };

        h_flex()
            .w_full()
            .max_w(px(MEDIA_MAX_WIDTH))
            .p_3()
            .gap_3()
            .items_center()
            .rounded(px(8.))
            .bg(theme.muted.opacity(0.5))
            .border_1()
            .border_color(theme.border)
            .child(
                Icon::default()
                    .path("icons/file.svg")
                    .size_8()
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .id(key.element_id("file-name"))
                            .truncate()
                            .text_sm()
                            .text_color(theme.link)
                            .hover(|this| this.text_color(theme.link_hover).underline())
                            .cursor_pointer()
                            .child(file.filename.clone())
                            .on_click(open.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format_size(file.size)),
                    ),
            )
            .child(
                Button::new(key.element_id("file-download"))
                    .icon(Icon::default().path("icons/download.svg"))
                    .ghost()
                    .small()
                    .tooltip("Download")
                    .on_click(open),
            )
            .into_any_element()
    }

    fn video_controls(
        &self,
        playback: &VideoPlayback,
        key: MediaKey,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let paused = playback.player.is_paused();
        let muted = playback.player.is_muted();

        let toggle = Button::new(key.element_id("video-pause"))
            .icon(Icon::default().path(if paused {
                "icons/play.svg"
            } else {
                "icons/pause.svg"
            }))
            .ghost()
            .xsmall()
            .text_color(white())
            .tooltip(if paused { "Play" } else { "Pause" })
            .on_click(cx.listener(|this, _, _window, cx| this.toggle_video_paused(cx)));

        let sound = Button::new(key.element_id("video-mute"))
            .icon(Icon::default().path(if muted {
                "icons/volume-x.svg"
            } else {
                "icons/volume-2.svg"
            }))
            .ghost()
            .xsmall()
            .text_color(white())
            .tooltip(if muted { "Unmute" } else { "Mute" })
            .on_click(cx.listener(|this, _, _window, cx| this.toggle_video_muted(cx)));

        h_flex()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .px_2()
            .py_1()
            .gap_2()
            .items_center()
            // Presses here are for the controls, not the pause toggle on the card beneath.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // A scrim: the bar sits over arbitrary video, so contrast must come from itself.
            .bg(black().opacity(0.55))
            .child(toggle)
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(white())
                    .child(format!(
                        "{} / {}",
                        format_time(playback.position),
                        format_time(playback.duration)
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Slider::new(&playback.scrubber).horizontal()),
            )
            .child(sound)
    }
}

/// Decoration only: the card under it takes the click.
fn play_badge() -> impl IntoElement {
    div()
        .size_12()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(black().opacity(0.6))
        .child(
            Icon::default()
                .path("icons/play.svg")
                .size_6()
                .text_color(white()),
        )
}

fn centered(child: impl IntoElement) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(child)
}

/// Fixed before decoding so playback doesn't reflow the message, and reused as the decode
/// target so frames arrive at the size they're drawn.
fn video_size(video: &discord::VideoAttachment) -> (f32, f32) {
    match (video.width, video.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            fit_preview(f64::from(width), f64::from(height))
        }
        _ => FALLBACK_VIDEO_SIZE,
    }
}

/// May scale up, unlike the decoder's fit, so a small video still gets room for controls.
fn fit_preview(width: f64, height: f64) -> (f32, f32) {
    let scale = (f64::from(MEDIA_MAX_WIDTH) / width).min(f64::from(MEDIA_MAX_HEIGHT) / height);
    ((width * scale) as f32, (height * scale) as f32)
}

fn format_time(position: Duration) -> String {
    let total = position.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}
