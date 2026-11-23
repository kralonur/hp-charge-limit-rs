use super::{ChargeSettings, Dashboard, Snapshot};
#[cfg(target_os = "linux")]
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    radio::RadioGroup,
    slider::{Slider, SliderEvent, SliderState, SliderValue},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use hp_charge_limit_desktop::{Action, Mode, ServiceStatus, StopAction};

fn threshold_value(settings: &mut ChargeSettings, resume: bool) -> &mut u8 {
    if resume {
        &mut settings.start
    } else {
        &mut settings.stop
    }
}

const MODES: [(StopAction, &str); 4] = [
    (StopAction::Auto, "Allow normal charging"),
    (StopAction::Inhibit, "Inhibit · may reset at shutdown"),
    (StopAction::Hold, "Hold · verify on your model"),
    (StopAction::Keep, "Keep the current mode"),
];

impl Dashboard {
    pub(super) fn update_snapshot(&mut self, snapshot: Snapshot) {
        // Don't keep editing against a different set of settings (for example,
        // if a package install or another user changed them outside the app).
        if self.snapshot.settings != snapshot.settings
            || (self.snapshot.service_state == ServiceStatus::Missing)
                != (snapshot.service_state == ServiceStatus::Missing)
        {
            self.discard_settings();
        }
        self.snapshot = snapshot;
    }

    fn discard_settings(&mut self) {
        self.settings_draft = None;
        self.settings_revision = self.settings_revision.wrapping_add(1);
    }

    fn current_settings(&self) -> Option<ChargeSettings> {
        self.snapshot.settings.as_ref()?.as_ref().ok().copied()
    }

    pub(super) fn can_save_settings(&self) -> bool {
        self.applying.is_none()
            && matches!(
                self.snapshot.service_state,
                ServiceStatus::Running | ServiceStatus::Stopped
            )
            && self.current_settings().is_some()
            && self
                .settings_draft
                .is_some_and(|settings| settings.validate().is_ok())
    }

    fn edit_settings(&mut self, edit: impl FnOnce(&mut ChargeSettings), cx: &mut Context<Self>) {
        if self.applying.is_none()
            && let Some(original) = self.current_settings()
        {
            let mut settings = self.settings_draft.unwrap_or(original);
            edit(&mut settings);
            self.settings_draft = (settings != original).then_some(settings);
            cx.notify();
        }
    }

