use std::{
    fs, io,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Local, NaiveDateTime};
use image::{DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};

use crate::{image_ops::Roi, platform};

fn write_jpeg(image: &RgbaImage, path: &Path) -> io::Result<()> {
    let mut file = fs::File::create(path)?;
    DynamicImage::ImageRgba8(image.clone())
        .write_to(&mut file, ImageFormat::Jpeg)
        .map_err(io::Error::other)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedCrop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub text: String,
}

impl From<&Roi> for SavedCrop {
    fn from(value: &Roi) -> Self {
        Self {
            x: value.x,
            y: value.y,
            width: value.width,
            height: value.height,
            text: value.text.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct HistoryState {
    image: String,
    crops: Vec<SavedCrop>,
}

pub fn entries(directory: &Path) -> Vec<PathBuf> {
    let Ok(iter) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut files: Vec<_> = iter
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("jpg"))
        })
        .collect();
    files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    files
}

pub fn display_name(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| NaiveDateTime::parse_from_str(stem, "%Y%m%d-%H%M%S-%6f").ok())
        .map(|stamp| stamp.format("%d/%m/%Y  %H:%M:%S").to_string())
        .unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
}

pub fn save_image_at(
    image: &RgbaImage,
    directory: &Path,
    now: DateTime<Local>,
) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    for offset in 0..100u32 {
        let stamp = now + chrono::Duration::microseconds(offset.into());
        let path = directory.join(stamp.format("%Y%m%d-%H%M%S-%6f.jpg").to_string());
        if path.exists() {
            continue;
        }
        let temp = path.with_extension("jpg.tmp");
        write_jpeg(image, &temp)?;
        platform::atomic_replace(&temp, &path)?;
        return Ok(path);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "หา filename สำหรับ History ไม่ได้",
    ))
}

pub fn save_image(image: &RgbaImage, directory: &Path) -> io::Result<PathBuf> {
    save_image_at(image, directory, Local::now())
}

pub fn replace_image(image: &RgbaImage, path: &Path) -> io::Result<()> {
    let temp = path.with_extension("jpg.tmp");
    write_jpeg(image, &temp)?;
    platform::atomic_replace(&temp, path)
}

pub fn meta_path(image_path: &Path) -> PathBuf {
    image_path.with_extension("json")
}

pub fn save_state(image_path: &Path, crops: &[Roi]) -> io::Result<()> {
    if !image_path.exists() {
        return Ok(());
    }
    let state = HistoryState {
        image: image_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        crops: crops.iter().map(SavedCrop::from).collect(),
    };
    let path = meta_path(image_path);
    let temp = path.with_extension("tmp");
    fs::write(
        &temp,
        serde_json::to_vec_pretty(&state).map_err(io::Error::other)?,
    )?;
    platform::atomic_replace(&temp, &path)
}

pub fn load_state(image_path: &Path) -> Vec<SavedCrop> {
    let Ok(bytes) = fs::read(meta_path(image_path)) else {
        return Vec::new();
    };
    let Ok(state) = serde_json::from_slice::<HistoryState>(&bytes) else {
        return Vec::new();
    };
    let expected = image_path.file_name().unwrap_or_default().to_string_lossy();
    if state.image != expected {
        return Vec::new();
    }
    state.crops
}

pub fn load_image(path: &Path) -> Result<RgbaImage, image::ImageError> {
    Ok(image::open(path)?.to_rgba8())
}

pub fn delete(image_path: &Path) -> io::Result<()> {
    if image_path.exists() {
        fs::remove_file(image_path)?;
    }
    let meta = meta_path(image_path);
    if meta.exists() {
        fs::remove_file(meta)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn image_and_roi_state_round_trip_and_replace() {
        let root = std::env::temp_dir().join(format!("roi-ocr-history-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let first = RgbaImage::from_pixel(12, 8, Rgba([255, 0, 0, 255]));
        let path = save_image(&first, &root).unwrap();
        let crops = vec![Roi {
            x: 1,
            y: 2,
            width: 8,
            height: 5,
            text: "เลขที่ 42".into(),
        }];
        save_state(&path, &crops).unwrap();
        assert_eq!(load_state(&path), vec![SavedCrop::from(&crops[0])]);

        let second = RgbaImage::from_pixel(8, 12, Rgba([0, 0, 255, 255]));
        replace_image(&second, &path).unwrap();
        assert_eq!(load_image(&path).unwrap().dimensions(), (8, 12));

        delete(&path).unwrap();
        assert!(!path.exists());
        assert!(!meta_path(&path).exists());
        let _ = fs::remove_dir_all(root);
    }
}
