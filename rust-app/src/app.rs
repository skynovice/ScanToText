use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use arboard::Clipboard;
use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui::{
    self, Color32, ColorImage, FontData, FontDefinitions, FontFamily, Pos2, Rect, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Vec2, ViewportCommand,
};
use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::{Code, HotKey, Modifiers},
};
use image::RgbaImage;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{
        ContextMenu, Icon as MenuIcon, IconMenuItem, Menu, MenuEvent, MenuId, PredefinedMenuItem,
    },
};

use crate::{
    APP_NAME, APP_UPDATED_DATE, APP_VERSION,
    camera::{self, CameraDevice, CameraEvent, CameraWorker},
    config::{self, Config},
    history,
    image_ops::{self, Roi},
    ocr, platform,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureKind {
    Document,
    Translate,
    OcrPaste,
}

#[derive(Debug, Clone, Copy)]
enum LauncherAction {
    Show,
    Welcome,
    Capture,
    OcrClipboard,
    Translate,
    History,
    Settings,
    Exit,
    SavePosition([i32; 2]),
}

#[derive(Debug, Clone, Copy)]
enum OverlayAction {
    Cancel,
    Finish,
}

struct CaptureSession {
    kind: CaptureKind,
    image: RgbaImage,
    geometry: [i32; 4],
    texture: TextureHandle,
    scope: Option<Roi>,
    crops: Vec<Roi>,
    drag_start: Option<Pos2>,
}

#[derive(Debug, Clone, Copy)]
enum ConfirmAction {
    ResumeLive,
    Rotate(bool),
    DeleteRoi(usize),
    ClearRois,
    DeleteHistory,
}

struct ConfirmDialog {
    title: String,
    text: String,
    action: ConfirmAction,
}

#[derive(Clone)]
struct DocumentBackup {
    image: Option<RgbaImage>,
    rois: Vec<Roi>,
    history_path: Option<PathBuf>,
    live: bool,
    main_visible: bool,
}

enum WorkerResult {
    Batch {
        id: u64,
        values: Result<Vec<String>, String>,
    },
    Capture {
        kind: CaptureKind,
        value: Result<(RgbaImage, [i32; 4]), String>,
    },
    Translation {
        id: u64,
        value: Result<(String, String), String>,
    },
    OcrPaste {
        id: u64,
        value: Result<String, String>,
    },
    PasteApplied(Result<(), String>),
}

#[derive(Default)]
struct TrayActions {
    show: MenuId,
    welcome: MenuId,
    capture: MenuId,
    ocr_clipboard: MenuId,
    translate: MenuId,
    history: MenuId,
    settings: MenuId,
    exit: MenuId,
}

struct UiIcons {
    add: TextureHandle,
    camera: TextureHandle,
    history: TextureHandle,
    rotate_left: TextureHandle,
    rotate_right: TextureHandle,
    delete_sweep: TextureHandle,
    info: TextureHandle,
    settings: TextureHandle,
    copy: TextureHandle,
    close: TextureHandle,
    delete: TextureHandle,
    check: TextureHandle,
    capture: TextureHandle,
    chevron_left: TextureHandle,
    chevron_right: TextureHandle,
}

impl UiIcons {
    fn load(ctx: &egui::Context) -> Self {
        Self {
            add: load_bundled_texture(
                ctx,
                "icon-add",
                include_bytes!("../../assets/icons/add.png"),
            ),
            camera: camera_icon_texture(ctx),
            history: load_bundled_texture(
                ctx,
                "icon-history",
                include_bytes!("../../assets/icons/history.png"),
            ),
            rotate_left: load_bundled_texture(
                ctx,
                "icon-rotate-left",
                include_bytes!("../../assets/icons/rotate_left.png"),
            ),
            rotate_right: load_bundled_texture(
                ctx,
                "icon-rotate-right",
                include_bytes!("../../assets/icons/rotate_right.png"),
            ),
            delete_sweep: load_bundled_texture(
                ctx,
                "icon-delete-sweep",
                include_bytes!("../../assets/icons/delete_sweep.png"),
            ),
            info: load_bundled_texture(
                ctx,
                "icon-info",
                include_bytes!("../../assets/icons/info.png"),
            ),
            settings: load_bundled_texture(
                ctx,
                "icon-settings",
                include_bytes!("../../assets/icons/settings.png"),
            ),
            copy: load_bundled_texture(
                ctx,
                "icon-copy",
                include_bytes!("../../assets/icons/copy.png"),
            ),
            close: load_bundled_texture(
                ctx,
                "icon-close",
                include_bytes!("../../assets/icons/close.png"),
            ),
            delete: load_bundled_texture(
                ctx,
                "icon-delete",
                include_bytes!("../../assets/icons/delete.png"),
            ),
            check: load_bundled_texture(
                ctx,
                "icon-check",
                include_bytes!("../../assets/icons/check.png"),
            ),
            capture: load_bundled_texture(
                ctx,
                "icon-capture",
                include_bytes!("../../assets/icons/capture.png"),
            ),
            chevron_left: load_bundled_texture(
                ctx,
                "icon-chevron-left",
                include_bytes!("../../assets/icons/chevron_left.png"),
            ),
            chevron_right: load_bundled_texture(
                ctx,
                "icon-chevron-right",
                include_bytes!("../../assets/icons/chevron_right.png"),
            ),
        }
    }
}

pub struct RoiOcrApp {
    config: Config,
    config_path: PathBuf,
    history_dir: PathBuf,
    cameras: Vec<CameraDevice>,
    selected_camera: u32,
    camera: Option<CameraWorker>,
    live_frame: Option<RgbaImage>,
    document: Option<RgbaImage>,
    texture: Option<TextureHandle>,
    rois: Vec<Roi>,
    current_history: Option<PathBuf>,
    drag_start: Option<Pos2>,
    status: String,
    settings_open: bool,
    history_open: bool,
    about_open: bool,
    batch_id: u64,
    batch_running: Option<u64>,
    translation_id: u64,
    ocr_paste_id: u64,
    ocr_paste_running: bool,
    ocr_paste_restore_main: Option<bool>,
    ocr_paste_click_position: Option<[i32; 2]>,
    ocr_paste_success_until: Option<Instant>,
    worker_tx: Sender<WorkerResult>,
    worker_rx: Receiver<WorkerResult>,
    pending_capture: Option<CaptureKind>,
    capture_session: Option<CaptureSession>,
    backup: Option<DocumentBackup>,
    translation_text: Option<(String, String)>,
    translation_open: bool,
    translation_position: Option<[f32; 2]>,
    welcome_open: bool,
    main_visible: bool,
    launcher_initialized: bool,
    launcher_native_styled: bool,
    launcher_hwnd: Option<isize>,
    launcher_capture_position: Option<[i32; 2]>,
    launcher_feedback: Option<platform::LauncherFeedback>,
    main_always_on_top: bool,
    camera_panel_visible: bool,
    confirm: Option<ConfirmDialog>,
    settings_draft: Config,
    settings_show_keys: bool,
    history_selection: HashSet<PathBuf>,
    history_current: Option<PathBuf>,
    history_thumbnails: HashMap<PathBuf, TextureHandle>,
    roi_previews: Vec<TextureHandle>,
    app_icon_texture: TextureHandle,
    ui_icons: UiIcons,
    window_icon: Arc<egui::IconData>,
    _hotkey_manager: Option<GlobalHotKeyManager>,
    hotkey_actions: HashMap<u32, String>,
    _tray: Option<TrayIcon>,
    tray_actions: TrayActions,
    launcher_menu: Menu,
    launcher_actions: TrayActions,
    exit_requested: bool,
}

impl RoiOcrApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_multilingual_fonts(&cc.egui_ctx);
        let icon_image = image::load_from_memory(include_bytes!("../../assets/app_icon.png"))
            .expect("bundled application icon must be a valid image")
            .to_rgba8();
        let app_icon_texture = cc.egui_ctx.load_texture(
            "app-icon",
            ColorImage::from_rgba_unmultiplied(
                [icon_image.width() as usize, icon_image.height() as usize],
                icon_image.as_raw(),
            ),
            TextureOptions::LINEAR,
        );
        let ui_icons = UiIcons::load(&cc.egui_ctx);
        let window_icon = Arc::new(egui::IconData {
            rgba: icon_image.into_raw(),
            width: app_icon_texture.size()[0] as u32,
            height: app_icon_texture.size()[1] as u32,
        });
        let config_path = config::config_path();
        let config = config::load(&config_path);
        let settings_draft = config.clone();
        apply_main_font_size(&cc.egui_ctx, config.main_font_size);
        let history_dir = config::history_dir();
        // ponytail: defer camera enumeration and streaming until the user opens the camera.
        let cameras = Vec::new();
        let selected_camera = config.camera;
        let camera = None;
        let (worker_tx, worker_rx) = unbounded();
        let (hotkey_manager, hotkey_actions, hotkey_error) = setup_hotkeys();
        let (tray, tray_actions) = setup_tray();
        let (launcher_menu, launcher_actions) = build_action_menu();
        let status = hotkey_error.unwrap_or_else(|| "เลือกแหล่งภาพเพื่อเริ่มแปลงข้อความ".into());
        Self {
            config,
            config_path,
            history_dir,
            cameras,
            selected_camera,
            camera,
            live_frame: None,
            document: None,
            texture: None,
            rois: Vec::new(),
            current_history: None,
            drag_start: None,
            status,
            settings_open: false,
            history_open: false,
            about_open: false,
            batch_id: 0,
            batch_running: None,
            translation_id: 0,
            ocr_paste_id: 0,
            ocr_paste_running: false,
            ocr_paste_restore_main: None,
            ocr_paste_click_position: None,
            ocr_paste_success_until: None,
            worker_tx,
            worker_rx,
            pending_capture: None,
            capture_session: None,
            backup: None,
            translation_text: None,
            translation_open: false,
            translation_position: None,
            welcome_open: false,
            main_visible: false,
            launcher_initialized: false,
            launcher_native_styled: false,
            launcher_hwnd: None,
            launcher_capture_position: None,
            launcher_feedback: None,
            main_always_on_top: false,
            camera_panel_visible: true,
            confirm: None,
            settings_draft,
            settings_show_keys: false,
            history_selection: HashSet::new(),
            history_current: None,
            history_thumbnails: HashMap::new(),
            roi_previews: Vec::new(),
            app_icon_texture,
            ui_icons,
            window_icon,
            _hotkey_manager: hotkey_manager,
            hotkey_actions,
            _tray: tray,
            tray_actions,
            launcher_menu,
            launcher_actions,
            exit_requested: false,
        }
    }

    fn displayed_image(&self) -> Option<&RgbaImage> {
        if self.document.is_some() {
            self.document.as_ref()
        } else {
            self.live_frame.as_ref()
        }
    }

    fn refresh_texture(&mut self, ctx: &egui::Context) {
        let Some(image) = self.displayed_image() else {
            return;
        };
        let color = ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        if let Some(texture) = &mut self.texture {
            texture.set(color, TextureOptions::LINEAR);
        } else {
            self.texture = Some(ctx.load_texture("document", color, TextureOptions::LINEAR));
        }
    }

    fn load_texture(
        ctx: &egui::Context,
        name: impl Into<String>,
        image: &RgbaImage,
    ) -> TextureHandle {
        ctx.load_texture(
            name,
            ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ),
            TextureOptions::LINEAR,
        )
    }

    fn rebuild_roi_previews(&mut self, ctx: &egui::Context) {
        self.roi_previews.clear();
        let Some(image) = &self.document else { return };
        for (index, roi) in self.rois.iter().enumerate() {
            if let Some(crop) = image_ops::crop(image, roi) {
                self.roi_previews.push(Self::load_texture(
                    ctx,
                    format!("roi-preview-{}-{index}", self.batch_id),
                    &crop,
                ));
            }
        }
    }

    fn poll_camera(&mut self, ctx: &egui::Context) {
        if self.document.is_some()
            || self.pending_capture.is_some()
            || self.capture_session.is_some()
        {
            return;
        }
        let event = self.camera.as_ref().and_then(CameraWorker::latest);
        if let Some(event) = event {
            match event {
                CameraEvent::Frame(frame) => {
                    self.live_frame = Some(frame);
                    self.refresh_texture(ctx);
                }
                CameraEvent::Error(error) => {
                    self.camera = None;
                    self.live_frame = None;
                    if self.document.is_none() {
                        self.texture = None;
                    }
                    self.status = format!("เปิดกล้องไม่สำเร็จ: {error}");
                }
            }
        }
    }

    fn toggle_camera(&mut self) {
        if let Some(mut camera) = self.camera.take() {
            camera.stop();
            if self.document.is_none() {
                self.live_frame = None;
                self.texture = None;
            }
            self.status = "ปิดกล้องแล้ว · ภาพที่ถ่ายไว้และผล OCR ยังคงอยู่".into();
            return;
        }

        self.cameras = match camera::devices() {
            Ok(cameras) => cameras,
            Err(error) => {
                self.status = format!("ค้นหากล้องไม่สำเร็จ: {error}");
                return;
            }
        };
        if self.cameras.is_empty() {
            self.status = "ไม่พบกล้องที่เชื่อมต่ออยู่".into();
            return;
        }
        if !self
            .cameras
            .iter()
            .any(|camera| camera.index == self.selected_camera)
        {
            self.selected_camera = self.cameras[0].index;
        }
        self.live_frame = None;
        if self.document.is_none() {
            self.texture = None;
        }
        self.camera = Some(CameraWorker::start(self.selected_camera));
        self.config.camera = self.selected_camera;
        self.save_config();
        self.status = "กำลังเปิดกล้อง…".into();
    }

    fn restart_camera(&mut self) {
        if let Some(mut camera) = self.camera.take() {
            camera.stop();
        }
        self.camera = Some(CameraWorker::start(self.selected_camera));
        self.live_frame = None;
        if self.document.is_none() {
            self.texture = None;
        }
        self.config.camera = self.selected_camera;
        self.save_config();
        self.status = "กำลังเปิดกล้อง…".into();
    }

    fn save_config(&mut self) {
        if let Err(error) = config::save(&self.config, &self.config_path) {
            self.status = format!("บันทึก Settings ไม่สำเร็จ: {error}");
        }
    }

    fn set_document(&mut self, image: RgbaImage, path: Option<PathBuf>, ctx: &egui::Context) {
        self.batch_id += 1;
        self.batch_running = None;
        self.document = Some(image);
        self.current_history = path;
        self.rois.clear();
        self.roi_previews.clear();
        self.texture = None;
        self.refresh_texture(ctx);
    }

    fn capture_photo(&mut self, ctx: &egui::Context) {
        let Some(frame) = self.live_frame.clone() else {
            self.status = "ยังไม่มีภาพจากกล้อง".into();
            return;
        };
        match history::save_image(&frame, &self.history_dir) {
            Ok(path) => {
                self.set_document(frame, Some(path), ctx);
                self.persist_state();
                self.status = format!(
                    "บันทึก History: {} · ลากกรอบที่ต้องการ",
                    self.current_history
                        .as_ref()
                        .map_or_else(String::new, |path| history::display_name(path))
                );
            }
            Err(error) => self.status = format!("บันทึก History ไม่สำเร็จ: {error}"),
        }
    }

    fn resume_live_now(&mut self) {
        self.batch_id += 1;
        self.batch_running = None;
        self.document = None;
        self.current_history = None;
        self.rois.clear();
        self.roi_previews.clear();
        self.texture = None;
        self.status = if self.camera.is_some() {
            "ภาพสด · คลิกที่ภาพ หรือกด Spacebar เพื่อถ่ายภาพ".into()
        } else {
            "เลือกแหล่งภาพเพื่อเริ่มแปลงข้อความ".into()
        };
    }

    fn persist_state(&mut self) {
        if let Some(path) = &self.current_history
            && let Err(error) = history::save_state(path, &self.rois)
        {
            self.status = format!("บันทึก ROI ไม่สำเร็จ: {error}");
        }
    }

    fn add_roi(&mut self, roi: Roi, ctx: &egui::Context) {
        let Some(image) = &self.document else { return };
        if let Some(roi) = roi.clipped(image) {
            self.rois.push(roi);
            self.rebuild_roi_previews(ctx);
            self.persist_state();
            self.status = format!(
                "เลือกแล้ว {} กรอบ · ลากเพิ่มหรือกดแปลงภาพเป็นข้อความ",
                self.rois.len()
            );
        }
    }

    fn delete_roi_now(&mut self, index: usize, ctx: &egui::Context) {
        if index < self.rois.len() {
            self.rois.remove(index);
            self.batch_id += 1;
            self.batch_running = None;
            self.rebuild_roi_previews(ctx);
            self.persist_state();
        }
    }

    fn rotate_now(&mut self, clockwise: bool, ctx: &egui::Context) {
        let Some(image) = self.document.as_ref() else {
            self.status = "กรุณาถ่ายหรือเปิดภาพก่อนหมุน".into();
            return;
        };
        let rotated = image_ops::rotate(image, clockwise);
        if let Some(path) = &self.current_history
            && let Err(error) = history::replace_image(&rotated, path)
        {
            self.status = format!("บันทึกภาพที่หมุนไม่สำเร็จ: {error}");
            return;
        }
        self.set_document(rotated, self.current_history.clone(), ctx);
        self.persist_state();
        self.status = "หมุนภาพและบันทึกใน History แล้ว".into();
    }

    fn convert_all(&mut self) {
        if self.batch_running.is_some() {
            return;
        }
        let Some(image) = self.document.clone() else {
            self.status = "กรุณาถ่ายภาพก่อน".into();
            return;
        };
        if self.rois.is_empty() {
            self.status = "ลากกรอบข้อความอย่างน้อยหนึ่งกรอบ".into();
            return;
        }
        let key_check = if self.config.engine == "gemini" {
            config::validate_key("Gemini", &self.config.gemini_key)
        } else {
            config::validate_key("OCR.space", &self.config.ocrspace_key)
        };
        if let Err(error) = key_check {
            self.status = format!("{error} → เปิด Settings");
            return;
        }
        let crops: Option<Vec<_>> = self
            .rois
            .iter()
            .map(|roi| image_ops::crop(&image, roi))
            .collect();
        let Some(crops) = crops else {
            self.status = "พบ ROI อยู่นอกภาพ".into();
            return;
        };
        for roi in &mut self.rois {
            roi.text.clear();
        }
        self.batch_id += 1;
        let id = self.batch_id;
        self.batch_running = Some(id);
        let config = self.config.clone();
        let sender = self.worker_tx.clone();
        self.status = format!("กำลังแปลง {} กรอบ…", crops.len());
        thread::spawn(move || {
            let values = (|| {
                let (composite, rows) = image_ops::compose_batch(&crops, 1400)?;
                let (jpeg, rows) = image_ops::encode_jpeg_under_limit(&composite, &rows, 950_000)?;
                ocr::ocr_batch(&jpeg, &config, &rows).map_err(|error| error.to_string())
            })();
            let _ = sender.send(WorkerResult::Batch { id, values });
        });
    }

    fn begin_capture(&mut self, kind: CaptureKind, ctx: &egui::Context) {
        if self.pending_capture.is_some() || self.capture_session.is_some() {
            return;
        }
        self.backup = Some(DocumentBackup {
            image: self.document.clone(),
            rois: self.rois.clone(),
            history_path: self.current_history.clone(),
            live: self.document.is_none(),
            main_visible: self.main_visible,
        });
        self.pending_capture = Some(kind);
        self.main_visible = false;
        self.welcome_open = false;
        self.set_launcher_visible(false);
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(100));
            let value = platform::capture_virtual_desktop();
            let _ = sender.send(WorkerResult::Capture { kind, value });
        });
        ctx.request_repaint();
    }

    fn begin_ocr_paste_capture(&mut self, ctx: &egui::Context, click_position: [i32; 2]) {
        self.begin_ocr_capture(ctx, Some(click_position));
    }

    fn begin_ocr_clipboard_capture(&mut self, ctx: &egui::Context) {
        self.begin_ocr_capture(ctx, None);
    }

    fn begin_ocr_capture(&mut self, ctx: &egui::Context, click_position: Option<[i32; 2]>) {
        if self.ocr_paste_running
            || self.pending_capture.is_some()
            || self.capture_session.is_some()
        {
            return;
        }
        // ponytail: only the keyboard shortcut supplies a click target; menu capture is copy-only.
        self.ocr_paste_click_position = click_position;
        self.ocr_paste_success_until = None;
        if let Some(feedback) = &mut self.launcher_feedback {
            feedback.hide();
        }
        self.ocr_paste_restore_main = None;
        self.begin_capture(CaptureKind::OcrPaste, ctx);
    }

    fn restore_backup(&mut self, ctx: &egui::Context) {
        if let Some(backup) = self.backup.take() {
            self.document = backup.image;
            self.rois = backup.rois;
            self.current_history = backup.history_path;
            if backup.live {
                self.document = None;
            }
            self.main_visible = backup.main_visible;
            self.texture = None;
            self.refresh_texture(ctx);
        }
    }

    fn restore_ocr_paste_visibility(&mut self) {
        if let Some(visible) = self.ocr_paste_restore_main.take()
            && !self.main_visible
            && self.pending_capture.is_none()
            && self.capture_session.is_none()
            && self.backup.is_none()
            && !self.translation_open
        {
            self.main_visible = visible;
        }
    }

    fn start_translation(&mut self, image: RgbaImage, ctx: &egui::Context) {
        self.restore_backup(ctx);
        self.main_visible = false;
        self.translation_text = Some(("กำลังอ่าน Text…".into(), "กำลังแปลภาษาไทย…".into()));
        self.translation_open = true;
        self.translation_position = self
            .launcher_feedback
            .as_ref()
            .and_then(|feedback| feedback.popup_position(420, 300))
            .map(|[x, y]| [x as f32, y as f32])
            .or_else(|| {
                platform::cursor_position().map(|[x, y]| [(x - 210) as f32, (y - 150) as f32])
            });
        self.translation_id += 1;
        let id = self.translation_id;
        let config = self.config.clone();
        let sender = self.worker_tx.clone();
        thread::spawn(move || {
            let result = image_ops::encode_jpeg(&image, 95)
                .map_err(|error| error.to_string())
                .and_then(|jpeg| {
                    ocr::translate_image(&jpeg, &config).map_err(|error| error.to_string())
                });
            let _ = sender.send(WorkerResult::Translation { id, value: result });
        });
    }

    fn poll_workers(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.worker_rx.try_recv() {
            match event {
                WorkerResult::Batch { id, values } if id == self.batch_id => {
                    self.batch_running = None;
                    match values {
                        Ok(values) => {
                            for (roi, text) in self.rois.iter_mut().zip(values) {
                                roi.text = text;
                            }
                            self.status = format!("สำเร็จ: แปลงข้อความแล้ว {} กรอบ", self.rois.len());
                            self.persist_state();
                        }
                        Err(error) => self.status = format!("แปลงไม่สำเร็จ: {error}"),
                    }
                }
                WorkerResult::Capture { kind, value } => {
                    self.pending_capture = None;
                    match value {
                        Ok((image, geometry)) => {
                            let texture = Self::load_texture(ctx, "desktop-capture", &image);
                            self.capture_session = Some(CaptureSession {
                                kind,
                                image,
                                geometry,
                                texture,
                                scope: None,
                                crops: Vec::new(),
                                drag_start: None,
                            });
                            self.status = match kind {
                                CaptureKind::Document => {
                                    "ลากครั้งแรกเพื่อกำหนด Scope · คลิกขวาหรือ Esc เพื่อยกเลิก".into()
                                }
                                CaptureKind::Translate => {
                                    "ลากเลือกข้อความ 1 จุด · คลิกขวาหรือ Esc เพื่อยกเลิก".into()
                                }
                                CaptureKind::OcrPaste if self.ocr_paste_click_position.is_some() => {
                                    "ลากคลุมข้อความหรือภาพเพื่อแปลง แล้วคลิกและวางที่ตำแหน่งเดิม · คลิกขวาหรือ Esc เพื่อยกเลิก".into()
                                }
                                CaptureKind::OcrPaste => {
                                    "ลากคลุมข้อความหรือภาพเพื่อแปลงและคัดลอก Clipboard · คลิกขวาหรือ Esc เพื่อยกเลิก".into()
                                }
                            };
                        }
                        Err(error) => {
                            if kind == CaptureKind::OcrPaste {
                                self.ocr_paste_running = false;
                                self.ocr_paste_click_position = None;
                            }
                            self.restore_backup(ctx);
                            if kind != CaptureKind::OcrPaste {
                                self.main_visible = true;
                            }
                            self.set_launcher_visible(true);
                            self.status = format!("จับภาพหน้าจอไม่สำเร็จ: {error}");
                        }
                    }
                }
                WorkerResult::Translation { id, value } if id == self.translation_id => match value
                {
                    Ok(pair) => {
                        self.translation_text = Some(pair);
                        self.status = "Capture Translate พร้อมใช้งาน".into();
                    }
                    Err(error) => {
                        self.translation_text = Some((format!("ERROR: {error}"), String::new()));
                        self.status = format!("Capture Translate ไม่สำเร็จ: {error}");
                    }
                },
                WorkerResult::OcrPaste { id, value } if id == self.ocr_paste_id => match value {
                    Ok(text) => {
                        self.ocr_paste_running = false;
                        let click_position = self.ocr_paste_click_position.take();
                        if let Some(click_position) = click_position {
                            self.ocr_paste_success_until =
                                Some(Instant::now() + Duration::from_secs(2));
                            if let Some(feedback) = &mut self.launcher_feedback {
                                feedback.show_ok();
                            }
                            self.status = "อ่านข้อความสำเร็จ · OK · กำลังคลิกและวางข้อความ…".into();
                            let sender = self.worker_tx.clone();
                            thread::spawn(move || {
                                thread::sleep(Duration::from_millis(350));
                                let result = platform::click_and_paste_text(click_position, &text);
                                let _ = sender.send(WorkerResult::PasteApplied(result));
                            });
                        } else {
                            self.restore_ocr_paste_visibility();
                            let copied = Clipboard::new()
                                .and_then(|mut clipboard| clipboard.set_text(text))
                                .is_ok();
                            self.ocr_paste_success_until =
                                copied.then(|| Instant::now() + Duration::from_secs(2));
                            if let Some(feedback) = &mut self.launcher_feedback {
                                if copied {
                                    feedback.show_ok();
                                } else {
                                    feedback.hide();
                                }
                            }
                            self.status = if copied {
                                "แปลงแล้ว คัดลอกข้อความไว้ใน Clipboard · กด Ctrl+V เพื่อวาง".into()
                            } else {
                                "แปลงแล้ว แต่บันทึก Clipboard ไม่สำเร็จ".into()
                            };
                        }
                    }
                    Err(error) => {
                        self.ocr_paste_running = false;
                        self.ocr_paste_click_position = None;
                        self.restore_ocr_paste_visibility();
                        self.status = format!("แปลงไม่สำเร็จ: {error}");
                    }
                },
                WorkerResult::PasteApplied(Ok(())) => {
                    self.restore_ocr_paste_visibility();
                    self.ocr_paste_running = false;
                    self.ocr_paste_success_until = Some(Instant::now() + Duration::from_secs(2));
                    if let Some(feedback) = &mut self.launcher_feedback {
                        feedback.show_ok();
                    }
                    self.status = "วางข้อความแล้ว · OK".into();
                }
                WorkerResult::PasteApplied(Err(error)) => {
                    self.restore_ocr_paste_visibility();
                    self.ocr_paste_running = false;
                    self.ocr_paste_success_until = None;
                    if let Some(feedback) = &mut self.launcher_feedback {
                        feedback.hide();
                    }
                    self.status = format!(
                        "วางข้อความไม่สำเร็จ ({error}) — ลองกด Ctrl+V หากคัดลอกลง Clipboard สำเร็จ"
                    );
                }
                _ => {}
            }
        }
    }

    fn process_selection(&mut self, roi: Roi, ctx: &egui::Context) {
        self.add_roi(roi, ctx);
    }

    fn cancel_capture(&mut self, ctx: &egui::Context) {
        let kind = self.capture_session.take().map(|session| session.kind);
        self.ocr_paste_running = false;
        self.ocr_paste_click_position = None;
        self.ocr_paste_success_until = None;
        self.restore_backup(ctx);
        if kind != Some(CaptureKind::OcrPaste) {
            self.main_visible = false;
        }
        self.set_launcher_visible(true);
        self.status = "ยกเลิกการจับภาพหน้าจอ".into();
    }

    fn finish_capture(&mut self, ctx: &egui::Context) {
        let Some(session) = self.capture_session.take() else {
            return;
        };
        self.set_launcher_visible(true);
        match session.kind {
            CaptureKind::OcrPaste => {
                let Some(crop) = session
                    .crops
                    .first()
                    .and_then(|roi| image_ops::crop(&session.image, roi))
                else {
                    self.capture_session = Some(session);
                    return;
                };
                self.ocr_paste_restore_main =
                    self.backup.as_ref().map(|backup| backup.main_visible);
                self.restore_backup(ctx);
                self.main_visible = false;
                self.start_ocr_paste(crop);
            }
            CaptureKind::Translate => {
                let Some(crop) = session
                    .crops
                    .first()
                    .and_then(|roi| image_ops::crop(&session.image, roi))
                else {
                    self.capture_session = Some(session);
                    return;
                };
                self.start_translation(crop, ctx);
            }
            CaptureKind::Document => {
                let Some(scope_roi) = session.scope else {
                    self.capture_session = Some(session);
                    return;
                };
                let Some(scope) = image_ops::crop(&session.image, &scope_roi) else {
                    self.status = "ภาพหน้าจอไม่ถูกต้อง".into();
                    self.main_visible = true;
                    return;
                };
                let path = match history::save_image(&scope, &self.history_dir) {
                    Ok(path) => path,
                    Err(error) => {
                        self.status = format!("บันทึกภาพไม่สำเร็จ: {error}");
                        self.main_visible = true;
                        return;
                    }
                };
                self.set_document(scope, Some(path), ctx);
                for crop in session.crops {
                    let local = Roi {
                        x: crop.x.saturating_sub(scope_roi.x),
                        y: crop.y.saturating_sub(scope_roi.y),
                        width: crop.width,
                        height: crop.height,
                        text: String::new(),
                    };
                    self.add_roi(local, ctx);
                }
                self.backup = None;
                self.main_visible = true;
                if !self.rois.is_empty() {
                    self.convert_all();
                }
            }
        }
    }

    fn start_ocr_paste(&mut self, image: RgbaImage) {
        self.ocr_paste_running = true;
        self.ocr_paste_success_until = None;
        self.ocr_paste_id += 1;
        let id = self.ocr_paste_id;
        let config = self.config.clone();
        let sender = self.worker_tx.clone();
        self.status = "กำลังแปลงภาพเพื่อวางลงช่องข้อความ…".into();
        thread::spawn(move || {
            let value = (|| {
                // ponytail: single-crop hotkey path sends the crop directly, without batch framing.
                let source_row = image_ops::BatchRow {
                    id: "ROI_001".into(),
                    top: 0,
                    bottom: image.height(),
                };
                let (jpeg, rows) =
                    image_ops::encode_jpeg_under_limit(&image, &[source_row], 950_000)?;
                let text = ocr::ocr_single_image(&jpeg, &config, &rows[0])
                    .map_err(|error| error.to_string())?;
                if text.trim().is_empty() {
                    Err("ไม่พบข้อความในบริเวณที่เลือก จึงไม่ได้วางทับข้อความเดิม".into())
                } else {
                    Ok(text)
                }
            })();
            let _ = sender.send(WorkerResult::OcrPaste { id, value });
        });
    }

    fn capture_overlay(&mut self, ctx: &egui::Context) {
        let ocr_paste_autoclick = self.ocr_paste_click_position.is_some();
        let Some(session) = self.capture_session.as_mut() else {
            return;
        };
        let [left, top, width, height] = session.geometry;
        let kind = session.kind;
        let initial_pixels_per_point = ctx.native_pixels_per_point().unwrap_or(1.0).max(0.1);
        let builder = egui::ViewportBuilder::default()
            .with_title(match kind {
                CaptureKind::Document => "เลือกพื้นที่หน้าจอ",
                CaptureKind::Translate => "Capture Translate",
                CaptureKind::OcrPaste => "OCR Paste Capture",
            })
            .with_position([
                left as f32 / initial_pixels_per_point,
                top as f32 / initial_pixels_per_point,
            ])
            .with_inner_size([
                width as f32 / initial_pixels_per_point,
                height as f32 / initial_pixels_per_point,
            ])
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top();
        let mut action = None;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("screen-capture-overlay"),
            builder,
            |root, _| {
                let overlay_ctx = root.ctx().clone();
                let (pixels_per_point, inner_rect) = overlay_ctx.input(|input| {
                    (
                        input
                            .viewport()
                            .native_pixels_per_point
                            .unwrap_or(1.0)
                            .max(0.1),
                        input.viewport().inner_rect,
                    )
                });
                let desired_position = Pos2::new(
                    left as f32 / pixels_per_point,
                    top as f32 / pixels_per_point,
                );
                let desired_size = Vec2::new(
                    width as f32 / pixels_per_point,
                    height as f32 / pixels_per_point,
                );
                // ponytail: keep one overlay aligned to the physical virtual-desktop bounds using
                // its actual DPI; per-monitor overlay windows are the upgrade if separate DPI surfaces are needed.
                if inner_rect.map_or(true, |rect| {
                    (rect.min - desired_position).length() > 0.5
                        || (rect.size() - desired_size).length() > 0.5
                }) {
                    overlay_ctx.send_viewport_cmd(ViewportCommand::OuterPosition(desired_position));
                    overlay_ctx.send_viewport_cmd(ViewportCommand::InnerSize(desired_size));
                }
                let close_requested = overlay_ctx.input(|input| input.viewport().close_requested());
                if close_requested
                    || overlay_ctx.input(|input| input.key_pressed(egui::Key::Escape))
                    || overlay_ctx.input(|input| input.pointer.secondary_clicked())
                {
                    action = Some(OverlayAction::Cancel);
                    return;
                }
                if kind == CaptureKind::Document
                    && overlay_ctx.input(|input| input.key_pressed(egui::Key::Backspace))
                    && !session.crops.is_empty()
                {
                    session.crops.pop();
                }
                if kind == CaptureKind::Document
                    && overlay_ctx.input(|input| input.key_pressed(egui::Key::Enter))
                    && session.scope.is_some()
                    && !session.crops.is_empty()
                {
                    action = Some(OverlayAction::Finish);
                    return;
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(root, |ui| {
                        let (response, painter) =
                            ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
                        let full = response.rect;
                        painter.image(
                            session.texture.id(),
                            full,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        painter.rect_filled(full, 0.0, Color32::from_black_alpha(145));
                        let image_size =
                            Vec2::new(session.image.width() as f32, session.image.height() as f32);
                        let draw_clear = |painter: &egui::Painter, roi: &Roi| {
                            let target = roi_to_screen(roi, full, image_size);
                            let uv = Rect::from_min_max(
                                Pos2::new(roi.x as f32 / image_size.x, roi.y as f32 / image_size.y),
                                Pos2::new(
                                    (roi.x + roi.width) as f32 / image_size.x,
                                    (roi.y + roi.height) as f32 / image_size.y,
                                ),
                            );
                            painter.image(session.texture.id(), target, uv, Color32::WHITE);
                            target
                        };
                        let mut delete_crop = None;
                        if let Some(scope) = &session.scope {
                            let scope_rect = draw_clear(&painter, scope);
                            painter.rect_stroke(
                                scope_rect,
                                0.0,
                                Stroke::new(3.0, Color32::from_rgb(250, 204, 21)),
                                StrokeKind::Inside,
                            );
                            for (index, crop) in session.crops.iter().enumerate() {
                                let rect = roi_to_screen(crop, full, image_size);
                                painter.rect_stroke(
                                    rect,
                                    0.0,
                                    Stroke::new(3.0, Color32::from_rgb(34, 197, 94)),
                                    StrokeKind::Inside,
                                );
                                painter.rect_filled(
                                    Rect::from_min_size(rect.left_top(), Vec2::new(42.0, 22.0)),
                                    0.0,
                                    Color32::from_rgb(34, 197, 94),
                                );
                                painter.text(
                                    rect.left_top() + Vec2::new(21.0, 11.0),
                                    egui::Align2::CENTER_CENTER,
                                    (index + 1).to_string(),
                                    egui::FontId::proportional(14.0),
                                    Color32::BLACK,
                                );
                                let pointer = response.hover_pos();
                                if pointer.is_some_and(|position| rect.contains(position)) {
                                    let delete_rect = Rect::from_min_size(
                                        Pos2::new(
                                            (rect.right() - 30.0).max(rect.left() + 2.0),
                                            rect.top() + 3.0,
                                        ),
                                        Vec2::splat(27.0),
                                    );
                                    painter.rect_filled(
                                        delete_rect,
                                        5.0,
                                        Color32::from_rgb(220, 38, 38),
                                    );
                                    painter.rect_stroke(
                                        delete_rect,
                                        5.0,
                                        Stroke::new(2.0, Color32::WHITE),
                                        StrokeKind::Inside,
                                    );
                                    painter.text(
                                        delete_rect.center(),
                                        egui::Align2::CENTER_CENTER,
                                        "×",
                                        egui::FontId::proportional(22.0),
                                        Color32::WHITE,
                                    );
                                    if response.clicked()
                                        && pointer
                                            .is_some_and(|position| delete_rect.contains(position))
                                    {
                                        delete_crop = Some(index);
                                    }
                                }
                            }
                        }
                        if let Some(index) = delete_crop {
                            session.crops.remove(index);
                            session.drag_start = None;
                        } else if response.drag_started() {
                            session.drag_start = ui
                                .input(|input| input.pointer.press_origin())
                                .or_else(|| response.interact_pointer_pos());
                        }
                        if delete_crop.is_none()
                            && let (Some(start), Some(end)) =
                                (session.drag_start, response.interact_pointer_pos())
                        {
                            let current = Rect::from_two_pos(start, end).intersect(full);
                            if response.dragged() {
                                painter.rect_stroke(
                                    current,
                                    0.0,
                                    Stroke::new(3.0, Color32::from_rgb(250, 204, 21)),
                                    StrokeKind::Inside,
                                );
                            }
                            if response.drag_stopped() {
                                session.drag_start = None;
                                if let Some(mut roi) = screen_to_roi(current, full, image_size) {
                                    if matches!(kind, CaptureKind::Translate | CaptureKind::OcrPaste) {
                                        session.crops = vec![roi];
                                        action = Some(OverlayAction::Finish);
                                    } else if let Some(scope) = &session.scope {
                                        let x1 = roi.x.max(scope.x);
                                        let y1 = roi.y.max(scope.y);
                                        let x2 = roi
                                            .x
                                            .saturating_add(roi.width)
                                            .min(scope.x.saturating_add(scope.width));
                                        let y2 = roi
                                            .y
                                            .saturating_add(roi.height)
                                            .min(scope.y.saturating_add(scope.height));
                                        if x2 > x1 + 4 && y2 > y1 + 4 {
                                            roi.x = x1;
                                            roi.y = y1;
                                            roi.width = x2 - x1;
                                            roi.height = y2 - y1;
                                            session.crops.push(roi);
                                        }
                                    } else {
                                        session.scope = Some(roi);
                                    }
                                }
                            }
                        }
                        let text = match (kind, session.scope.is_some()) {
                            (CaptureKind::Translate, _) => {
                                "ลากเลือกข้อความ 1 จุด · คลิกขวาหรือ Esc เพื่อยกเลิก"
                            }
                            (CaptureKind::OcrPaste, _) if ocr_paste_autoclick => {
                                "ลากคลุมเพื่อแปลง แล้วคลิกและวางที่ตำแหน่งเดิม · คลิกขวาหรือ Esc เพื่อยกเลิก"
                            }
                            (CaptureKind::OcrPaste, _) => {
                                "ลากคลุมเพื่อแปลงและคัดลอก Clipboard · คลิกขวาหรือ Esc เพื่อยกเลิก"
                            }
                            (CaptureKind::Document, false) => {
                                "ลากครั้งแรกเพื่อกำหนด Scope · คลิกขวาหรือ Esc เพื่อยกเลิก"
                            }
                            (CaptureKind::Document, true) => {
                                "ลาก Crop หลายจุด · Hover แล้วกด × เพื่อลบ · Enter แปลง · คลิกขวาหรือ Esc เพื่อยกเลิก"
                            }
                        };
                        let banner = Rect::from_min_max(
                            full.min + Vec2::splat(20.0),
                            Pos2::new(full.right() - 20.0, full.top() + 62.0),
                        );
                        painter.rect_filled(banner, 0.0, Color32::from_black_alpha(190));
                        painter.text(
                            banner.center(),
                            egui::Align2::CENTER_CENTER,
                            text,
                            egui::FontId::proportional(16.0),
                            Color32::WHITE,
                        );
                    });
            },
        );
        match action {
            Some(OverlayAction::Cancel) => self.cancel_capture(ctx),
            Some(OverlayAction::Finish) => self.finish_capture(ctx),
            None => {}
        }
    }

    fn process_hotkeys(&mut self, ctx: &egui::Context) {
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            if event.state != HotKeyState::Pressed {
                continue;
            }
            let Some(action) = self.hotkey_actions.get(&event.id).cloned() else {
                continue;
            };
            let ocr_click_position = (action == "ocr-paste")
                .then(platform::cursor_position)
                .flatten();
            platform::mask_alt();
            match action.as_str() {
                "capture" => self.begin_capture(CaptureKind::Document, ctx),
                "translate" => self.begin_capture(CaptureKind::Translate, ctx),
                "ocr-paste" => {
                    if let Some(click_position) = ocr_click_position {
                        self.begin_ocr_paste_capture(ctx, click_position);
                    } else {
                        self.status = "อ่านตำแหน่งเมาส์ไม่สำเร็จ จึงไม่เริ่ม Alt+A".into();
                    }
                }
                digit => {
                    if let Ok(index) = digit.parse::<usize>() {
                        let text = if self.translation_open && index <= 2 {
                            self.translation_text.as_ref().map(|pair| {
                                if index == 1 {
                                    pair.0.clone()
                                } else {
                                    pair.1.clone()
                                }
                            })
                        } else {
                            self.rois.get(index - 1).map(|roi| roi.text.clone())
                        };
                        if let Some(text) = text {
                            if self.translation_open && index <= 2 {
                                let _ = Clipboard::new()
                                    .and_then(|mut clipboard| clipboard.set_text(text));
                            } else {
                                thread::spawn(move || {
                                    let _ = platform::paste_text(&text);
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn process_tray(&mut self, ctx: &egui::Context) {
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(event, TrayIconEvent::DoubleClick { .. }) {
                self.show_main(false);
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = self.action_for_menu_id(&event.id) {
                self.perform_launcher_action(action, ctx);
            }
        }
    }

    fn action_for_menu_id(&self, id: &MenuId) -> Option<LauncherAction> {
        [&self.tray_actions, &self.launcher_actions]
            .into_iter()
            .find_map(|actions| {
                if *id == actions.show {
                    Some(LauncherAction::Show)
                } else if *id == actions.welcome {
                    Some(LauncherAction::Welcome)
                } else if *id == actions.capture {
                    Some(LauncherAction::Capture)
                } else if *id == actions.ocr_clipboard {
                    Some(LauncherAction::OcrClipboard)
                } else if *id == actions.translate {
                    Some(LauncherAction::Translate)
                } else if *id == actions.history {
                    Some(LauncherAction::History)
                } else if *id == actions.settings {
                    Some(LauncherAction::Settings)
                } else if *id == actions.exit {
                    Some(LauncherAction::Exit)
                } else {
                    None
                }
            })
    }

    fn perform_launcher_action(&mut self, action: LauncherAction, ctx: &egui::Context) {
        match action {
            LauncherAction::Show => self.show_main(false),
            LauncherAction::Welcome => self.welcome_open = true,
            LauncherAction::Capture => self.begin_capture(CaptureKind::Document, ctx),
            LauncherAction::OcrClipboard => self.begin_ocr_clipboard_capture(ctx),
            LauncherAction::Translate => self.begin_capture(CaptureKind::Translate, ctx),
            LauncherAction::History => {
                self.history_open = true;
                self.history_current = None;
                self.history_selection.clear();
                self.show_main(false);
            }
            LauncherAction::Settings => {
                self.open_settings();
            }
            LauncherAction::Exit => self.request_exit(ctx),
            LauncherAction::SavePosition(position) => {
                self.config.launcher_pos = Some(position);
                self.save_config();
            }
        }
    }

    fn show_main(&mut self, show_welcome: bool) {
        self.main_visible = true;
        if show_welcome {
            self.welcome_open = true;
        }
    }

    fn open_settings(&mut self) {
        self.settings_draft = self.config.clone();
        self.settings_show_keys = false;
        self.settings_open = true;
    }

    fn ask_confirm(
        &mut self,
        title: impl Into<String>,
        text: impl Into<String>,
        action: ConfirmAction,
    ) {
        self.confirm = Some(ConfirmDialog {
            title: title.into(),
            text: text.into(),
            action,
        });
    }

    fn execute_confirm(&mut self, action: ConfirmAction, ctx: &egui::Context) {
        match action {
            ConfirmAction::ResumeLive => self.resume_live_now(),
            ConfirmAction::Rotate(clockwise) => self.rotate_now(clockwise, ctx),
            ConfirmAction::DeleteRoi(index) => self.delete_roi_now(index, ctx),
            ConfirmAction::ClearRois => {
                self.rois.clear();
                self.roi_previews.clear();
                self.batch_id += 1;
                self.batch_running = None;
                self.persist_state();
            }
            ConfirmAction::DeleteHistory => self.delete_selected_history(ctx),
        }
    }

    fn request_exit(&mut self, ctx: &egui::Context) {
        self.exit_requested = true;
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, ViewportCommand::Close);
    }

    fn set_launcher_visible(&mut self, visible: bool) {
        let Some(hwnd) = self.launcher_hwnd else {
            return;
        };
        if visible {
            if let Some(position) = self.launcher_capture_position.take() {
                platform::move_window(hwnd, position);
            }
        } else if self.launcher_capture_position.is_none() {
            if let Some(feedback) = &mut self.launcher_feedback {
                feedback.hide();
            }
            self.launcher_capture_position = platform::window_position(hwnd);
            platform::move_window(hwnd, [-32_000, -32_000]);
        }
    }

    fn floating_launcher(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, frame: &eframe::Frame) {
        if self
            .ocr_paste_success_until
            .is_some_and(|until| Instant::now() >= until)
        {
            self.ocr_paste_success_until = None;
            if let Some(feedback) = &mut self.launcher_feedback {
                feedback.hide();
            }
        }
        let show_ocr_progress = self.ocr_paste_running;
        let show_ocr_success = self.ocr_paste_success_until.is_some();
        if !self.launcher_initialized {
            let [x, y] = self
                .config
                .launcher_pos
                .unwrap_or_else(platform::launcher_default_position);
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new(
                x as f32, y as f32,
            )));
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::splat(32.0)));
            self.launcher_initialized = true;
        } else if !self.launcher_native_styled
            && let Some(hwnd) = launcher_hwnd(frame)
        {
            platform::style_transparent_launcher(hwnd);
            self.launcher_hwnd = Some(hwnd);
            let [x, y] = self
                .config
                .launcher_pos
                .unwrap_or_else(platform::launcher_default_position);
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(Pos2::new(
                x as f32, y as f32,
            )));
            self.launcher_feedback = platform::create_launcher_feedback(hwnd);
            if show_ocr_success && let Some(feedback) = &mut self.launcher_feedback {
                feedback.show_ok();
            }
            self.launcher_native_styled = true;
        }

        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let launcher = ui.add(
                    egui::Image::new(&self.app_icon_texture)
                        .fit_to_exact_size(Vec2::splat(32.0))
                        .sense(Sense::click_and_drag()),
                );
                let painter = ui.painter();
                let center = launcher.rect.center();
                if show_ocr_progress {
                    painter.circle_filled(center, 14.0, Color32::from_black_alpha(205));
                    let start_angle = ctx.input(|input| input.time as f32) * 4.5;
                    let arc = (0..=12)
                        .map(|step| {
                            let angle =
                                start_angle + step as f32 / 12.0 * std::f32::consts::PI * 1.5;
                            Pos2::new(center.x + 10.0 * angle.cos(), center.y + 10.0 * angle.sin())
                        })
                        .collect();
                    painter.add(egui::Shape::line(
                        arc,
                        Stroke::new(3.0, Color32::from_rgb(255, 255, 255)),
                    ));
                }
                if launcher.clicked() {
                    self.show_launcher_context_menu(frame);
                }
                if launcher.drag_started() {
                    ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                }
                if launcher.drag_stopped()
                    && let Some(rect) = ui.input(|input| input.viewport().outer_rect)
                {
                    action = Some(LauncherAction::SavePosition([
                        rect.left().round() as i32,
                        rect.top().round() as i32,
                    ]));
                }
            });
        if let Some(action) = action {
            self.perform_launcher_action(action, ctx);
        }
    }

    fn show_launcher_context_menu(&self, frame: &eframe::Frame) {
        if let Some(hwnd) = launcher_hwnd(frame) {
            // SAFETY: eframe owns this HWND for the lifetime of `frame`, and the
            // synchronous popup returns before this method releases the handle.
            unsafe {
                self.launcher_menu.show_context_menu_for_hwnd(hwnd, None);
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.vertical(|ui| {
            ui.horizontal_wrapped(|ui| {
                if self.camera.is_some() {
                    if icon_text_button(ui, &self.ui_icons.camera, "ปิดกล้อง")
                        .on_hover_text("หยุดภาพสดโดยไม่ลบภาพหรือผลลัพธ์")
                        .clicked()
                    {
                        self.toggle_camera();
                    }
                    if self.document.is_none() {
                        let old_camera = self.selected_camera;
                        egui::ComboBox::from_id_salt("camera")
                            .selected_text(
                                self.cameras
                                    .iter()
                                    .find(|camera| camera.index == self.selected_camera)
                                    .map(|camera| format!("กล้อง {}", camera.index))
                                    .unwrap_or_else(|| "เลือกกล้อง".into()),
                            )
                            .show_ui(ui, |ui| {
                                for camera in &self.cameras {
                                    ui.selectable_value(
                                        &mut self.selected_camera,
                                        camera.index,
                                        format!("กล้อง {}", camera.index),
                                    )
                                    .on_hover_text(&camera.name);
                                }
                            });
                        if old_camera != self.selected_camera {
                            self.restart_camera();
                        }
                    }
                } else if self.document.is_none() {
                    if icon_text_button(ui, &self.ui_icons.camera, "เปิดกล้อง")
                        .on_hover_text("ใช้ภาพสดจากกล้อง แล้วคลิกภาพหรือกด Spacebar เพื่อถ่าย")
                        .clicked()
                    {
                        self.toggle_camera();
                    }
                }

                if icon_text_button(ui, &self.ui_icons.capture, "จับภาพหน้าจอ")
                    .on_hover_text("ลากครอบบริเวณที่ต้องการ · Alt+S")
                    .clicked()
                {
                    self.begin_capture(CaptureKind::Document, ctx);
                }
                if icon_text_button(ui, &self.ui_icons.history, "ประวัติภาพ")
                    .on_hover_text("เปิดภาพที่เคยจับและบันทึกไว้")
                    .clicked()
                {
                    self.history_open = true;
                    self.history_selection.clear();
                    self.history_current = None;
                }
                if self.document.is_some()
                    && icon_text_button(ui, &self.ui_icons.add, "ภาพใหม่")
                        .on_hover_text("เริ่มงานใหม่ โดยกลับไปเลือกแหล่งภาพ")
                        .clicked()
                {
                    if self.rois.is_empty() {
                        self.resume_live_now();
                    } else {
                        self.ask_confirm(
                            "เริ่มภาพใหม่",
                            "การเริ่มภาพใหม่จะล้างกรอบ ROI ปัจจุบัน ดำเนินการต่อหรือไม่?",
                            ConfirmAction::ResumeLive,
                        );
                    }
                }
                if icon_text_button(ui, &self.ui_icons.settings, "ตั้งค่า")
                    .on_hover_text("ตั้งค่า OCR และ API Key")
                    .clicked()
                {
                    self.open_settings();
                }
                if icon_text_button(ui, &self.ui_icons.info, "เกี่ยวกับ")
                    .on_hover_text("ข้อมูลโปรแกรมและเวอร์ชัน")
                    .clicked()
                {
                    self.about_open = true;
                }
            });

            if self.document.is_some() {
                ui.horizontal_wrapped(|ui| {
                    if icon_text_button(ui, &self.ui_icons.rotate_left, "หมุนซ้าย")
                        .on_hover_text("หมุนภาพทวนเข็มนาฬิกา")
                        .clicked()
                    {
                        if self.rois.is_empty() {
                            self.rotate_now(false, ctx);
                        } else {
                            self.ask_confirm(
                                "หมุนภาพ",
                                "การหมุนภาพจะล้างกรอบ ROI ปัจจุบัน ดำเนินการต่อหรือไม่?",
                                ConfirmAction::Rotate(false),
                            );
                        }
                    }
                    if icon_text_button(ui, &self.ui_icons.rotate_right, "หมุนขวา")
                        .on_hover_text("หมุนภาพตามเข็มนาฬิกา")
                        .clicked()
                    {
                        if self.rois.is_empty() {
                            self.rotate_now(true, ctx);
                        } else {
                            self.ask_confirm(
                                "หมุนภาพ",
                                "การหมุนภาพจะล้างกรอบ ROI ปัจจุบัน ดำเนินการต่อหรือไม่?",
                                ConfirmAction::Rotate(true),
                            );
                        }
                    }
                    if icon_text_button(ui, &self.ui_icons.delete_sweep, "ล้างกรอบ")
                        .on_hover_text("ลบกรอบข้อความทั้งหมด")
                        .clicked()
                        && !self.rois.is_empty()
                    {
                        self.ask_confirm(
                            "ล้างกรอบ",
                            format!("ลบ ROI ทั้งหมด {} กรอบหรือไม่?", self.rois.len()),
                            ConfirmAction::ClearRois,
                        );
                    }
                });
            }
        });
    }

    fn image_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(texture) = &self.texture else {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    if self.camera.is_some() {
                        ui.add(egui::Spinner::new().size(34.0));
                        ui.add_space(12.0);
                        ui.heading("กำลังเปิดกล้อง");
                        ui.label("เมื่อภาพพร้อม คลิกบนภาพหรือกด Spacebar เพื่อถ่าย");
                    } else {
                        ui.add(
                            egui::Image::new(&self.app_icon_texture)
                                .fit_to_exact_size(Vec2::splat(88.0)),
                        );
                        ui.add_space(12.0);
                        ui.heading("เริ่มแปลงภาพเป็นข้อความ");
                        ui.label("เลือกวิธีนำเข้าภาพจากแถบด้านบนได้ตามสะดวก");
                        ui.add_space(4.0);
                        ui.label("เปิดกล้อง · จับภาพหน้าจอ · เลือกภาพจากประวัติ");
                    }
                });
            });
            return;
        };
        let available = ui.available_size();
        let image_size = texture.size_vec2();
        let scale = (available.x / image_size.x)
            .min(available.y / image_size.y)
            .max(0.05);
        let shown = image_size * scale;
        let response = ui.add(
            egui::Image::new(texture)
                .fit_to_exact_size(shown)
                .sense(Sense::click_and_drag()),
        );
        let rect = response.rect;
        let painter = ui.painter_at(rect);
        if self.document.is_none() {
            let hint = Rect::from_min_max(
                Pos2::new(rect.left() + 20.0, rect.bottom() - 58.0),
                Pos2::new(rect.right() - 20.0, rect.bottom() - 16.0),
            );
            painter.rect_filled(hint, 0.0, Color32::from_black_alpha(175));
            painter.text(
                hint.center(),
                egui::Align2::CENTER_CENTER,
                "คลิกที่หน้าจอ หรือกด Spacebar เพื่อจับภาพ",
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        let pointer = response.hover_pos();
        let mut delete_roi = None;
        for (index, roi) in self.rois.iter().enumerate() {
            let mapped = roi_to_screen(roi, rect, image_size);
            painter.rect_stroke(
                mapped,
                0.0,
                Stroke::new(2.0, Color32::GREEN),
                StrokeKind::Inside,
            );
            painter.text(
                mapped.left_top(),
                egui::Align2::LEFT_TOP,
                format!("{}", index + 1),
                egui::FontId::proportional(15.0),
                Color32::YELLOW,
            );
            if pointer.is_some_and(|position| mapped.contains(position)) {
                let delete_rect = Rect::from_min_size(
                    Pos2::new(
                        (mapped.right() - 28.0).max(mapped.left() + 2.0),
                        mapped.top() + 3.0,
                    ),
                    Vec2::splat(25.0),
                );
                painter.rect_filled(delete_rect, 5.0, Color32::from_rgb(220, 38, 38));
                painter.rect_stroke(
                    delete_rect,
                    5.0,
                    Stroke::new(2.0, Color32::WHITE),
                    StrokeKind::Inside,
                );
                painter.text(
                    delete_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "×",
                    egui::FontId::proportional(21.0),
                    Color32::WHITE,
                );
                if response.clicked()
                    && pointer.is_some_and(|position| delete_rect.contains(position))
                {
                    delete_roi = Some(index);
                }
            }
        }
        if let Some(index) = delete_roi {
            self.drag_start = None;
            self.ask_confirm(
                "ลบ ROI",
                format!("ลบ ROI #{} หรือไม่?", index + 1),
                ConfirmAction::DeleteRoi(index),
            );
        } else if response.drag_started() {
            self.drag_start = ui
                .input(|input| input.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos());
        }
        if delete_roi.is_none()
            && response.dragged()
            && let (Some(start), Some(end)) = (self.drag_start, response.interact_pointer_pos())
        {
            painter.rect_stroke(
                Rect::from_two_pos(start, end).intersect(rect),
                0.0,
                Stroke::new(2.0, Color32::YELLOW),
                StrokeKind::Inside,
            );
        }
        if delete_roi.is_none() && response.drag_stopped() {
            if let (Some(start), Some(end)) =
                (self.drag_start.take(), response.interact_pointer_pos())
                && let Some(roi) = screen_to_roi(
                    Rect::from_two_pos(start, end).intersect(rect),
                    rect,
                    image_size,
                )
            {
                self.process_selection(roi, ctx);
            }
        } else if delete_roi.is_none() && response.clicked() && self.document.is_none() {
            self.capture_photo(ctx);
        }
    }

    fn roi_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut delete = None;
        let mut changed = false;
        let mut engine_changed = false;
        ui.horizontal(|ui| {
            ui.strong("เลือก OCR:");
            if ui
                .add_enabled(
                    self.batch_running.is_none(),
                    egui::Button::new("OCR.Space (เร็ว)").selected(self.config.engine != "gemini"),
                )
                .clicked()
                && self.config.engine != "ocrspace"
            {
                self.config.engine = "ocrspace".into();
                engine_changed = true;
            }
            if ui
                .add_enabled(
                    self.batch_running.is_none(),
                    egui::Button::new("Gemini (แม่นยำ)").selected(self.config.engine == "gemini"),
                )
                .clicked()
                && self.config.engine != "gemini"
            {
                self.config.engine = "gemini".into();
                engine_changed = true;
            }
        });
        if engine_changed {
            self.config.ocrspace_engine = "3".into();
            self.settings_draft.engine = self.config.engine.clone();
            self.settings_draft.ocrspace_engine = "3".into();
            let selected = if self.config.engine == "gemini" {
                "Gemini (แม่นยำ)"
            } else {
                "OCR.Space (เร็ว)"
            };
            self.status = format!("เลือก {selected} แล้ว");
            self.save_config();
        }
        ui.add_space(4.0);
        let engine_name = if self.config.engine == "gemini" {
            "Gemini"
        } else {
            "OCR.Space"
        };
        let retry_error = self.status.strip_prefix("แปลงไม่สำเร็จ: ").map(str::to_owned);
        if self.batch_running.is_some() {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new());
                ui.label("กำลังประมวลผล OCR…");
            });
        } else if let Some(error) = retry_error {
            egui::Frame::new()
                .fill(Color32::from_rgb(255, 235, 235))
                .stroke(Stroke::new(1.0, Color32::from_rgb(190, 35, 35)))
                .inner_margin(8.0)
                .show(ui, |ui| {
                    ui.colored_label(
                        Color32::from_rgb(150, 20, 20),
                        format!("แปลงไม่สำเร็จ\n{error}"),
                    );
                    ui.add_space(6.0);
                    let retry_label = format!("ลองแปลงอีกครั้ง ({engine_name})");
                    let retry = ui.scope(|ui| {
                        let widgets = &mut ui.style_mut().visuals.widgets;
                        widgets.inactive.weak_bg_fill = Color32::from_rgb(217, 119, 6);
                        widgets.hovered.weak_bg_fill = Color32::from_rgb(245, 158, 11);
                        widgets.active.weak_bg_fill = Color32::from_rgb(146, 64, 14);
                        widgets.inactive.bg_stroke =
                            Stroke::new(2.0, Color32::from_rgb(146, 64, 14));
                        widgets.hovered.bg_stroke =
                            Stroke::new(2.0, Color32::from_rgb(120, 53, 15));
                        widgets.active.bg_stroke = Stroke::new(2.0, Color32::from_rgb(69, 26, 3));
                        ui.add_enabled(
                            !self.rois.is_empty(),
                            egui::Button::image_and_text(
                                egui::Image::new(&self.ui_icons.capture)
                                    .fit_to_exact_size(Vec2::splat(24.0))
                                    .tint(Color32::WHITE),
                                egui::RichText::new(retry_label)
                                    .size(16.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .corner_radius(7.0)
                            .min_size(Vec2::new(ui.available_width(), 50.0)),
                        )
                    });
                    if retry.inner.clicked() {
                        self.convert_all();
                    }
                });
        } else {
            let convert_label = format!("คลิกเพื่อแปลงภาพเป็นข้อความ ({engine_name})");
            let convert = ui.scope(|ui| {
                let widgets = &mut ui.style_mut().visuals.widgets;
                widgets.inactive.weak_bg_fill = Color32::from_rgb(37, 99, 235);
                widgets.hovered.weak_bg_fill = Color32::from_rgb(59, 130, 246);
                widgets.active.weak_bg_fill = Color32::from_rgb(30, 64, 175);
                widgets.inactive.bg_stroke = Stroke::new(2.0, Color32::from_rgb(30, 64, 175));
                widgets.hovered.bg_stroke = Stroke::new(3.0, Color32::from_rgb(29, 78, 216));
                widgets.active.bg_stroke = Stroke::new(2.0, Color32::from_rgb(23, 37, 84));
                ui.add_enabled(
                    !self.rois.is_empty(),
                    egui::Button::image_and_text(
                        egui::Image::new(&self.ui_icons.capture)
                            .fit_to_exact_size(Vec2::splat(24.0))
                            .tint(Color32::WHITE),
                        egui::RichText::new(convert_label)
                            .size(16.0)
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .corner_radius(7.0)
                    .min_size(Vec2::new(ui.available_width(), 50.0)),
                )
            });
            if convert.inner.clicked() {
                self.convert_all();
            }
        }
        ui.separator();
        let close_icon = &self.ui_icons.close;
        let copy_icon = &self.ui_icons.copy;
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.rois.is_empty() {
                ui.add_space(18.0);
                ui.vertical_centered(|ui| {
                    ui.strong("ยังไม่มีกรอบข้อความ");
                    ui.add_space(4.0);
                    ui.label(if self.document.is_some() {
                        "ลากครอบข้อความบนภาพด้านซ้ายเพื่อเพิ่มผลลัพธ์"
                    } else {
                        "เลือกแหล่งภาพก่อน แล้วลากครอบข้อความที่ต้องการ"
                    });
                });
            }
            for (index, roi) in self.rois.iter_mut().enumerate() {
                ui.group(|ui| {
                    let (preview_rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 126.0),
                        Sense::hover(),
                    );
                    let painter = ui.painter_at(preview_rect);
                    painter.rect_filled(preview_rect, 0.0, Color32::from_rgb(17, 17, 17));
                    if let Some(texture) = self.roi_previews.get(index) {
                        let source = texture.size_vec2();
                        let scale = ((preview_rect.width() - 8.0) / source.x)
                            .min((preview_rect.height() - 8.0) / source.y)
                            .min(1.0);
                        let target = Rect::from_center_size(preview_rect.center(), source * scale);
                        painter.image(
                            texture.id(),
                            target,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    let badge_text = (index + 1).to_string();
                    let badge_pos = preview_rect.left_top() + Vec2::new(4.0, 3.0);
                    let badge_font = egui::FontId::proportional(17.0);
                    for offset in [
                        Vec2::new(-2.0, -2.0),
                        Vec2::new(0.0, -2.0),
                        Vec2::new(2.0, -2.0),
                        Vec2::new(-2.0, 0.0),
                        Vec2::new(2.0, 0.0),
                        Vec2::new(-2.0, 2.0),
                        Vec2::new(0.0, 2.0),
                        Vec2::new(2.0, 2.0),
                    ] {
                        painter.text(
                            badge_pos + offset,
                            egui::Align2::LEFT_TOP,
                            &badge_text,
                            badge_font.clone(),
                            Color32::BLACK,
                        );
                    }
                    painter.text(
                        badge_pos,
                        egui::Align2::LEFT_TOP,
                        badge_text,
                        badge_font,
                        Color32::WHITE,
                    );
                    let close = Rect::from_min_size(
                        Pos2::new(preview_rect.right() - 34.0, preview_rect.top() + 7.0),
                        Vec2::splat(27.0),
                    );
                    if ui.input(|input| {
                        input
                            .pointer
                            .hover_pos()
                            .is_some_and(|position| preview_rect.contains(position))
                    }) && ui
                        .place(
                            close,
                            egui::Button::image(
                                egui::Image::new(close_icon).fit_to_exact_size(Vec2::splat(17.0)),
                            )
                            .fill(Color32::from_white_alpha(235))
                            .stroke(Stroke::new(2.0, Color32::from_rgb(5, 5, 5)))
                            .corner_radius(5.0),
                        )
                        .on_hover_text("ลบ ROI")
                        .clicked()
                    {
                        delete = Some(index);
                    }
                    ui.horizontal(|ui| {
                        let text_width =
                            (ui.available_width() - 38.0 - ui.spacing().item_spacing.x).max(80.0);
                        changed |= ui
                            .add(
                                egui::TextEdit::multiline(&mut roi.text)
                                    .desired_rows(3)
                                    .desired_width(text_width)
                                    .hint_text("รอกด แปลงภาพเป็นข้อความ"),
                            )
                            .changed();
                        ui.vertical_centered(|ui| {
                            if ui
                                .add_sized(
                                    [38.0, 32.0],
                                    egui::Button::image(
                                        egui::Image::new(copy_icon)
                                            .fit_to_exact_size(Vec2::splat(20.0)),
                                    ),
                                )
                                .on_hover_text("คัดลอกข้อความ")
                                .clicked()
                            {
                                let _ =
                                    Clipboard::new().and_then(|mut c| c.set_text(roi.text.clone()));
                            }
                            if index < 9 {
                                ui.label(
                                    egui::RichText::new(format!("Alt+{}", index + 1)).size(10.0),
                                );
                            }
                        });
                    });
                });
                ui.add_space(5.0);
            }
        });
        if let Some(index) = delete {
            self.ask_confirm(
                "ลบ ROI",
                format!("ลบ ROI #{} หรือไม่?", index + 1),
                ConfirmAction::DeleteRoi(index),
            );
        }
        if changed {
            self.persist_state();
        }
        let _ = ctx;
    }

    fn settings_viewport(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let viewport_id = egui::ViewportId::from_hash_of("ocr-settings-window");
        let builder = egui::ViewportBuilder::default()
            .with_title("ตั้งค่า OCR")
            .with_inner_size([760.0, 850.0])
            .with_min_inner_size([520.0, 480.0])
            .with_resizable(true)
            .with_clamp_size_to_monitor_size(true)
            .with_icon(self.window_icon.clone());
        let mut close = false;
        let mut save = false;
        let mut cancel = false;
        let mut open_link = None;
        let check_icon = self.ui_icons.check.clone();
        let close_icon = self.ui_icons.close.clone();
        ctx.show_viewport_immediate(viewport_id, builder, |root, _| {
            let settings_ctx = root.ctx().clone();
            if settings_ctx.input(|input| input.viewport().close_requested()) {
                close = true;
                return;
            }
            egui::CentralPanel::default().show(root, |ui| {
                let content_height = (ui.available_height() - 48.0).max(100.0);
                egui::ScrollArea::vertical()
                    .max_height(content_height)
                    .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("OCR engine");
                    egui::ComboBox::from_id_salt("ocr-engine")
                        .selected_text(if self.settings_draft.engine == "gemini" {
                            "Google Gemini API"
                        } else {
                            "OCR.space API"
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.settings_draft.engine,
                                "ocrspace".into(),
                                "OCR.space API",
                            );
                            ui.selectable_value(
                                &mut self.settings_draft.engine,
                                "gemini".into(),
                                "Google Gemini API",
                            );
                        });
                });
                ui.label("OCR.space API key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_draft.ocrspace_key)
                        .password(!self.settings_show_keys),
                );
                ui.horizontal(|ui| {
                    ui.label("OCR.space engine");
                    ui.strong("3");
                    ui.label("(กำหนดตายตัว)");
                });
                ui.label("Gemini API key");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_draft.gemini_key)
                        .password(!self.settings_show_keys),
                );
                ui.label("Gemini model");
                ui.horizontal(|ui| {
                    ui.strong("gemini-flash-latest");
                    ui.label("(อัปเดตเป็น Flash รุ่นล่าสุดอัตโนมัติ)");
                });
                ui.label("Font");
                ui.text_edit_singleline(&mut self.settings_draft.font_family);
                ui.add(
                    egui::Slider::new(&mut self.settings_draft.main_font_size, 9..=32)
                        .text("ขนาด Text หลัก"),
                );
                ui.add(
                    egui::Slider::new(&mut self.settings_draft.sticky_font_size, 10..=42)
                        .text("ขนาดตัวอักษร Capture Translate"),
                );
                ui.checkbox(&mut self.settings_show_keys, "แสดง API keys");
                ui.add_space(6.0);
                egui::Frame::group(ui.style())
                    .fill(Color32::from_rgb(244, 248, 251))
                    .show(ui, |ui| {
                        ui.strong("🔑 วิธีสมัคร API Key ฟรี");
                        ui.label("ไม่ต้องผูกบัตรเครดิต (โควตาอาจเปลี่ยนตามนโยบายผู้ให้บริการ)");
                        ui.add_space(5.0);
                        ui.strong("1. OCR.space API");
                        ui.label("• ฟรีประมาณ 25,000 ครั้ง/เดือน");
                        ui.horizontal_wrapped(|ui| {
                            ui.label("• เข้า");
                            if ui.link("ocr.space/ocrapi").clicked() {
                                open_link = Some("https://ocr.space/ocrapi");
                            }
                        });
                        ui.label("• กรอก Email, First Name, Last Name และยอมรับข้อตกลง");
                        ui.label(
                            "• กด Register แล้วรอ API Key ทาง Email จากนั้นคัดลอกมาใส่ช่อง OCR.space API key",
                        );
                        ui.add_space(5.0);
                        ui.strong("2. Google Gemini API ผ่าน Google AI Studio");
                        ui.label("• ฟรีประมาณ 1,500 ครั้ง/วัน");
                        ui.horizontal_wrapped(|ui| {
                            ui.label("• เข้า");
                            if ui.link("aistudio.google.com").clicked() {
                                open_link = Some("https://aistudio.google.com");
                            }
                            ui.label("และล็อกอินด้วย Gmail");
                        });
                        ui.label(
                            "• กด Get API key → Create API key → คัดลอก API Key มาใส่ช่อง Gemini API key",
                        );
                        ui.label("• ไม่ต้องกรอกข้อมูลบัตรเครดิต");
                    });
                ui.horizontal(|ui| {
                    if ui.link("เปิดหน้า OCR.space API").clicked() {
                        open_link = Some("https://ocr.space/ocrapi/freekey");
                    }
                    ui.label("·");
                    if ui.link("เปิด Google AI Studio").clicked() {
                        open_link = Some("https://aistudio.google.com/apikey");
                    }
                });
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_text_button(ui, &check_icon, "Save").clicked() {
                        save = true;
                    }
                    if icon_text_button(ui, &close_icon, "Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        });
        if let Some(url) = open_link
            && let Err(error) = platform::open_url(url)
        {
            self.status = format!("เปิดลิงก์ไม่สำเร็จ: {error}");
        }
        if save {
            let mut validation = Ok(());
            for (name, key, selected) in [
                (
                    "OCR.space",
                    self.settings_draft.ocrspace_key.as_str(),
                    self.settings_draft.engine == "ocrspace",
                ),
                (
                    "Gemini",
                    self.settings_draft.gemini_key.as_str(),
                    self.settings_draft.engine == "gemini",
                ),
            ] {
                if (selected || !key.is_empty()) && validation.is_ok() {
                    validation = config::validate_key(name, key);
                }
            }
            match validation {
                Ok(()) => {
                    self.settings_draft.ocrspace_engine = "3".into();
                    self.settings_draft.gemini_model = "gemini-flash-latest".into();
                    self.config = self.settings_draft.clone();
                    self.save_config();
                    apply_main_font_size(ctx, self.config.main_font_size);
                    self.batch_id += 1;
                    self.batch_running = None;
                    self.status = "บันทึก Settings แล้ว · กดแปลงภาพเป็นข้อความเมื่อพร้อม".into();
                    close = true;
                }
                Err(error) => self.status = format!("ตั้งค่า: {error}"),
            }
        }
        if cancel || close {
            self.settings_open = false;
        }
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.history_window(ctx);
        if self.about_open {
            egui::Window::new("About - แปลงภาพเป็นข้อความ")
                .open(&mut self.about_open)
                .show(ctx, |ui| {
                    ui.strong("แปลงภาพเป็นข้อความ");
                    ui.add_space(8.0);
                    ui.label("โปรแกรมสำหรับ แปลงภาพเป็นข้อความ");
                    ui.label("พัฒนาโดย Tussarat Yawichai");
                    ui.label(format!("วันที่ {APP_UPDATED_DATE}"));
                    ui.label(format!("Version {APP_VERSION}"));
                });
        }
        self.confirm_window(ctx);
    }

    fn welcome_viewport(&mut self, ctx: &egui::Context) {
        if !self.welcome_open {
            return;
        }
        let builder = egui::ViewportBuilder::default()
            .with_title("แปลงภาพเป็นข้อความ")
            .with_inner_size([500.0, 800.0])
            .with_resizable(false)
            .with_clamp_size_to_monitor_size(true)
            .with_always_on_top()
            .with_icon(self.window_icon.clone());
        let mut close = false;
        let mut configure_api = false;
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("welcome-window"),
            builder,
            |root, _| {
                let welcome_ctx = root.ctx().clone();
                if welcome_ctx.input(|input| input.viewport().close_requested()) {
                    close = true;
                    return;
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(Color32::WHITE).inner_margin(24.0))
                    .show(root, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(ui.available_height())
                            .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            ui.add(
                                egui::Image::new(&self.app_icon_texture)
                                    .fit_to_exact_size(Vec2::splat(150.0)),
                            );
                            ui.heading(
                                egui::RichText::new("แปลงภาพเป็นข้อความ")
                                    .size(25.0)
                                    .color(Color32::from_rgb(21, 90, 128)),
                            );
                            ui.label(
                                egui::RichText::new(
                                    format!(
                                        "Version {APP_VERSION}  •  พัฒนาโดย Tussarat Yawichai  •  {APP_UPDATED_DATE}"
                                    ),
                                )
                                .size(11.0)
                                .color(Color32::from_rgb(82, 96, 109)),
                            );
                            ui.add_space(8.0);
                            ui.strong(
                                egui::RichText::new("พร้อมใช้งาน")
                                    .size(14.0)
                                    .color(Color32::from_rgb(21, 90, 128)),
                            );
                            ui.add(egui::ProgressBar::new(1.0).show_percentage());
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(255, 247, 220))
                                .show(ui, |ui| {
                                    ui.strong("ก่อนใช้งานครั้งแรก ต้องตั้งค่า API Key ใน Settings");
                                    ui.label("เลือก OCR engine และกรอก API Key ให้ตรงกับบริการที่จะใช้ · Alt+A ใช้ Engine ที่เลือก · Alt+D ใช้ Gemini แปลเป็นไทย");
                                    if ui.button("ตั้งค่า API ตอนนี้").clicked() {
                                        configure_api = true;
                                        close = true;
                                    }
                                });
                            ui.add_space(8.0);
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(244, 248, 251))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.strong("วิธีใช้งานแบบย่อ");
                                    ui.label("• กล้อง: กด ‘เปิดกล้อง’ ก่อน แล้วคลิกภาพหรือกด Spacebar เพื่อถ่าย");
                                    ui.label("• หน้าจอ: กด Alt+S");
                                    ui.label("• Capture Translate: กด Alt+D แล้วลากข้อความ 1 จุดเพื่อแปลเป็นไทยด้วย Gemini");
                                    ui.label("  ผลต้นฉบับและคำแปลจะแสดงข้างไอคอนลอยจนกด ×");
                                    ui.label("• OCR วางทันที: ชี้เมาส์ที่ช่องข้อความ กด Alt+A แล้วลากครอบ · ระบบคลิกตำแหน่งเดิมเพื่อวาง (มี Clipboard สำรอง)");
                                    ui.label("• เมนูไอคอนลอย/Tray: เลือก OCR to Clipboard (Alt+A) เพื่อคัดลอกผลลง Clipboard โดยไม่คลิกหรือวางอัตโนมัติ");
                                    ui.label(
                                        "• ลากครั้งแรกเพื่อกำหนด Scope จากนั้นลาก Crop ได้หลายรายการ",
                                    );
                                    ui.label(
                                        "• กด Enter เพื่อแปลงทันที · คลิกขวาหรือ Esc เพื่อยกเลิก Alt+S",
                                    );
                                    ui.label("• ผลลัพธ์จะรวมส่ง OCR แบบ Batch เพื่อลดจำนวน API calls");
                                    ui.label("• ใช้ Alt+1…9 เพื่อวางข้อความลงโปรแกรมปลายทาง");
                                });
                            ui.add_space(12.0);
                            if ui.button("ปิดหน้าต่างนี้").clicked() {
                                close = true;
                            }
                        });
                            });
                    });
            },
        );
        if close {
            self.welcome_open = false;
        }
        if configure_api {
            self.open_settings();
        }
    }

    fn translation_viewport(&mut self, ctx: &egui::Context) {
        if !self.translation_open {
            return;
        }
        let popup_id = egui::ViewportId::from_hash_of("translate-text-popup");
        let mut builder = egui::ViewportBuilder::default()
            .with_title("Capture Translate")
            .with_inner_size([420.0, 220.0])
            .with_min_inner_size([280.0, 160.0])
            .with_clamp_size_to_monitor_size(true)
            .with_decorations(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_icon(self.window_icon.clone());
        if let Some(position) = self.translation_position.take() {
            builder = builder.with_position(position);
        }
        let mut close = false;
        let close_icon = self.ui_icons.close.clone();
        let translation_text = self.translation_text.clone();
        let text_font_size = self.config.sticky_font_size as f32;
        ctx.show_viewport_immediate(popup_id, builder, |root, _| {
            let popup_ctx = root.ctx().clone();
            let (monitor_size, inner_rect) = popup_ctx
                .input(|input| (input.viewport().monitor_size, input.viewport().inner_rect));
            let current_size = inner_rect.map_or(Vec2::new(420.0, 220.0), |rect| rect.size());
            let width = monitor_size.map_or(current_size.x, |size| {
                current_size.x.min((size.x - 24.0).max(280.0))
            });
            let text_width = (width - 24.0).max(80.0);
            let display_text = translation_text.as_ref().map(|(source, thai)| {
                let source = if source.is_empty() {
                    "(ไม่พบข้อความ)"
                } else {
                    source.as_str()
                };
                let thai = if thai.is_empty() {
                    "(ไม่พบคำแปล)"
                } else {
                    thai.as_str()
                };
                (source, thai)
            });
            let (source_height, thai_height) = display_text.map_or((0.0, 0.0), |(source, thai)| {
                popup_ctx.fonts_mut(|fonts| {
                    let font = egui::FontId::proportional(text_font_size);
                    let color = Color32::from_rgb(29, 39, 48);
                    let source_height = fonts
                        .layout(source.to_owned(), font.clone(), color, text_width)
                        .size()
                        .y;
                    let thai_height = fonts
                        .layout(thai.to_owned(), font, color, text_width)
                        .size()
                        .y;
                    (source_height, thai_height)
                })
            });
            // ponytail: keep the text popup compact and scroll long translations instead of covering the screen.
            let max_height =
                monitor_size.map_or(300.0, |size| (size.y * 0.85).min(300.0).max(160.0));
            let desired_height = (source_height + thai_height + 112.0)
                .max(180.0)
                .min(max_height);
            if (current_size.x - width).abs() > 2.0 || (current_size.y - desired_height).abs() > 2.0
            {
                popup_ctx.send_viewport_cmd(ViewportCommand::InnerSize(Vec2::new(
                    width,
                    desired_height,
                )));
            }
            if popup_ctx.input(|input| input.viewport().close_requested()) {
                close = true;
                return;
            }
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(Color32::WHITE)
                        .stroke(Stroke::new(1.0, Color32::from_rgb(178, 188, 198)))
                        .corner_radius(4.0)
                        .inner_margin(8.0),
                )
                .show(root, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Capture Translate")
                                .small()
                                .color(Color32::from_rgb(90, 102, 114)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_sized(
                                    [24.0, 24.0],
                                    egui::Button::image(
                                        egui::Image::new(&close_icon)
                                            .fit_to_exact_size(Vec2::splat(16.0))
                                            .tint(Color32::WHITE),
                                    )
                                    .fill(Color32::from_rgb(217, 83, 79))
                                    .corner_radius(3.0),
                                )
                                .on_hover_text("ปิดผลแปล")
                                .clicked()
                            {
                                close = true;
                            }
                        });
                    });
                    ui.separator();
                    if let Some((source, thai)) = display_text {
                        egui::ScrollArea::vertical()
                            .max_height((desired_height - 64.0).max(100.0))
                            .show(ui, |ui| {
                                if source.starts_with("ERROR:") {
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(source)
                                                .size(text_font_size)
                                                .color(Color32::from_rgb(176, 35, 35)),
                                        )
                                        .selectable(true),
                                    );
                                } else {
                                    ui.label(
                                        egui::RichText::new("ข้อความต้นฉบับ")
                                            .small()
                                            .strong()
                                            .color(Color32::from_rgb(90, 102, 114)),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(source)
                                                .size(text_font_size)
                                                .color(Color32::from_rgb(29, 39, 48)),
                                        )
                                        .selectable(true),
                                    );
                                    ui.separator();
                                    ui.label(
                                        egui::RichText::new("คำแปลภาษาไทย")
                                            .small()
                                            .strong()
                                            .color(Color32::from_rgb(21, 90, 128)),
                                    );
                                    ui.add(
                                        egui::Label::new(
                                            egui::RichText::new(thai)
                                                .size(text_font_size)
                                                .color(Color32::from_rgb(29, 39, 48)),
                                        )
                                        .selectable(true),
                                    );
                                }
                            });
                    }
                });
        });
        if close {
            self.translation_open = false;
            ctx.send_viewport_cmd_to(popup_id, ViewportCommand::Close);
        }
    }

    fn history_window(&mut self, ctx: &egui::Context) {
        if !self.history_open {
            return;
        }
        let entries = history::entries(&self.history_dir);
        for path in &entries {
            if !self.history_thumbnails.contains_key(path)
                && let Ok(image) = history::load_image(path)
            {
                let thumb = image::imageops::thumbnail(&image, 180, 120);
                self.history_thumbnails.insert(
                    path.clone(),
                    Self::load_texture(ctx, format!("history-thumb-{}", path.display()), &thumb),
                );
            }
        }
        let mut open = self.history_open;
        let mut choose = false;
        let mut delete = false;
        let mut cancel = false;
        let delete_icon = self.ui_icons.delete.clone();
        let check_icon = self.ui_icons.check.clone();
        let close_icon = self.ui_icons.close.clone();
        let window_rect = ctx.content_rect().shrink(8.0);
        egui::Window::new("ประวัติภาพถ่าย")
            .open(&mut open)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .fixed_pos(window_rect.min)
            .fixed_size(window_rect.size())
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("ประวัติภาพถ่าย");
                    ui.separator();
                    ui.strong(format!("ทั้งหมด {} ภาพ", entries.len()));
                    ui.label(format!("เลือกแล้ว {} ภาพ", self.history_selection.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_text_button(ui, &close_icon, "ปิด").clicked() {
                            cancel = true;
                        }
                        if ui
                            .add_enabled_ui(self.history_current.is_some(), |ui| {
                                icon_text_button(ui, &check_icon, "เปิดภาพ")
                            })
                            .inner
                            .clicked()
                        {
                            choose = true;
                        }
                        if ui
                            .add_enabled_ui(!self.history_selection.is_empty(), |ui| {
                                icon_text_button(ui, &delete_icon, "ลบที่เลือก")
                            })
                            .inner
                            .clicked()
                        {
                            delete = true;
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if entries.is_empty() {
                            ui.vertical_centered(|ui| {
                                ui.add_space(80.0);
                                ui.heading("ยังไม่มีภาพใน History");
                            });
                            return;
                        }
                        let card_width = 172.0;
                        let columns = ((ui.available_width() + 10.0) / (card_width + 10.0))
                            .floor()
                            .max(1.0) as usize;
                        egui::Grid::new("history-gallery-grid")
                            .num_columns(columns)
                            .spacing([10.0, 12.0])
                            .show(ui, |ui| {
                                for (index, path) in entries.iter().enumerate() {
                                    let selected = self.history_selection.contains(path);
                                    ui.allocate_ui(Vec2::new(card_width, 156.0), |ui| {
                                        ui.set_width(card_width);
                                        ui.vertical_centered(|ui| {
                                            let response = if let Some(texture) =
                                                self.history_thumbnails.get(path)
                                            {
                                                ui.add(
                                                    egui::Button::image(
                                                        egui::Image::new(texture)
                                                            .fit_to_exact_size(Vec2::new(
                                                                160.0, 106.0,
                                                            )),
                                                    )
                                                    .selected(selected),
                                                )
                                            } else {
                                                ui.add_sized(
                                                    [160.0, 106.0],
                                                    egui::Button::new("โหลดภาพไม่ได้")
                                                        .selected(selected),
                                                )
                                            }
                                            .on_hover_text(path.display().to_string());
                                            if response.double_clicked() {
                                                self.history_current = Some(path.clone());
                                                self.history_selection.insert(path.clone());
                                                choose = true;
                                            } else if response.clicked() {
                                                if selected {
                                                    self.history_selection.remove(path);
                                                    if self.history_current.as_ref() == Some(path) {
                                                        self.history_current = None;
                                                    }
                                                } else {
                                                    self.history_selection.insert(path.clone());
                                                    self.history_current = Some(path.clone());
                                                }
                                            }
                                            ui.add_sized(
                                                [160.0, 34.0],
                                                egui::Label::new(
                                                    egui::RichText::new(history::display_name(
                                                        path,
                                                    ))
                                                    .small(),
                                                )
                                                .wrap(),
                                            );
                                        });
                                    });
                                    if (index + 1) % columns == 0 {
                                        ui.end_row();
                                    }
                                }
                            });
                    });
            });
        if cancel {
            open = false;
        }
        if delete {
            if self.history_selection.is_empty() {
                self.status = "ประวัติ: กรุณาเลือกภาพที่ต้องการลบ".into();
            } else {
                self.ask_confirm(
                    "ยืนยันการลบ",
                    format!(
                        "ต้องการลบภาพ {} รายการหรือไม่?\n\nการกระทำนี้ไม่สามารถย้อนกลับได้",
                        self.history_selection.len()
                    ),
                    ConfirmAction::DeleteHistory,
                );
            }
        }
        if choose {
            if let Some(path) = self.history_current.clone() {
                match history::load_image(&path) {
                    Ok(image) => {
                        let restored: Vec<Roi> = history::load_state(&path)
                            .into_iter()
                            .filter_map(|crop| {
                                Roi {
                                    x: crop.x,
                                    y: crop.y,
                                    width: crop.width,
                                    height: crop.height,
                                    text: crop.text,
                                }
                                .clipped(&image)
                            })
                            .collect();
                        self.set_document(image, Some(path.clone()), ctx);
                        self.rois = restored;
                        self.rebuild_roi_previews(ctx);
                        self.persist_state();
                        self.status = format!("เปิด History: {}", history::display_name(&path));
                        open = false;
                    }
                    Err(error) => self.status = format!("เปิดภาพไม่สำเร็จ: {error}"),
                }
            } else {
                self.status = "ประวัติ: กรุณาเลือกภาพก่อน".into();
            }
        }
        self.history_open = open;
    }

    fn delete_selected_history(&mut self, _ctx: &egui::Context) {
        let paths: Vec<_> = self.history_selection.drain().collect();
        let mut deleted = 0usize;
        let mut failed = Vec::new();
        for path in paths {
            match history::delete(&path) {
                Ok(()) => {
                    deleted += 1;
                    self.history_thumbnails.remove(&path);
                    if self.history_current.as_ref() == Some(&path) {
                        self.history_current = None;
                    }
                    if self.current_history.as_ref() == Some(&path) {
                        self.current_history = None;
                    }
                }
                Err(error) => failed.push(format!(
                    "{}: {error}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                )),
            }
        }
        self.status = if failed.is_empty() {
            format!("ลบภาพ {deleted} รายการเรียบร้อยแล้ว")
        } else {
            format!("ลบไม่สำเร็จบางรายการ: {}", failed.join(" | "))
        };
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.confirm else { return };
        let title = dialog.title.clone();
        let text = dialog.text.clone();
        let action = dialog.action;
        let mut yes = false;
        let mut no = false;
        let check_icon = self.ui_icons.check.clone();
        let close_icon = self.ui_icons.close.clone();
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(text);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if icon_text_button(ui, &check_icon, "Yes").clicked() {
                        yes = true;
                    }
                    if icon_text_button(ui, &close_icon, "No").clicked() {
                        no = true;
                    }
                });
            });
        if yes {
            self.confirm = None;
            self.execute_confirm(action, ctx);
        } else if no {
            self.confirm = None;
        }
    }

    fn main_viewport(&mut self, ctx: &egui::Context) {
        if !self.main_visible {
            return;
        }
        let viewport_id = egui::ViewportId::from_hash_of("roi-ocr-main-window");
        ctx.send_viewport_cmd_to(viewport_id, ViewportCommand::Visible(true));
        let builder = egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([1180.0, 720.0])
            .with_min_inner_size([760.0, 480.0])
            .with_window_level(if self.main_always_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            })
            .with_icon(self.window_icon.clone());
        let mut hide_main = false;
        ctx.show_viewport_immediate(viewport_id, builder, |root, _class| {
            let main_ctx = root.ctx().clone();
            let close_requested = main_ctx.input(|input| input.viewport().close_requested());
            let minimized = main_ctx.input(|input| input.viewport().minimized) == Some(true);
            if close_requested || minimized {
                if close_requested {
                    main_ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                }
                if minimized {
                    main_ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                }
                main_ctx.send_viewport_cmd(ViewportCommand::Visible(false));
                hide_main = true;
                return;
            }
            if main_ctx.input(|input| input.key_pressed(egui::Key::Space))
                && self.document.is_none()
            {
                self.capture_photo(&main_ctx);
            }
            if main_ctx.input(|input| input.key_pressed(egui::Key::Enter)) && !self.rois.is_empty()
            {
                self.convert_all();
            }
            egui::Panel::bottom("status").show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.label(&self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(if self.config.engine == "gemini" {
                            "OCR: Gemini · แม่นยำ"
                        } else {
                            "OCR: OCR.Space · เร็ว"
                        });
                    });
                });
            });
            egui::Panel::right("roi")
                .resizable(true)
                .default_size(430.0)
                .show(root, |ui| {
                    ui.horizontal(|ui| {
                        let icon = if self.camera_panel_visible {
                            &self.ui_icons.chevron_left
                        } else {
                            &self.ui_icons.chevron_right
                        };
                        let canvas_label = if self.camera_panel_visible {
                            "ซ่อนภาพ"
                        } else {
                            "แสดงภาพ"
                        };
                        if icon_text_button(ui, icon, canvas_label)
                            .on_hover_text("ซ่อนหรือแสดงพื้นที่ภาพ")
                            .clicked()
                        {
                            self.camera_panel_visible = !self.camera_panel_visible;
                            main_ctx.send_viewport_cmd(ViewportCommand::InnerSize(
                                if self.camera_panel_visible {
                                    Vec2::new(1180.0, 720.0)
                                } else {
                                    Vec2::new(430.0, 720.0)
                                },
                            ));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .toggle_value(&mut self.main_always_on_top, "📌 ปักหมุด")
                                .on_hover_text("เปิด/ปิดการแสดงหน้าต่างหลักไว้บนสุด")
                                .changed()
                            {
                                main_ctx.send_viewport_cmd(ViewportCommand::WindowLevel(
                                    if self.main_always_on_top {
                                        egui::WindowLevel::AlwaysOnTop
                                    } else {
                                        egui::WindowLevel::Normal
                                    },
                                ));
                            }
                        });
                    });
                    self.roi_panel(ui, &main_ctx);
                });
            if self.camera_panel_visible {
                egui::Panel::top("top").show(root, |ui| self.top_bar(ui, &main_ctx));
                egui::CentralPanel::default().show(root, |ui| {
                    self.image_panel(ui, &main_ctx);
                    if self.document.is_some() {
                        ui.vertical_centered(|ui| {
                            ui.strong("ให้ลากตีกรอบข้อความที่ต้องการ");
                        });
                    }
                });
            } else {
                egui::CentralPanel::default().show(root, |_| {});
            }
            self.windows(&main_ctx);
        });
        if hide_main {
            self.main_visible = false;
            self.welcome_open = false;
            self.camera_panel_visible = true;
            ctx.send_viewport_cmd_to(
                viewport_id,
                ViewportCommand::InnerSize(Vec2::new(1180.0, 720.0)),
            );
        }
    }
}

