#![cfg_attr(windows, windows_subsystem = "windows")]

mod assets;
mod backend;
mod settings;
mod tools;

use backend::Snapshot;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt, Theme, ThemeMode,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    progress::ProgressCircle,
    scroll::ScrollableElement,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use hp_charge_limit_desktop::{self as desktop, Action, ChargeSettings, ServiceStatus};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const STATUS_REFRESH: Duration = Duration::from_secs(5);
const COUNTDOWN_TICK: Duration = Duration::from_secs(1);

fn seconds_until_check(deadline: Option<u64>, now: SystemTime) -> Option<u64> {
    let now_ms = u64::try_from(now.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()?;
    Some(deadline?.saturating_sub(now_ms).div_ceil(1_000))
}

fn next_check_copy(deadline: Option<u64>, now: SystemTime) -> String {
    match seconds_until_check(deadline, now) {
        Some(0) => "Waiting for the next service check… Battery readings may lag.".into(),
        Some(seconds) => format!("Next service check in {seconds}s. Battery readings may lag."),
        None => {
            "Mode changes are picked up on the service’s next check. Countdown unavailable.".into()
        }
    }
}

struct Dashboard {
    snapshot: Snapshot,
    loaded: bool,
    refreshing: bool,
    applying: Option<Action>,
    notice: Option<(bool, String)>,
    tool_report: Option<(Action, bool, String)>,
    watch_report: Option<(bool, String)>,
    watch_task: Option<Task<()>>,
    #[cfg(target_os = "linux")]
    notifications: Option<Task<()>>,
    #[cfg(target_os = "linux")]
    notification_error: Option<String>,
    advanced_open: bool,
    settings_revision: u64,
    diagnostics_passed: bool,
    setup_consent: bool,
    settings_draft: Option<ChargeSettings>,
    tray_autostart: bool,
    refresh_task: Option<Task<()>>,
    _poll: Task<()>,
}

impl Dashboard {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_bounds(window, |_, _, cx| cx.notify())
            .detach();
        let poll = cx.spawn(async move |this, cx| {
            let mut refresh_at = Instant::now();
            let mut requested_deadline = None;
            loop {
                if this
                    .update(cx, |view, cx| {
                        let deadline = view.snapshot.next_check_unix_ms;
                        let due = deadline != requested_deadline
                            && seconds_until_check(deadline, SystemTime::now()) == Some(0);
                        let now = Instant::now();
                        if (now >= refresh_at || due) && !view.refreshing && view.applying.is_none()
                        {
                            view.refresh(cx);
                            refresh_at = now + STATUS_REFRESH;
                            if due {
                                requested_deadline = deadline;
                            }
                        }
                        if deadline.is_some() {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
                cx.background_executor().timer(COUNTDOWN_TICK).await;
            }
        });
        Self {
            tray_autostart: desktop::autostart_enabled(),
            ..Self::initial(poll)
        }
    }

    fn initial(poll: Task<()>) -> Self {
        Self {
            snapshot: Snapshot::default(),
            loaded: false,
            refreshing: false,
            applying: None,
            notice: None,
            tool_report: None,
            watch_report: None,
            watch_task: None,
            #[cfg(target_os = "linux")]
            notifications: None,
            #[cfg(target_os = "linux")]
            notification_error: None,
            advanced_open: false,
            settings_revision: 0,
            diagnostics_passed: false,
            setup_consent: false,
            settings_draft: None,
            tray_autostart: false,
            refresh_task: None,
            _poll: poll,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.refreshing || self.applying.is_some() {
            return;
        }
        self.refreshing = true;
        self.refresh_task = Some(cx.spawn(async move |this, cx| {
            let snapshot = cx.background_spawn(async { Snapshot::read() }).await;
            let _ = this.update(cx, |view, cx| {
                view.update_snapshot(snapshot);
                view.loaded = true;
                view.refreshing = false;
                cx.notify();
            });
        }));
    }

    fn can_toggle_bypass(&self) -> bool {
        self.loaded
            && self.applying.is_none()
            && self.snapshot.service_state == ServiceStatus::Running
    }

    fn toggle_bypass(&mut self, cx: &mut Context<Self>) {
        if !self.can_toggle_bypass() {
            return;
        }
        self.run_action(Action::Bypass(!self.snapshot.bypass), cx);
    }

    fn can_install(&self) -> bool {
        self.loaded
            && self.applying.is_none()
            && self.snapshot.service_state == ServiceStatus::Missing
            && self.snapshot.prerequisite.is_none()
            && self.diagnostics_passed
            && self.setup_consent
            && self
                .settings_draft
                .is_none_or(|settings| settings.validate().is_ok())
    }

    fn can_start(&self) -> bool {
        self.loaded
            && self.applying.is_none()
            && self.snapshot.service_state == ServiceStatus::Stopped
            && self.snapshot.prerequisite.is_none()
    }

    fn charge_control_copy(&self) -> (&'static str, &'static str) {
        if !self.loaded {
            ("Connecting…", "Reading battery and service status.")
        } else if self.snapshot.service_state != ServiceStatus::Running {
            (
                "Charge controls unavailable",
                "The service must be running to change charge mode.",
            )
        } else if self.snapshot.bypass {
            (
                "Charge limiting paused",
                "Full charging is allowed until you resume limiting or reboot.",
            )
        } else {
            (
                "Charge limiting on",
                "Charging follows your configured thresholds.",
            )
        }
    }

    fn finish_action(
        &mut self,
        action: Action,
        result: Result<String, String>,
        snapshot: Snapshot,
    ) {
        self.update_snapshot(snapshot);
        self.applying = None;
        if action == Action::Diagnose {
            self.diagnostics_passed = result.is_ok();
        }
        let notice = match result {
            // The updated mode and button already confirm a successful toggle.
            Ok(_) if let Action::Bypass(enabled) = action => (self.snapshot.bypass != enabled)
                .then(|| (true, "Could not verify the requested charge mode.".into())),
            Ok(_) if matches!(action, Action::Setup(_) | Action::Start) => {
                match &self.snapshot.service_state {
                    ServiceStatus::Running => Some((
                        false,
                        if matches!(action, Action::Setup(_)) {
                            "Service installed and started.".into()
                        } else {
                            "Service started.".into()
                        },
                    )),
                    // The setup card shows live progress while startup is pending.
                    ServiceStatus::Starting => None,
                    ServiceStatus::Stopped => Some((
                        true,
                        "The service is not running. Check the service log before retrying.".into(),
                    )),
                    ServiceStatus::Missing => {
                        Some((true, "The service was not found after the request.".into()))
                    }
                    ServiceStatus::Unavailable(message) => Some((
                        true,
                        format!("Could not confirm that the service is running: {message}"),
                    )),
                }
            }
            Ok(_) if let Action::Configure(expected) = action => {
                if self.snapshot.settings.as_ref() == Some(&Ok(expected)) {
                    self.settings_draft = None;
                    Some((false, "Charging settings saved.".into()))
                } else {
                    Some((true, "Could not verify the saved charging settings. Refresh and check the service status.".into()))
                }
            }
            Ok(_) if action == Action::Stop => {
                Some(if self.snapshot.service_state == ServiceStatus::Stopped {
                    (false, "Service stopped.".into())
                } else {
                    (true, "Could not verify that the service stopped.".into())
                })
            }
            Ok(_) if let Action::Startup(enabled) = action => {
                Some(if self.snapshot.startup == Some(Ok(enabled)) {
                    (false, "Service startup updated.".into())
                } else {
                    (true, "Could not verify the new startup setting.".into())
                })
            }
            Ok(_) if action == Action::Uninstall => {
                Some(if self.snapshot.service_state == ServiceStatus::Missing {
                    (false, "Service removed.".into())
                } else {
                    (
                        true,
                        "The service still appears installed. Refresh and check its status.".into(),
                    )
                })
            }
            Ok(message) => Some((false, message)),
            Err(error) => Some((true, error)),
        };
        if tools::is_tool(action) || action == Action::Diagnose {
            self.tool_report = notice
                .clone()
                .map(|(error, message)| (action, error, message));
        }
        if !tools::is_tool(action) || self.snapshot.service_state == ServiceStatus::Missing {
            self.notice = notice;
        }
    }

    fn run_action(&mut self, action: Action, cx: &mut Context<Self>) {
        if self.applying.is_some()
            || (matches!(action, Action::Setup(_)) && !self.can_install())
            || (action == Action::Start && !self.can_start())
            || (matches!(action, Action::Configure(_)) && !self.can_save_settings())
            || (tools::is_tool(action) && !self.can_run_tool(action))
            || (matches!(action, Action::Bypass(_))
                && !matches!(
                    self.snapshot.service_state,
                    ServiceStatus::Running | ServiceStatus::Stopped
                ))
        {
            return;
        }
        let report = tools::is_tool(action) || action == Action::Diagnose;
        if let Some(message) = self.snapshot.action_blocker(action) {
            if report {
                self.tool_report = Some((action, true, message.clone()));
            }
            if !tools::is_tool(action) || self.snapshot.service_state == ServiceStatus::Missing {
                self.notice = Some((true, message));
            }
            cx.notify();
            return;
        }
        if report {
            self.tool_report = Some((
                action,
                false,
                "Waiting for authorization or a result…".into(),
            ));
        }
        // Discard any in-flight read so it cannot overwrite the action's newer state.
        self.refresh_task.take();
        self.refreshing = false;
        self.applying = Some(action);
        self.notice = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let (result, snapshot) = cx
                .background_spawn(async move {
                    let result = desktop::perform(action);
                    (result, Snapshot::read())
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.finish_action(action, result, snapshot);
                cx.notify();
            });
        })
        .detach();
    }

    fn setup_card(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let missing = self.snapshot.service_state == ServiceStatus::Missing;
        let stopped = self.snapshot.service_state == ServiceStatus::Stopped;
        let start_pending = self.snapshot.service_state == ServiceStatus::Starting;
        let busy = self.applying.is_some();
        let diagnosing = self.applying == Some(Action::Diagnose);
        #[cfg(target_os = "linux")]
        let needs_driver = self.snapshot.driver_hint.is_some();
        let installing = matches!(self.applying, Some(Action::Setup(_)));
        let starting = self.applying == Some(Action::Start);
        div().v_flex().gap_4().p_6().rounded_xl().border_1().border_color(cx.theme().border)
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD)
                .child(if missing { "Set up charge limiting" } else if stopped { "Your service is stopped" } else if start_pending { "Your service is starting" } else { "Service status unavailable" }))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(
                if missing {
                    "To enable automatic charge limiting, check your model and run read-only diagnostics. Installation starts the service now and at every boot."
                } else if stopped {
                    "Your existing installation and settings will be kept. Starting the service does not change whether it starts at boot."
                } else if start_pending {
                    "The dashboard checks the service status every five seconds."
                } else {
                    "Refresh after the service finishes transitioning, or check your operating system's service manager. Setup is disabled while status is uncertain."
                }))
            .when_some(self.snapshot.prerequisite.clone(), |this, message| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(message))
            })
            .when(missing, |this| {
                #[cfg(target_os = "linux")]
                let this = this.when(needs_driver, |this| this.child(Button::new("load-driver-setup").outline().label("Load ACPI driver")
                    .disabled(busy).on_click(cx.listener(|view, _, _, cx| view.run_action(Action::LoadDriver, cx)))));
                this.child(Button::new("compatibility-guide").ghost().icon(IconName::ExternalLink).label("Compatibility and verification guide")
                    .on_click(|_, _, cx| cx.open_url("https://github.com/kralonur/hp-charge-limit-rs#tested-models")))
                    .child(Button::new("diagnose").outline().icon(IconName::SquareTerminal).loading(diagnosing)
                        .label(if diagnosing { "Reading diagnostics…" } else { "Run read-only diagnostics" })
                        .disabled(busy && !diagnosing)
                        .on_click(cx.listener(|view, _, _, cx| view.run_action(Action::Diagnose, cx))))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground)
                        .child("Readback alone does not prove that firmware control is compatible or safe on your model. Hold behavior must be verified manually; on some boards it only slows charging."))
                    .child(self.charging_settings(window, cx))
                    .child(Checkbox::new("setup-consent").w_full().text_sm().checked(self.setup_consent)
                        .label("I checked compatibility and agree to install and enable the service.")
                        .disabled(busy)
                        .on_click(cx.listener(|view, checked, _, cx| { view.setup_consent = *checked; cx.notify(); })))
                    .child(Button::new("install-service").primary().icon(IconName::ShieldCheck).loading(installing)
                        .label(if installing { "Setting up service…" } else { "Install and enable service" })
                        .disabled(!self.can_install() && !installing)
                        .on_click(cx.listener(|view, _, _, cx| view.run_action(Action::Setup(view.settings_draft), cx))))
            })
            .when(stopped, |this| {
                this.child(Button::new("start-service").primary().icon(IconName::Play).loading(starting)
                    .label(if starting { "Starting service…" } else { "Start existing service" })
                    .disabled(!self.can_start() && !starting)
                    .on_click(cx.listener(|view, _, _, cx| view.run_action(Action::Start, cx))))
            })
            .when_some(self.notice.clone(), |this, (error, message)| {
                this.child(Self::feedback(error, message, cx))
            })
    }

    fn metric(label: &'static str, value: String, cx: &App) -> impl IntoElement {
        div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .p_3()
            .rounded_lg()
            .bg(cx.theme().muted)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(value),
            )
    }

    fn feedback(error: bool, message: String, cx: &App) -> impl IntoElement {
        let color = if error {
            cx.theme().danger
        } else {
            cx.theme().success
        };
        div()
            .h_flex()
            .items_start()
            .gap_2()
            .p_3()
            .rounded_lg()
            .bg(cx.theme().muted)
            .child(
                Icon::new(if error {
                    IconName::CircleAlert
                } else {
                    IconName::CircleCheck
                })
                .size_4()
                .text_color(color),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .line_height(relative(1.5))
                    .text_color(color)
                    .whitespace_normal()
                    .child(message),
            )
    }

    fn detail(label: &'static str, value: String, cx: &App) -> impl IntoElement {
        div()
            .h_flex()
            .justify_between()
            .gap_4()
            .items_start()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(
                div()
                    .min_w_0()
                    .max_w(relative(0.65))
                    .text_right()
                    .whitespace_normal()
                    .child(value),
            )
    }
}

