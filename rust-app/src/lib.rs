pub mod app;
pub mod camera;
pub mod config;
pub mod history;
pub mod image_ops;
pub mod ocr;
pub mod platform;

pub const APP_NAME: &str = "แปลงภาพเป็นข้อความ";
pub const APP_DIR_NAME: &str = "ROI-OCR-Helper";
pub const APP_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION_MAJOR"),
    ".",
    env!("CARGO_PKG_VERSION_MINOR"),
    ".",
    env!("CARGO_PKG_VERSION_PATCH")
);
pub const APP_UPDATED_DATE: &str = "27OCT2026";
