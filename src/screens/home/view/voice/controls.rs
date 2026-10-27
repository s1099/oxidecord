//! The control bar under the stage, and the device pickers behind the mute
//! and deafen buttons.

use gpui_kit::component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable as _, h_flex,
    menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem},
};
use gpui_kit::*;

use crate::screens::home::HomeScreen;
use crate::ui::button::Button;
use crate::voice;

use super::CAMERA_UNAVAILABLE;

impl HomeScreen {
    pub(super) fn render_call_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_shrink_0()
            .w_full()
            .py_3()
            .gap_2()
            .items_center()
            .justify_center()
            .child(
                self.with_mic_menu(
                    "call-mic-menu",
                    control(
                        "call-mute",
                        if self.voice_muted {
                            "icons/mic-off.svg"
                        } else {
                            "icons/mic.svg"
                        },
                        if self.voice_muted { "Unmute" } else { "Mute" },
                        self.voice_muted,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_mute(cx))),
                    cx,
                ),
            )
            .child(
                div()
                    .id("call-output-menu")
                    .child(
                        control(
                            "call-deafen",
                            if self.voice_deafened {
                                "icons/headphone-off.svg"
                            } else {
                                "icons/headphones.svg"
                            },
                            if self.voice_deafened {
                                "Undeafen"
                            } else {
                                "Deafen"
                            },
                            self.voice_deafened,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_voice_deafen(cx))),
                    )
                    .context_menu(output_menu),
            )
            .child(
                control(
                    "call-camera",
                    "icons/video-off.svg",
                    CAMERA_UNAVAILABLE,
                    false,
                )
                .disabled(true),
            )
            .child(
                control(
                    "call-share",
                    "icons/screen-share.svg",
                    self.share_tooltip(),
                    self.screen_share.is_some(),
                )
                .disabled(!self.share_button_enabled())
                .on_click(cx.listener(|this, _, window, cx| this.toggle_screen_share(window, cx))),
            )
            .child(
                Button::new("call-hangup")
                    .icon(Icon::default().path("icons/phone-off.svg"))
                    .danger()
                    .large()
                    .tooltip("Disconnect")
                    .on_click(cx.listener(|this, _, _, cx| this.leave_voice(cx))),
            )
    }

    /// Puts the microphone picker behind a right-click on `button`.
    ///
    /// The menu wraps the button rather than being hung off it: a `Button` with
    /// children lays itself out as a labelled button, which would stretch an icon
    /// one out of shape.
    pub(in crate::screens::home) fn with_mic_menu(
        &self,
        id: &'static str,
        button: impl IntoElement,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        div().id(id).child(button).context_menu(self.mic_menu(cx))
    }

    /// Builds the microphone picker, for a right-click or a dropdown to open.
    pub(in crate::screens::home) fn mic_menu(
        &self,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + Clone + 'static
    {
        let screen = cx.entity().downgrade();
        move |menu, _, cx| {
            let Some(screen) = screen.upgrade() else {
                return menu;
            };
            let chosen = screen.read(cx).voice_input_device.clone();

            // Enumerated as the menu opens, so a microphone plugged in since
            // the app started is in the list.
            let devices: Vec<voice::InputDevice> = voice::input_devices();
            let menu = menu.label("Input Device").item(device_item(
                "System Default",
                None,
                chosen.is_none(),
                &screen,
            ));

            devices.into_iter().fold(menu, |menu, device| {
                let selected = chosen.as_deref() == Some(device.id.as_str());
                menu.item(device_item(device.name, Some(device.id), selected, &screen))
            })
        }
    }
}

/// The speaker picker behind the deafen buttons. Audio always plays through
/// the system default for now, so that's all it offers.
pub(in crate::screens::home) fn output_menu(
    menu: PopupMenu,
    _: &mut Window,
    _: &mut Context<PopupMenu>,
) -> PopupMenu {
    menu.label("Output Device")
        .item(PopupMenuItem::new("System Default").icon(Icon::new(IconName::Check)))
}

/// One microphone in the picker, ticked when it's the one in use.
fn device_item(
    label: impl Into<SharedString>,
    id: Option<String>,
    selected: bool,
    screen: &Entity<HomeScreen>,
) -> PopupMenuItem {
    let screen = screen.downgrade();
    let item = PopupMenuItem::new(label.into()).on_click(move |_, _, cx| {
        if let Some(screen) = screen.upgrade() {
            screen.update(cx, |this, cx| this.set_input_device(id.clone(), cx));
        }
    });

    if selected {
        item.icon(Icon::new(IconName::Check))
    } else {
        item
    }
}

/// One square toggle on the control bar, pushed in while its thing is on.
fn control(id: &'static str, icon: &'static str, tooltip: &'static str, active: bool) -> Button {
    Button::new(id)
        .icon(Icon::default().path(icon))
        .outline()
        .large()
        .selected(active)
        .tooltip(tooltip)
}
