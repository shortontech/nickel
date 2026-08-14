use thiserror::Error;

#[derive(Clone, Debug)]
pub struct CameraDescriptor {
    pub index: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Error)]
pub enum CameraError {
    #[error("camera enumeration failed: {0}")]
    Enumeration(String),
    #[error("camera {0} was not found")]
    NotFound(String),
    #[error("camera initialization failed: {0}")]
    Initialization(String),
    #[error("camera frame failed: {0}")]
    Frame(String),
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{CameraDescriptor, CameraError};
    use image::RgbImage;
    use std::{
        fs,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc::{self, Receiver, SyncSender},
        },
        thread,
    };
    use v4l::{
        Device, Format, FourCC,
        buffer::Type,
        io::{mmap::Stream as MmapStream, traits::CaptureStream},
        video::Capture,
    };

    pub struct CameraSource {
        receiver: Receiver<Result<RgbImage, CameraError>>,
        running: Arc<AtomicBool>,
        description: String,
    }

    impl CameraSource {
        pub fn enumerate() -> Result<Vec<CameraDescriptor>, CameraError> {
            let directory = fs::read_dir("/sys/class/video4linux")
                .map_err(|error| CameraError::Enumeration(error.to_string()))?;
            let mut cameras = Vec::new();
            for entry in directory {
                let entry = entry.map_err(|error| CameraError::Enumeration(error.to_string()))?;
                let node = entry.file_name().to_string_lossy().into_owned();
                let Some(index) = node.strip_prefix("video") else {
                    continue;
                };
                let name = fs::read_to_string(entry.path().join("name"))
                    .unwrap_or_else(|_| "Unknown camera".to_owned())
                    .trim()
                    .to_owned();
                cameras.push(CameraDescriptor {
                    index: index.to_owned(),
                    name,
                    description: format!("Video4Linux device /dev/{node}"),
                });
            }
            cameras.sort_by_key(|camera| camera.index.parse::<u32>().unwrap_or(u32::MAX));
            Ok(cameras)
        }

        pub fn open(selector: Option<&str>) -> Result<Self, CameraError> {
            let cameras = Self::enumerate()?;
            let descriptor = choose_camera(&cameras, selector)?.clone();
            let path = PathBuf::from(format!("/dev/video{}", descriptor.index));
            let (sender, receiver) = mpsc::sync_channel(1);
            let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
            let running = Arc::new(AtomicBool::new(true));
            let thread_running = Arc::clone(&running);
            thread::Builder::new()
                .name("nickel-gaze-camera".to_owned())
                .spawn(move || capture(path, sender, ready_sender, thread_running))
                .map_err(|error| CameraError::Initialization(error.to_string()))?;
            let description = ready_receiver
                .recv()
                .map_err(|error| CameraError::Initialization(error.to_string()))??;
            Ok(Self {
                receiver,
                running,
                description: format!("{} [{}] {description}", descriptor.name, descriptor.index),
            })
        }

        pub fn description(&self) -> &str {
            &self.description
        }

        pub fn frame(&mut self) -> Result<RgbImage, CameraError> {
            self.receiver
                .recv()
                .map_err(|error| CameraError::Frame(error.to_string()))?
        }
    }

    impl Drop for CameraSource {
        fn drop(&mut self) {
            self.running.store(false, Ordering::SeqCst);
        }
    }

    fn capture(
        path: PathBuf,
        sender: SyncSender<Result<RgbImage, CameraError>>,
        ready: SyncSender<Result<String, CameraError>>,
        running: Arc<AtomicBool>,
    ) {
        let result = (|| -> Result<(), CameraError> {
            let device = Device::with_path(&path)
                .map_err(|error| CameraError::Initialization(error.to_string()))?;
            let requested = Format::new(1920, 1080, FourCC::new(b"UYVY"));
            let format = device
                .set_format(&requested)
                .map_err(|error| CameraError::Initialization(error.to_string()))?;
            if format.fourcc != FourCC::new(b"UYVY") {
                return Err(CameraError::Initialization(format!(
                    "{} negotiated unsupported pixel format {}",
                    path.display(),
                    format.fourcc
                )));
            }
            let mut stream = MmapStream::with_buffers(&device, Type::VideoCapture, 4)
                .map_err(|error| CameraError::Initialization(error.to_string()))?;
            let _ = ready.send(Ok(format!(
                "{}x{} {}",
                format.width, format.height, format.fourcc
            )));
            while running.load(Ordering::SeqCst) {
                let (bytes, _) = stream
                    .next()
                    .map_err(|error| CameraError::Frame(error.to_string()))?;
                let image = decode_uyvy(bytes, format.width, format.height)?;
                if sender.send(Ok(image)).is_err() {
                    break;
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            let message = error.to_string();
            let _ = ready.send(Err(CameraError::Initialization(message.clone())));
            let _ = sender.send(Err(CameraError::Frame(message)));
        }
    }

    fn decode_uyvy(bytes: &[u8], width: u32, height: u32) -> Result<RgbImage, CameraError> {
        let expected = width as usize * height as usize * 2;
        if bytes.len() < expected {
            return Err(CameraError::Frame(format!(
                "UYVY frame has {} bytes; expected at least {expected}",
                bytes.len()
            )));
        }
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for group in bytes[..expected].chunks_exact(4) {
            let u = i32::from(group[0]) - 128;
            let y0 = i32::from(group[1]);
            let v = i32::from(group[2]) - 128;
            let y1 = i32::from(group[3]);
            rgb.extend_from_slice(&yuv_to_rgb(y0, u, v));
            rgb.extend_from_slice(&yuv_to_rgb(y1, u, v));
        }
        RgbImage::from_raw(width, height, rgb)
            .ok_or_else(|| CameraError::Frame("decoded RGB dimensions are invalid".to_owned()))
    }

    fn yuv_to_rgb(y: i32, u: i32, v: i32) -> [u8; 3] {
        let red = y + ((359 * v) >> 8);
        let green = y - ((88 * u + 183 * v) >> 8);
        let blue = y + ((454 * u) >> 8);
        [
            red.clamp(0, 255) as u8,
            green.clamp(0, 255) as u8,
            blue.clamp(0, 255) as u8,
        ]
    }

    fn choose_camera<'a>(
        cameras: &'a [CameraDescriptor],
        selector: Option<&str>,
    ) -> Result<&'a CameraDescriptor, CameraError> {
        match selector {
            None => cameras
                .first()
                .ok_or_else(|| CameraError::NotFound("default".to_owned())),
            Some(selector) => cameras
                .iter()
                .find(|camera| {
                    camera.index == selector
                        || camera
                            .name
                            .to_ascii_lowercase()
                            .contains(&selector.to_ascii_lowercase())
                })
                .ok_or_else(|| CameraError::NotFound(selector.to_owned())),
        }
    }

    pub use CameraSource as Source;

    #[cfg(test)]
    mod tests {
        use super::{decode_uyvy, yuv_to_rgb};

        #[test]
        fn neutral_chroma_produces_grayscale() {
            assert_eq!(yuv_to_rgb(80, 0, 0), [80, 80, 80]);
        }

        #[test]
        fn decodes_two_pixel_uyvy_group() {
            let image = decode_uyvy(&[128, 40, 128, 90], 2, 1).expect("frame should decode");
            assert_eq!(image.as_raw(), &[40, 40, 40, 90, 90, 90]);
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::{CameraDescriptor, CameraError};
    use image::RgbImage;
    use nokhwa::{
        Camera,
        pixel_format::RgbFormat,
        query,
        utils::{ApiBackend, RequestedFormat, RequestedFormatType},
    };

    pub struct CameraSource {
        camera: Camera,
        description: String,
    }

    impl CameraSource {
        pub fn enumerate() -> Result<Vec<CameraDescriptor>, CameraError> {
            query(ApiBackend::Auto)
                .map_err(|error| CameraError::Enumeration(error.to_string()))
                .map(|cameras| {
                    cameras
                        .into_iter()
                        .map(|camera| CameraDescriptor {
                            index: camera.index().to_string(),
                            name: camera.human_name(),
                            description: camera.description().to_owned(),
                        })
                        .collect()
                })
        }

        pub fn open(selector: Option<&str>) -> Result<Self, CameraError> {
            let cameras = query(ApiBackend::Auto)
                .map_err(|error| CameraError::Enumeration(error.to_string()))?;
            let camera = match selector {
                None => cameras.first(),
                Some(selector) => cameras.iter().find(|camera| {
                    camera.index().to_string() == selector
                        || camera
                            .human_name()
                            .to_ascii_lowercase()
                            .contains(&selector.to_ascii_lowercase())
                }),
            }
            .ok_or_else(|| CameraError::NotFound(selector.unwrap_or("default").to_owned()))?;
            let request =
                RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestResolution);
            let mut source = Camera::new(camera.index().clone(), request)
                .map_err(|error| CameraError::Initialization(error.to_string()))?;
            source
                .open_stream()
                .map_err(|error| CameraError::Initialization(error.to_string()))?;
            let format = source.camera_format();
            Ok(Self {
                camera: source,
                description: format!("{} [{}] {}", camera.human_name(), camera.index(), format),
            })
        }

        pub fn description(&self) -> &str {
            &self.description
        }

        pub fn frame(&mut self) -> Result<RgbImage, CameraError> {
            self.camera
                .frame()
                .and_then(|frame| frame.decode_image::<RgbFormat>())
                .map_err(|error| CameraError::Frame(error.to_string()))
        }
    }

    pub use CameraSource as Source;
}

pub use platform::Source as CameraSource;
