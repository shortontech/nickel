use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::{GenericImageView, ImageFormat, ImageReader, RgbaImage};

const MAX_ENCODED_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_PREVIEW_EDGE: u32 = 1024;

#[derive(Clone, Debug)]
pub struct DecodedPreview {
    pub image: Arc<RgbaImage>,
    pub source_width: u32,
    pub source_height: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewDecodeError {
    Missing(PathBuf),
    Unreadable(String),
    Empty,
    EncodedTooLarge(u64),
    UnsupportedFormat,
    AnimatedUnsupported,
    DimensionsTooLarge { width: u32, height: u32 },
    Corrupt(String),
}

impl std::fmt::Display for PreviewDecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(path) => write!(
                formatter,
                "image is no longer available: {}",
                path.display()
            ),
            Self::Unreadable(error) => write!(formatter, "image cannot be read: {error}"),
            Self::Empty => formatter.write_str("image file is empty"),
            Self::EncodedTooLarge(bytes) => {
                write!(formatter, "encoded image is too large ({bytes} bytes)")
            }
            Self::UnsupportedFormat => formatter.write_str("image format is unsupported"),
            Self::AnimatedUnsupported => {
                formatter.write_str("animated images are not supported; choose a static image")
            }
            Self::DimensionsTooLarge { width, height } => write!(
                formatter,
                "image dimensions are too large ({width}x{height})"
            ),
            Self::Corrupt(error) => write!(formatter, "image is corrupt: {error}"),
        }
    }
}

impl std::error::Error for PreviewDecodeError {}

pub fn decode_image_preview(path: &Path) -> Result<DecodedPreview, PreviewDecodeError> {
    let metadata = std::fs::metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PreviewDecodeError::Missing(path.to_owned())
        } else {
            PreviewDecodeError::Unreadable(error.to_string())
        }
    })?;
    if metadata.len() == 0 {
        return Err(PreviewDecodeError::Empty);
    }
    if metadata.len() > MAX_ENCODED_BYTES {
        return Err(PreviewDecodeError::EncodedTooLarge(metadata.len()));
    }

    let reader = ImageReader::open(path)
        .map_err(|error| PreviewDecodeError::Unreadable(error.to_string()))?
        .with_guessed_format()
        .map_err(|error| PreviewDecodeError::Corrupt(error.to_string()))?;
    let format = reader
        .format()
        .ok_or(PreviewDecodeError::UnsupportedFormat)?;
    if format == ImageFormat::Gif {
        return Err(PreviewDecodeError::AnimatedUnsupported);
    }
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Bmp
    ) {
        return Err(PreviewDecodeError::UnsupportedFormat);
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|error| PreviewDecodeError::Corrupt(error.to_string()))?;
    admit_dimensions(width, height)?;

    let decoded = ImageReader::open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                PreviewDecodeError::Missing(path.to_owned())
            } else {
                PreviewDecodeError::Unreadable(error.to_string())
            }
        })?
        .with_guessed_format()
        .map_err(|error| PreviewDecodeError::Corrupt(error.to_string()))?
        .decode()
        .map_err(|error| PreviewDecodeError::Corrupt(error.to_string()))?;
    let preview = if width > MAX_PREVIEW_EDGE || height > MAX_PREVIEW_EDGE {
        decoded.thumbnail(MAX_PREVIEW_EDGE, MAX_PREVIEW_EDGE)
    } else {
        decoded
    };
    let (preview_width, preview_height) = preview.dimensions();
    debug_assert!(preview_width <= MAX_PREVIEW_EDGE && preview_height <= MAX_PREVIEW_EDGE);
    Ok(DecodedPreview {
        image: Arc::new(preview.to_rgba8()),
        source_width: width,
        source_height: height,
    })
}

fn admit_dimensions(width: u32, height: u32) -> Result<(), PreviewDecodeError> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || pixels > MAX_PIXELS
    {
        Err(PreviewDecodeError::DimensionsTooLarge { width, height })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_are_admitted_before_large_allocation() {
        assert!(admit_dimensions(1920, 1080).is_ok());
        assert_eq!(
            admit_dimensions(100_000, 100_000),
            Err(PreviewDecodeError::DimensionsTooLarge {
                width: 100_000,
                height: 100_000,
            })
        );
        assert_eq!(
            admit_dimensions(16_000, 16_000),
            Err(PreviewDecodeError::DimensionsTooLarge {
                width: 16_000,
                height: 16_000,
            })
        );
    }

    #[test]
    fn portrait_landscape_square_corrupt_missing_and_animated_inputs_are_explicit() {
        let directory = tempfile::tempdir().unwrap();
        for (name, width, height) in [
            ("portrait", 12, 20),
            ("landscape", 20, 12),
            ("square", 16, 16),
        ] {
            let path = directory.path().join(format!("{name}.png"));
            RgbaImage::new(width, height).save(&path).unwrap();
            let decoded = decode_image_preview(&path).unwrap();
            assert_eq!(
                (decoded.source_width, decoded.source_height),
                (width, height)
            );
        }
        let corrupt = directory.path().join("corrupt.png");
        std::fs::write(&corrupt, b"not an image").unwrap();
        assert!(matches!(
            decode_image_preview(&corrupt),
            Err(PreviewDecodeError::UnsupportedFormat | PreviewDecodeError::Corrupt(_))
        ));
        assert!(matches!(
            decode_image_preview(&directory.path().join("gone.png")),
            Err(PreviewDecodeError::Missing(_))
        ));
        let animated = directory.path().join("animated.gif");
        std::fs::write(&animated, b"GIF89a\x01\0\x01\0\0\0\0").unwrap();
        assert!(matches!(
            decode_image_preview(&animated),
            Err(PreviewDecodeError::AnimatedUnsupported)
        ));
    }
}