impl Render for Dashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = window.viewport_size().width < px(720.);
        let s = &self.snapshot;
        let active = self.loaded && s.service_state == ServiceStatus::Running;
        let toggling = matches!(self.applying, Some(Action::Bypass(_)));
        let primary = cx.theme().primary;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let charge = if self.loaded {
            s.capacity.map_or_else(|| "—".into(), |v| format!("{v}%"))
        } else {
            "…".into()
        };
        let (state, description) = self.charge_control_copy();

        div()
            .id("dashboard")
            .size_full()
            .overflow_y_scrollbar()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm().line_height(relative(1.5))
            .child(
                div().v_flex().w_full().min_w_0().max_w(px(960.)).mx_auto().p_6().gap_5()
                    .when(compact, |this| this.p_4().gap_4())
                    .child(
                        div().h_flex().items_center().justify_between().gap_4()
                            .child(
                                div().h_flex().gap_3().flex_1().min_w_0()
                                    .child(div().p_3().flex_shrink_0().rounded_lg().bg(cx.theme().muted).child(Icon::new(IconName::BatteryCharging).size_6().text_color(primary)))
                                    .child(div().v_flex().gap_1().flex_1().min_w_0()
                                        .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child("HP Charge Limit"))
                                        .child(div().text_sm().text_color(muted).child("A little less charge. A longer battery life.")))
                            )
                            .child(Button::new("refresh").ghost().flex_shrink_0().icon(IconName::RefreshCw).label("Refresh")
                                .disabled(self.applying.is_some())
                                .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))))
                    )
                    .when_some(s.cli_error.clone(), |this, error| this.child(Self::feedback(true, error, cx)))
                    .when(self.loaded && !active && s.cli_error.is_none(), |this| this.child(self.setup_card(window, cx)))
                    .child(
                        div().h_flex().items_center().gap_5().p_5().rounded_xl().border_1().border_color(border)
                            .when(compact, |this| this.flex_col().items_stretch().p_4())
                            .child(
                                div().v_flex().items_center().justify_center().gap_2().w(px(168.)).flex_shrink_0()
                                    .when(compact, |this| this.w_full())
                                    .child(ProgressCircle::new("battery-level")
                                        .value(s.capacity.unwrap_or(0) as f32)
                                        .with_size(px(160.)).color(primary)
                                        .child(div().text_3xl().font_weight(FontWeight::SEMIBOLD).child(charge)))
                                    .child(div().h_flex().gap_2().text_sm().text_color(muted)
                                        .child(Icon::new(if s.on_ac { IconName::PlugZap } else { IconName::Battery }).size_4())
                                        .child(if !self.loaded { "Reading power source" } else if s.cli_error.is_some() { "Power source unavailable" } else if s.on_ac { "Plugged in" } else { "On battery" }))
                            )
                            .child(
                                div().v_flex().flex_1().min_w_0().justify_center().gap_3()
                                    .when(compact, |this| this.flex_none().w_full())
                                    .child(div().text_sm().text_color(muted).child("Battery status · reported by your system"))
                                    .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(if self.loaded { s.status.clone() } else { "Reading battery…".into() }))
                                    .child(div().text_xs().text_color(muted).whitespace_normal().child(if !self.loaded { "Reading battery information…".into() } else if s.pack.is_empty() { "Battery model unavailable".into() } else { s.pack.clone() }))
                                    .child(div().h_flex().gap_3()
                                        .child(Self::metric("Health", if self.loaded { s.health.clone() } else { "—".into() }, cx))
                                        .child(Self::metric("Cycles", if self.loaded { s.cycles.clone() } else { "—".into() }, cx)))
                            )
                    )
                    .when(!self.loaded || active, |this| this.child(
                        div().v_flex().gap_3().p_5().rounded_xl().border_1().border_color(border)
                            .when(compact, |this| this.p_4())
                            .child(div().h_flex().items_center().gap_4()
                                .when(compact, |this| this.flex_col().items_stretch())
                                .child(div().h_flex().items_start().gap_3().flex_1().min_w_0()
                                    .when(compact, |this| this.flex_none())
                                    .child(div().flex().items_center().justify_center().size_9().flex_shrink_0().rounded_lg().bg(cx.theme().muted)
                                        .child(Icon::new(if !self.loaded { IconName::LoaderCircle } else if !active { IconName::CircleAlert } else if s.bypass { IconName::BatteryFull } else { IconName::ShieldCheck })
                                            .size_5().text_color(if active && !s.bypass { cx.theme().success } else { primary })))
                                    .child(div().v_flex().gap_1().flex_1().min_w_0()
                                        .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(state))
                                        .child(div().text_sm().text_color(muted).child(description))))
                                .child(Button::new("bypass").primary().min_w(px(164.)).flex_shrink_0()
                                    .loading(toggling || !self.loaded)
                                    .icon(if s.bypass { IconName::ShieldCheck } else { IconName::BatteryFull })
                                    .label(if !self.loaded { "Reading service…" } else if toggling { "Authorizing…" } else if s.bypass { "Resume limiting" } else { "Charge to full" })
                                    .disabled(!active)
                                    .on_click(cx.listener(|view, _, _, cx| view.toggle_bypass(cx)))))
                            .when(active, |this| this.child(div().text_xs().text_color(muted)
                                .child(next_check_copy(s.next_check_unix_ms, SystemTime::now()))))
                            .when(active, |this| this.when_some(self.notice.clone(), |this, (error, message)| {
                                this.child(Self::feedback(error, message, cx))
                            }))
                    ))
                    .when(active || (self.loaded && s.service_state == ServiceStatus::Stopped), |this| {
                        this.child(div().p_5().rounded_xl().border_1().border_color(border).child(self.charging_settings(window, cx)))
                    })
                    .child(
                        div().h_flex().items_start().gap_4()
                            .when(compact, |this| this.flex_col().items_stretch().gap_4())
                            .child(div().v_flex().flex_1().min_w_0().gap_3().p_5().rounded_xl().border_1().border_color(border)
                                .when(compact, |this| this.flex_none().p_4())
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("Battery details"))
                                .child(Self::detail("Stored energy", if self.loaded { s.energy.clone() } else { "—".into() }, cx))
                                .child(Self::detail("Power draw / input", if self.loaded { s.watts.clone() } else { "—".into() }, cx))
                                .child(Self::detail("Voltage", if self.loaded { s.voltage.clone() } else { "—".into() }, cx))
                                .child(Self::detail("Service", if self.loaded { s.service_state.label().into() } else { "…".into() }, cx)))
                            .child(div().v_flex().flex_1().min_w_0().gap_3().p_5().rounded_xl().border_1().border_color(border)
                                .when(compact, |this| this.flex_none().p_4())
                                .child(div().h_flex().items_center().gap_2().font_weight(FontWeight::SEMIBOLD)
                                    .child(Icon::new(IconName::Settings2).size_4()).child("Preferences & help"))
                                .child(Checkbox::new("tray-autostart").w_full().text_sm().checked(self.tray_autostart).label("Start tray at login")
                                    .on_click(cx.listener(|view, checked, _, cx| {
                                        match desktop::set_autostart(*checked) {
                                            Ok(()) => { view.tray_autostart = *checked; view.notice = Some((false, "Tray login startup updated. The service is unchanged.".into())); }
                                            Err(error) => view.notice = Some((true, error)),
                                        }
                                        cx.notify();
                                    })))
                                .child(div().text_sm().text_color(muted).child("Closing this window doesn’t stop the service."))
                                .child(Button::new("charging-guide").ghost().icon(IconName::ExternalLink).label("Charging & setup guide")
                                    .on_click(|_, _, cx| cx.open_url("https://github.com/kralonur/hp-charge-limit-rs#how-it-works"))))
                    )
                    .when(self.loaded && (matches!(s.service_state, ServiceStatus::Running | ServiceStatus::Stopped)
                        || self.tool_report.as_ref().is_some_and(|(action, _, _)| *action == Action::Uninstall)),
                        |this| this.child(self.service_tools(cx)))
                    .when(self.loaded && s.cli_error.is_none(), |this| this.child(self.diagnostic_tools(window, cx)))
                    .when_some(s.last_off.clone(), |this, last| {
                        this.child(div().text_sm().text_color(muted).child(format!("Last shutdown · {last}")))
                    })
                    .child(div().h_flex().justify_between().gap_4().text_xs().text_color(muted)
                        .when(compact, |this| this.flex_col().items_start().gap_2())
                        .child("Refreshes every 5 seconds · — = unavailable")
                        .child("Charge changes require authentication"))
            )
    }
}

