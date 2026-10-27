//! The control bar under the stage, and the microphone picker behind the
//! mute buttons.

use gpui_kit::component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable as _, h_flex,
    menu::{ContextMenuExt as _, PopupMenuItem},
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
                self.with_device_menu(
                    "call-mic-menu",
                    "Input Device",
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
                    DeviceMenu::input(),
                    cx,
                ),
            )
            .child(
                self.with_device_menu(
                    "call-speaker-menu",
                    "Output Device",
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
                    DeviceMenu::output(),
                    cx,
                ),
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

    /// Puts a device picker behind a right-click on `button`.
    ///
    /// The menu wraps the button rather than being hung off it: a `Button` with
    /// children lays itself out as a labelled button, which would stretch an icon
    /// one out of shape.
    pub(in crate::screens::home) fn with_device_menu(
        &self,
        id: &'static str,
        label: &'static str,
        button: impl IntoElement,
        devices: DeviceMenu,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let screen = cx.entity().downgrade();

        div().id(id).child(button).context_menu(move |menu, _, cx| {
            let Some(screen) = screen.upgrade() else {
                return menu;
            };
            let chosen = (devices.chosen)(screen.read(cx));

            // Enumerated as the menu opens, so a device plugged in since the
            // app started is in the list.
            let menu = menu.label(label).item(device_item(
                "System Default",
                None,
                chosen.is_none(),
                devices.set,
                &screen,
            ));

            (devices.list)().into_iter().fold(menu, |menu, device| {
                let selected = chosen.as_deref() == Some(device.id.as_str());
                menu.item(device_item(
                    device.name,
                    Some(device.id),
                    selected,
                    devices.set,
                    &screen,
                ))
            })
        })
    }
}

/// Which direction a [`HomeScreen::with_device_menu`] picker is for: what to
/// enumerate, which stored choice is ticked, and what picking an item runs.
pub(in crate::screens::home) struct DeviceMenu {
    list: fn() -> Vec<voice::AudioDevice>,
    chosen: fn(&HomeScreen) -> Option<String>,
    set: fn(&mut HomeScreen, Option<String>, &mut Context<HomeScreen>),
}

impl DeviceMenu {
    /// Microphones: capture the call sends.
    pub(in crate::screens::home) fn input() -> Self {
        Self {
            list: voice::input_devices,
            chosen: HomeScreen::chosen_input_device,
            set: HomeScreen::set_input_device,
        }
    }

    /// Speakers: play what the call receives.
    pub(in crate::screens::home) fn output() -> Self {
        Self {
            list: voice::output_devices,
            chosen: HomeScreen::chosen_output_device,
            set: HomeScreen::set_output_device,
        }
    }
}

/// One device in a picker, ticked when it's the one in use.
fn device_item(
    label: impl Into<SharedString>,
    id: Option<String>,
    selected: bool,
    set: fn(&mut HomeScreen, Option<String>, &mut Context<HomeScreen>),
    screen: &Entity<HomeScreen>,
) -> PopupMenuItem {
    let screen = screen.downgrade();
    let item = PopupMenuItem::new(label.into()).on_click(move |_, _, cx| {
        if let Some(screen) = screen.upgrade() {
            screen.update(cx, |this, cx| set(this, id.clone(), cx));
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
