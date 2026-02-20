use png::{BitDepth, ColorType, Encoder};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum PasteClipboardImageError {
    #[error(transparent)]
    ResolveStateDir(#[from] super::ResolveCcboxStateDirError),

    #[error("clipboard error: {0}")]
    Clipboard(String),

    #[error("clipboard has no image")]
    NoImage,

    #[error("failed to create images dir {path}: {source}")]
    CreateDir { path: String, source: io::Error },

    #[error("failed to create image file {path}: {source}")]
    CreateFile { path: String, source: io::Error },

    #[error("failed to write image file {path}: {source}")]
    WriteFile { path: String, source: io::Error },

    #[error("failed to encode png: {0}")]
    EncodePng(String),
}

pub fn paste_clipboard_image_to_task_images_dir() -> Result<PathBuf, PasteClipboardImageError> {
    let state_dir = super::resolve_ccbox_state_dir()?;
    let images_dir = state_dir.join("task_images");
    fs::create_dir_all(&images_dir).map_err(|error| PasteClipboardImageError::CreateDir {
        path: images_dir.display().to_string(),
        source: error,
    })?;

    let file_path = images_dir.join(format!("clipboard-{}.png", Uuid::new_v4()));
    write_clipboard_png(&file_path)?;
    Ok(file_path)
}

fn write_clipboard_png(dest: &Path) -> Result<(), PasteClipboardImageError> {
    let mut clipboard = arboard::Clipboard::new()
        .map_err(|error| PasteClipboardImageError::Clipboard(error.to_string()))?;

    let image = match clipboard.get_image() {
        Ok(image) => image,
        Err(arboard::Error::ContentNotAvailable) => return Err(PasteClipboardImageError::NoImage),
        Err(error) => return Err(PasteClipboardImageError::Clipboard(error.to_string())),
    };

    let file =
        std::fs::File::create(dest).map_err(|error| PasteClipboardImageError::CreateFile {
            path: dest.display().to_string(),
            source: error,
        })?;

    let width = u32::try_from(image.width).unwrap_or(u32::MAX);
    let height = u32::try_from(image.height).unwrap_or(u32::MAX);
    let mut encoder = Encoder::new(file, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);

    let mut writer = encoder
        .write_header()
        .map_err(|error| PasteClipboardImageError::EncodePng(error.to_string()))?;
    writer
        .write_image_data(image.bytes.as_ref())
        .map_err(|error| match error {
            png::EncodingError::IoError(io) => PasteClipboardImageError::WriteFile {
                path: dest.display().to_string(),
                source: io,
            },
            other => PasteClipboardImageError::EncodePng(other.to_string()),
        })?;

    Ok(())
}
