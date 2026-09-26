//! Inline previews for a message's image and video attachments.

use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, h_flex, slider::Slider, spinner::Spinner, v_flex,
};

use crate::discord;
use crate::screens::home::HomeScreen;
use crate::screens::home::state::{MediaKey, PlaybackState, VideoPlayback};
use crate::ui::button::Button;
use crate::ui::depth::media_shadow;

use super::super::super::data::attachments::format_size;
use super::{MEDIA_MAX_HEIGHT, MEDIA_MAX_WIDTH, fit_within};

/// What a video card falls back to when Discord reports no dimensions —
/// roughly 16:9 at the width cap, which is what most attachments turn out to
/// be, so the card rarely resizes once the first frame arrives.
const FALLBACK_VIDEO_SIZE: (f32, f32) = (MEDIA_MAX_WIDTH, MEDIA_MAX_WIDTH * 9. / 16.);

/// What a spoiler image with no reported dimensions is covered at. Its real
/// shape is unknown until it's revealed, so the cover is just a card.
const FALLBACK_COVER_SIZE: (f32, f32) = (MEDIA_MAX_WIDTH, MEDIA_MAX_WIDTH * 9. / 16.);

impl HomeScreen {
    /// One image attachment, or the cover over it while it's an unrevealed
    /// spoiler.
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

