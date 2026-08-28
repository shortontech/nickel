use crate::cache::ModelBundle;
use image::{RgbImage, imageops};
use ort::{session::Session, value::Tensor};
use std::path::Path;
use thiserror::Error;

const IMAGE_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const IMAGE_STD: [f32; 3] = [0.229, 0.224, 0.225];
type EyeGazePair = ((f32, f32), (f32, f32), f32);

#[derive(Clone, Copy, Debug)]
pub struct ModelObservation {
    pub face_confidence: f32,
    pub left_eye_openness: f32,
    pub right_eye_openness: f32,
    pub horizontal: f32,
    pub vertical: f32,
    pub gaze_confidence: f32,
    pub left_gaze: (f32, f32),
    pub right_gaze: (f32, f32),
}

#[derive(Clone, Debug)]
pub struct EyePatches {
    pub left: RgbImage,
    pub right: RgbImage,
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("model role {0} is missing from the acquired bundle")]
    MissingRole(&'static str),
    #[error("failed to load or run model: {0}")]
    Ort(#[from] ort::Error),
    #[error("model output {0} has an unexpected shape")]
    UnexpectedOutput(&'static str),
}

pub struct GazeModel {
    detection: Session,
    landmarks: Session,
    gaze: Session,
}

impl GazeModel {
    pub fn load(bundle: &ModelBundle) -> Result<Self, ModelError> {
        Ok(Self {
            detection: load_role(bundle, "face_detection")?,
            landmarks: load_role(bundle, "face_landmarks_wink_optimized")?,
            gaze: load_role(bundle, "pupil_gaze")?,
        })
    }

    pub fn observe(&mut self, frame: &RgbImage) -> Result<Option<ModelObservation>, ModelError> {
        Ok(self.observe_debug(frame)?.map(|result| result.0))
    }

    pub fn observe_debug(
        &mut self,
        frame: &RgbImage,
    ) -> Result<Option<(ModelObservation, EyePatches)>, ModelError> {
        let Some(face) = self.detect_face(frame)? else {
            return Ok(None);
        };
        let landmarks = self.face_landmarks(frame, face)?;
        let right_openness = eye_aspect_ratio(&landmarks, [36, 37, 38, 39, 40, 41]);
        let left_openness = eye_aspect_ratio(&landmarks, [42, 43, 44, 45, 46, 47]);
        let ((left_gaze, right_gaze, gaze_confidence), patches) =
            self.pupil_gaze(frame, &landmarks)?;
        let horizontal = (left_gaze.0 + right_gaze.0) * 0.5;
        let vertical = (left_gaze.1 + right_gaze.1) * 0.5;
        Ok(Some((
            ModelObservation {
                face_confidence: face.confidence,
                left_eye_openness: openness_score(left_openness),
                right_eye_openness: openness_score(right_openness),
                horizontal,
                vertical,
                gaze_confidence,
                left_gaze,
                right_gaze,
            },
            patches,
        )))
    }

    fn detect_face(&mut self, frame: &RgbImage) -> Result<Option<FaceBox>, ModelError> {
        let input = image_tensor(frame, 224, 224);
        let outputs = self.detection.run(ort::inputs![input])?;
        if outputs.len() < 2 {
            return Err(ModelError::UnexpectedOutput("face detector"));
        }
        let scores = outputs[0].try_extract_tensor::<f32>()?.1;
        let pooled = outputs[1].try_extract_tensor::<f32>()?.1;
        let plane = 56 * 56;
        if scores.len() < plane * 2 || pooled.len() < plane {
            return Err(ModelError::UnexpectedOutput("face detector"));
        }
        let mut best: Option<(usize, f32)> = None;
        for index in 0..plane {
            let score = scores[index];
            if (score - pooled[index]).abs() < 1e-6
                && score >= 0.55
                && best.is_none_or(|(_, current)| score > current)
            {
                best = Some((index, score));
            }
        }
        let Some((index, confidence)) = best else {
            return Ok(None);
        };
        let row = index / 56;
        let column = index % 56;
        let radius = scores[plane + index] * 112.0;
        let x = column as f32 * 4.0 - radius;
        let y = row as f32 * 4.0 - radius;
        let scale_x = frame.width() as f32 / 224.0;
        let scale_y = frame.height() as f32 / 224.0;
        Ok(Some(FaceBox {
            x: x * scale_x,
            y: y * scale_y,
            width: radius * 2.0 * scale_x,
            height: radius * 2.0 * scale_y,
            confidence,
        }))
    }

    fn face_landmarks(
        &mut self,
        frame: &RgbImage,
        face: FaceBox,
    ) -> Result<Vec<Point>, ModelError> {
        let crop = expanded_crop(frame, face, 0.10, 0.125);
        let input = image_tensor(&crop.image, 224, 224);
        let outputs = self.landmarks.run(ort::inputs![input])?;
        if outputs.len() == 0 {
            return Err(ModelError::UnexpectedOutput("landmarks"));
        }
        let output = outputs[0].try_extract_tensor::<f32>()?.1;
        let values = output;
        let plane = 28 * 28;
        if values.len() < 198 * plane {
            return Err(ModelError::UnexpectedOutput("landmarks"));
        }
        let mut landmarks = Vec::with_capacity(66);
        for landmark in 0..66 {
            let heatmap = &values[landmark * plane..(landmark + 1) * plane];
            let (maximum, _confidence) = heatmap
                .iter()
                .copied()
                .enumerate()
                .max_by(|left, right| left.1.total_cmp(&right.1))
                .unwrap_or((0, 0.0));
            let row = maximum / 28;
            let column = maximum % 28;
            let offset_x = 27.0 * logit(values[(66 + landmark) * plane + maximum], 16.0);
            let offset_y = 27.0 * logit(values[(132 + landmark) * plane + maximum], 16.0);
            let normalized_x = (column as f32 + offset_y) / 27.0;
            let normalized_y = (row as f32 + offset_x) / 27.0;
            landmarks.push(Point {
                x: crop.x + normalized_x * crop.width,
                y: crop.y + normalized_y * crop.height,
            });
        }
        Ok(landmarks)
    }

    fn pupil_gaze(
        &mut self,
        frame: &RgbImage,
        landmarks: &[Point],
    ) -> Result<(EyeGazePair, EyePatches), ModelError> {
        let right_patch = eye_patch(frame, landmarks[36], landmarks[39], false);
        let left_patch = eye_patch(frame, landmarks[42], landmarks[45], true);
        let mut values = Vec::with_capacity(2 * 3 * 32 * 32);
        append_gaze_normalized(&right_patch, &mut values);
        append_gaze_normalized(&left_patch, &mut values);
        let input = Tensor::from_array(([2_usize, 3, 32, 32], values))?;
        let outputs = self.gaze.run(ort::inputs![input])?;
        if outputs.len() == 0 {
            return Err(ModelError::UnexpectedOutput("gaze"));
        }
        let output = outputs[0].try_extract_tensor::<f32>()?.1;
        let values = output;
        if values.len() < 2 * 3 * 8 * 8 {
            return Err(ModelError::UnexpectedOutput("gaze"));
        }
        let right = pupil_from_heatmaps(values, 0);
        let left = pupil_from_heatmaps(values, 1);
        let right_gaze = ((right.0 - 0.5) * 2.0, (right.1 - 0.5) * 2.0);
        let left_gaze = (((1.0 - left.0) - 0.5) * 2.0, (left.1 - 0.5) * 2.0);
        let gaze = (
            (left_gaze.0.clamp(-1.0, 1.0), left_gaze.1.clamp(-1.0, 1.0)),
            (right_gaze.0.clamp(-1.0, 1.0), right_gaze.1.clamp(-1.0, 1.0)),
            right.2.min(left.2),
        );
        Ok((
            gaze,
            EyePatches {
                left: left_patch,
                right: right_patch,
            },
        ))
    }
}

fn load_role(bundle: &ModelBundle, role: &'static str) -> Result<Session, ModelError> {
    let artifact = bundle
        .manifest
        .artifacts
        .iter()
        .find(|artifact| artifact.role == role)
        .ok_or(ModelError::MissingRole(role))?;
    load_model(&bundle.directory.join(&artifact.file))
}

fn load_model(path: &Path) -> Result<Session, ModelError> {
    Ok(Session::builder()?.commit_from_file(path)?)
}

#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}

#[derive(Clone, Copy)]
struct FaceBox {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    confidence: f32,
}

struct Crop {
    image: RgbImage,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn expanded_crop(frame: &RgbImage, face: FaceBox, horizontal: f32, vertical: f32) -> Crop {
    let x1 = (face.x - face.width * horizontal).clamp(0.0, frame.width() as f32 - 1.0);
    let y1 = (face.y - face.height * vertical).clamp(0.0, frame.height() as f32 - 1.0);
    let x2 = (face.x + face.width * (1.0 + horizontal)).clamp(x1 + 1.0, frame.width() as f32);
    let y2 = (face.y + face.height * (1.0 + vertical)).clamp(y1 + 1.0, frame.height() as f32);
    Crop {
        image: imageops::crop_imm(
            frame,
            x1 as u32,
            y1 as u32,
            (x2 - x1) as u32,
            (y2 - y1) as u32,
        )
        .to_image(),
        x: x1,
        y: y1,
        width: x2 - x1,
        height: y2 - y1,
    }
}

fn eye_patch(frame: &RgbImage, outer: Point, inner: Point, flip: bool) -> RgbImage {
    let center_x = (outer.x + inner.x) * 0.5;
    let center_y = (outer.y + inner.y) * 0.5;
    let delta_x = inner.x - outer.x;
    let delta_y = inner.y - outer.y;
    let distance = (delta_x.powi(2) + delta_y.powi(2)).sqrt().max(4.0);
    let axis_x = delta_x / distance;
    let axis_y = delta_y / distance;
    let perpendicular_x = -axis_y;
    let perpendicular_y = axis_x;
    let radius_x = distance * 0.70;
    let radius_y = distance * 0.60;
    let mut patch = RgbImage::new(32, 32);
    for output_y in 0..32 {
        for output_x in 0..32 {
            let normalized_x = (output_x as f32 + 0.5) / 32.0 * 2.0 - 1.0;
            let normalized_x = if flip { -normalized_x } else { normalized_x };
            let normalized_y = (output_y as f32 + 0.5) / 32.0 * 2.0 - 1.0;
            let source_x = center_x
                + axis_x * normalized_x * radius_x
                + perpendicular_x * normalized_y * radius_y;
            let source_y = center_y
                + axis_y * normalized_x * radius_x
                + perpendicular_y * normalized_y * radius_y;
            patch.put_pixel(
                output_x,
                output_y,
                bilinear_pixel(frame, source_x, source_y),
            );
        }
    }
    patch
}

fn bilinear_pixel(frame: &RgbImage, x: f32, y: f32) -> image::Rgb<u8> {
    let x = x.clamp(0.0, frame.width().saturating_sub(1) as f32);
    let y = y.clamp(0.0, frame.height().saturating_sub(1) as f32);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(frame.width() - 1);
    let y1 = (y0 + 1).min(frame.height() - 1);
    let fraction_x = x - x0 as f32;
    let fraction_y = y - y0 as f32;
    let mut result = [0_u8; 3];
    for (channel, output) in result.iter_mut().enumerate() {
        let top = frame.get_pixel(x0, y0)[channel] as f32 * (1.0 - fraction_x)
            + frame.get_pixel(x1, y0)[channel] as f32 * fraction_x;
        let bottom = frame.get_pixel(x0, y1)[channel] as f32 * (1.0 - fraction_x)
            + frame.get_pixel(x1, y1)[channel] as f32 * fraction_x;
        *output = (top * (1.0 - fraction_y) + bottom * fraction_y).round() as u8;
    }
    image::Rgb(result)
}

fn image_tensor(image: &RgbImage, width: u32, height: u32) -> Tensor<f32> {
    let resized = imageops::resize(image, width, height, imageops::FilterType::Triangle);
    let mut values = Vec::with_capacity((width * height * 3) as usize);
    append_normalized(&resized, &mut values);
    Tensor::from_array(([1_usize, 3, height as usize, width as usize], values))
        .expect("image tensor dimensions are fixed")
}

fn append_normalized(image: &RgbImage, output: &mut Vec<f32>) {
    for channel in 0..3 {
        output.extend(image.pixels().map(|pixel| {
            (pixel[channel] as f32 / 255.0 - IMAGE_MEAN[channel]) / IMAGE_STD[channel]
        }));
    }
}

// OpenSeeFace's gaze preprocessing uses NumPy transpose (N, C, X, Y), unlike
// the detector and landmark models' conventional (N, C, Y, X) tensors.
fn append_gaze_normalized(image: &RgbImage, output: &mut Vec<f32>) {
    // OpenSeeFace reverses the RGB eye crop immediately before inference, so
    // this model receives BGR channels while retaining its published channel
    // normalization constants.
    for (output_channel, source_channel) in [2, 1, 0].into_iter().enumerate() {
        for x in 0..image.width() {
            for y in 0..image.height() {
                let pixel = image.get_pixel(x, y);
                output.push(
                    (pixel[source_channel] as f32 / 255.0 - IMAGE_MEAN[output_channel])
                        / IMAGE_STD[output_channel],
                );
            }
        }
    }
}

fn pupil_from_heatmaps(values: &[f32], eye: usize) -> (f32, f32, f32) {
    let plane = 8 * 8;
    let base = eye * 3 * plane;
    let (maximum, confidence) = values[base..base + plane]
        .iter()
        .copied()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .unwrap_or((0, 0.0));
    let eye_x = maximum / 8;
    let eye_y = maximum % 8;
    let offset_x = 32.0 * logit(values[base + plane + maximum], 8.0);
    let offset_y = 32.0 * logit(values[base + plane * 2 + maximum], 8.0);
    (
        ((eye_x as f32 * 4.0 + offset_x) / 32.0).clamp(0.0, 1.0),
        ((eye_y as f32 * 4.0 + offset_y) / 32.0).clamp(0.0, 1.0),
        confidence,
    )
}

fn eye_aspect_ratio(landmarks: &[Point], indices: [usize; 6]) -> f32 {
    let horizontal = distance(landmarks[indices[0]], landmarks[indices[3]]).max(0.001);
    (distance(landmarks[indices[1]], landmarks[indices[5]])
        + distance(landmarks[indices[2]], landmarks[indices[4]]))
        / (2.0 * horizontal)
}

fn openness_score(ratio: f32) -> f32 {
    ((ratio - 0.10) / 0.20).clamp(0.0, 1.0)
}

fn distance(left: Point, right: Point) -> f32 {
    ((left.x - right.x).powi(2) + (left.y - right.y).powi(2)).sqrt()
}

fn logit(value: f32, factor: f32) -> f32 {
    let value = value.clamp(0.000_000_1, 0.999_999_9);
    (value / (1.0 - value)).ln() / factor
}

#[cfg(test)]
mod tests {
    use super::{
        Point, append_gaze_normalized, eye_aspect_ratio, eye_patch, logit, openness_score,
    };
    use image::{Rgb, RgbImage};

    #[test]
    fn aspect_ratio_distinguishes_open_and_closed_geometry() {
        let open = vec![
            Point { x: 0.0, y: 0.0 },
            Point { x: 1.0, y: -1.0 },
            Point { x: 3.0, y: -1.0 },
            Point { x: 4.0, y: 0.0 },
            Point { x: 3.0, y: 1.0 },
            Point { x: 1.0, y: 1.0 },
        ];
        let mut closed = open.clone();
        for point in &mut closed {
            point.y = 0.0;
        }
        assert!(eye_aspect_ratio(&open, [0, 1, 2, 3, 4, 5]) > 0.4);
        assert_eq!(eye_aspect_ratio(&closed, [0, 1, 2, 3, 4, 5]), 0.0);
        assert!(openness_score(0.3) > openness_score(0.1));
    }

    #[test]
    fn logit_is_finite_at_probability_edges() {
        assert!(logit(0.0, 8.0).is_finite());
        assert!(logit(1.0, 8.0).is_finite());
    }

    #[test]
    fn gaze_tensor_orders_x_before_y() {
        let mut image = RgbImage::new(2, 2);
        image.put_pixel(0, 0, Rgb([0, 0, 10]));
        image.put_pixel(1, 0, Rgb([0, 0, 20]));
        image.put_pixel(0, 1, Rgb([0, 0, 30]));
        image.put_pixel(1, 1, Rgb([0, 0, 40]));
        let mut values = Vec::new();
        append_gaze_normalized(&image, &mut values);
        let published_openseeface_values = [-1.946_656_3, -1.604_161_3, -1.775_408_9, -1.432_913_8];
        for (actual, expected) in values[..4].iter().zip(published_openseeface_values) {
            assert!((actual - expected).abs() < 0.000_001);
        }
    }

    #[test]
    fn eye_patch_aligns_a_diagonal_eye_axis_horizontally() {
        let mut image = RgbImage::new(64, 64);
        for coordinate in 12..52 {
            image.put_pixel(coordinate, coordinate, Rgb([255, 255, 255]));
        }
        let patch = eye_patch(
            &image,
            Point { x: 16.0, y: 16.0 },
            Point { x: 48.0, y: 48.0 },
            false,
        );
        let middle_row: u32 = (0..32).map(|x| u32::from(patch.get_pixel(x, 16)[0])).sum();
        let middle_column: u32 = (0..32).map(|y| u32::from(patch.get_pixel(16, y)[0])).sum();
        assert!(middle_row > middle_column * 2);
    }
}
