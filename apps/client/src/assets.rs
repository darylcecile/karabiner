//! Asset source: files under `apps/client/assets/` embedded at build time (Kubai feature
//! icons, rendered with `img()`), so release binaries work outside the checkout; then the
//! full Lucide catalog, so any `IconName` a feature view uses renders.

use std::borrow::Cow;

use gpui_kit::assets::AllAssets;
use gpui_kit::*;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "$CARGO_MANIFEST_DIR/assets"]
#[exclude = "*.md"]
struct AppFiles;

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(file) = AppFiles::get(path) {
            return Ok(Some(file.data));
        }
        AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths: Vec<SharedString> =
            AppFiles::iter().filter(|name| name.starts_with(path)).map(|name| name.to_string().into()).collect();
        paths.extend(AllAssets.list(path)?);
        Ok(paths)
    }
}
