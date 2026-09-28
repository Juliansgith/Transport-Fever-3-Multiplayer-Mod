//! The window's icon: the page's TF3 MP logo (`images/logo.png`).

use eframe::egui::IconData;

/// The icon, as RGBA pixels.
pub fn icon() -> IconData {
    let image = image::load_from_memory(include_bytes!("../images/logo.png"))
        .expect("the launcher's built-in logo decodes")
        .to_rgba8();
    IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_logo_is_the_icon() {
        let icon = super::icon();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }
}
