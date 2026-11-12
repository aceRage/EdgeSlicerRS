//! The page's own layout, kept between runs in the app's settings
//! (`layout` in `settings.json`, [`super::store::Data`]) rather than in
//! the browser's storage, which the window and `--serve` do not share:
//! the sidebar's width as it was dragged.
//!
//! This is the owner's choice for this app and not the reference's: its
//! sidebar has a fixed width (UI_PARITY.md section 2, "Sidebar").

use serde_json::{Value, json};

use super::{Session, lock};

/// The narrowest the sidebar is dragged to, its usual width in a narrow
/// window, which the pages were laid out for: an option row keeps its
/// label, a 112 px control and its reset arrow, and the pages' tabs and
/// the search their room.
pub const SIDEBAR_MIN: u32 = 300;
/// The widest. The page also keeps the stage at least
/// [`STAGE_MIN`] wide, so in a narrow window the sidebar stops sooner.
pub const SIDEBAR_MAX: u32 = 640;
/// The least the 3D stage is left beside the sidebar (`style.css` and the
/// grip in `app.js` hold to it): at 900 px with the strip at its usual
/// width that stops the sidebar at 392 px.
pub const STAGE_MIN: u32 = 420;

impl Session {
    /// `layout`: what the page keeps of its layout, with the limits it
    /// holds to. `sidebar_width` is null for the usual width (the style's
    /// own, which follows the window).
    pub(super) fn layout(&self) -> Value {
        let state = lock(&self.state);
        let data = &state.store.data;
        json!({
            "sidebar_width": data.sidebar_width,
            "sidebar_min": SIDEBAR_MIN,
            "sidebar_max": SIDEBAR_MAX,
            "stage_min": STAGE_MIN,
        })
    }

    /// `layout_set`: `sidebar_width`, a whole number of pixels from
    /// [`SIDEBAR_MIN`] to [`SIDEBAR_MAX`], or null for the usual width.
    /// Kept at once; the answer is `layout`'s.
    pub(super) fn layout_set(&self, args: &Value) -> Result<Value, String> {
        if let Some(width) = args.get("sidebar_width") {
            let width = match width {
                Value::Null => None,
                width => Some(
                    width
                        .as_u64()
                        .and_then(|width| u32::try_from(width).ok())
                        .filter(|width| (SIDEBAR_MIN..=SIDEBAR_MAX).contains(width))
                        .ok_or_else(|| {
                            format!(
                                "sidebar_width must be a whole number from {SIDEBAR_MIN} to {SIDEBAR_MAX}, or null"
                            )
                        })?,
                ),
            };
            lock(&self.state)
                .store
                .change(|data| data.sidebar_width = width)?;
        }
        Ok(self.layout())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn the_sidebar_width_is_kept_between_runs_within_its_limits() {
        let dir = tempfile::tempdir().unwrap();
        let session = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
        let answer = session.call("layout", &json!({})).unwrap();
        assert_eq!(answer["sidebar_width"], Value::Null);
        assert_eq!(answer["sidebar_min"], SIDEBAR_MIN);
        assert_eq!(answer["sidebar_max"], SIDEBAR_MAX);
        assert_eq!(answer["stage_min"], STAGE_MIN);

        let answer = session
            .call("layout_set", &json!({ "sidebar_width": 452 }))
            .unwrap();
        assert_eq!(answer["sidebar_width"], 452);
        // Another run finds it.
        let again = Arc::new(Session::with_settings(None, Some(dir.path().to_owned())));
        assert_eq!(again.call("layout", &json!({})).unwrap()["sidebar_width"], 452);

        // Outside the limits, or not a whole number: refused, and nothing changes.
        for wrong in [
            json!(SIDEBAR_MIN - 1),
            json!(SIDEBAR_MAX + 1),
            json!(400.5),
            json!("400"),
            json!(-1),
        ] {
            let said = again
                .call("layout_set", &json!({ "sidebar_width": wrong }))
                .unwrap_err();
            assert!(said.contains("sidebar_width"), "{said}");
        }
        assert_eq!(again.call("layout", &json!({})).unwrap()["sidebar_width"], 452);

        // Null is the usual width again, and nothing is kept for it.
        let answer = again
            .call("layout_set", &json!({ "sidebar_width": null }))
            .unwrap();
        assert_eq!(answer["sidebar_width"], Value::Null);
        let written: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
        )
        .unwrap();
        assert!(written.get("layout").is_none(), "{written}");
    }
}
