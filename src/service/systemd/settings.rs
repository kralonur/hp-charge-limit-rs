use super::*;
use crate::cli::{Mode, StopAction};
use clap::ValueEnum;
use std::os::unix::fs::MetadataExt;

const START: &str = "HP_CHARGE_LIMIT_RS_START";
const STOP: &str = "HP_CHARGE_LIMIT_RS_STOP";
const OFF_MODE: &str = "HP_CHARGE_LIMIT_RS_OFF_MODE";

/// Only manage the shipped unit. Other units or drop-ins may override the
/// environment file or pass CLI options that take precedence over it.
pub(super) fn check_unit() -> Result<String> {
    let unit = service_show()?;
    let property = |key| unit.lines().find_map(|line| line.strip_prefix(key));
    let fragment = property("FragmentPath=").filter(|path| UNIT_SOURCES.contains(path));
    ensure!(
        property("LoadState=") == Some("loaded")
            && property("DropInPaths=") == Some("")
            && property("NeedDaemonReload=") == Some("no")
            && fragment.is_some_and(|path| fs::read_to_string(path).is_ok_and(|text| text == SERVICE_UNIT)),
        "Dashboard editing requires the shipped systemd unit, loaded and without drop-ins."
    );
    Ok(fragment.unwrap().to_owned())
}

fn read_config(path: &Path) -> Result<Option<(String, u32)>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    let mode = metadata.mode() & 0o777;
    ensure!(
        metadata.is_file() && metadata.uid() == 0 && metadata.gid() == 0 && mode & 0o022 == 0,
        "{} is not a regular, protected root-owned settings file",
        path.display()
    );
    Ok(Some((
        fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?,
        mode,
    )))
}

/// Parse only single-line scalar assignments. In particular, do not mistake
/// text inside a multiline value for an active setting.
fn parse_settings(text: &str) -> Result<ChargeSettings> {
    let mut settings = ChargeSettings::default();
    for line in text
        .lines()
        .map(|line| line.trim_matches([' ', '\t', '\r']))
    {
        ensure!(
            !line.ends_with('\\'),
            "The settings file contains a continued line and cannot be edited safely."
        );
        if line.is_empty() || line.starts_with(['#', ';']) {
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .context("The settings file contains a customized assignment.")?;
        let (key, value) = (
            key.trim_end_matches([' ', '\t', '\r']),
            value.trim_matches([' ', '\t', '\r']),
        );
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(value);
        ensure!(
            !key.is_empty()
                && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._/-".contains(&b)),
            "The settings file contains a customized assignment. Only simple values can be edited in the dashboard."
        );
        match key {
            START => {
                settings.start = value
                    .parse()
                    .context("Invalid resume percentage in the settings file")?
            }
            STOP => {
                settings.stop = value
                    .parse()
                    .context("Invalid stop percentage in the settings file")?
            }
            OFF_MODE => {
                settings.off_mode =
                    StopAction::from_str(value, false).map_err(anyhow::Error::msg)?
            }
            "HP_CHARGE_LIMIT_RS_INTERVAL" => {
                settings.advanced.interval =
                    Some(value.parse().context("Invalid polling interval")?)
            }
            "HP_CHARGE_LIMIT_RS_RUN_MODE" => {
                settings.advanced.run_mode =
                    Some(Mode::from_str(value, false).map_err(anyhow::Error::msg)?)
            }
            "HP_CHARGE_LIMIT_RS_OFF_MIN" => {
                settings.advanced.off_min = Some(
                    value
                        .parse()
                        .context("Invalid shutdown exception percentage")?,
                )
            }
            "HP_CHARGE_LIMIT_RS_STOP_MODE" => {
                settings.advanced.stop_mode =
                    Some(StopAction::from_str(value, false).map_err(anyhow::Error::msg)?)
            }
            "HP_CHARGE_LIMIT_RS_NOTIFY" => {
                settings.advanced.notify =
                    Some(value.parse().context("Invalid notification setting")?)
            }
            _ => {}
        }
    }
    settings.validate()?;
    Ok(settings)
}

pub(super) fn with_settings(text: &str, settings: ChargeSettings) -> String {
    let assignments: Vec<_> = settings
        .values()
        .into_iter()
        .map(|(key, value)| {
            (
                format!(
                    "HP_CHARGE_LIMIT_RS_{}",
                    key.replace('-', "_").to_ascii_uppercase()
                ),
                value,
            )
        })
        .collect();
    let mut updated: String = text
        .split_inclusive('\n')
        .filter(|line| {
            !line.split_once('=').is_some_and(|(key, _)| {
                assignments
                    .iter()
                    .any(|(name, _)| name == key.trim_matches([' ', '\t', '\r']))
            })
        })
        .collect();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    for (key, value) in assignments {
        updated.push_str(&format!("{key}={value}\n"));
    }
    updated
}

fn replace_config(path: &Path, text: &str, mode: u32) -> Result<()> {
    let mut file =
        tempfile::NamedTempFile::new_in(path.parent().context("settings directory is missing")?)?;
    file.write_all(text.as_bytes())?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))?;
    file.as_file().sync_all()?;
    file.persist(path)
        .with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