impl eframe::App for RoiOcrApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(30));
        if ctx.input(|input| input.viewport().minimized) == Some(true) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        }
        self.process_hotkeys(ctx);
        self.process_tray(ctx);
        self.poll_workers(ctx);
        self.poll_camera(ctx);
    }

    fn ui(&mut self, root: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.floating_launcher(root, &ctx, frame);
        if ctx.input(|input| input.viewport().close_requested()) && !self.exit_requested {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
        }
        self.capture_overlay(&ctx);
        self.welcome_viewport(&ctx);
        self.translation_viewport(&ctx);
        self.main_viewport(&ctx);
        self.settings_viewport(&ctx);
    }
}

fn launcher_hwnd(frame: &eframe::Frame) -> Option<isize> {
    let window = frame.window_handle().ok()?;
    match window.as_raw() {
        RawWindowHandle::Win32(window) => Some(window.hwnd.get()),
        _ => None,
    }
}

fn roi_to_screen(roi: &Roi, rect: Rect, image_size: Vec2) -> Rect {
    Rect::from_min_max(
        Pos2::new(
            rect.left() + roi.x as f32 / image_size.x * rect.width(),
            rect.top() + roi.y as f32 / image_size.y * rect.height(),
        ),
        Pos2::new(
            rect.left() + (roi.x + roi.width) as f32 / image_size.x * rect.width(),
            rect.top() + (roi.y + roi.height) as f32 / image_size.y * rect.height(),
        ),
    )
}

