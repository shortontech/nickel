use std::{io::Cursor, sync::Arc};

use base64::{Engine, engine::general_purpose::STANDARD};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits, RgbaImage};
use nickel_codex::TurnImage;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AttachmentId(pub u64);

#[derive(Clone)]
pub struct PendingAttachment {
    pub id: AttachmentId,
    pub width: u32,
    pub height: u32,
    pub encoded_size: usize,
    pub preview: Arc<RgbaImage>,
    turn_image: TurnImage,
}

impl std::fmt::Debug for PendingAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingAttachment")
            .field("id", &self.id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("encoded_size", &self.encoded_size)
            .finish_non_exhaustive()
    }
}

impl PendingAttachment {
    pub fn turn_image(&self) -> TurnImage {
        self.turn_image.clone()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AttachmentLimits {
    pub count: usize,
    pub encoded_bytes_per_image: usize,
    pub decoded_pixels_per_image: u64,
    pub aggregate_decoded_bytes: usize,
}

impl Default for AttachmentLimits {
    fn default() -> Self {
        Self {
            count: 8,
            encoded_bytes_per_image: 12 * 1024 * 1024,
            decoded_pixels_per_image: 20_000_000,
            aggregate_decoded_bytes: 96 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentError {
    UnsupportedFormat,
    Malformed,
    TooLarge,
    TooMany,
    AggregateLimit,
}

impl std::fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnsupportedFormat => "Clipboard image format is not supported",
            Self::Malformed => "Clipboard image is malformed",
            Self::TooLarge => "Clipboard image exceeds the attachment limit",
            Self::TooMany => "Too many image attachments",
            Self::AggregateLimit => "Image attachments exceed the preview memory limit",
        })
    }
}

pub struct ClipboardOffer<'a> {
    pub image: Option<&'a [u8]>,
    pub text: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardPaste<'a> {
    Image(&'a [u8]),
    Text(&'a str),
    Empty,
}

impl<'a> ClipboardOffer<'a> {
    /// Images deliberately win for mixed offers; text remains untouched in the composer.
    pub fn preferred(self) -> ClipboardPaste<'a> {
        self.image
            .map(ClipboardPaste::Image)
            .or_else(|| self.text.map(ClipboardPaste::Text))
            .unwrap_or(ClipboardPaste::Empty)
    }
}

impl PendingAttachment {
    pub fn decode(
        id: AttachmentId,
        bytes: &[u8],
        limits: AttachmentLimits,
    ) -> Result<Self, AttachmentError> {
        if bytes.len() > limits.encoded_bytes_per_image {
            return Err(AttachmentError::TooLarge);
        }
        let format = image::guess_format(bytes).map_err(|_| AttachmentError::UnsupportedFormat)?;
        if !matches!(
            format,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
        ) {
            return Err(AttachmentError::UnsupportedFormat);
        }
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        let max_side = (limits.decoded_pixels_per_image as f64).sqrt().ceil() as u32;
        let mut decoder_limits = Limits::default();
        decoder_limits.max_image_width = Some(max_side);
        decoder_limits.max_image_height = Some(max_side);
        decoder_limits.max_alloc = Some(limits.aggregate_decoded_bytes as u64);
        reader.limits(decoder_limits);
        let mut decoder = reader
            .into_decoder()
            .map_err(|_| AttachmentError::Malformed)?;
        let orientation = decoder
            .orientation()
            .unwrap_or(image::metadata::Orientation::NoTransforms);
        let mut decoded =
            image::DynamicImage::from_decoder(decoder).map_err(|_| AttachmentError::Malformed)?;
        decoded.apply_orientation(orientation);
        let image = decoded.into_rgba8();
        let pixels = u64::from(image.width()) * u64::from(image.height());
        if pixels > limits.decoded_pixels_per_image {
            return Err(AttachmentError::TooLarge);
        }
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .map_err(|_| AttachmentError::Malformed)?;
        let data_url = format!("data:image/png;base64,{}", STANDARD.encode(&png));
        Ok(Self {
            id,
            width: image.width(),
            height: image.height(),
            encoded_size: png.len(),
            preview: Arc::new(image),
            turn_image: TurnImage { data_url },
        })
    }

    pub fn from_rgba(
        id: AttachmentId,
        width: u32,
        height: u32,
        rgba: &[u8],
        limits: AttachmentLimits,
    ) -> Result<Self, AttachmentError> {
        let pixels = u64::from(width) * u64::from(height);
        let expected = pixels
            .checked_mul(4)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or(AttachmentError::TooLarge)?;
        if pixels > limits.decoded_pixels_per_image || expected > limits.aggregate_decoded_bytes {
            return Err(AttachmentError::TooLarge);
        }
        let image =
            RgbaImage::from_raw(width, height, rgba.to_vec()).ok_or(AttachmentError::Malformed)?;
        Self::from_rgba_image(id, image)
    }

    fn from_rgba_image(id: AttachmentId, image: RgbaImage) -> Result<Self, AttachmentError> {
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .map_err(|_| AttachmentError::Malformed)?;
        let data_url = format!("data:image/png;base64,{}", STANDARD.encode(&png));
        Ok(Self {
            id,
            width: image.width(),
            height: image.height(),
            encoded_size: png.len(),
            preview: Arc::new(image),
            turn_image: TurnImage { data_url },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_supported_image_without_debugging_bytes() {
        let image = RgbaImage::from_pixel(2, 3, image::Rgba([1, 2, 3, 255]));
        let mut bytes = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        let attachment =
            PendingAttachment::decode(AttachmentId(7), &bytes, AttachmentLimits::default())
                .unwrap();
        assert_eq!((attachment.width, attachment.height), (2, 3));
        assert!(
            attachment
                .turn_image()
                .data_url
                .starts_with("data:image/png;base64,")
        );
        assert!(!format!("{attachment:?}").contains("AQID"));
    }

    #[test]
    fn rejects_malformed_and_oversized_inputs() {
        assert_eq!(
            PendingAttachment::decode(AttachmentId(1), b"not image", AttachmentLimits::default())
                .unwrap_err(),
            AttachmentError::UnsupportedFormat
        );
        let limits = AttachmentLimits {
            encoded_bytes_per_image: 2,
            ..AttachmentLimits::default()
        };
        assert_eq!(
            PendingAttachment::decode(AttachmentId(1), &[1, 2, 3], limits).unwrap_err(),
            AttachmentError::TooLarge
        );
    }

    #[test]
    fn mixed_clipboard_offer_prefers_image_and_text_only_stays_text() {
        assert!(matches!(
            ClipboardOffer {
                image: Some(&[1]),
                text: Some("text")
            }
            .preferred(),
            ClipboardPaste::Image(_)
        ));
        assert_eq!(
            ClipboardOffer {
                image: None,
                text: Some("世界")
            }
            .preferred(),
            ClipboardPaste::Text("世界")
        );
    }
}