        // The cache has to be named on the element itself. An ancestor
        // `image_cache(..)` only pushes onto the cache stack during layout and
        // paint, and `list` renders its items during *prepaint* — so images
        // inside the message list would otherwise miss the stack entirely and
        // fall back to gpui's global asset cache, which never evicts.
        let element = img(image.url.clone())
            .image_cache(&self.image_cache)
            .rounded(px(8.))
            .shadow(media_shadow())
            .max_w(px(MEDIA_MAX_WIDTH));
        // With intrinsic dimensions we can lay out the exact scaled box, so the
        // message doesn't reflow once the image finishes loading.
        match size {
            Some((width, height)) => element.w(px(width)).h(px(height)),
            None => element.max_h(px(MEDIA_MAX_HEIGHT)),
        }
        .into_any_element()
    }

    /// The cover an attachment marked as a spoiler shows, if it hasn't been
    /// clicked open yet.
    fn unrevealed_spoiler<'a>(
        &self,
        spoiler: Option<&'a discord::SpoilerCover>,
        key: MediaKey,
        role: &str,
    ) -> Option<&'a discord::SpoilerCover> {
        spoiler.filter(|_| !self.revealed_spoilers.contains(&key.spoiler_key(role)))
    }

    /// A blurred stand-in for a spoiler attachment, `width` by `height`, which
    /// reveals it when clicked. The attachment itself isn't fetched until then.
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

        // The picture is a few pixels across, stretched into a blur. gpui packs
        // its atlas without padding, so the outer half of each edge pixel would
        // be sampled against a neighbouring sprite; drawing the picture half a
        // pixel larger all round pushes that fringe outside the clip. The
        // picture's size is estimated the way the proxy was asked for it.
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
            // A scrim rather than a theme colour, like the video controls: the
            // label has to read against whatever colours the blur comes out.
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

    /// One video attachment: a poster card that becomes the player in place
    /// when it's the clip being played.
    ///
    /// Nothing is decoded until the play button is pressed, and only one clip
    /// decodes at a time, so a channel full of videos costs no more than a
    /// channel full of images.
    pub(super) fn render_video(
        &self,
        video: &discord::VideoAttachment,
        key: MediaKey,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();

        // The playback belonging to *this* attachment, if it's the live one.
        let playback = self.video.as_ref().filter(|playback| playback.key == key);

        // Once a frame has arrived the card takes its shape, because the frame
        // is the only thing that knows it: Discord's reported dimensions can
        // disagree with the file over anamorphic pixels, and an attachment it
        // reported nothing for is only a guess until then. Sizing to the frame
        // is also what stops the picture being letterboxed inside its own card.
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

        // A spoiler covers the whole card, play button and all, so the clip
        // can't be started without first being revealed.
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
            // Black rather than a theme colour: this is the letterbox around a
            // picture, and it reads as part of the video in either theme, the
            // same way an image's own pixels do.
            .bg(black())
            .border_1()
            .border_color(theme.border);

        let surface = match playback.and_then(|playback| playback.frame.clone()) {
            // A decoded frame is raw pixels, so it never touches the image
            // cache — `ImageSource::Render` is handed straight to the renderer.
            Some(frame) => surface.child(img(frame).size_full()),
            None => surface.child(
                img(video.poster_url.clone())
                    .image_cache(&self.image_cache)
                    .size_full()
                    // A poster Discord can't produce leaves the card black,
                    // which the play button still reads against.
                    .object_fit(ObjectFit::Contain),
            ),
        };

        // A clip that has finished or failed has no decoder behind it any more
        // — the thread exits on both, releasing the output device and the
        // downloaded file — so the card goes back to offering to play, which
        // opens it again from the start. Only a live playback gets controls.
        let finished = playback.is_some_and(|playback| {
            matches!(
                playback.state,
                PlaybackState::Ended | PlaybackState::Failed(_)
            )
        });
        let surface = match playback.filter(|_| !finished) {
            Some(playback) => surface
                .when(playback.state == PlaybackState::Loading, |this| {
                    this.child(centered(Spinner::new().large().color(white())))
                })
                .child(self.video_controls(playback, key, cx)),
            None => surface
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
                .child(self.video_play_button(video, key, finished, cx)),
        };

        v_flex()
            .gap(px(2.))
            .child(surface)
            .child(caption)
            .into_any_element()
    }

    /// The button that starts a clip, centred over its poster. `again` only
    /// changes the label — replaying opens the clip from scratch, because the
    /// decoder that played it has already been torn down.
    fn video_play_button(
        &self,
        video: &discord::VideoAttachment,
        key: MediaKey,
        again: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let url = video.url.clone();

        centered(
            Button::new(key.element_id("play-video"))
                .icon(Icon::default().path("icons/play.svg"))
                .primary()
                .large()
                .tooltip(if again { "Play again" } else { "Play" })
                .on_click(cx.listener(move |this, _, window, cx| {
                    // The whole preview box rather than the card's guessed
                    // size: the decoder fits the picture's true shape inside
                    // this, so a card built from wrong dimensions can't make
                    // the frame small or the wrong shape.
                    //
                    // Physical pixels, because that's what the decoder scales
                    // to — asking for logical ones would decode a soft picture
                    // on any display above 1x.
                    let scale = window.scale_factor();
                    let target = (
                        (MEDIA_MAX_WIDTH * scale).round().max(1.) as u32,
                        (MEDIA_MAX_HEIGHT * scale).round().max(1.) as u32,
                    );
                    this.play_video(key, url.clone(), target, window, cx);
                })),
        )
    }

    /// The transport bar along the bottom of a playing clip.
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

        let close = Button::new(key.element_id("video-close"))
            .icon(Icon::default().path("icons/close.svg"))
            .ghost()
            .xsmall()
            .text_color(white())
            .tooltip("Stop")
            .on_click(cx.listener(|this, _, window, cx| this.stop_video(window, cx)));

        h_flex()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .px_2()
            .py_1()
            .gap_2()
            .items_center()
            // A scrim rather than a theme surface: the controls sit over
            // whatever the video happens to be showing, so the contrast has to
            // come from the bar itself.
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
            .child(close)
    }
}

/// Lays one element over the middle of the card it's a child of.
fn centered(child: impl IntoElement) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .child(child)
}

/// The box a video card occupies, scaled into the preview bounds with its
/// aspect ratio kept.
///
/// Fixed before anything is decoded so the message doesn't reflow when
/// playback starts, and reused as the decode target so frames arrive at
/// exactly the size they're drawn.
fn video_size(video: &discord::VideoAttachment) -> (f32, f32) {
    match (video.width, video.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            fit_preview(f64::from(width), f64::from(height))
        }
        _ => FALLBACK_VIDEO_SIZE,
    }
}

/// Scales a picture's dimensions into the preview bounds, keeping its shape.
/// Unlike the decoder's own fit this one may scale up, because a small video
/// still gets a card big enough to put controls in.
fn fit_preview(width: f64, height: f64) -> (f32, f32) {
    let scale = (f64::from(MEDIA_MAX_WIDTH) / width).min(f64::from(MEDIA_MAX_HEIGHT) / height);
    ((width * scale) as f32, (height * scale) as f32)
}

fn format_time(position: Duration) -> String {
    let total = position.as_secs();
    format!("{}:{:02}", total / 60, total % 60)
}
