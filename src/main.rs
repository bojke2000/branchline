use branchline::{desktop, recovery::RecoveryCoordinator, ui::NativeApp};
use directories::ProjectDirs;
use eframe::egui;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let renderer = renderer_from_args(std::env::args().skip(1))?;
    let directories = ProjectDirs::from("org", "Branchline", "Branchline")
        .ok_or("Cannot locate the platform application-data directory")?;
    let store = RecoveryCoordinator::open(directories.data_local_dir().into())?;
    let app = NativeApp::new(store)?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([850.0, 560.0])
            .with_app_id("org.branchline.desktop"),
        ..Default::default()
    };
    #[cfg(windows)]
    if renderer == "software" {
        let mut app = app;
        let context = egui::Context::default();
        desktop::configure_context(&context);
        let result = desktop::software::run(context, options.viewport.with_title("BMM"), |ui| {
            app.show(ui);
        });
        if result.is_err()
            && let Err(error) = app.checkpoint_recovery()
        {
            eprintln!("Could not checkpoint recovery after renderer failure: {error}");
        }
        return result;
    }
    let _ = renderer;
    eframe::run_native(
        "BMM",
        options,
        Box::new(|context| {
            desktop::configure_context(&context.egui_ctx);
            Ok(Box::new(app))
        }),
    )?;
    Ok(())
}

fn renderer_from_args(args: impl IntoIterator<Item = String>) -> Result<String, String> {
    let mut args = args.into_iter();
    let renderer = match args.next().as_deref() {
        None => if cfg!(windows) { "software" } else { "wgpu" }.into(),
        Some("--renderer") => args
            .next()
            .ok_or("Missing renderer: use software or wgpu")?,
        Some(argument) => return Err(format!("Unknown argument: {argument}")),
    };
    if args.next().is_some() {
        return Err("Unexpected arguments after --renderer".into());
    }
    if renderer != "wgpu" && !(cfg!(windows) && renderer == "software") {
        return Err("Use --renderer wgpu, or --renderer software on Windows".into());
    }
    Ok(renderer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_arguments_are_explicit_and_validated_before_opening_recovery() {
        assert_eq!(
            renderer_from_args([]).unwrap(),
            if cfg!(windows) { "software" } else { "wgpu" }
        );
        assert_eq!(
            renderer_from_args(["--renderer".into(), "wgpu".into()]).unwrap(),
            "wgpu"
        );
        for args in [
            vec!["--renderer"],
            vec!["--renderer", "unknown"],
            vec!["--renderer", "wgpu", "extra"],
            vec!["--unknown"],
        ] {
            assert!(renderer_from_args(args.into_iter().map(String::from)).is_err());
        }
    }
}
