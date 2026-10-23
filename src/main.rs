use branchline::{storage::RecoveryStore, ui::NativeApp};
use directories::ProjectDirs;
use eframe::egui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directories = ProjectDirs::from("org", "Branchline", "Branchline")
        .ok_or("Cannot locate the platform application-data directory")?;
    let store = RecoveryStore::open(directories.data_local_dir().into())?;
    let app = NativeApp::new(store)?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([850.0, 560.0])
            .with_app_id("org.branchline.desktop"),
        ..Default::default()
    };
    eframe::run_native(
        "BMM",
        options,
        Box::new(|context| {
            context.egui_ctx.set_theme(egui::Theme::Light);
            let mut style = (*context.egui_ctx.global_style()).clone();
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            style.spacing.button_padding = egui::vec2(10.0, 6.0);
            context.egui_ctx.set_global_style(style);
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}
