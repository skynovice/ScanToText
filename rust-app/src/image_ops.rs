use std::io::Cursor;

use image::{DynamicImage, GenericImage, Rgba, RgbaImage, imageops};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Roi {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub text: String,
}

impl Roi {
    pub fn clipped(&self, image: &RgbaImage) -> Option<Self> {
        let x = self.x.min(image.width());
        let y = self.y.min(image.height());
        let right = self.x.saturating_add(self.width).min(image.width());
        let bottom = self.y.saturating_add(self.height).min(image.height());
        (right > x + 4 && bottom > y + 4).then(|| Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
            text: self.text.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRow {
    pub id: String,
    pub top: u32,
    pub bottom: u32,
}

pub fn crop(image: &RgbaImage, roi: &Roi) -> Option<RgbaImage> {
    let roi = roi.clipped(image)?;
    Some(imageops::crop_imm(image, roi.x, roi.y, roi.width, roi.height).to_image())
}

pub fn rotate(image: &RgbaImage, clockwise: bool) -> RgbaImage {
    if clockwise {
        imageops::rotate90(image)
    } else {
        imageops::rotate270(image)
    }
}

pub fn compose_batch(
    crops: &[RgbaImage],
    max_width: u32,
) -> Result<(RgbaImage, Vec<BatchRow>), String> {
    if crops.is_empty() {
        return Err("ไม่มี ROI".into());
    }
    let margin = 18;
    let header = 46;
    let gap = 18;
    let prepared: Vec<RgbaImage> = crops
        .iter()
        .map(|image| {
            if image.width() > max_width {
                let height = ((image.height() as f64 * max_width as f64 / image.width() as f64)
                    .round() as u32)
                    .max(1);
                imageops::resize(image, max_width, height, imageops::FilterType::Triangle)
            } else {
                image.clone()
            }
        })
        .collect();
    if prepared
        .iter()
        .any(|image| image.width() == 0 || image.height() == 0)
    {
        return Err("พบ ROI ว่าง".into());
    }
    let width = (prepared
        .iter()
        .map(RgbaImage::width)
        .max()
        .ok_or("ไม่มี ROI")?
        + margin * 2)
        .max(204);
    let height = margin
        + prepared
            .iter()
            .map(|image| header + image.height() + gap)
            .sum::<u32>();
    let mut output = RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]));
    let mut rows = Vec::with_capacity(prepared.len());
    let mut y = margin;
    for (index, image) in prepared.iter().enumerate() {
        for py in y..y + header {
            for px in 0..width {
                output.put_pixel(px, py, Rgba([225, 225, 225, 255]));
            }
        }
        draw_roi_label(&mut output, margin, y + 12, index + 1);
        let top = y + header;
        output
            .copy_from(image, margin, top)
            .map_err(|error| error.to_string())?;
        rows.push(BatchRow {
            id: format!("ROI_{:03}", index + 1),
            top,
            bottom: top + image.height(),
        });
        y = top + image.height() + gap;
    }
    Ok((output, rows))
}

fn draw_roi_label(image: &mut RgbaImage, x: u32, y: u32, number: usize) {
    let label = format!("ROI_{number:03}");
    let glyph = |ch| -> [u8; 7] {
        match ch {
            'R' => [
                0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
            ],
            'O' | '0' => [
                0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
            ],
            'I' => [
                0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
            ],
            '_' => [0, 0, 0, 0, 0, 0, 0b11111],
            '1' => [
                0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
            ],
            '2' => [
                0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
            ],
            '3' => [
                0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
            ],
            '4' => [
                0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
            ],
            '5' => [
                0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
            ],
            '6' => [
                0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
            ],
            '7' => [
                0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
            ],
            '8' => [
                0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
            ],
            '9' => [
                0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
            ],
            _ => [0; 7],
        }
    };
    let scale = 4u32;
    let mut cursor = x;
    for ch in label.chars() {
        for (row, bits) in glyph(ch).into_iter().enumerate() {
            for col in 0..5u32 {
                if bits & (1 << (4 - col)) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = cursor + col * scale + dx;
                            let py = y + row as u32 * scale + dy;
                            if px < image.width() && py < image.height() {
                                image.put_pixel(px, py, Rgba([0, 0, 0, 255]));
                            }
                        }
                    }
                }
            }
        }
        cursor += 6 * scale;
    }
}

pub fn encode_jpeg_under_limit(
    image: &RgbaImage,
    rows: &[BatchRow],
    max_bytes: usize,
) -> Result<(Vec<u8>, Vec<BatchRow>), String> {
    let mut current = image.clone();
    let mut current_rows = rows.to_vec();
    for _ in 0..7 {
        for quality in [90, 80, 70] {
            let mut cursor = Cursor::new(Vec::new());
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
            encoder
                .encode_image(&DynamicImage::ImageRgba8(current.clone()))
                .map_err(|error| error.to_string())?;
            if cursor.get_ref().len() <= max_bytes {
                return Ok((cursor.into_inner(), current_rows));
            }
        }
        if current.width().min(current.height()) < 160 {
            break;
        }
        let width = (current.width() as f32 * 0.8).round().max(1.0) as u32;
        let height = (current.height() as f32 * 0.8).round().max(1.0) as u32;
        current = imageops::resize(&current, width, height, imageops::FilterType::Triangle);
        current_rows = current_rows
            .into_iter()
            .map(|row| BatchRow {
                id: row.id,
                top: (row.top as f32 * 0.8).round() as u32,
                bottom: (row.bottom as f32 * 0.8).round() as u32,
            })
            .collect();
    }
    Err("ภาพ ROI รวมมีขนาดใหญ่เกินไป กรุณาลดจำนวนหรือขนาดกรอบ".into())
}

pub fn encode_jpeg(image: &RgbaImage, quality: u8) -> Result<Vec<u8>, image::ImageError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, quality);
    encoder.encode_image(&DynamicImage::ImageRgba8(image.clone()))?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_rows_do_not_overlap_and_fit_limit() {
        let red = RgbaImage::from_pixel(100, 40, Rgba([255, 0, 0, 255]));
        let green = RgbaImage::from_pixel(70, 25, Rgba([0, 255, 0, 255]));
        let (image, rows) = compose_batch(&[red, green], 1400).unwrap();
        assert_eq!(rows[0].id, "ROI_001");
        assert!(rows[0].bottom <= rows[1].top);
        let (jpeg, encoded_rows) = encode_jpeg_under_limit(&image, &rows, 950_000).unwrap();
        assert_eq!(&jpeg[..2], &[0xff, 0xd8]);
        assert_eq!(encoded_rows.len(), 2);
    }

    #[test]
    fn rotation_swaps_dimensions() {
        let image = RgbaImage::new(3, 2);
        assert_eq!(rotate(&image, true).dimensions(), (2, 3));
    }
}
