#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use roi_ocr_helper::{APP_NAME, app::RoiOcrApp, platform};

fn main() -> eframe::Result<()> {
    platform::enable_per_monitor_dpi();
    let Some(_instance) = platform::acquire_single_instance()
        .map_err(|error| eframe::Error::AppCreation(Box::new(std::io::Error::other(error))))?
    else {
        platform::show_warning("โปรแกรมทำงานอยู่", "แปลงภาพเป็นข้อความเปิดอยู่แล้ว");
        return Ok(());
    };
    let icon = image::load_from_memory(include_bytes!("../../assets/app_icon.png"))
        .ok()
        .map(|image| {
            let rgba = image.to_rgba8();
            egui::IconData {
                rgba: rgba.into_raw(),
                width: image.width(),
                height: image.height(),
            }
        });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ROI OCR Launcher")
            .with_inner_size([32.0, 32.0])
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_transparent(true)
            .with_icon(icon.unwrap_or_default()),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| Ok(Box::new(RoiOcrApp::new(cc)))),
    )
}
