use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, bounded};
use image::{DynamicImage, RgbaImage};
use nokhwa::{
    Camera,
    pixel_format::RgbFormat,
    query,
    utils::{
        ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
        Resolution,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDevice {
    pub index: u32,
    pub name: String,
}

pub fn devices() -> Result<Vec<CameraDevice>, String> {
    query(ApiBackend::MediaFoundation)
        .map_err(|error| error.to_string())
        .map(|items| {
            items
                .into_iter()
                .filter_map(|info| {
                    Some(CameraDevice {
                        index: info.index().as_index().ok()?,
                        name: info.human_name(),
                    })
                })
                .collect()
        })
}

#[derive(Debug)]
pub enum CameraEvent {
    Frame(RgbaImage),
    Error(String),
}

pub struct CameraWorker {
    stop: Arc<AtomicBool>,
    receiver: Receiver<CameraEvent>,
    thread: Option<JoinHandle<()>>,
}

impl CameraWorker {
    pub fn start(index: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = bounded(2);
        let thread_stop = stop.clone();
        let thread = thread::spawn(move || run_camera(index, thread_stop, sender));
        Self {
            stop,
            receiver,
            thread: Some(thread),
        }
    }

    pub fn latest(&self) -> Option<CameraEvent> {
        let mut latest = None;
        while let Ok(event) = self.receiver.try_recv() {
            latest = Some(event);
        }
        latest
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Some drivers block in frame(); never freeze the UI waiting indefinitely.
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

impl Drop for CameraWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn send_latest(sender: &Sender<CameraEvent>, event: CameraEvent) {
    if sender.try_send(event).is_err() {
        // A full bounded channel deliberately drops intermediate camera frames.
    }
}

fn run_camera(index: u32, stop: Arc<AtomicBool>, sender: Sender<CameraEvent>) {
    let wanted = CameraFormat::new(Resolution::new(1920, 1080), FrameFormat::MJPEG, 30);
    let request = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(wanted));
    let mut camera = match Camera::new(CameraIndex::Index(index), request) {
        Ok(camera) => camera,
        Err(error) => {
            send_latest(&sender, CameraEvent::Error(error.to_string()));
            return;
        }
    };
    if let Err(error) = camera.open_stream() {
        send_latest(&sender, CameraEvent::Error(error.to_string()));
        return;
    }
    while !stop.load(Ordering::Acquire) {
        match camera
            .frame()
            .and_then(|frame| frame.decode_image::<RgbFormat>())
        {
            Ok(rgb) => send_latest(
                &sender,
                CameraEvent::Frame(DynamicImage::ImageRgb8(rgb).to_rgba8()),
            ),
            Err(error) => {
                send_latest(&sender, CameraEvent::Error(error.to_string()));
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    let _ = camera.stop_stream();
}