    fn threshold(
        &self,
        resume: bool,
        value: u8,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (label, key, min, max) = if resume {
            ("Resume at or below", "resume", 0, 99)
        } else {
            ("Stop at or above", "stop", 1, 100)
        };
        let busy = self.applying.is_some();
        let dashboard = cx.entity().downgrade();
        let state = window.use_keyed_state(key, cx, |_, cx| {
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(f32::from(min))
                    .max(f32::from(max))
                    .step(1.0)
                    .default_value(f32::from(value))
            });
            let subscription = App::subscribe(cx, &slider, move |_, event, cx| {
                if let SliderEvent::Change(SliderValue::Single(value)) = event {
                    let _ = dashboard.update(cx, |view, cx| {
                        view.edit_settings(
                            |settings| *threshold_value(settings, resume) = *value as u8,
                            cx,
                        )
                    });
                }
            });
            (slider, subscription)
        });
        let slider = state.read(cx).0.clone();
        if slider.read(cx).value().start() != f32::from(value) {
            slider.update(cx, |slider, cx| {
                slider.set_value(f32::from(value), window, cx)
            });
        }
        let step = move |settings: &mut ChargeSettings, delta: i8| {
            let value = threshold_value(settings, resume);
            *value = value.saturating_add_signed(delta).clamp(min, max);
        };
        div().v_flex().gap_1().child(label).child(
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(
                    div().flex_1().min_w_0().child(
                        Slider::new(&slider)
                            .disabled(busy)
                            .accessibility_label(label),
                    ),
                )
                .child(
                    Button::new(format!("{key}-minus"))
                        .outline()
                        .label("−")
                        .accessibility_label(format!("Decrease {key} percentage"))
                        .disabled(busy || value == min)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.edit_settings(|settings| step(settings, -1), cx)
                        })),
                )
                .child(div().w_12().text_center().child(format!("{value}%")))
                .child(
                    Button::new(format!("{key}-plus"))
                        .outline()
                        .label("+")
                        .accessibility_label(format!("Increase {key} percentage"))
                        .disabled(busy || value == max)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.edit_settings(|settings| step(settings, 1), cx)
                        })),
                ),
        )
    }

    fn numeric_setting(
        &self,
        key: &'static str,
        label: &'static str,
        value: u64,
        edit: fn(&mut ChargeSettings, Option<u64>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dashboard = cx.entity().downgrade();
        let state = window.use_keyed_state((key, self.settings_revision), cx, |window, cx| {
            let field = cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value.to_string())
                    .validate(|text, _| {
                        text.len() <= 20 && text.bytes().all(|byte| byte.is_ascii_digit())
                    })
            });
            let subscription = App::subscribe(cx, &field, move |field, event, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = field.read(cx).value().parse().ok();
                    let _ = dashboard.update(cx, |view, cx| {
                        view.edit_settings(|settings| edit(settings, value), cx)
                    });
                }
            });
            (field, subscription)
        });
        div()
            .h_flex()
            .items_center()
            .gap_3()
            .child(div().flex_1().child(label))
            .child(
                Input::new(&state.read(cx).0)
                    .w_32()
                    .disabled(self.applying.is_some()),
            )
    }

    fn advanced_settings(
        &self,
        settings: ChargeSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        const RUN: [(Mode, &str); 3] = [
            (Mode::Auto, "Auto"),
            (Mode::Inhibit, "Inhibit"),
            (Mode::Hold, "Hold"),
        ];
        let advanced = settings.advanced;
        let busy = self.applying.is_some();
        let fields = div()
            .v_flex()
            .gap_3()
            .child(self.numeric_setting(
                "poll-seconds",
                "Poll interval (seconds)",
                advanced.interval.unwrap_or_default(),
                |settings, value| settings.advanced.interval = Some(value.unwrap_or(0)),
                window,
                cx,
            ))
            .child(self.numeric_setting(
                "shutdown-min",
                "Charge normally on shutdown at or below (%)",
                advanced.off_min.unwrap_or_default().into(),
                |settings, value| {
                    settings.advanced.off_min = Some(
                        value
                            .and_then(|value| u8::try_from(value).ok())
                            .unwrap_or(255),
                    )
                },
                window,
                cx,
            ))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("When the stop percentage is reached"),
            )
            .child(
                RadioGroup::horizontal("run-mode")
                    .children(RUN.map(|(_, name)| name))
                    .selected_index(
                        RUN.iter()
                            .position(|(mode, _)| Some(*mode) == advanced.run_mode),
                    )
                    .disabled(busy)
                    .on_change(cx.listener(|view, index: &usize, _, cx| {
                        view.edit_settings(
                            |settings| settings.advanced.run_mode = Some(RUN[*index].0),
                            cx,
                        )
                    })),
            )
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("When stopping only the service"),
            )
            .child(
                RadioGroup::vertical("stop-mode")
                    .children(MODES.map(|(_, label)| label))
                    .selected_index(
                        MODES
                            .iter()
                            .position(|(mode, _)| Some(*mode) == advanced.stop_mode),
                    )
                    .disabled(busy)
                    .on_change(cx.listener(|view, index: &usize, _, cx| {
                        view.edit_settings(
                            |settings| settings.advanced.stop_mode = Some(MODES[*index].0),
                            cx,
                        )
                    })),
            );
        #[cfg(target_os = "linux")]
        let fields = fields.child(
            Checkbox::new("emit-events")
                .label("Broadcast Linux notification events")
                .checked(advanced.notify.unwrap_or_default())
                .disabled(busy)
                .on_click(cx.listener(|view, enabled: &bool, _, cx| {
                    view.edit_settings(|settings| settings.advanced.notify = Some(*enabled), cx)
                })),
        );
        fields
    }

    pub(super) fn charging_settings(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let missing = self.snapshot.service_state == ServiceStatus::Missing;
        let busy = self.applying.is_some();
        let card = div().v_flex().gap_3().child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Charging settings"),
        );
        let Some(settings) = self.settings_draft.or_else(|| self.current_settings()) else {
            let message = match self.snapshot.settings.as_ref() {
                Some(Err(message)) => message.as_str(),
                _ => "Charging settings are unavailable.",
            };
            return card.child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(message.to_owned()),
            );
        };
        let validation_error = settings.validate().err().map(|error| error.to_string());
        let saving = matches!(self.applying, Some(Action::Configure(_)));
        card.child(self.threshold(true, settings.start, window, cx))
            .child(self.threshold(false, settings.stop, window, cx))

            .child(div().font_weight(FontWeight::SEMIBOLD).child("Shutdown behavior"))
            .child(RadioGroup::vertical("shutdown-mode").children(MODES.map(|(_, label)| label))
                .selected_index(MODES.iter().position(|(mode, _)| *mode == settings.off_mode))
                .disabled(busy)
                .on_change(cx.listener(|view, index: &usize, _, cx| {
                    view.edit_settings(|settings| settings.off_mode = MODES[*index].0, cx);
                })))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child("Hold can still allow slow charging on some models. The battery may charge while off when its level is low."))
            .child(Button::new("advanced-settings").ghost().label(if self.advanced_open { "Hide advanced settings" } else { "Show advanced settings" })
                .on_click(cx.listener(|view, _, _, cx| { view.advanced_open = !view.advanced_open; cx.notify(); })))
            .when(self.advanced_open, |this| this.child(self.advanced_settings(settings, window, cx)))
            .when_some(validation_error, |this, error| this.child(div().text_color(cx.theme().danger).child(error)))
            .when(!missing, |this| this
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(
                    "Saving restarts the service if it is running. Boot startup is unchanged."
                ))
                .child(div().h_flex().gap_2()
                    .child(Button::new("save-settings").primary().label("Save settings").loading(saving)
                        .disabled(!self.can_save_settings() && !saving)
                        .on_click(cx.listener(|view, _, _, cx| {
                            if let Some(settings) = view.settings_draft {
                                view.run_action(Action::Configure(settings), cx);
                            }
                        })))
                    .child(Button::new("discard-settings").ghost().label("Discard edits")
                        .disabled(busy || self.settings_draft.is_none())
                        .on_click(cx.listener(|view, _, _, cx| { view.discard_settings(); cx.notify(); })))))
    }
}
