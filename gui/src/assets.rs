use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

// These names exist in the catalog but are not in the default component bundle.
gpui_kit::assets::icon_assets!(ExtraIcons, [ShieldCheck, PlugZap]);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::assets::IconName;

    #[test]
    fn every_dashboard_and_action_icon_is_embedded() {
        for icon in [
            IconName::Battery,
            IconName::BatteryCharging,
            IconName::BatteryFull,
            IconName::PlugZap,
            IconName::ShieldCheck,
            IconName::RefreshCw,
            IconName::Settings2,
            IconName::LoaderCircle,
            IconName::CircleAlert,
            IconName::CircleCheck,
            IconName::ExternalLink,
            IconName::SquareTerminal,
            IconName::Play,
        ] {
            let bytes = Assets.load(&icon.path()).unwrap().unwrap();
            assert!(bytes.windows(4).any(|chunk| chunk == b"<svg"), "{icon:?}");
        }
        // Also retain the component assets needed by controls and spinners.
        let defaults = gpui_kit::assets::Assets.list("icons/").unwrap();
        for path in defaults {
            assert!(Assets.load(&path).unwrap().is_some(), "{path}");
        }
    }
}