fn screen_to_roi(selection: Rect, image_rect: Rect, image_size: Vec2) -> Option<Roi> {
    if selection.width() <= 4.0 || selection.height() <= 4.0 {
        return None;
    }
    Some(Roi {
        x: ((selection.left() - image_rect.left()) / image_rect.width() * image_size.x)
            .round()
            .max(0.0) as u32,
        y: ((selection.top() - image_rect.top()) / image_rect.height() * image_size.y)
            .round()
            .max(0.0) as u32,
        width: (selection.width() / image_rect.width() * image_size.x).round() as u32,
        height: (selection.height() / image_rect.height() * image_size.y).round() as u32,
        text: String::new(),
    })
}

fn install_multilingual_fonts(ctx: &egui::Context) {
    // ponytail: use installed Windows fonts to avoid bundling large font files; add candidates as needed.
    let font_groups: [(&str, &[&str]); 13] = [
        (
            "thai",
            &[
                r"C:\Windows\Fonts\LeelawUI.ttf",
                r"C:\Windows\Fonts\tahoma.ttf",
            ],
        ),
        (
            "cjk",
            &[
                r"C:\Windows\Fonts\simhei.ttf",
                r"C:\Windows\Fonts\msyh.ttc",
                r"C:\Windows\Fonts\simsun.ttc",
                r"C:\Windows\Fonts\SimsunExtG.ttf",
            ],
        ),
        (
            "japanese",
            &[
                r"C:\Windows\Fonts\YuGothR.ttc",
                r"C:\Windows\Fonts\msgothic.ttc",
            ],
        ),
        ("korean", &[r"C:\Windows\Fonts\malgun.ttf"]),
        ("indic", &[r"C:\Windows\Fonts\Nirmala.ttc"]),
        (
            "arabic_hebrew",
            &[
                r"C:\Windows\Fonts\tahoma.ttf",
                r"C:\Windows\Fonts\arial.ttf",
            ],
        ),
        ("myanmar", &[r"C:\Windows\Fonts\mmrtext.ttf"]),
        (
            "african_scripts",
            &[
                r"C:\Windows\Fonts\ebrima.ttf",
                r"C:\Windows\Fonts\gadugi.ttf",
            ],
        ),
        (
            "extended_latin",
            &[
                r"C:\Windows\Fonts\segoeui.ttf",
                r"C:\Windows\Fonts\arial.ttf",
            ],
        ),
        ("symbols", &[r"C:\Windows\Fonts\seguisym.ttf"]),
        ("khmer", &[r"C:\Windows\Fonts\KhmerUI.ttf"]),
        ("lao", &[r"C:\Windows\Fonts\LaoUI.ttf"]),
        ("ethiopic", &[r"C:\Windows\Fonts\Nyala.ttf"]),
    ];
    let mut fonts = FontDefinitions::default();
    let mut fallback_font_names = Vec::new();
    for (name, candidates) in font_groups {
        if let Some(bytes) = candidates.iter().find_map(|path| std::fs::read(path).ok()) {
            fonts
                .font_data
                .insert(name.into(), FontData::from_owned(bytes).into());
            fallback_font_names.push(name.to_owned());
        }
    }
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let family_fonts = fonts.families.entry(family).or_default();
        for (index, name) in fallback_font_names.iter().enumerate() {
            family_fonts.insert(index, name.clone());
        }
    }
    ctx.set_fonts(fonts);
}

