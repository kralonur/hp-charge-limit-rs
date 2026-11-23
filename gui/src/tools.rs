use super::Dashboard;
use gpui_kit::component::{
    ActiveTheme, Disableable, StyledExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    input::{Input, InputState},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use hp_charge_limit_desktop::{self as desktop, Action, Mode, ServiceStatus, WatchResult};

fn cli_process(args: &[&str]) -> Result<async_process::Command, String> {
    let mut command = async_process::Command::from(desktop::cli_command(args)?);
    command.kill_on_drop(true);
    Ok(command)
}

async fn cli_output(args: &[&str]) -> Result<Vec<u8>, String> {
    let output = cli_process(args)?
        .output()
        .await
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let detail = String::from_utf8_lossy(&output.stderr);
        if detail.trim().is_empty() {
            Err(format!("The CLI command failed ({}).", output.status))
        } else {
            Err(detail.trim().into())
        }
    }
}

#[cfg(target_os = "linux")]
async fn notifications() -> Result<(), String> {
    use futures_lite::StreamExt;
    use futures_lite::io::{AsyncBufReadExt, BufReader};
    use std::process::Stdio;

    let mut child = cli_process(&["notify"])?
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let mut lines = BufReader::new(child.stderr.take().expect("piped stderr")).lines();
    let mut last_line = None;
    while let Some(line) = lines.try_next().await.map_err(|error| error.to_string())? {
        if !line.trim().is_empty() {
            last_line = Some(line);
        }
    }
    let status = child.status().await.map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(last_line.unwrap_or_else(|| format!("The notification listener stopped ({status}).")))
    }
}

fn is_service_tool(action: Action) -> bool {
    match action {
        Action::Stop | Action::Startup(_) | Action::Uninstall => true,
        #[cfg(windows)]
        Action::Upgrade => true,
        _ => false,
    }
}

pub(super) fn is_tool(action: Action) -> bool {
    match action {
        Action::ReadMode | Action::SetMode(_) | Action::Logs => true,
        #[cfg(target_os = "linux")]
        Action::LoadDriver => true,
        _ => is_service_tool(action),
    }
}

impl Dashboard {
    pub(super) fn can_run_tool(&self, action: Action) -> bool {
        self.loaded
            && self.snapshot.cli_error.is_none()
            && self.applying.is_none()
            && match action {
                Action::Stop => self.snapshot.service_state == ServiceStatus::Running,
                Action::Startup(enabled) => self.snapshot.startup == Some(Ok(!enabled)),
                Action::Uninstall => self.snapshot.removable,
                #[cfg(windows)]
                Action::Upgrade => self.snapshot.settings.as_ref().is_some_and(Result::is_ok),
                Action::SetMode(_) => matches!(
                    self.snapshot.service_state,
                    ServiceStatus::Stopped | ServiceStatus::Missing
                ),
                _ => true,
            }
    }

