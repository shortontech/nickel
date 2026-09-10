//! Window capture results. Raw pixels stay out of logs and metrics.
use base64::Engine;
use image::ImageEncoder;

pub struct CapturedWindow {
    pub window_id: String,
    pub generation: u64,
    pub capture_generation: u64,
    pub submitted_at_us: u64,
    pub completed_at_us: u64,
    pub width: u16,
    pub height: u16,
    pub rgba: Vec<u8>,
}

pub struct WindowImage {
    pub window_id: String,
    pub generation: u64,
    pub capture_generation: u64,
    pub submitted_at_us: u64,
    pub completed_at_us: u64,
    pub width: u16,
    pub height: u16,
    pub mime_type: &'static str,
    pub data_base64: String,
}

impl WindowImage {
    pub fn into_mcp(self) -> rmcp::model::CallToolResult {
        self.into_mcp_resource("window_id")
    }

    pub fn into_surface_mcp(self) -> rmcp::model::CallToolResult {
        self.into_mcp_resource("surface_id")
    }

    fn into_mcp_resource(self, identity_key: &str) -> rmcp::model::CallToolResult {
        let metadata = serde_json::json!({
            (identity_key): self.window_id, "generation": self.generation,
            "capture_generation": self.capture_generation,
            "submitted_at_us": self.submitted_at_us, "completed_at_us": self.completed_at_us,
            "width": self.width, "height": self.height, "mime_type": self.mime_type,
        });
        let mut result = rmcp::model::CallToolResult::structured(metadata);
        result.content.push(rmcp::model::ContentBlock::image(
            self.data_base64,
            self.mime_type,
        ));
        result
    }
}

impl CapturedWindow {
    pub fn encode(self) -> Result<WindowImage, String> {
        let pixels = usize::from(self.width) * usize::from(self.height);
        if pixels == 0 || pixels > 16_777_216 || self.rgba.len() != pixels * 4 {
            return Err("invalid capture pixel dimensions".into());
        }
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        )
        .write_image(
            &self.rgba,
            u32::from(self.width),
            u32::from(self.height),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("capture encoding failed: {error}"))?;
        Ok(WindowImage {
            window_id: self.window_id,
            generation: self.generation,
            capture_generation: self.capture_generation,
            submitted_at_us: self.submitted_at_us,
            completed_at_us: self.completed_at_us,
            width: self.width,
            height: self.height,
            mime_type: "image/png",
            data_base64: base64::engine::general_purpose::STANDARD.encode(png),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(rgba: Vec<u8>) -> CapturedWindow {
        CapturedWindow {
            window_id: "1".into(),
            generation: 1,
            capture_generation: 9,
            submitted_at_us: 100,
            completed_at_us: 120,
            width: 2,
            height: 1,
            rgba,
        }
    }
    #[test]
    fn png_preserves_pixels_and_capture_times_and_rejects_incomplete_buffers() {
        let pixels = vec![255, 0, 0, 255, 0, 100, 255, 128];
        let result = frame(pixels.clone()).encode().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(result.data_base64)
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
        assert_eq!(decoded.as_raw(), &pixels);
        assert_eq!(
            (
                result.submitted_at_us,
                result.completed_at_us,
                result.capture_generation
            ),
            (100, 120, 9)
        );
        let response = frame(pixels).encode().unwrap().into_mcp();
        assert_eq!(
            response
                .content
                .iter()
                .filter(|content| content.as_image().is_some())
                .count(),
            1
        );
        assert!(
            response
                .structured_content
                .as_ref()
                .unwrap()
                .get("data_base64")
                .is_none()
        );
        assert_eq!(response.structured_content.as_ref().unwrap()["width"], 2);
        assert!(frame(vec![0; 7]).encode().is_err());
        assert!(frame(vec![0; 9]).encode().is_err());
    }
}