fn apply_main_font_size(ctx: &egui::Context, size: u32) {
    let size = size.clamp(9, 32) as f32;
    ctx.all_styles_mut(|style| {
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional((size - 2.0).max(8.0)),
        );
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(size));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(size));
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(size + 6.0),
        );
        style
            .text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(size));
    });
}

fn setup_hotkeys() -> (
    Option<GlobalHotKeyManager>,
    HashMap<u32, String>,
    Option<String>,
) {
    let Ok(manager) = GlobalHotKeyManager::new() else {
        return (
            None,
            HashMap::new(),
            Some("สร้าง Global Hotkey manager ไม่สำเร็จ".into()),
        );
    };
    let mut actions = HashMap::new();
    let digit_codes = [
        Code::Digit1,
        Code::Digit2,
        Code::Digit3,
        Code::Digit4,
        Code::Digit5,
        Code::Digit6,
        Code::Digit7,
        Code::Digit8,
        Code::Digit9,
    ];
    let mut failed = Vec::new();
    for (index, code) in digit_codes.into_iter().enumerate() {
        let hotkey = HotKey::new(Some(Modifiers::ALT), code);
        if manager.register(hotkey).is_ok() {
            actions.insert(hotkey.id(), (index + 1).to_string());
        } else {
            failed.push(format!("Alt+{}", index + 1));
        }
    }
    for (code, name, action) in [
        (Code::KeyS, "Alt+S", "capture"),
        (Code::KeyA, "Alt+A", "ocr-paste"),
        (Code::KeyD, "Alt+D", "translate"),
    ] {
        let hotkey = HotKey::new(Some(Modifiers::ALT), code);
        if manager.register(hotkey).is_ok() {
            actions.insert(hotkey.id(), action.into());
        } else {
            failed.push(name.into());
        }
    }
    let error =
        (!failed.is_empty()).then(|| format!("Hotkey ถูกโปรแกรมอื่นใช้: {}", failed.join(", ")));
    (Some(manager), actions, error)
}