pub fn charge_settings(installed: bool) -> Result<ChargeSettings> {
    if installed {
        check_unit()?;
    }
    let config = read_config(Path::new(CONFIG_PATH))?;
    ensure!(
        installed || config.is_none(),
        "The existing settings file will be kept. You can edit it here after installing the service."
    );
    parse_settings(config.as_ref().map_or("", |(text, _)| text))
}

pub fn configure(settings: ChargeSettings) -> Result<()> {
    settings.validate()?;
    require_root()?;
    let _lock = setup_lock()?;
    check_unit()?;
    let state = run_systemctl(&["show", "--property=ActiveState", "--value", SERVICE])?;
    ensure!(
        matches!(state.as_str(), "active" | "inactive" | "failed"),
        "Wait for the service to finish transitioning before saving."
    );
    let path = Path::new(CONFIG_PATH);
    let config = read_config(path)?;
    let (text, mode) = config
        .as_ref()
        .map_or(("", 0o644), |(text, mode)| (text.as_str(), *mode));
    parse_settings(text)?;
    replace_config(path, &with_settings(text, settings), mode)?;
    if state == "active" {
        let restart = || {
            run_systemctl(&["restart", SERVICE])
                .and_then(|_| run_systemctl(&["is-active", "--quiet", SERVICE]))
        };
        if let Err(error) = restart() {
            let restore = if config.is_some() {
                replace_config(path, text, mode)
            } else {
                fs::remove_file(path).context("remove new settings file")
            };
            return match restore.and_then(|()| restart().map(drop)) {
                Ok(()) => Err(error.context(
                    "Could not restart with the new settings. Previous settings restored.",
                )),
                Err(rollback) => Err(error.context(format!(
                    "Could not restart the service or complete rollback: {rollback:#}"
                ))),
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_only_selected_settings_and_preserves_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.conf");
        let original = "# example\nHP_CHARGE_LIMIT_RS_INTERVAL=17\nHP_CHARGE_LIMIT_RS_NOTIFY=false\nHP_CHARGE_LIMIT_RS_OFF_MIN=15\nHP_CHARGE_LIMIT_RS_START='50'\nHP_CHARGE_LIMIT_RS_START=55\nHP_CHARGE_LIMIT_RS_STOP=60\n";
        assert_eq!(parse_settings(original).unwrap().start, 55);
        let selected = ChargeSettings {
            start: 75,
            stop: 80,
            off_mode: StopAction::Inhibit,
            advanced: Default::default(),
        };
        replace_config(&file, &with_settings(original, selected), 0o640).unwrap();
        let latest = fs::read_to_string(&file).unwrap();
        let previous = parse_settings(original).unwrap();
        assert_eq!(
            parse_settings(&latest).unwrap(),
            ChargeSettings {
                advanced: previous.advanced,
                ..selected
            }
        );
        let full = ChargeSettings {
            advanced: crate::cli::AdvancedSettings {
                interval: Some(99),
                off_min: Some(35),
                run_mode: Some(Mode::Hold),
                stop_mode: Some(StopAction::Keep),
                notify: Some(true),
            },
            ..selected
        };
        assert_eq!(parse_settings(&with_settings(&latest, full)).unwrap(), full);
        assert!(latest.starts_with("# example\nHP_CHARGE_LIMIT_RS_INTERVAL=17\nHP_CHARGE_LIMIT_RS_NOTIFY=false\nHP_CHARGE_LIMIT_RS_OFF_MIN=15\n"));
        assert_eq!(latest.matches(START).count(), 1);
        assert_eq!(fs::metadata(&file).unwrap().mode() & 0o777, 0o640);
    }

    #[test]
    fn rejects_multiline_assignments_and_invalid_thresholds() {
        assert!(parse_settings("OTHER=\"a\nHP_CHARGE_LIMIT_RS_STOP=80\n\"").is_err());
        assert!(parse_settings("OTHER=a\\\nHP_CHARGE_LIMIT_RS_STOP=80").is_err());
        assert!(parse_settings("# comment\\\nHP_CHARGE_LIMIT_RS_STOP=80").is_err());
        assert!(parse_settings("HP_CHARGE_LIMIT_RS_STOP=25").is_err());
    }
}
