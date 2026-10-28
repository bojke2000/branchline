use eframe::egui;

#[cfg(windows)]
pub mod software;

pub fn configure_context(context: &egui::Context) {
    context.set_theme(egui::Theme::Dark);
    let mut style = (*context.global_style()).clone();
    style.spacing.item_spacing = egui::vec2(10.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 6.0);
    context.set_global_style(style);
}