fn icon_text_button(ui: &mut egui::Ui, texture: &TextureHandle, text: &str) -> egui::Response {
    ui.add(egui::Button::image_and_text(
        egui::Image::new(texture).fit_to_exact_size(Vec2::splat(18.0)),
        text,
    ))
}

fn camera_icon_texture(ctx: &egui::Context) -> TextureHandle {
    let size = 24;
    let mut rgba = vec![0_u8; size * size * 4];
    let mut set_pixel = |x: usize, y: usize| {
        let offset = (y * size + x) * 4;
        rgba[offset..offset + 4].copy_from_slice(&[48, 53, 60, 255]);
    };
    for y in 8..20 {
        for x in 3..21 {
            if x < 5 || x >= 19 || y < 10 || y >= 18 {
                set_pixel(x, y);
            }
        }
    }
    for y in 5..9 {
        for x in 8..15 {
            if y < 7 || x == 8 || x == 14 {
                set_pixel(x, y);
            }
        }
    }
    for y in 10..18 {
        for x in 8..17 {
            let dx = x as i32 - 12;
            let dy = y as i32 - 14;
            let distance = dx * dx + dy * dy;
            if (7..=20).contains(&distance) {
                set_pixel(x, y);
            }
        }
    }
    let image = ColorImage::from_rgba_unmultiplied([size, size], &rgba);
    ctx.load_texture("icon-camera", image, TextureOptions::LINEAR)
}