    fn confirm_tool(
        &mut self,
        action: Action,
        title: &str,
        detail: &str,
        button: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_run_tool(action) {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            title,
            Some(detail),
            &[PromptButton::cancel("Cancel"), PromptButton::new(button)],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |view, cx| view.run_action(action, cx));
            }
        })
        .detach();
    }

    fn tool_button(
        &self,
        id: &'static str,
        label: &'static str,
        action: Action,
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .outline()
            .label(label)
            .disabled(!self.can_run_tool(action))
            .on_click(cx.listener(move |view, _, _, cx| view.run_action(action, cx)))
    }

    pub(super) fn service_tools(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let status = &self.snapshot.service_state;
        let card = div()
            .v_flex()
            .gap_3()
            .p_5()
            .rounded_xl()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Service management"),
            );
        #[cfg(windows)]
        let card = card.when(
            self.can_run_tool(Action::Upgrade) || self.applying == Some(Action::Upgrade),
            |this| {
                this.child(self.tool_button(
                    "upgrade-service",
                    "Update service from this app",
                    Action::Upgrade,
                    cx,
                ))
            },
        );
        card.when(*status == ServiceStatus::Stopped, |this| {
                this.child(Checkbox::new("bypass-on-next-start").label("Allow full charging when the service starts")
                    .checked(self.snapshot.bypass).disabled(self.applying.is_some())
                    .on_click(cx.listener(|view, enabled, _, cx| view.run_action(Action::Bypass(*enabled), cx))))
            })
            .when(*status == ServiceStatus::Running, |this| this.child(self.tool_button("stop-service", "Stop service", Action::Stop, cx)))
            .when_some(self.snapshot.startup.clone(), |this, setting| match setting {
                Ok(enabled) => this.child(Checkbox::new("service-startup").label("Start service at boot")
                    .checked(enabled).disabled(self.applying.is_some())
                    .on_click(cx.listener(|view, enabled, _, cx| view.run_action(Action::Startup(*enabled), cx))))
                    .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Turning off boot startup does not stop a running service. You can still start it manually.")),
                Err(error) => this.child(div().text_xs().text_color(cx.theme().muted_foreground).child(error)),
            })
            .when(self.snapshot.removable, |this| this.child(Button::new("remove-service").outline().label("Remove service")
                .disabled(!self.can_run_tool(Action::Uninstall))
                .on_click(cx.listener(|view, _, window, cx| view.confirm_tool(Action::Uninstall, "Remove the charge limit service?",
                    "This stops the service and removes automatic charge limiting. Your desktop app and service logs are kept.", "Remove service", window, cx)))))
            .when(cfg!(target_os = "linux") && !self.snapshot.removable, |this| this.child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child("Packaged or customized units are not removed by this app. Use your system’s package manager to remove a packaged unit.")))
            .when_some(self.tool_report.clone().filter(|(action, _, _)| is_service_tool(*action)), |this, (_, error, message)| {
                this.child(Self::feedback(error, message, cx))
            })
    }

    pub(super) fn diagnostic_tools(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let diagnostic = self
            .tool_report
            .as_ref()
            .filter(|(action, _, _)| !is_service_tool(*action));
        let card = div().v_flex().gap_3().p_5().rounded_xl().border_1().border_color(cx.theme().border)
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Diagnostics & manual control"))
            .child(div().h_flex().flex_wrap().gap_2()
                .child(self.tool_button("read-diagnostics", "Run diagnostics", Action::Diagnose, cx))
                .child(self.tool_button("read-mode", "Read firmware mode", Action::ReadMode, cx))
                .child(self.tool_button("read-logs", "View service logs", Action::Logs, cx)))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child("Stop the service before setting a mode manually. A manual setting is temporary; the service will take over again when started."))
            .child(div().h_flex().flex_wrap().gap_2().children([
                (Mode::Auto, "manual-auto", "Set Auto"),
                (Mode::Inhibit, "manual-inhibit", "Set Inhibit"),
                (Mode::Hold, "manual-hold", "Set Hold"),
            ].map(|(mode, id, label)| {
                let action = Action::SetMode(mode);
                Button::new(id).outline().label(label).disabled(!self.can_run_tool(action))
                    .on_click(cx.listener(move |view, _, window, cx| view.confirm_tool(action, label,
                        "Firmware readback does not prove the battery is not charging. Verify with a charging check.", label, window, cx)))
            })))
            .when_some(diagnostic, |this, (_, error, message)| this
                .child(Self::feedback(*error, message.clone(), cx))
                .child(Button::new("copy-diagnostics").ghost().label("Copy report")
                    .on_click({ let message = message.clone(); move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(message.clone())) })))
            .child(self.charging_check(window, cx));
        #[cfg(target_os = "linux")]
        let card = card
            .when(self.snapshot.driver_hint.is_some(), |this| {
                this.child(self.tool_button(
                    "driver-tools",
                    "Load ACPI driver",
                    Action::LoadDriver,
                    cx,
                ))
            })
            .child(
                Checkbox::new("listen-notify")
                    .label("Show charge notifications while this window is open")
                    .checked(self.notifications.is_some())
                    .disabled(self.applying.is_some())
                    .on_click(cx.listener(|view, enabled: &bool, _, cx| {
                        view.notification_error = None;
                        view.notifications = enabled.then(|| {
                            cx.spawn(async move |this, cx| {
                                let result = notifications().await;
                                let _ = this.update(cx, |view, cx| {
                                    view.notifications = None;
                                    view.notification_error = result.err();
                                    cx.notify();
                                });
                            })
                        });
                        cx.notify();
                    })),
            )
            .when_some(self.notification_error.clone(), |this, error| {
                this.child(Self::feedback(true, error, cx))
            });
        card
    }

    fn charging_check(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.watch_task.is_some();
        let input = window.use_keyed_state("watch-seconds", cx, |window, cx| {
            InputState::new(window, cx).default_value("300")
        });
        div().v_flex().gap_2()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Timed charging check"))
            .child(div().text_xs().text_color(cx.theme().muted_foreground)
                .child("Plug in the adapter and keep the power source and charge mode unchanged during the check. A few minutes is usually more useful than a few seconds."))
            .child(div().h_flex().gap_2().items_center()
                .child(Input::new(&input).w_24().disabled(busy))
                .child(div().child("seconds"))
                .child(Button::new("watch-charge").outline().label(if busy { "Cancel check" } else { "Start check" })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if view.watch_task.take().is_some() {
                            view.watch_report = Some((false, "Check cancelled.".into()));
                        } else {
                            match input.read(cx).value().trim().parse::<u64>() {
                                Ok(seconds) if seconds > 0 => view.watch_energy(seconds, cx),
                                _ => view.watch_report = Some((true, "Enter a duration in whole seconds greater than zero.".into())),
                            }
                        }
                        cx.notify();
                    }))))
            .when_some(self.watch_report.clone(), |this, (error, message)| this.child(Self::feedback(error, message, cx)))
    }

    fn watch_energy(&mut self, seconds: u64, cx: &mut Context<Self>) {
        self.watch_report = Some((
            false,
            format!("Checking charge flow for {seconds} seconds…"),
        ));
        self.watch_task = Some(cx.spawn(async move |this, cx| {
            let result = cli_output(&["status", "--json", "--watch", &seconds.to_string()])
                .await
                .and_then(|stdout| {
                    serde_json::from_slice::<WatchResult>(&stdout)
                        .map_err(|_| "The CLI returned an unreadable charging check result.".into())
                });
            let _ = this.update(cx, |view, cx| {
                view.watch_task = None;
                view.watch_report = Some(match result {
                    Ok(result) => (
                        false,
                        format!(
                            "{} · {:+} mWh in {} s",
                            result.flow, result.delta_mwh, result.seconds
                        ),
                    ),
                    Err(error) => (true, error),
                });
                cx.notify();
            });
        }));
    }
}
