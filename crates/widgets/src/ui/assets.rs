//! Icons: the kit's bundled set plus the few Lucide icons the views need that it does not ship. The kit embeds only
//! its 101 component icons by default, so an icon from its full catalog draws nothing until it is added here.

use std::borrow::Cow;

use gpui_kit::*;

/// Icons added to the kit's set: the path a `gpui_kit::assets::IconName` resolves to, and its SVG.
const EXTRA: &[(&str, &str)] = &[(
    "icons/funnel.svg",
    include_str!("../../assets/icons/funnel.svg"),
)];

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, svg)) = EXTRA.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(svg.as_bytes())));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = gpui_kit::assets::Assets.list(path)?;
        all.extend(
            EXTRA
                .iter()
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(*p)),
        );
        Ok(all)
    }
}