fn load_bundled_texture(ctx: &egui::Context, name: &str, bytes: &[u8]) -> TextureHandle {
    let image = image::load_from_memory(bytes)
        .expect("bundled UI icon must be a valid image")
        .to_rgba8();
    ctx.load_texture(
        name,
        ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        ),
        TextureOptions::LINEAR,
    )
}

fn menu_icon(bytes: &[u8]) -> Option<MenuIcon> {
    image::load_from_memory(bytes).ok().and_then(|image| {
        let rgba = image.to_rgba8();
        MenuIcon::from_rgba(rgba.into_raw(), image.width(), image.height()).ok()
    })
}

fn build_action_menu() -> (Menu, TrayActions) {
    let menu = Menu::new();
    let show = IconMenuItem::new(
        "แสดงโปรแกรม",
        true,
        menu_icon(include_bytes!("../../assets/icons/open.png")),
        None,
    );
    let welcome = IconMenuItem::new(
        "Welcome / วิธีใช้งาน",
        true,
        menu_icon(include_bytes!("../../assets/icons/help.png")),
        None,
    );
    let capture = IconMenuItem::new(
        "จับภาพหน้าจอ (Alt+S)",
        true,
        menu_icon(include_bytes!("../../assets/icons/capture.png")),
        None,
    );
    let ocr_clipboard = IconMenuItem::new(
        "OCR to Clipboard (Alt+A)",
        true,
        menu_icon(include_bytes!("../../assets/icons/copy.png")),
        None,
    );
    let translate = IconMenuItem::new(
        "Capture Translate (Alt+D)",
        true,
        menu_icon(include_bytes!("../../assets/icons/translate.png")),
        None,
    );
    let history = IconMenuItem::new(
        "เปิด History",
        true,
        menu_icon(include_bytes!("../../assets/icons/history.png")),
        None,
    );
    let settings = IconMenuItem::new(
        "Settings",
        true,
        menu_icon(include_bytes!("../../assets/icons/settings.png")),
        None,
    );
    let exit = IconMenuItem::new(
        "ออกจากโปรแกรม",
        true,
        menu_icon(include_bytes!("../../assets/icons/exit.png")),
        None,
    );
    let separator1 = PredefinedMenuItem::separator();
    let separator2 = PredefinedMenuItem::separator();
    let separator3 = PredefinedMenuItem::separator();
    let _ = menu.append_items(&[
        &show,
        &welcome,
        &separator1,
        &capture,
        &ocr_clipboard,
        &translate,
        &history,
        &separator2,
        &settings,
        &separator3,
        &exit,
    ]);
    let actions = TrayActions {
        show: show.id().clone(),
        welcome: welcome.id().clone(),
        capture: capture.id().clone(),
        ocr_clipboard: ocr_clipboard.id().clone(),
        translate: translate.id().clone(),
        history: history.id().clone(),
        settings: settings.id().clone(),
        exit: exit.id().clone(),
    };
    (menu, actions)
}

fn setup_tray() -> (Option<TrayIcon>, TrayActions) {
    let (menu, actions) = build_action_menu();
    let icon = image::load_from_memory(include_bytes!("../../assets/app_icon.png"))
        .ok()
        .and_then(|image| {
            let rgba = image.to_rgba8();
            Icon::from_rgba(rgba.into_raw(), image.width(), image.height()).ok()
        });
    let mut builder = TrayIconBuilder::new()
        .with_tooltip(APP_NAME)
        .with_menu(Box::new(menu));
    if let Some(icon) = icon {
        builder = builder.with_icon(icon);
    }
    (builder.build().ok(), actions)
}