fn main() {
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            Theme::change(ThemeMode::Dark, None, cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(900.), px(820.)),
                    cx,
                ))),
                window_min_size: Some(size(px(480.), px(480.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("HP Charge Limit".into()),
                    ..Default::default()
                }),
                app_id: Some("io.github.kralonur.HpChargeLimit.Gui".into()),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Dashboard::new(window, cx))
            })
            .expect("could not open the HP Charge Limit window");
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::{Dashboard, Snapshot};
    use gpui_kit::Task;

    fn view() -> Dashboard {
        Dashboard {
            loaded: true,
            ..Dashboard::initial(Task::ready(()))
        }
    }

    #[test]
    fn countdown_rounds_up_and_does_not_guess_when_missing() {
        use super::{Duration, UNIX_EPOCH, next_check_copy, seconds_until_check};
        let now = UNIX_EPOCH + Duration::from_millis(1_234);
        assert_eq!(seconds_until_check(Some(3_235), now), Some(3));
        assert_eq!(seconds_until_check(Some(1_235), now), Some(1));
        assert_eq!(seconds_until_check(Some(1_234), now), Some(0));
        assert_eq!(seconds_until_check(Some(1_233), now), Some(0));
        assert!(next_check_copy(Some(1_233), now).starts_with("Waiting"));
        assert!(next_check_copy(None, now).ends_with("Countdown unavailable."));
    }

    #[test]
    fn installing_requires_missing_service_diagnostics_and_explicit_consent() {
        let mut view = view();
        assert!(!view.can_install());
        view.diagnostics_passed = true;
        assert!(!view.can_install());
        view.setup_consent = true;
        assert!(view.can_install());
        view.snapshot.prerequisite = Some("module missing".into());
        assert!(!view.can_install());
        view.snapshot.prerequisite = None;
        for status in [
            super::ServiceStatus::Running,
            super::ServiceStatus::Stopped,
            super::ServiceStatus::Unavailable("unknown".into()),
        ] {
            view.snapshot.service_state = status;
            assert!(!view.can_install());
        }
        view.snapshot.service_state = super::ServiceStatus::Missing;
        view.applying = Some(super::Action::Setup(None));
        assert!(!view.can_install());
        view.applying = None;
        view.loaded = false;
        assert!(!view.can_install());
    }

    #[test]
    fn settings_draft_survives_refresh_but_not_changes_from_outside_the_app() {
        use super::{ChargeSettings, ServiceStatus};
        let mut view = view();
        let installed = ChargeSettings::default();
        let snapshot = |settings| Snapshot {
            service_state: ServiceStatus::Running,
            settings: Some(Ok(settings)),
            ..Default::default()
        };
        view.update_snapshot(snapshot(installed));
        view.settings_draft = Some(ChargeSettings {
            stop: installed.start,
            ..installed
        });
        assert!(!view.can_save_settings());
        view.settings_draft = Some(ChargeSettings {
            stop: 80,
            ..installed
        });
        view.update_snapshot(snapshot(installed));
        assert!(view.can_save_settings());
        view.update_snapshot(snapshot(ChargeSettings {
            stop: 70,
            ..installed
        }));
        assert!(!view.can_save_settings());
        assert!(view.settings_draft.is_none());
    }

    #[test]
    fn starting_an_existing_service_does_not_require_setup_consent_or_diagnostics() {
        let mut view = view();
        view.snapshot.service_state = super::ServiceStatus::Stopped;
        assert!(view.can_start());
        assert!(!view.can_install());
        view.snapshot.prerequisite = Some("authentication unavailable".into());
        assert!(!view.can_start());
    }

    #[test]
    fn background_refresh_does_not_disable_the_charge_button() {
        let mut view = view();
        view.snapshot.service_state = super::ServiceStatus::Running;
        assert!(view.can_toggle_bypass());
        view.refreshing = true;
        assert!(view.can_toggle_bypass());
        view.applying = Some(super::Action::Bypass(true));
        assert!(!view.can_toggle_bypass());
        view.applying = None;
        view.loaded = false;
        assert!(!view.can_toggle_bypass());
        view.loaded = true;
        view.snapshot.service_state = super::ServiceStatus::Stopped;
        assert!(!view.can_toggle_bypass());
    }

    #[test]
    fn charge_copy_follows_mode_not_the_battery_reading() {
        let mut view = view();
        view.snapshot.service_state = super::ServiceStatus::Running;
        view.snapshot.status = "Not charging".into();
        assert_eq!(view.charge_control_copy().0, "Charge limiting on");
        view.snapshot.bypass = true;
        assert_eq!(
            view.charge_control_copy(),
            (
                "Charge limiting paused",
                "Full charging is allowed until you resume limiting or reboot.",
            )
        );
        view.snapshot.service_state = super::ServiceStatus::Stopped;
        assert_eq!(view.charge_control_copy().0, "Charge controls unavailable");
        view.loaded = false;
        assert_eq!(view.charge_control_copy().0, "Connecting…");
    }

    #[test]
    fn successful_toggles_do_not_dump_cli_output_or_fake_charging() {
        let mut view = view();
        for bypass in [true, false] {
            view.applying = Some(super::Action::Bypass(bypass));
            view.finish_action(
                super::Action::Bypass(bypass),
                Ok("raw CLI output".into()),
                Snapshot {
                    bypass,
                    status: "Not charging".into(),
                    ..Default::default()
                },
            );
            assert!(view.notice.is_none());
            assert!(view.applying.is_none());
            assert_eq!(view.snapshot.bypass, bypass);
            assert_eq!(view.snapshot.status, "Not charging");
        }
    }

    #[test]
    fn action_feedback_preserves_errors_and_diagnostics() {
        let mut view = view();
        view.finish_action(
            super::Action::Bypass(true),
            Err("Authentication cancelled.".into()),
            Snapshot::default(),
        );
        assert_eq!(
            view.notice,
            Some((true, "Authentication cancelled.".into()))
        );
        view.finish_action(
            super::Action::Diagnose,
            Ok("Diagnostic report".into()),
            Snapshot::default(),
        );
        assert_eq!(view.notice, Some((false, "Diagnostic report".into())));
        assert!(view.diagnostics_passed);
        for (action, expected) in [
            (super::Action::Setup(None), "Service installed and started."),
            (super::Action::Start, "Service started."),
        ] {
            view.finish_action(
                action,
                Ok("raw CLI output".into()),
                Snapshot {
                    service_state: super::ServiceStatus::Running,
                    ..Default::default()
                },
            );
            assert_eq!(view.notice, Some((false, expected.into())));
        }
    }

    #[test]
    fn manual_mode_and_service_actions_follow_the_live_state() {
        use super::{Action, ServiceStatus};
        use hp_charge_limit_desktop::Mode;
        let mut view = view();
        assert!(view.can_run_tool(Action::SetMode(Mode::Auto)));
        assert!(!view.can_run_tool(Action::Uninstall));
        view.snapshot.service_state = ServiceStatus::Running;
        view.snapshot.removable = true;
        view.snapshot.startup = Some(Ok(false));
        assert!(!view.can_run_tool(Action::SetMode(Mode::Auto)));
        assert!(view.can_run_tool(Action::Stop));
        assert!(view.can_run_tool(Action::Uninstall));
        assert!(view.can_run_tool(Action::Startup(true)));
        assert!(!view.can_run_tool(Action::Startup(false)));
        view.applying = Some(Action::Stop);
        assert!(!view.can_run_tool(Action::Uninstall));
        view.finish_action(
            Action::Stop,
            Ok(String::new()),
            Snapshot {
                service_state: ServiceStatus::Stopped,
                ..Default::default()
            },
        );
        assert!(view.can_run_tool(Action::SetMode(Mode::Auto)));
        assert!(matches!(view.tool_report, Some((Action::Stop, false, _))));
    }

    #[test]
    fn lifecycle_messages_require_verified_results() {
        use super::{Action, ServiceStatus};
        let mut view = view();
        for action in [Action::Stop, Action::Startup(true), Action::Uninstall] {
            view.finish_action(
                action,
                Ok(String::new()),
                Snapshot {
                    service_state: ServiceStatus::Running,
                    startup: Some(Ok(false)),
                    ..Default::default()
                },
            );
            assert!(matches!(view.tool_report, Some((_, true, _))));
        }
    }

    #[test]
    fn saved_settings_must_match_the_requested_values() {
        use super::{Action, ChargeSettings, ServiceStatus};
        let mut view = view();
        let old = ChargeSettings::default();
        let desired = ChargeSettings { stop: 80, ..old };
        let snapshot = |settings| Snapshot {
            service_state: ServiceStatus::Stopped,
            settings: Some(Ok(settings)),
            ..Default::default()
        };
        view.update_snapshot(snapshot(old));
        view.settings_draft = Some(desired);
        view.finish_action(Action::Configure(desired), Ok(String::new()), snapshot(old));
        assert!(matches!(view.notice, Some((true, _))));
        assert_eq!(view.settings_draft, Some(desired));
        view.finish_action(
            Action::Configure(desired),
            Ok(String::new()),
            snapshot(desired),
        );
        assert!(matches!(view.notice, Some((false, _))));
        assert!(view.settings_draft.is_none());
    }

    #[test]
    fn a_successful_command_is_not_proof_that_the_service_started() {
        use super::ServiceStatus;

        let mut view = view();
        for action in [super::Action::Setup(None), super::Action::Start] {
            for service_state in [
                ServiceStatus::Stopped,
                ServiceStatus::Missing,
                ServiceStatus::Unavailable("unknown".into()),
            ] {
                view.finish_action(
                    action,
                    Ok(String::new()),
                    Snapshot {
                        service_state,
                        ..Default::default()
                    },
                );
                assert!(matches!(view.notice, Some((true, _))));
            }
            view.finish_action(
                action,
                Ok(String::new()),
                Snapshot {
                    service_state: ServiceStatus::Starting,
                    ..Default::default()
                },
            );
            assert!(view.notice.is_none());
            assert!(!view.can_start());
            assert!(!view.can_toggle_bypass());
        }
    }
}
