use crate::{
    audio::{self, InputDevice, InputMode, Playback, RecordSettings, Recorder},
    export::{self, ExportResult, ExportSettings, Format},
    host,
    model::{Clip, History, PEAK_BLOCK, Peak, Project, TrackDrop, time_label},
    session,
    theme::{self, *},
};
use eframe::egui::{self, *};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

struct ExportJob {
    rx: mpsc::Receiver<Result<Option<ExportResult>, String>>,
    progress: Arc<AtomicU32>,
}
#[derive(Clone, Copy)]
enum DragKind {
    Move,
    FadeIn,
    FadeOut,
    TrimLeft,
    TrimRight,
    Gain,
}
struct ClipDrag {
    id: u64,
    original: Clip,
    kind: DragKind,
    target: TrackDrop,
    lanes_top: f32,
    track_count: usize,
    delta: Vec2,
    press: Pos2,
    view_at_press: f64,
    zoom_at_press: f32,
    end_before: f64,
    originals: Vec<Clip>,
    clip_height: f32,
}

#[derive(Clone, Copy)]
enum RenameTarget {
    Track(usize),
    Clip(u64),
}

struct Marquee {
    start_time: f64,
    start_y: f32,
    base: std::collections::BTreeSet<u64>,
}

#[derive(Clone, Copy)]
struct SnapGuide {
    time: f64,
    target: Option<u64>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct TestGeometry {
    origin: f32,
    lanes_top: f32,
    row_h: f32,
}

pub struct SoriApp {
    project: Project,
    history: History,
    record_settings: RecordSettings,
    export_settings: ExportSettings,
    devices: Vec<InputDevice>,
    device: usize,
    recorder: Option<Recorder>,
    playback: Option<Playback>,
    rec_start: f64,
    rec_track: usize,
    rec_id: u64,
    live_peaks: Vec<Peak>,
    cursor: f64,
    active_track: usize,
    selected: Option<u64>,
    clipboard: Vec<(Clip, Arc<crate::model::Source>)>,
    selection: std::collections::BTreeSet<u64>,
    drag: Option<ClipDrag>,
    zoom: f32,
    last_fit_zoom: f32,
    view_start: f64,
    snap: bool,
    level: f32,
    pending_edit: Option<Project>,
    settings_open: bool,
    export_open: bool,
    export_job: Option<ExportJob>,
    message: String,
    message_error: bool,
    message_at: Instant,
    last_export: Option<PathBuf>,
    host: Option<String>,
    host_export_dir: Option<PathBuf>,
    host_note: Option<String>,
    takes: Vec<PathBuf>,
    exports: Vec<PathBuf>,
    demo: bool,
    close_dialog: bool,
    allow_close: bool,
    pan: Option<(Pos2, f64, f32)>,
    scroll_y: f32,
    capture_path: Option<PathBuf>,
    capture_frames: u32,
    capture_sent: bool,
    seeking: bool,
    rename_target: Option<RenameTarget>,
    rename_text: String,
    rename_focus: bool,
    marquee: Option<Marquee>,
    snap_guide: Option<SnapGuide>,
    snap_release: Option<Instant>,
    #[cfg(test)]
    test_geometry: Option<TestGeometry>,
    #[cfg(test)]
    test_rects: Vec<(String, Rect)>,
}

impl SoriApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::setup(&cc.egui_ctx);
        let args: Vec<_> = std::env::args().collect();
        let capture_path = args
            .windows(2)
            .find(|a| a[0] == "--screenshot")
            .map(|a| PathBuf::from(&a[1]));
        let demo = args.iter().any(|v| v == "--demo") || capture_path.is_some();
        let mut project = Project::default();
        let mut record_settings = RecordSettings::default();
        let mut export_settings = ExportSettings::default();
        let mut message = String::new();
        let mut message_error = false;
        if demo {
            project = export::demo_project();
        } else {
            match session::load() {
                Ok(Some((p, r, e))) => {
                    project = p;
                    record_settings = r;
                    export_settings = e;
                }
                Ok(None) => {}
                Err(e) => {
                    message = format!("지난 작업 복구: {e}");
                    message_error = true;
                }
            }
        }
        for c in &mut project.clips {
            if c.fade_in + c.fade_out > c.envelope_len {
                c.fade_out = (c.envelope_len - c.fade_in).max(0.0);
            }
        }
        project.resolve_overlaps();
        project.compact_tracks();
        if capture_path.is_some() && project.tracks.len() > 1 {
            project.tracks[1].muted = true;
        }
        let preview_selected = if capture_path.is_some() {
            project.clips.first().map(|c| c.id)
        } else {
            None
        };
        let host = host::host();
        let host_export_dir = host::export_dir();
        let host_note = host.as_ref().map(|name| match &host_export_dir {
            Some(dir) => format!(
                "{name} — 녹음이 끝나면 창을 닫으세요 · 내보내기: {}",
                dir.display()
            ),
            None => format!("{name} — 녹음이 끝나면 창을 닫으세요"),
        });
        Self {
            project,
            history: History::default(),
            record_settings,
            export_settings,
            devices: if capture_path.is_some() {
                vec![]
            } else {
                audio::input_devices()
            },
            device: 0,
            recorder: None,
            playback: None,
            rec_start: 0.0,
            rec_track: 0,
            rec_id: 0,
            live_peaks: vec![],
            cursor: 0.0,
            active_track: 0,
            selected: preview_selected,
            clipboard: vec![],
            selection: preview_selected.into_iter().collect(),
            drag: None,
            zoom: 64.0,
            last_fit_zoom: 0.0,
            view_start: 0.0,
            snap: true,
            level: 0.0,
            pending_edit: None,
            settings_open: args.iter().any(|a| a == "--capture-settings"),
            export_open: false,
            export_job: None,
            message,
            message_error,
            message_at: Instant::now(),
            last_export: None,
            host,
            host_export_dir,
            host_note,
            takes: vec![],
            exports: vec![],
            demo,
            close_dialog: false,
            allow_close: false,
            pan: None,
            scroll_y: 0.0,
            capture_path,
            capture_frames: 0,
            capture_sent: false,
            seeking: false,
            rename_target: None,
            rename_text: String::new(),
            rename_focus: false,
            marquee: None,
            snap_guide: None,
            snap_release: None,
            #[cfg(test)]
            test_geometry: None,
            #[cfg(test)]
            test_rects: vec![],
        }
    }
    fn busy(&self) -> bool {
        self.recorder.is_some() || self.export_job.is_some()
    }
    fn capture_frame(&mut self, ctx: &Context) {
        let Some(path) = self.capture_path.clone() else {
            return;
        };
        let captured = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(frame) = captured {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let rgba: Vec<u8> = frame.pixels.iter().flat_map(|p| p.to_array()).collect();
            if let Err(error) = image::save_buffer(
                &path,
                &rgba,
                frame.size[0] as u32,
                frame.size[1] as u32,
                image::ColorType::Rgba8,
            ) {
                let _ = std::fs::write(path.with_extension("error.txt"), error.to_string());
            }
            self.allow_close = true;
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        self.capture_frames += 1;
        if self.capture_frames >= 12
            && self.message_at.elapsed() >= Duration::from_millis(500)
            && !self.capture_sent
        {
            self.capture_sent = true;
            ctx.send_viewport_cmd(ViewportCommand::Screenshot(UserData::default()));
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }
    fn tell(&mut self, message: impl Into<String>, error: bool) {
        self.message = message.into();
        self.message_error = error;
        self.message_at = Instant::now();
    }
    fn persist(&mut self) {
        if self.drag.is_none() && self.recorder.is_none() {
            let mapping = self.project.compact_tracks();
            self.active_track = mapping.get(self.active_track).and_then(|v| *v).unwrap_or(0);
            let clips = &self.project.clips;
            self.project
                .sources
                .retain(|id, _| clips.iter().any(|c| c.source == *id));
            self.selection
                .retain(|id| clips.iter().any(|c| c.id == *id));
            if self
                .selected
                .is_some_and(|id| !clips.iter().any(|c| c.id == id))
            {
                self.selected = self.selection.first().copied();
            }
        }
        if !self.demo {
            if let Err(e) =
                session::save(&self.project, &self.record_settings, &self.export_settings)
            {
                self.tell(format!("작업 자동 저장 실패: {e}"), true);
            }
        }
    }
    /// 호스트가 `SORI_DATA_DIR`를 정해 준 경우에만 `handoff.json`을 갱신한다. 혼자 쓸 때는 아무것도 하지 않는다.
    fn handoff(&self, closed: bool) {
        if let Some(dir) = host::handoff_dir() {
            let _ = host::write_handoff(
                &dir,
                self.host.as_deref(),
                &self.takes,
                &self.exports,
                closed,
            );
        }
    }
    fn begin_edit(&mut self) {
        self.stop_playback();
        self.history.push(&self.project);
    }
    fn finish_slider(&mut self, before: Project, response: &Response) {
        if response.changed() && self.pending_edit.is_none() {
            self.pending_edit = Some(before);
            self.stop_playback();
        }
        if response.drag_stopped() || (response.changed() && !response.dragged()) {
            self.finish_pending();
        }
    }
    fn finish_pending(&mut self) {
        if let Some(before) = self.pending_edit.take() {
            self.history.push(&before);
            self.persist();
        }
    }
    fn stop_playback(&mut self) {
        if let Some(p) = self.playback.take() {
            self.cursor = p.seconds().min(self.project.duration());
        }
    }
    fn play_pause(&mut self) {
        if self.busy() {
            return;
        }
        if self.playback.is_some() {
            self.stop_playback();
            return;
        }
        if self.project.clips.is_empty() {
            self.tell("먼저 목소리를 녹음해 주세요.", false);
            return;
        }
        if self.cursor >= self.project.duration() {
            self.cursor = 0.0;
        }
        match Playback::start(Arc::new(self.project.clone()), self.cursor, false) {
            Ok(p) => self.playback = Some(p),
            Err(e) => self.tell(e, true),
        }
    }
    fn toggle_record(&mut self) {
        if self.export_job.is_some() {
            return;
        }
        if self.recorder.is_some() {
            self.stop_record();
            return;
        }
        self.stop_playback();
        self.finish_pending();
        let Some(device) = self.devices.get(self.device) else {
            self.tell(
                "마이크를 찾지 못했습니다. 설정에서 새로고침을 눌러주세요.",
                true,
            );
            self.settings_open = true;
            return;
        };
        let id = session::next_id();
        let path = match session::recording_path(id) {
            Ok(p) => p,
            Err(e) => {
                self.tell(e, true);
                return;
            }
        };
        match Recorder::start(device, &self.record_settings, id, &path) {
            Ok(r) => {
                if self.project.tracks.is_empty() {
                    self.project.add_track();
                }
                self.rec_start = self.cursor;
                self.rec_track = self.active_track;
                self.rec_id = id;
                self.live_peaks.clear();
                self.recorder = Some(r);
                if self.record_settings.overdub && !self.project.clips.is_empty() {
                    let mut backing = self.project.clone();
                    backing.tracks[self.rec_track].muted = true;
                    match Playback::start(Arc::new(backing), self.cursor, true) {
                        Ok(p) => self.playback = Some(p),
                        Err(e) => self.tell(
                            format!("녹음은 시작했지만 기존 트랙을 재생하지 못했습니다: {e}"),
                            true,
                        ),
                    }
                }
                self.tell("녹음 중 · 버튼을 다시 누르면 멈춥니다.", false);
            }
            Err(e) => self.tell(e, true),
        }
    }
    fn stop_record(&mut self) {
        self.playback.take();
        if let Some(recorder) = self.recorder.take() {
            let dropped = recorder.dropped.load(Ordering::Relaxed);
            match recorder.stop() {
                Ok(source) if source.frames() > 0 => {
                    self.history.push(&self.project);
                    self.cursor = self.rec_start + source.duration();
                    let clip_id = self.rec_id.wrapping_add(1);
                    if !source.path.is_empty() {
                        self.takes.push(PathBuf::from(&source.path));
                    }
                    self.project
                        .add_source(source, clip_id, self.rec_track, self.rec_start);
                    self.project.overwrite_clip(clip_id);
                    self.select_single(clip_id);
                    self.persist();
                    self.handoff(false);
                    if dropped > 0 {
                        self.tell(format!("녹음 저장 · 처리 지연으로 {dropped}프레임이 누락됐습니다. 다른 앱을 닫거나 버퍼를 ‘기기 자동’으로 설정해 주세요."), true);
                    } else {
                        self.tell("녹음 완료. 클립을 옮기거나 재생해 보세요.", false);
                    }
                }
                Ok(_) => self.tell(
                    "마이크에서 아직 소리를 받지 못했습니다. 입력 장치를 확인해 주세요.",
                    true,
                ),
                Err(e) => self.tell(e, true),
            }
            self.live_peaks.clear();
        }
    }
    fn undo(&mut self, redo: bool) {
        if self.busy() {
            return;
        }
        self.stop_playback();
        self.finish_pending();
        let changed = if redo {
            self.history.redo(&mut self.project)
        } else {
            self.history.undo(&mut self.project)
        };
        if changed {
            self.selection
                .retain(|id| self.project.clips.iter().any(|c| c.id == *id));
            self.active_track = self
                .active_track
                .min(self.project.tracks.len().saturating_sub(1));
            if self
                .selected
                .is_some_and(|id| !self.project.clips.iter().any(|c| c.id == id))
            {
                self.selected = None;
            }
            self.persist();
        }
    }
    fn selected_ids(&self) -> Vec<u64> {
        if self.selection.is_empty() {
            self.selected.into_iter().collect()
        } else {
            self.selection.iter().copied().collect()
        }
    }
    fn select_single(&mut self, id: u64) {
        self.selected = Some(id);
        self.selection.clear();
        self.selection.insert(id);
    }
    fn split(&mut self) {
        if self.busy() {
            return;
        }
        self.stop_playback();
        let mut ids = self.selected_ids();
        if ids.is_empty() {
            ids.extend(
                self.project
                    .clips
                    .iter()
                    .filter(|c| {
                        c.track == self.active_track
                            && self.cursor > c.start
                            && self.cursor < c.end()
                    })
                    .map(|c| c.id),
            );
        }
        let targets: Vec<_> = ids
            .into_iter()
            .filter(|id| {
                self.project.clips.iter().any(|c| {
                    c.id == *id && self.cursor > c.start + 1e-8 && self.cursor < c.end() - 1e-8
                })
            })
            .collect();
        if targets.is_empty() {
            return;
        }
        self.begin_edit();
        self.selection.clear();
        let mut next = session::next_id();
        for id in targets {
            while self.project.clips.iter().any(|c| c.id == next) {
                next += 1;
            }
            if self.project.split(id, self.cursor, next) {
                self.selection.insert(next);
                self.selected = Some(next);
                next += 1;
            }
        }
        self.persist();
    }
    fn copy(&mut self, ctx: &Context) {
        let ids = self.selected_ids();
        self.clipboard = self
            .project
            .clips
            .iter()
            .filter(|c| ids.contains(&c.id))
            .filter_map(|c| {
                self.project
                    .sources
                    .get(&c.source)
                    .map(|s| (c.clone(), s.clone()))
            })
            .collect();
        if !self.clipboard.is_empty() {
            ctx.copy_text("Sori audio clips".into());
        }
    }
    fn paste(&mut self) {
        if self.busy() || self.clipboard.is_empty() {
            return;
        }
        let clips = self.clipboard.clone();
        let first_time = clips
            .iter()
            .map(|(c, _)| c.start)
            .fold(f64::INFINITY, f64::min);
        let first_track = clips.iter().map(|(c, _)| c.track).min().unwrap();
        let span = clips.iter().map(|(c, _)| c.track).max().unwrap() - first_track + 1;
        self.begin_edit();
        while self.project.tracks.len() < self.active_track + span {
            self.project.add_track();
        }
        self.selection.clear();
        let mut next = session::next_id();
        let mut end = self.cursor;
        for (mut clip, source) in clips {
            while self.project.clips.iter().any(|c| c.id == next) {
                next += 1;
            }
            clip.id = next;
            next += 1;
            clip.start = self.cursor + clip.start - first_time;
            clip.track = self.active_track + clip.track - first_track;
            end = end.max(clip.end());
            self.selection.insert(clip.id);
            self.selected = Some(clip.id);
            self.project.sources.insert(source.id, source);
            self.project.clips.push(clip);
        }
        for id in self.selected_ids() {
            self.project.overwrite_clip(id);
        }
        self.cursor = end;
        self.persist();
    }
    fn delete_clip(&mut self) {
        if self.busy() {
            return;
        }
        let ids = self.selected_ids();
        if ids.is_empty() {
            return;
        }
        self.begin_edit();
        self.project.clips.retain(|c| !ids.contains(&c.id));
        self.selection.clear();
        self.selected = None;
        self.persist();
    }
    fn join_selected(&mut self) {
        if self.busy() {
            return;
        }
        self.stop_playback();
        let before = self.project.clone();
        let mut changed = false;
        for id in self.selected_ids() {
            changed |= self.project.join_clip(id);
        }
        if changed {
            self.history.push(&before);
            self.selection
                .retain(|id| self.project.clips.iter().any(|c| c.id == *id));
            self.selected = self.selection.first().copied();
            self.persist();
        }
    }
    fn begin_rename(&mut self, target: RenameTarget) {
        if self.busy() {
            return;
        }
        self.stop_playback();
        self.rename_text = match target {
            RenameTarget::Track(i) => self
                .project
                .tracks
                .get(i)
                .map_or_else(String::new, |t| t.name.clone()),
            RenameTarget::Clip(id) => self
                .project
                .clips
                .iter()
                .find(|c| c.id == id)
                .map_or_else(String::new, |c| c.name.clone()),
        };
        self.rename_target = Some(target);
        self.rename_focus = true;
    }
    fn commit_rename(&mut self) {
        let Some(target) = self.rename_target.take() else {
            return;
        };
        let name: String = self.rename_text.trim().chars().take(100).collect();
        let before = self.project.clone();
        match target {
            RenameTarget::Track(i) => {
                if let Some(t) = self.project.tracks.get_mut(i) {
                    t.name = name;
                }
            }
            RenameTarget::Clip(id) => {
                if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id) {
                    if !name.is_empty() {
                        c.name = name;
                    }
                }
            }
        }
        self.history.push(&before);
        self.persist();
    }
    fn rename_window(&mut self, ctx: &Context) {
        if self.rename_target.is_none() {
            return;
        }
        let mut open = true;
        let mut commit = false;
        let mut cancel = false;
        Window::new("이름 변경")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let response = ui.add_sized(
                    vec2(310.0, 32.0),
                    TextEdit::singleline(&mut self.rename_text).char_limit(100),
                );
                if self.rename_focus {
                    response.request_focus();
                    self.rename_focus = false;
                }
                commit |= ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
                cancel |= ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
                ui.horizontal(|ui| {
                    if ui.button("취소").clicked() {
                        cancel = true;
                    }
                    if theme::primary(ui, "변경", vec2(80.0, 32.0)).clicked() {
                        commit = true;
                    }
                });
            });
        if commit {
            self.commit_rename();
        } else if cancel || !open {
            self.rename_target = None;
        }
    }

    fn keyboard(&mut self, ctx: &Context) {
        if ctx.wants_keyboard_input()
            || self.settings_open
            || self.export_open
            || self.close_dialog
            || self.rename_target.is_some()
        {
            return;
        }
        let (magnet, join) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::N),
                i.consume_key(Modifiers::CTRL, Key::J),
            )
        });
        if magnet {
            self.snap = !self.snap;
            self.snap_guide = None;
        }
        if join {
            self.join_selected();
        }
        let (redo, undo, split, copy, paste, play, rec, delete, home, escape, export) = ctx
            .input_mut(|i| {
                let redo = i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Z);
                let undo = i.consume_key(Modifiers::CTRL, Key::Z);
                let split = i.consume_key(Modifiers::CTRL, Key::B);
                let copy = i.consume_key(Modifiers::CTRL, Key::C)
                    || i.events.iter().any(|e| matches!(e, Event::Copy));
                let paste = i.consume_key(Modifiers::CTRL, Key::V)
                    || i.events.iter().any(|e| matches!(e, Event::Paste(_)));
                (
                    redo,
                    undo,
                    split,
                    copy,
                    paste,
                    i.consume_key(Modifiers::NONE, Key::Space),
                    i.consume_key(Modifiers::NONE, Key::R),
                    i.consume_key(Modifiers::NONE, Key::Delete),
                    i.consume_key(Modifiers::NONE, Key::Home),
                    i.consume_key(Modifiers::NONE, Key::Escape),
                    i.consume_key(Modifiers::CTRL, Key::E),
                )
            });
        if redo {
            self.undo(true);
        } else if undo {
            self.undo(false);
        }
        if split {
            self.split();
        }
        if copy {
            self.copy(ctx);
        }
        if paste {
            self.paste();
        }
        if play {
            if self.recorder.is_some() {
                self.stop_record();
            } else {
                self.play_pause();
            }
        }
        if rec {
            self.toggle_record();
        }
        if delete {
            self.delete_clip();
        }
        if home && !self.busy() {
            self.stop_playback();
            self.cursor = 0.0;
            self.view_start = 0.0;
        }
        if escape {
            if self.recorder.is_some() {
                self.stop_record();
            } else {
                self.stop_playback();
                self.selected = None;
                self.selection.clear();
            }
        }
        if export && !self.busy() {
            self.export_open = true;
        }
    }
    fn poll(&mut self, ctx: &Context) {
        let mut record_error = None;
        let mut playback_error = None;
        if let Some(r) = &self.recorder {
            self.cursor = self.rec_start + r.seconds();
            self.level = self
                .level
                .max(f32::from_bits(r.meter.load(Ordering::Relaxed)));
            if let Ok(live) = r.live.lock() {
                self.live_peaks
                    .extend_from_slice(&live.peaks[self.live_peaks.len().min(live.peaks.len())..]);
            }
            if let Ok(mut error) = r.error.lock() {
                record_error = error.take();
            }
            if record_error.is_none() && r.worker_finished() {
                record_error = Some("녹음 저장 작업이 중단됐습니다.".into());
            }
        } else if let Some(p) = &self.playback {
            self.cursor = p.seconds().min(self.project.duration());
            self.level = self
                .level
                .max(f32::from_bits(p.meter.load(Ordering::Relaxed)));
            if p.finished.load(Ordering::Relaxed) {
                self.playback.take();
            }
        }
        if let Some(p) = &self.playback {
            if let Ok(mut e) = p.error.lock() {
                playback_error = e.take();
            }
        }
        if let Some(e) = record_error {
            self.stop_record();
            self.tell(
                format!("마이크 연결이 중단됐습니다. 받은 구간까지 저장했습니다. {e}"),
                true,
            );
        }
        if let Some(e) = playback_error {
            self.stop_playback();
            self.tell(format!("출력 장치 오류: {e}"), true);
        }
        if self.export_job.is_some() {
            let result = self.export_job.as_ref().unwrap().rx.try_recv();
            match result {
                Ok(Ok(Some(result))) => {
                    self.export_job.take();
                    self.last_export = Some(result.path.clone());
                    self.exports.push(result.path.clone());
                    self.handoff(false);
                    if result.clipped {
                        self.tell("파일 저장 완료 · 겹친 소리가 0 dB를 넘어서 잘렸습니다. 전체 볼륨을 낮추거나 ‘크기 맞추기’를 켜고 다시 저장하세요.", true);
                    } else {
                        self.tell(
                            format!(
                                "저장 완료 · {}",
                                result
                                    .path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                            ),
                            false,
                        );
                    }
                }
                Ok(Ok(None)) => {
                    self.export_job.take();
                    self.tell("저장을 취소했습니다.", false);
                }
                Ok(Err(e)) => {
                    self.export_job.take();
                    self.tell(format!("내보내기 실패: {e}"), true);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.export_job.take();
                    self.tell("내보내기 작업이 예기치 않게 종료됐습니다.", true);
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.level *= 0.88;
        if self.recorder.is_some() || self.playback.is_some() || self.export_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else if self.level > 0.001 {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
    fn start_export(&mut self) {
        if self.busy() || self.project.clips.is_empty() {
            return;
        }
        self.stop_playback();
        self.finish_pending();
        self.persist();
        let project = self.project.clone();
        let settings = self.export_settings.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let worker_progress = progress.clone();
        let start_dir =
            host::dialog_dir(self.last_export.as_deref(), self.host_export_dir.as_deref());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut dialog = rfd::FileDialog::new();
            if let Some(dir) = &start_dir {
                dialog = dialog.set_directory(dir);
            }
            let result = if settings.split_tracks {
                match dialog.set_title("트랙 파일을 저장할 폴더").pick_folder() {
                    Some(folder) => {
                        export::export_tracks(&project, &settings, &folder, worker_progress)
                            .map(Some)
                    }
                    None => Ok(None),
                }
            } else {
                let file_name = format!(
                    "{}.{}",
                    export::safe_name(&project.name),
                    settings.format.extension()
                );
                match dialog
                    .set_title("오디오 저장")
                    .set_file_name(file_name)
                    .add_filter(settings.format.label(), &[settings.format.extension()])
                    .save_file()
                {
                    Some(path) => {
                        let path = if path.extension().is_none() {
                            path.with_extension(settings.format.extension())
                        } else {
                            path
                        };
                        if path
                            .extension()
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| s.eq_ignore_ascii_case(settings.format.extension()))
                        {
                            export::export(&project, &settings, &path, worker_progress).map(Some)
                        } else {
                            Err(format!(
                                "파일 확장자를 .{}로 지정해 주세요.",
                                settings.format.extension()
                            ))
                        }
                    }
                    None => Ok(None),
                }
            };
            let _ = tx.send(result);
        });
        self.export_job = Some(ExportJob { rx, progress });
        self.export_open = false;
    }

    fn recorder_panel(&mut self, ui: &mut Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(vec2(width, 92.0), Sense::hover());
        theme::pixel_panel(ui.painter(), rect, SURFACE, BORDER);
        let button = Rect::from_center_size(rect.center(), vec2(52.0, 52.0));
        let response = ui
            .interact(button, ui.id().with("record"), Sense::click())
            .on_hover_text("녹음 / 정지 · R\n현재 재생선에서 선택 트랙에 녹음합니다.");
        theme::pixel_panel(
            ui.painter(),
            button,
            if response.hovered() { RAISED } else { BG },
            if self.recorder.is_some() { RED } else { BORDER },
        );
        theme::icon(
            ui.painter(),
            button.center(),
            32.0,
            if self.recorder.is_some() {
                Icon::Stop
            } else {
                Icon::Record
            },
            RED,
        );
        let timer = Rect::from_min_size(
            pos2(button.right() + 20.0, rect.center().y - 16.0),
            vec2(158.0, 32.0),
        );
        let time = self
            .recorder
            .as_ref()
            .map_or(self.cursor, Recorder::seconds);
        ui.painter().with_clip_rect(timer).text(
            timer.left_center(),
            Align2::LEFT_CENTER,
            time_label(time),
            FontId::monospace(24.0),
            TEXT,
        );
        let time_response = ui
            .interact(timer, ui.id().with("time"), Sense::click())
            .on_hover_text("현재 위치 · 녹음 중에는 녹음 시간\n더블클릭하면 처음으로 돌아갑니다.");
        if theme::double_click(&time_response) && !self.busy() {
            self.stop_playback();
            self.cursor = 0.0;
            self.view_start = 0.0;
        }
        let mic_width = (button.left() - rect.left() - 44.0).min(304.0).max(180.0);
        let input_rect = Rect::from_min_size(rect.min + vec2(18.0, 20.0), vec2(mic_width, 56.0));
        ui.scope_builder(UiBuilder::new().max_rect(input_rect), |ui| {
            ui.add_enabled_ui(!self.busy(), |ui| {
                let name = if self.capture_path.is_some() {
                    "기본 마이크"
                } else {
                    self.devices
                        .get(self.device)
                        .map_or("마이크 없음", |d| d.name.as_str())
                };
                ComboBox::from_id_salt("microphone")
                    .selected_text(name)
                    .width(mic_width - 8.0)
                    .truncate()
                    .show_ui(ui, |ui| {
                        for (i, d) in self.devices.iter().enumerate() {
                            ui.selectable_value(&mut self.device, i, &d.name);
                        }
                    })
                    .response
                    .on_hover_text(
                        "녹음할 마이크를 선택합니다. 아래 막대는 녹음 중 입력 크기입니다.",
                    );
            });
            self.meter(ui, mic_width);
        });
        let settings_rect =
            Rect::from_center_size(pos2(rect.right() - 32.0, rect.center().y), vec2(34.0, 34.0));
        ui.scope_builder(UiBuilder::new().max_rect(settings_rect), |ui| {
            let response = theme::icon_button(
                ui,
                Icon::Settings,
                "입력 설정 · 오른쪽 클릭으로 녹음 폴더 열기",
            );
            if response.clicked() {
                self.settings_open = !self.settings_open;
            }
            response.context_menu(|ui| {
                if ui.button("녹음 폴더 열기").clicked() {
                    reveal(&session::data_dir());
                    ui.close();
                }
                if self.host.is_some() && ui.button("내보내기 폴더 열기").clicked() {
                    reveal(
                        self.host_export_dir
                            .as_deref()
                            .unwrap_or(&session::data_dir()),
                    );
                    ui.close();
                }
            });
        });
        #[cfg(test)]
        {
            self.test_rects.extend([
                ("record".into(), button),
                ("timer".into(), timer),
                ("input".into(), input_rect),
                ("settings".into(), settings_rect),
            ]);
        }
        ui.advance_cursor_after_rect(rect);
        if response.clicked() {
            self.toggle_record();
        }
    }

    fn meter(&self, ui: &mut Ui, width: f32) {
        ui.horizontal(|ui| {
            let (r, response) = ui.allocate_exact_size(vec2(width - 64.0, 6.0), Sense::hover());
            let fraction = ((self.level.max(0.00001).log10() * 20.0 + 60.0) / 60.0).clamp(0.0, 1.0);
            let cell = (r.width() / 24.0).floor().max(2.0);
            for i in 0..24 {
                let fill = if i as f32 / 24.0 < fraction {
                    if self.level >= 0.98 {
                        RED
                    } else {
                        TRACK_COLORS[1]
                    }
                } else {
                    BORDER.gamma_multiply(0.6)
                };
                ui.painter().rect_filled(
                    Rect::from_min_size(r.min + vec2(i as f32 * cell, 0.0), vec2(cell - 2.0, 6.0)),
                    0.0,
                    fill,
                );
            }
            response.on_hover_text(
                "입력 크기. 빨간색이 자주 뜨면 마이크 게인이나 입력 크기를 낮춰주세요.",
            );
            ui.label(
                RichText::new(if self.level > 0.001 {
                    format!("{:.0} dB", 20.0 * self.level.log10())
                } else {
                    "-∞ dB".into()
                })
                .size(12.0)
                .color(MUTED),
            );
        });
    }

    fn timeline(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!self.busy(),|ui| {
                ui.add_enabled_ui(self.history.can_undo(),|ui| {if theme::icon_button(ui,Icon::Undo,"실행 취소 · Ctrl+Z").clicked(){self.undo(false);}});
                ui.add_enabled_ui(self.history.can_redo(),|ui| {if theme::icon_button(ui,Icon::Redo,"다시 실행 · Ctrl+Shift+Z").clicked(){self.undo(true);}});
                if theme::icon_button(ui,Icon::Cut,"재생선에서 선택 클립 분할 · Ctrl+B").clicked(){self.split();}
            });
            ui.with_layout(Layout::right_to_left(Align::Center),|ui| {
                let (rect,response)=ui.allocate_exact_size(vec2(34.0,34.0),Sense::click());
                if self.snap {theme::pixel_panel(ui.painter(),rect,Color32::from_rgb(64,57,43),BORDER);}
                else if response.hovered(){theme::pixel_panel(ui.painter(),rect,RAISED,BORDER);}
                theme::icon(ui.painter(),rect.center(),20.0,Icon::Magnet,if self.snap{ACCENT}else{MUTED});
                if response.on_hover_text("클립 시작·끝에 자석 맞춤 · N\nAlt를 누르는 동안 해제. 붙잡힌 위치는 빨간 선으로 표시합니다.").clicked(){self.snap=!self.snap;self.snap_guide=None;}
            });
        });
        ui.add_space(5.0);
        let width = ui.available_width();
        let height =
            (ui.available_height() - if self.message_error { 24.0 } else { 0.0 }).max(230.0);
        let (canvas, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let painter = ui.painter().clone();
        theme::pixel_panel(&painter, canvas, SURFACE, BORDER);
        let header_w = 148.0;
        let ruler_h = 30.0;
        let gutter = 15.0;
        let origin = canvas.left() + header_w;
        let right = canvas.right() - 10.0;
        let lanes_top = canvas.top() + ruler_h + gutter;
        let row_h = (canvas.bottom() - lanes_top - 8.0) / 5.0;
        #[cfg(test)]
        {
            self.test_geometry = Some(TestGeometry {
                origin,
                lanes_top,
                row_h,
            });
        }
        let live_end = self
            .recorder
            .as_ref()
            .map_or(0.0, |r| self.rec_start + r.seconds());
        let mut end = self.project.duration().max(live_end);
        if let Some(d) = &self.drag {
            end = end.max(d.end_before);
        }
        if self.recorder.is_some() {
            end = end
                .max(self.rec_start + 10.0)
                .max((live_end / 5.0).ceil() * 5.0);
        }
        if end <= 0.0 {
            end = 10.0;
        }
        let min_zoom = ((right - origin) as f64 / end) as f32;
        let idle = self.drag.is_none()
            && self.recorder.is_none()
            && self.marquee.is_none()
            && !self.seeking
            && self.pan.is_none();
        if idle
            && self.last_fit_zoom > 0.0
            && (self.zoom - self.last_fit_zoom).abs() < self.last_fit_zoom * 0.001
        {
            self.zoom = min_zoom;
        }
        if idle {
            self.last_fit_zoom = min_zoom;
        }
        self.zoom = self.zoom.clamp(min_zoom, min_zoom.max(40000.0));
        let scroll_max = ((self.project.tracks.len() + 1) as f32 * row_h
            - (canvas.bottom() - lanes_top))
            .max(0.0);
        self.scroll_y = self.scroll_y.clamp(0.0, scroll_max);
        let pointer = ui.input(|i| i.pointer.interact_pos());
        if self.drag.is_none() && self.marquee.is_none() && !self.seeking {
            if let Some(p) = pointer.filter(|p| canvas.contains(*p)) {
                if p.x >= origin {
                    let wheel = ui.input_mut(|i| {
                        let d = i.raw_scroll_delta;
                        i.raw_scroll_delta = Vec2::ZERO;
                        i.smooth_scroll_delta = Vec2::ZERO;
                        d
                    });
                    if wheel.y.abs() > 0.01 {
                        let time = self.view_start + (p.x - origin) as f64 / self.zoom as f64;
                        self.zoom = (self.zoom * (wheel.y * 0.003).exp())
                            .clamp(min_zoom, min_zoom.max(40000.0));
                        self.view_start = time - (p.x - origin) as f64 / self.zoom as f64;
                    } else if wheel.x.abs() > 0.01 {
                        self.view_start -= wheel.x as f64 / self.zoom as f64;
                    }
                }
                if ui.input(|i| i.pointer.button_pressed(PointerButton::Middle)) {
                    self.pan = Some((p, self.view_start, self.scroll_y));
                }
            }
        }
        if let Some((press, start, start_y)) = self.pan {
            if let Some(p) = pointer {
                self.view_start = start - (p.x - press.x) as f64 / self.zoom as f64;
                self.scroll_y = start_y - (p.y - press.y);
            }
            if ui.input(|i| i.pointer.button_down(PointerButton::Middle)) {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            } else {
                self.pan = None;
            }
        }
        self.scroll_y = self.scroll_y.clamp(0.0, scroll_max);
        if (self.marquee.is_some()
            || self
                .drag
                .as_ref()
                .is_some_and(|d| matches!(d.kind, DragKind::Move)))
            && ui.input(|i| i.pointer.primary_down())
        {
            if let Some(p) = pointer {
                let speed = if p.y < lanes_top + 20.0 {
                    -(lanes_top + 20.0 - p.y) * 8.0
                } else if p.y > canvas.bottom() - 20.0 {
                    (p.y - canvas.bottom() + 20.0) * 8.0
                } else {
                    0.0
                };
                if speed != 0.0 {
                    self.scroll_y = (self.scroll_y
                        + speed.clamp(-1000.0, 1000.0)
                            * ui.input(|i| i.stable_dt).clamp(0.001, 0.05))
                    .clamp(0.0, scroll_max);
                    ui.ctx().request_repaint();
                }
            }
        }
        let span = (right - origin) as f64 / self.zoom as f64;
        self.view_start = self.view_start.clamp(0.0, (end - span).max(0.0));
        if (self.recorder.is_some() || self.playback.is_some())
            && (self.cursor > self.view_start + span || self.cursor < self.view_start)
        {
            self.view_start = (self.cursor - span * 0.9).clamp(0.0, (end - span).max(0.0));
        }
        let dt = ui.input(|i| i.stable_dt);
        if let Some(p) = pointer {
            if self.drag.is_some() && ui.input(|i| i.pointer.primary_down()) {
                let extend = self
                    .drag
                    .as_ref()
                    .is_some_and(|d| matches!(d.kind, DragKind::Move));
                if !self
                    .drag
                    .as_ref()
                    .is_some_and(|d| matches!(d.kind, DragKind::Gain))
                    && self.edge_pan(p.x, origin, right, end, dt, extend)
                {
                    ui.ctx().request_repaint();
                }
                let d = self.drag.as_ref().unwrap();
                let id = d.id;
                let delta = p - d.press;
                self.apply_drag(id, delta, p.y, row_h, ui.input(|i| i.modifiers.alt));
            } else if (self.seeking || self.marquee.is_some())
                && ui.input(|i| i.pointer.primary_down())
            {
                if self.edge_pan(p.x, origin, right, end, dt, false) {
                    ui.ctx().request_repaint();
                }
            }
        }
        let view = self.view_start;
        let zoom = self.zoom;
        let x_at = |time: f64| origin + ((time - view) * zoom as f64) as f32;
        let time_at = |x: f32| {
            (view + (x - origin).clamp(0.0, right - origin) as f64 / zoom as f64).clamp(0.0, end)
        };
        let step = [
            0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0,
            120.0, 300.0,
        ]
        .into_iter()
        .find(|s| s * zoom as f64 >= 65.0)
        .unwrap_or(600.0);
        let mut t = (view / step).ceil() * step;
        while x_at(t) < right {
            let label = if step < 1.0 {
                format!("{t:.3}")
            } else {
                format!("{:02}:{:02}", t as u64 / 60, t as u64 % 60)
            };
            painter.text(
                pos2(x_at(t) + 4.0, canvas.top() + 14.0),
                Align2::LEFT_CENTER,
                label,
                FontId::monospace(12.0),
                MUTED,
            );
            t += step;
        }
        let can_edit = !self.busy() && self.rename_target.is_none();
        let body = Rect::from_min_max(
            pos2(origin, lanes_top - gutter),
            pos2(right, canvas.bottom()),
        );
        let background = ui.interact(
            body,
            ui.id().with("timeline-background"),
            Sense::click_and_drag(),
        );
        let ruler = ui
            .interact(
                Rect::from_min_max(pos2(origin, canvas.top()), pos2(right, lanes_top - gutter)),
                ui.id().with("ruler"),
                Sense::click_and_drag(),
            )
            .on_hover_text("클릭 / 드래그로 재생선 이동. 화면 밖으로 끌면 타임라인이 따라갑니다.");
        if can_edit
            && (ruler.drag_started_by(PointerButton::Primary)
                || (ruler.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down())))
        {
            self.stop_playback();
            self.seeking = true;
        }
        if can_edit && background.clicked() {
            if let Some(p) = background.interact_pointer_pos() {
                self.stop_playback();
                self.cursor = time_at(p.x);
                self.selection.clear();
                self.selected = None;
                self.active_track = (((p.y - lanes_top + self.scroll_y) / row_h).floor().max(0.0)
                    as usize)
                    .min(self.project.tracks.len().saturating_sub(1));
            }
        }
        if can_edit && background.drag_started_by(PointerButton::Primary) && self.drag.is_none() {
            self.stop_playback();
            let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(body.min);
            let base = if ui.input(|i| i.modifiers.ctrl) {
                self.selection.clone()
            } else {
                Default::default()
            };
            self.marquee = Some(Marquee {
                start_time: time_at(press.x),
                start_y: press.y + self.scroll_y,
                base,
            });
        }
        for track in 0..self.project.tracks.len() {
            let top = lanes_top + track as f32 * row_h - self.scroll_y;
            if top + row_h <= lanes_top || top >= canvas.bottom() {
                continue;
            }
            let painter = painter.with_clip_rect(Rect::from_min_max(
                pos2(canvas.left(), lanes_top),
                canvas.max,
            ));
            painter.rect_filled(
                Rect::from_min_size(
                    pos2(canvas.left() + 4.0, top),
                    vec2(canvas.width() - 12.0, row_h),
                ),
                0.0,
                if track % 2 == 0 {
                    Color32::from_rgb(30, 36, 38)
                } else {
                    Color32::from_rgb(39, 46, 47)
                },
            );
            if self.project.tracks[track].muted {
                painter.rect_filled(
                    Rect::from_min_size(
                        pos2(canvas.left() + 4.0, top + 3.0),
                        vec2(canvas.width() - 12.0, row_h - 6.0),
                    ),
                    0.0,
                    RED.gamma_multiply(0.13),
                );
            }
            let header = Rect::from_min_size(
                pos2(canvas.left() + 4.0, top + 3.0),
                vec2(header_w - 12.0, row_h - 6.0),
            );
            if self.active_track == track {
                painter.rect_filled(
                    header,
                    0.0,
                    if self.project.tracks[track].muted {
                        Color32::from_rgb(64, 39, 39)
                    } else {
                        RAISED
                    },
                );
            }
            let response = ui.interact(
                header.intersect(Rect::from_min_max(
                    pos2(canvas.left(), lanes_top),
                    canvas.max,
                )),
                ui.id().with(("track", track)),
                Sense::click(),
            );
            if can_edit && response.clicked() {
                self.active_track = track;
            }
            if can_edit && theme::double_click(&response) {
                self.begin_rename(RenameTarget::Track(track));
            }
            let name = if self.project.tracks[track].name.trim().is_empty() {
                format!("{:02}", track + 1)
            } else {
                self.project.tracks[track].name.clone()
            };
            let title_rect = Rect::from_min_size(
                header.min + vec2(12.0, 4.0),
                vec2(header.width() - 51.0, 25.0),
            );
            #[cfg(test)]
            {
                self.test_rects
                    .push((format!("track-title-{track}"), title_rect));
            }
            painter.with_clip_rect(title_rect).text(
                title_rect.left_center(),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(12.0),
                if self.active_track == track {
                    TEXT
                } else {
                    MUTED
                },
            );
            ui.scope_builder(
                UiBuilder::new().max_rect(Rect::from_min_size(
                    header.right_top() + vec2(-34.0, 0.0),
                    vec2(32.0, 32.0),
                )),
                |ui| {
                    ui.set_clip_rect(ui.clip_rect().intersect(Rect::from_min_max(
                        pos2(canvas.left(), lanes_top),
                        canvas.max,
                    )));
                    ui.add_enabled_ui(can_edit, |ui| {
                        let muted = self.project.tracks[track].muted;
                        if theme::mini_icon_button(
                            ui,
                            if muted { Icon::Muted } else { Icon::Volume },
                            "트랙 음소거",
                        )
                        .clicked()
                        {
                            self.begin_edit();
                            self.project.tracks[track].muted = !muted;
                            self.persist();
                        }
                    });
                },
            );
            let control = Rect::from_min_size(
                header.min + vec2(10.0, (row_h - 37.0).min(36.0).max(28.0)),
                vec2(header.width() - 20.0, 28.0),
            );
            #[cfg(test)]
            {
                self.test_rects
                    .push((format!("track-volume-{track}"), control));
            }
            ui.scope_builder(UiBuilder::new().max_rect(control), |ui| {
                ui.add_enabled_ui(can_edit, |ui| {
                    let before = self.project.clone();
                    let response = theme::value_slider(
                        ui,
                        &mut self.project.tracks[track].gain,
                        0.0..=2.0,
                        1.0,
                        control.width(),
                        true,
                    )
                    .on_hover_text("트랙 볼륨. 더블클릭하면 100%.");
                    self.finish_slider(before, &response);
                });
            });
        }
        let all_bounds = Rect::from_min_max(
            pos2(origin, lanes_top - gutter),
            pos2(right, canvas.bottom()),
        );
        let mut clips = self.project.clips.clone();
        clips.sort_by_key(|c| self.selection.contains(&c.id));
        let mut drag_ended = false;
        let preview = if let Some(d) = self
            .drag
            .as_ref()
            .filter(|d| matches!(d.kind, DragKind::Move))
        {
            let mut project = self.project.clone();
            let ids: Vec<_> = d.originals.iter().map(|c| c.id).collect();
            let target = match d.target {
                TrackDrop::Existing(i) => i as isize,
                TrackDrop::BeforeFirst => -1,
                TrackDrop::AfterLast => d.track_count as isize,
            };
            if project.move_group(&ids, target - d.original.track as isize) {
                Some(project)
            } else {
                None
            }
        } else {
            None
        };
        for clip in &clips {
            let moving = self.drag.as_ref().is_some_and(|d| {
                matches!(d.kind, DragKind::Move) && d.originals.iter().any(|c| c.id == clip.id)
            });
            if !moving && (clip.end() < view || clip.start > view + span) {
                continue;
            }
            let top = lanes_top + clip.track as f32 * row_h - self.scroll_y + 7.0;
            let rect = Rect::from_min_max(
                pos2(x_at(clip.start), top),
                pos2(x_at(clip.end()), top + row_h - 14.0),
            );
            let row_bounds = Rect::from_min_max(pos2(origin, top), pos2(right, top + row_h - 14.0))
                .intersect(all_bounds);
            let drawn = if moving {
                let track = preview
                    .as_ref()
                    .and_then(|p| p.clips.iter().find(|c| c.id == clip.id))
                    .map_or(clip.track, |c| c.track);
                rect.translate(vec2(
                    0.0,
                    (track as isize - clip.track as isize) as f32 * row_h,
                ))
            } else {
                rect
            };
            self.draw_clip(
                ui,
                &painter,
                clip,
                drawn,
                if moving { all_bounds } else { row_bounds },
                false,
            );
            let hit = rect.intersect(row_bounds).intersect(ui.clip_rect());
            let hit = if hit.is_positive() {
                hit
            } else {
                Rect::from_min_size(pos2(origin, top), Vec2::ZERO)
            };
            let response = ui.interact(
                hit,
                ui.id().with(("clip", clip.id)),
                Sense::click_and_drag(),
            );
            let kind = response
                .hover_pos()
                .map(|p| clip_drag_kind(clip, rect, zoom, p))
                .unwrap_or(DragKind::Move);
            if response.hovered() {
                ui.ctx().set_cursor_icon(match kind {
                    DragKind::Move => CursorIcon::Grab,
                    DragKind::Gain => CursorIcon::ResizeVertical,
                    _ => CursorIcon::ResizeHorizontal,
                });
            }
            if can_edit && response.clicked() {
                if ui.input(|i| i.modifiers.ctrl) {
                    if !self.selection.remove(&clip.id) {
                        self.selection.insert(clip.id);
                    }
                    self.selected = self.selection.first().copied();
                } else {
                    self.select_single(clip.id);
                }
                self.active_track = clip.track;
            }
            if can_edit && theme::double_click(&response) {
                self.select_single(clip.id);
                if matches!(kind, DragKind::Gain) {
                    self.begin_edit();
                    if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == clip.id) {
                        c.gain = 1.0;
                    }
                    self.persist();
                } else if matches!(kind, DragKind::FadeIn | DragKind::FadeOut) {
                    self.begin_edit();
                    if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == clip.id) {
                        c.set_fade(matches!(kind, DragKind::FadeIn), 0.0);
                    }
                    self.persist();
                } else {
                    self.begin_rename(RenameTarget::Clip(clip.id));
                }
            }
            if can_edit && response.drag_started_by(PointerButton::Primary) {
                self.begin_edit();
                if !self.selection.contains(&clip.id) {
                    self.select_single(clip.id);
                }
                self.selected = Some(clip.id);
                self.active_track = clip.track;
                let originals = self
                    .project
                    .clips
                    .iter()
                    .filter(|c| self.selection.contains(&c.id))
                    .cloned()
                    .collect();
                let press = ui
                    .input(|i| i.pointer.press_origin())
                    .unwrap_or(rect.center());
                self.drag = Some(ClipDrag {
                    id: clip.id,
                    original: clip.clone(),
                    kind: clip_drag_kind(clip, rect, zoom, press),
                    target: TrackDrop::Existing(clip.track),
                    lanes_top,
                    track_count: self.project.tracks.len(),
                    delta: Vec2::ZERO,
                    press,
                    view_at_press: view,
                    zoom_at_press: zoom,
                    end_before: self.project.duration(),
                    originals,
                    clip_height: rect.height(),
                });
                self.snap_release = None;
            }
            if can_edit && response.dragged_by(PointerButton::Primary) {
                if let (Some(delta), Some(p)) =
                    (response.total_drag_delta(), response.interact_pointer_pos())
                {
                    self.apply_drag(clip.id, delta, p.y, row_h, ui.input(|i| i.modifiers.alt));
                    ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                }
            }
            drag_ended |= response.drag_stopped_by(PointerButton::Primary);
            response.clone().on_hover_text(match kind {
                DragKind::Gain => "클립 볼륨 · 위아래로 드래그 · 더블클릭하면 0 dB",
                DragKind::FadeIn => "페이드인 손잡이",
                DragKind::FadeOut => "페이드아웃 손잡이",
                DragKind::TrimLeft | DragKind::TrimRight => "원본 범위 안에서 길이 다듬기",
                _ => "드래그로 이동 · 더블클릭으로 이름 변경 · Ctrl+클릭으로 다중 선택",
            });
            response.context_menu(|ui| {
                if !self.selection.contains(&clip.id) {
                    self.select_single(clip.id);
                }
                self.active_track = clip.track;
                ui.add_enabled_ui(can_edit, |ui| {
                    if ui.button("분할       Ctrl+B").clicked() {
                        self.split();
                        ui.close();
                    }
                    if ui
                        .button("이어붙이기  Ctrl+J")
                        .on_hover_text("같은 원본의 이어진 구간을 다시 합칩니다.")
                        .clicked()
                    {
                        self.join_selected();
                        ui.close();
                    }
                    if ui.button("복사       Ctrl+C").clicked() {
                        self.copy(ui.ctx());
                        ui.close();
                    }
                    if ui
                        .add_enabled(!self.clipboard.is_empty(), Button::new("붙여넣기  Ctrl+V"))
                        .clicked()
                    {
                        self.paste();
                        ui.close();
                    }
                    if ui.button("이름 변경").clicked() {
                        self.begin_rename(RenameTarget::Clip(clip.id));
                        ui.close();
                    }
                    if ui.button("삭제       Delete").clicked() {
                        self.delete_clip();
                        ui.close();
                    }
                });
            });
        }
        if let (Some(marquee), Some(p)) = (&self.marquee, pointer) {
            let start = marquee.start_time;
            let finish = time_at(p.x);
            let a = start.min(finish);
            let b = start.max(finish);
            let y0 = (marquee.start_y - self.scroll_y).min(p.y);
            let y1 = (marquee.start_y - self.scroll_y).max(p.y);
            let mut selected = marquee.base.clone();
            for c in &self.project.clips {
                let top = lanes_top + c.track as f32 * row_h - self.scroll_y + 7.0;
                if c.start < b && c.end() > a && top < y1 && top + row_h - 14.0 > y0 {
                    selected.insert(c.id);
                }
            }
            if self.selection != selected {
                ui.ctx().request_repaint();
            }
            self.cursor = finish;
            self.selection = selected;
            self.selected = self.selection.first().copied();
            let r = Rect::from_two_pos(
                pos2(x_at(start), marquee.start_y - self.scroll_y),
                pos2(p.x, p.y),
            )
            .intersect(all_bounds);
            painter.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0_f32, ACCENT), StrokeKind::Inside);
            if !ui.input(|i| i.pointer.primary_down()) {
                self.marquee = None;
            }
        }
        if self.seeking {
            if let Some(p) = pointer {
                self.cursor = time_at(p.x);
            }
            if !ui.input(|i| i.pointer.primary_down()) {
                self.seeking = false;
            }
        }
        if self.drag.is_some() && (drag_ended || !ui.input(|i| i.pointer.primary_down())) {
            if let (Some(d), Some(p)) = (&self.drag, pointer) {
                let id = d.id;
                let delta = p - d.press;
                self.apply_drag(id, delta, p.y, row_h, ui.input(|i| i.modifiers.alt));
            }
            self.finish_drag();
            ui.ctx().request_repaint();
        }
        if let Some(r) = &self.recorder {
            let top = lanes_top + self.rec_track as f32 * row_h - self.scroll_y + 7.0;
            let rect = Rect::from_min_max(
                pos2(x_at(self.rec_start), top),
                pos2(x_at(live_end), top + row_h - 14.0),
            );
            let p = painter.with_clip_rect(all_bounds.intersect(ui.clip_rect()));
            p.rect_filled(rect, 0.0, Color32::from_rgb(61, 32, 40));
            p.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, RED), StrokeKind::Inside);
            let mut x = rect.left().max(origin);
            while x < rect.right().min(right) {
                let local = (x - rect.left()).max(0.0) as f64 / zoom as f64;
                let from = (local * r.rate as f64 / PEAK_BLOCK as f64) as usize;
                let to = (((local + 3.0 / zoom as f64) * r.rate as f64 / PEAK_BLOCK as f64).ceil()
                    as usize)
                    .max(from + 1)
                    .min(self.live_peaks.len());
                let peak = self
                    .live_peaks
                    .get(from..to)
                    .unwrap_or(&[])
                    .iter()
                    .fold(0.0_f32, |a, p| a.max(p.min.abs()).max(p.max.abs()));
                let h = (peak * (rect.height() * 0.35)).clamp(1.0, rect.height() * 0.4);
                p.line_segment(
                    [pos2(x, rect.center().y - h), pos2(x, rect.center().y + h)],
                    Stroke::new(1.7_f32, RED),
                );
                x += 3.0;
            }
        }
        if let Some(guide) = self.snap_guide {
            if self
                .snap_release
                .is_some_and(|t| t.elapsed() > Duration::from_millis(650))
            {
                self.snap_guide = None;
                self.snap_release = None;
            } else {
                let x = x_at(guide.time);
                if x >= origin && x <= right {
                    painter.line_segment(
                        [pos2(x, canvas.top() + 24.0), pos2(x, canvas.bottom() - 5.0)],
                        Stroke::new(1.2_f32, RED),
                    );
                    painter.rect_filled(
                        Rect::from_center_size(pos2(x, canvas.top() + 25.0), vec2(4.0, 4.0)),
                        0.0,
                        RED,
                    );
                    if let Some(c) = guide
                        .target
                        .and_then(|id| self.project.clips.iter().find(|c| c.id == id))
                    {
                        let top = lanes_top + c.track as f32 * row_h - self.scroll_y + 7.0;
                        painter.line_segment(
                            [pos2(x, top), pos2(x, top + row_h - 14.0)],
                            Stroke::new(3.0_f32, RED),
                        );
                    }
                }
                if self.snap_release.is_some() {
                    ui.ctx().request_repaint_after(Duration::from_millis(650));
                }
            }
        }
        let x = x_at(self.cursor);
        if x >= origin && x <= right {
            let color = if self.recorder.is_some() { RED } else { TEXT };
            painter.line_segment(
                [pos2(x, canvas.top() + 22.0), pos2(x, canvas.bottom() - 5.0)],
                Stroke::new(1.0_f32, color),
            );
            painter.add(Shape::convex_polygon(
                vec![
                    pos2(x - 4.0, canvas.top() + 21.0),
                    pos2(x + 4.0, canvas.top() + 21.0),
                    pos2(x, canvas.top() + 28.0),
                ],
                color,
                Stroke::NONE,
            ));
        }
        ui.advance_cursor_after_rect(canvas);
    }

    fn draw_clip(
        &self,
        ui: &Ui,
        painter: &Painter,
        clip: &Clip,
        rect: Rect,
        bounds: Rect,
        _live: bool,
    ) {
        if !rect.intersect(bounds).is_positive() {
            return;
        }
        let muted = self.project.tracks.get(clip.track).is_some_and(|t| t.muted);
        let color = if muted {
            RED
        } else {
            theme::track_color(clip.track)
        };
        let selected = self.selection.contains(&clip.id)
            || (self.selection.is_empty() && self.selected == Some(clip.id));
        let p = painter.with_clip_rect(bounds.intersect(ui.clip_rect()));
        let source = self.project.sources.get(&clip.source);
        let epsilon = source.map_or(1e-8, |s| 0.5 / s.rate as f64);
        let left_cut = clip.offset > epsilon;
        let right_cut = source.is_some_and(|s| clip.offset + clip.len < s.duration() - epsilon);
        let muted = self.project.tracks.get(clip.track).is_some_and(|t| t.muted);
        let blend = if selected {
            0.34
        } else if muted {
            0.22
        } else {
            0.11
        };
        let tint = Color32::from_rgb(
            (SURFACE.r() as f32 + (color.r() as f32 - SURFACE.r() as f32) * blend) as u8,
            (SURFACE.g() as f32 + (color.g() as f32 - SURFACE.g() as f32) * blend) as u8,
            (SURFACE.b() as f32 + (color.b() as f32 - SURFACE.b() as f32) * blend) as u8,
        );
        theme::pixel_clip(&p, rect, left_cut, right_cut, tint, Stroke::NONE);
        let hovered = ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|point| rect.intersect(bounds).contains(point));
        let gain_active = self
            .drag
            .as_ref()
            .is_some_and(|d| d.id == clip.id && matches!(d.kind, DragKind::Gain));
        let show_gain = hovered || gain_active;
        let gain_y = rect.top() + gain_line_y(clip.gain, rect.height());
        if show_gain {
            let fill = Rect::from_min_max(
                pos2(rect.left() + 4.0, gain_y),
                rect.right_bottom() - vec2(4.0, 4.0),
            );
            p.rect_filled(
                fill,
                0.0,
                color.gamma_multiply(if selected { 0.17 } else { 0.22 }),
            );
        }
        let (fi, fo) = clip.visible_fades();
        let a = pos2(
            rect.left() + (fi * self.zoom as f64) as f32,
            rect.top() + 8.0,
        );
        let b = pos2(
            rect.right() - (fo * self.zoom as f64) as f32,
            rect.top() + 8.0,
        );
        if fi > 0.0 {
            p.add(Shape::convex_polygon(
                vec![rect.left_top(), pos2(rect.left(), rect.bottom()), a],
                Color32::from_black_alpha(115),
                Stroke::NONE,
            ));
        }
        if fo > 0.0 {
            p.add(Shape::convex_polygon(
                vec![b, rect.right_top(), rect.right_bottom()],
                Color32::from_black_alpha(115),
                Stroke::NONE,
            ));
        }
        theme::pixel_clip(
            &p,
            rect,
            left_cut,
            right_cut,
            Color32::TRANSPARENT,
            Stroke::new(
                if selected { 2.0_f32 } else { 0.8 },
                if selected {
                    TEXT
                } else {
                    color.gamma_multiply(0.35)
                },
            ),
        );
        let visible = rect.intersect(bounds);
        if visible.width() < 2.0 {
            return;
        }
        let title = Rect::from_min_max(
            pos2(visible.left() + 9.0, rect.top() + 6.0),
            pos2(visible.right() - 9.0, rect.top() + 25.0),
        );
        if title.width() > 20.0 {
            p.with_clip_rect(title).text(
                title.left_center(),
                Align2::LEFT_CENTER,
                &clip.name,
                FontId::proportional(12.0),
                if selected {
                    TEXT
                } else {
                    color.gamma_multiply(0.9)
                },
            );
        }
        let wave_top = rect.top() + 28.0;
        let wave_bottom = rect.bottom() - 8.0;
        let center = (wave_top + wave_bottom) * 0.5;
        let maximum = ((wave_bottom - wave_top) * 0.5).max(3.0);
        if let Some(source) = source {
            let mut x = visible.left() + 2.0;
            while x < visible.right() - 2.0 {
                let local = (x - rect.left()) as f64 / self.zoom as f64;
                let peak = source.peak(
                    clip.offset + local,
                    clip.offset + local + 3.0 / self.zoom as f64,
                );
                let amplitude = clip.envelope(local).max(0.0);
                let hi = (peak.max * amplitude * maximum).clamp(0.6, maximum);
                let lo = (-peak.min * amplitude * maximum).clamp(0.6, maximum);
                p.line_segment(
                    [pos2(x, center - hi), pos2(x, center + lo)],
                    Stroke::new(
                        1.6_f32,
                        if muted {
                            RED.gamma_multiply(0.7)
                        } else if selected {
                            TEXT
                        } else {
                            color.gamma_multiply(0.7)
                        },
                    ),
                );
                x += 3.0;
            }
        }
        if fi > 0.0 {
            p.line_segment(
                [pos2(rect.left() + 2.0, rect.bottom() - 2.0), a],
                Stroke::new(1.6_f32, if hovered { TEXT } else { color }),
            );
        }
        if fo > 0.0 {
            p.line_segment(
                [b, pos2(rect.right() - 2.0, rect.bottom() - 2.0)],
                Stroke::new(1.6_f32, if hovered { TEXT } else { color }),
            );
        }
        let (_, out_handle) = fade_handle_positions(clip, rect, self.zoom);
        if out_handle.y != b.y {
            p.line_segment([b, out_handle], Stroke::new(1.0_f32, color));
        }
        for point in [a, out_handle] {
            let r = Rect::from_center_size(point, vec2(8.0, 10.0));
            p.rect_filled(r, 0.0, if selected || hovered { TEXT } else { color });
            p.rect_stroke(r, 0.0, Stroke::new(1.0_f32, BG), StrokeKind::Inside);
        }
        if show_gain {
            p.line_segment(
                [
                    pos2(rect.left() + 5.0, gain_y),
                    pos2(rect.right() - 5.0, gain_y),
                ],
                Stroke::new(
                    if gain_active { 2.0_f32 } else { 1.2 },
                    TEXT.gamma_multiply(0.9),
                ),
            );
            if visible.width() > 75.0 {
                let label = Rect::from_min_size(
                    pos2(visible.right() - 64.0, rect.bottom() - 25.0),
                    vec2(58.0, 18.0),
                );
                p.rect_filled(label, 0.0, Color32::from_black_alpha(155));
                let db = 20.0 * clip.gain.max(0.000001).log10();
                p.text(
                    label.center(),
                    Align2::CENTER_CENTER,
                    format!("{db:+.1} dB"),
                    FontId::monospace(12.0),
                    TEXT,
                );
            }
        }
        if selected {
            for x in [rect.left() + 3.0, rect.right() - 3.0] {
                p.line_segment(
                    [pos2(x, rect.bottom() - 16.0), pos2(x, rect.bottom() - 7.0)],
                    Stroke::new(2.0_f32, TEXT),
                );
            }
        }
    }

    fn apply_drag(&mut self, id: u64, delta: Vec2, pointer_y: f32, row_h: f32, alt: bool) {
        let Some(drag) = self.drag.as_ref().filter(|d| d.id == id) else {
            return;
        };
        let original = drag.original.clone();
        let originals = drag.originals.clone();
        let kind = drag.kind;
        let clip_height = drag.clip_height;
        let content_y = pointer_y + self.scroll_y;
        let target = if content_y < drag.lanes_top {
            TrackDrop::BeforeFirst
        } else if content_y >= drag.lanes_top + drag.track_count as f32 * row_h {
            TrackDrop::AfterLast
        } else {
            TrackDrop::Existing(((content_y - drag.lanes_top) / row_h).floor() as usize)
        };
        let mut dx =
            delta.x as f64 / drag.zoom_at_press as f64 + self.view_start - drag.view_at_press;
        let min_start = originals
            .iter()
            .map(|c| c.start)
            .fold(original.start, f64::min);
        if matches!(kind, DragKind::Move) {
            dx = dx.max(-min_start);
        }
        self.snap_guide = None;
        if self.snap && !alt {
            let moving_ids: Vec<_> = originals.iter().map(|c| c.id).collect();
            let moving_edges: Vec<_> = match kind {
                DragKind::Move => originals
                    .iter()
                    .flat_map(|c| [c.start + dx, c.end() + dx])
                    .collect(),
                DragKind::TrimLeft => vec![original.start + dx],
                DragKind::TrimRight => vec![original.end() + dx],
                _ => vec![],
            };
            let mut best = 9.0 / self.zoom as f64;
            let mut correction = 0.0;
            for other in self
                .project
                .clips
                .iter()
                .filter(|c| !moving_ids.contains(&c.id))
            {
                for edge in [other.start, other.end()] {
                    for moving in &moving_edges {
                        let distance = (edge - moving).abs();
                        if distance < best {
                            best = distance;
                            correction = edge - moving;
                            self.snap_guide = Some(SnapGuide {
                                time: edge,
                                target: Some(other.id),
                            });
                        }
                    }
                }
            }
            if self.snap_guide.is_some() {
                dx += correction;
            }
        }
        if matches!(kind, DragKind::Move) {
            for base in &originals {
                if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == base.id) {
                    *c = base.clone();
                    c.start = (base.start + dx).max(0.0);
                }
            }
        } else if let Some(c) = self.project.clips.iter_mut().find(|c| c.id == id) {
            *c = original.clone();
            let source = self.project.sources.get(&c.source);
            let duration = source.map_or(original.offset + original.len, |s| s.duration());
            let min_len = source
                .map_or(0.00001, |s| 1.0 / s.rate as f64)
                .min(original.len);
            match kind {
                DragKind::Gain => {
                    let y = gain_line_y(original.gain, clip_height) + delta.y;
                    c.gain = crate::model::db_gain(gain_from_y(y, clip_height));
                }
                DragKind::FadeIn => c.set_fade(
                    true,
                    (original.visible_fades().0 + dx).clamp(0.0, original.len),
                ),
                DragKind::FadeOut => c.set_fade(
                    false,
                    (original.visible_fades().1 - dx).clamp(0.0, original.len),
                ),
                DragKind::TrimLeft => {
                    let change = dx
                        .clamp(-original.offset, original.len - min_len)
                        .max(-original.start);
                    c.start += change;
                    c.offset += change;
                    c.envelope_offset += change;
                    c.len -= change;
                }
                DragKind::TrimRight => {
                    c.len = (original.len + dx)
                        .clamp(min_len, (duration - original.offset).max(min_len))
                }
                _ => {}
            }
        }
        self.project.constrain_clip(id);
        if let Some(guide) = self.snap_guide {
            let actual = self
                .project
                .clips
                .iter()
                .filter(|c| originals.iter().any(|o| o.id == c.id))
                .flat_map(|c| [c.start, c.end()])
                .any(|edge| (edge - guide.time).abs() < 1e-6);
            if !actual {
                self.snap_guide = None;
            }
        }
        if let Some(drag) = &mut self.drag {
            drag.target = target;
            drag.delta = delta;
        }
    }
    fn finish_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        let ids: Vec<_> = drag.originals.iter().map(|c| c.id).collect();
        if matches!(drag.kind, DragKind::Move) {
            let target = match drag.target {
                TrackDrop::Existing(i) => i as isize,
                TrackDrop::BeforeFirst => -1,
                TrackDrop::AfterLast => drag.track_count as isize,
            };
            if !self
                .project
                .move_group(&ids, target - drag.original.track as isize)
            {
                self.tell("트랙 이동을 적용하지 못했습니다.", true);
            }
            for id in &ids {
                self.project.overwrite_clip(*id);
            }
        } else if !matches!(
            drag.kind,
            DragKind::Gain | DragKind::FadeIn | DragKind::FadeOut
        ) {
            self.project.overwrite_clip(drag.id);
        }
        if let Some(c) = self.project.clips.iter().find(|c| c.id == drag.id) {
            self.active_track = c.track;
        }
        if self.snap_guide.is_some() {
            self.snap_release = Some(Instant::now());
        }
        self.persist();
    }
    fn edge_pan(
        &mut self,
        pointer_x: f32,
        origin: f32,
        right: f32,
        end: f64,
        dt: f32,
        extend: bool,
    ) -> bool {
        let edge = 22.0;
        let speed = if pointer_x < origin + edge {
            -((origin + edge - pointer_x) * 7.0 + 70.0).min(1600.0)
        } else if pointer_x > right - edge {
            ((pointer_x - right + edge) * 7.0 + 70.0).min(1600.0)
        } else {
            0.0
        };
        if speed == 0.0 {
            return false;
        }
        let span = (right - origin) as f64 / self.zoom as f64;
        let max = if extend {
            f64::MAX
        } else {
            (end - span).max(0.0)
        };
        let next = (self.view_start
            + speed as f64 * dt.clamp(0.001, 0.05) as f64 / self.zoom as f64)
            .clamp(0.0, max);
        let moved = (next - self.view_start).abs() > 1e-9;
        self.view_start = next;
        moved
    }

    fn footer(&mut self, ctx: &Context) {
        let host_note = self.host_note.clone();
        TopBottomPanel::bottom("transport").frame(egui::Frame::new().fill(SURFACE).inner_margin(Margin::symmetric(22,12))).show(ctx,|ui|{
            ui.horizontal(|ui|{
                ui.add_enabled_ui(!self.busy(),|ui|{
                    let(rect,response)=ui.allocate_exact_size(vec2(42.0,34.0),Sense::click());
                    theme::pixel_panel(ui.painter(),rect,if response.hovered(){RAISED}else{BG},BORDER);
                    theme::icon(ui.painter(),rect.center(),19.0,if self.playback.is_some(){Icon::Pause}else{Icon::Play},TEXT);
                    if response.on_hover_text("재생 / 일시정지 · Space").clicked(){self.play_pause();}
                    #[cfg(test)]{self.test_rects.push(("play".into(),rect));}
                    ui.add_space(12.0);let before=self.project.clone();
                    let response=theme::value_slider(ui,&mut self.project.master_db,-36.0..=6.0,0.0,166.0,false).on_hover_text("전체 볼륨 · 더블클릭하면 0 dB");
                    #[cfg(test)]{self.test_rects.push(("master-volume".into(),response.rect));}
                    self.finish_slider(before,&response);
                });
                ui.with_layout(Layout::right_to_left(Align::Center),|ui|{
                    if let Some(job)=&self.export_job{let n=job.progress.load(Ordering::Relaxed);ui.add_sized(vec2(170.0,28.0),ProgressBar::new(n as f32/100.0).text(if n==0{"저장 위치".into()}else{format!("{n}%")}));}
                    else{ui.add_enabled_ui(!self.busy()&&!self.project.clips.is_empty(),|ui|{
                        let response=theme::primary(ui,&format!("{} 내보내기",self.export_settings.format.label()),vec2(142.0,34.0)).on_hover_text("내보내기 · Ctrl+E\n오른쪽 클릭하면 마지막 저장 위치를 열 수 있습니다.");
                        #[cfg(test)]{self.test_rects.push(("download".into(),response.rect));}
                        if response.clicked(){self.export_open=true;}
                        response.context_menu(|ui|{if let Some(path)=&self.last_export{if ui.button("저장 위치 열기").clicked(){reveal(path);ui.close();}}else{ui.add_enabled(false,Button::new("아직 저장한 파일 없음"));}});
                    });}
                    if let Some(note)=&host_note{ui.add_space(14.0);ui.add(Label::new(RichText::new(note).size(12.0).color(MUTED)).truncate()).on_hover_text(note);}
                });
            });
        });
    }

    fn settings(&mut self, ctx: &Context) {
        if !self.settings_open {
            return;
        }
        let mut open = true;
        let mut changed = false;
        Window::new("입력 설정").open(&mut open).collapsible(false).resizable(false).default_width(424.0).anchor(Align2::RIGHT_TOP,vec2(-28.0,126.0))
            .frame(egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32,BORDER)).inner_margin(16)).show(ctx,|ui|{
            ui.add_enabled_ui(!self.busy()&&self.playback.is_none(),|ui|{
                ui.horizontal(|ui|{
                    let name=if self.capture_path.is_some(){"기본 마이크"}else{self.devices.get(self.device).map_or("마이크 없음",|d|d.name.as_str())};
                    ComboBox::from_id_salt("settings-mic").selected_text(name).width(344.0).truncate().show_ui(ui,|ui|{for(i,d)in self.devices.iter().enumerate(){ui.selectable_value(&mut self.device,i,&d.name);}}).response.on_hover_text("사용할 입력 장치");
                    if theme::mini_icon_button(ui,Icon::Redo,"입력 장치 새로고침").clicked(){self.devices=audio::input_devices();self.device=0;}
                });
                ui.add_space(12.0);
                Grid::new("input-config").num_columns(2).spacing([18.0,14.0]).show(ui,|ui|{
                    ui.label(RichText::new("채널").color(MUTED));
                    let r=ComboBox::from_id_salt("input-mode").selected_text(self.record_settings.mode.label()).width(280.0).show_ui(ui,|ui|{for mode in [InputMode::Left,InputMode::Right,InputMode::Mix,InputMode::Stereo]{let enabled=!matches!(mode,InputMode::Right|InputMode::Stereo)||self.devices.get(self.device).is_some_and(|d|d.channels>=2);ui.add_enabled_ui(enabled,|ui|{changed|=ui.selectable_value(&mut self.record_settings.mode,mode,mode.label()).on_hover_text(mode.help()).changed();});}}).response;
                    changed|=theme::reset_combo(&mut self.record_settings.mode,InputMode::Left,r).on_hover_text(self.record_settings.mode.help()).changed();ui.end_row();
                    ui.label(RichText::new("샘플레이트").color(MUTED));
                    let r=ComboBox::from_id_salt("input-rate").selected_text(rate_label(self.record_settings.rate)).width(280.0).show_ui(ui,|ui|{for n in [0,44100,48000,96000]{changed|=ui.selectable_value(&mut self.record_settings.rate,n,rate_label(n)).changed();}}).response;
                    changed|=theme::reset_combo(&mut self.record_settings.rate,0,r).on_hover_text("자동 권장. 영상용은 48 kHz, 일반 오디오는 44.1 kHz. 높은 값이 마이크에 없던 음질을 만들지는 않습니다. 더블클릭하면 자동.").changed();ui.end_row();
                    ui.label(RichText::new("입력 크기").color(MUTED));
                    changed|=theme::value_slider(ui,&mut self.record_settings.gain_db,-24.0..=24.0,0.0,280.0,false).on_hover_text("디지털 입력 볼륨. 0 dB가 원래 크기. 더블클릭하면 0 dB. 이미 마이크에서 찢어진 소리는 복구하지 못합니다.").changed();ui.end_row();
                    ui.label(RichText::new("버퍼").color(MUTED));
                    let r=ComboBox::from_id_salt("buffer").selected_text(if self.record_settings.buffer==0{"자동".into()}else{format!("{} frames",self.record_settings.buffer)}).width(280.0).show_ui(ui,|ui|{for n in [0,256,512,1024]{changed|=ui.selectable_value(&mut self.record_settings.buffer,n,if n==0{"자동".into()}else{format!("{n} frames")}).changed();}}).response;
                    changed|=theme::reset_combo(&mut self.record_settings.buffer,0,r).on_hover_text("자동 권장. 작을수록 반응은 빠르지만 끊김에 민감합니다. 더블클릭하면 자동.").changed();ui.end_row();
                });
                ui.add_space(12.0);changed|=ui.checkbox(&mut self.record_settings.overdub,"다른 트랙을 들으며 녹음").on_hover_text("이어폰을 사용하면 재생 소리가 마이크에 다시 들어가는 것을 피할 수 있습니다. 입력 마이크를 직접 모니터링하지 않습니다.").changed();
            });
            if self.busy()||self.playback.is_some(){ui.label(RichText::new("정지하면 설정할 수 있습니다.").color(MUTED));}
        });
        self.settings_open &= open;
        if changed {
            self.persist();
        }
    }

    fn export_window(&mut self, ctx: &Context) {
        if !self.export_open {
            return;
        }
        let mut open = true;
        let mut changed = false;
        let mut start = false;
        Window::new("내보내기").open(&mut open).collapsible(false).resizable(false).default_width(424.0)
            .frame(egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0_f32,BORDER)).inner_margin(16)).show(ctx,|ui|{
            ui.horizontal(|ui|{changed|=ui.selectable_value(&mut self.export_settings.split_tracks,false,"합친 파일").changed();changed|=ui.selectable_value(&mut self.export_settings.split_tracks,true,"트랙별 파일").on_hover_text("각 파일의 시작과 길이를 같게 저장합니다. 음소거한 트랙은 제외합니다.").changed();});ui.add_space(12.0);
            Grid::new("export-settings").num_columns(2).spacing([18.0,14.0]).show(ui,|ui|{
                ui.label(RichText::new("형식").color(MUTED));let r=ComboBox::from_id_salt("format").selected_text(self.export_settings.format.label()).width(280.0).show_ui(ui,|ui|{for n in Format::ALL{changed|=ui.selectable_value(&mut self.export_settings.format,n,n.label()).on_hover_text(n.help()).changed();}}).response;changed|=theme::reset_combo(&mut self.export_settings.format,Format::Wav,r).on_hover_text(self.export_settings.format.help()).changed();ui.end_row();
                ui.label(RichText::new("샘플레이트").color(MUTED));let r=ComboBox::from_id_salt("export-rate").selected_text(rate_label(self.export_settings.rate)).width(280.0).show_ui(ui,|ui|{for n in [44100,48000,96000]{ui.add_enabled_ui(n<=48000||self.export_settings.format!=Format::Mp3,|ui|{changed|=ui.selectable_value(&mut self.export_settings.rate,n,rate_label(n)).changed();});}}).response;changed|=theme::reset_combo(&mut self.export_settings.rate,48000,r).on_hover_text("48 kHz: 영상용. 44.1 kHz: 일반 오디오용. MP3는 최대 48 kHz. 더블클릭하면 48 kHz.").changed();ui.end_row();
                ui.label(RichText::new("채널").color(MUTED));let r=ComboBox::from_id_salt("export-channels").selected_text(if self.export_settings.channels==1{"모노"}else{"스테레오"}).width(280.0).show_ui(ui,|ui|{changed|=ui.selectable_value(&mut self.export_settings.channels,1,"모노").changed();changed|=ui.selectable_value(&mut self.export_settings.channels,2,"스테레오").changed();}).response;changed|=theme::reset_combo(&mut self.export_settings.channels,2,r).on_hover_text("목소리 하나라면 모노로 충분합니다. 스테레오는 좌우를 보존합니다. 더블클릭하면 스테레오.").changed();ui.end_row();
                if matches!(self.export_settings.format,Format::Wav|Format::Flac){ui.label(RichText::new("비트 깊이").color(MUTED));let r=ComboBox::from_id_salt("bits").selected_text(bits_label(self.export_settings.bits)).width(280.0).show_ui(ui,|ui|{for n in [16,24,32]{ui.add_enabled_ui(n!=32||self.export_settings.format==Format::Wav,|ui|{changed|=ui.selectable_value(&mut self.export_settings.bits,n,bits_label(n)).changed();});}}).response;changed|=theme::reset_combo(&mut self.export_settings.bits,24,r).on_hover_text("24 bit: 편집용 권장. 16 bit: 작은 용량. 32 bit float: 과한 믹스 피크도 보존. 더블클릭하면 24 bit.").changed();ui.end_row();}
                else{ui.label(RichText::new("비트레이트").color(MUTED));let r=ComboBox::from_id_salt("bitrate").selected_text(format!("{} kbps",self.export_settings.bitrate)).width(280.0).show_ui(ui,|ui|{for n in [128,192,256,320]{changed|=ui.selectable_value(&mut self.export_settings.bitrate,n,format!("{n} kbps")).changed();}}).response;changed|=theme::reset_combo(&mut self.export_settings.bitrate,192,r).on_hover_text("높을수록 파일은 크고 손실은 적습니다. 목소리는 128~192 kbps, 음악은 256~320 kbps가 무난합니다. 더블클릭하면 192.").changed();ui.end_row();}
            });
            if self.export_settings.format==Format::Mp3&&self.export_settings.rate>48000{self.export_settings.rate=48000;changed=true;}
            if self.export_settings.format==Format::Flac&&self.export_settings.bits==32{self.export_settings.bits=24;changed=true;}
            ui.add_space(12.0);changed|=ui.checkbox(&mut self.export_settings.normalize,"전체 피크 -1 dB").on_hover_text("현재 믹스를 같은 배율로 키우거나 줄입니다. 미리듣기와 최종 크기가 달라질 수 있습니다. 트랙별 파일에도 같은 전체 배율을 적용합니다.").changed();
            if matches!(self.export_settings.format,Format::Wav|Format::Flac)&&self.export_settings.bits<32{changed|=ui.checkbox(&mut self.export_settings.dither,"디더").on_hover_text("정수 비트 깊이로 바꿀 때 작은 변환 왜곡을 완화합니다. 일반적으로 켜두면 됩니다.").changed();}
            ui.add_space(12.0);let label=if self.export_settings.format==Format::Wav&&!self.export_settings.split_tracks{let mb=self.project.duration()*self.export_settings.rate as f64*self.export_settings.channels as f64*self.export_settings.bits as f64/8.0/1_000_000.0;format!("저장 위치 선택 / {mb:.1} MB")}else{"저장 위치 선택".into()};if theme::primary(ui,&label,vec2(ui.available_width(),34.0)).clicked(){start=true;}
        });
        self.export_open &= open;
        if changed {
            self.persist();
        }
        if start {
            self.start_export();
        }
    }
}

impl eframe::App for SoriApp {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.capture_frame(ctx);
        self.poll(ctx);
        self.keyboard(ctx);
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            if self.recorder.is_some() || self.export_job.is_some() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.close_dialog = true;
            } else {
                self.persist();
            }
        }
        self.footer(ctx);
        CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG)
                    .inner_margin(Margin::symmetric(28, 18)),
            )
            .show(ctx, |ui| {
                self.recorder_panel(ui);
                ui.add_space(15.);
                self.timeline(ui);

                if self.message_error {
                    ui.add(
                        Label::new(RichText::new(&self.message).size(12.0).color(RED)).truncate(),
                    )
                    .on_hover_text(&self.message);
                }
            });
        self.settings(ctx);
        self.export_window(ctx);
        self.rename_window(ctx);
        self.rename_window(ctx);
        if self.close_dialog {
            let busy_export = self.export_job.is_some();
            Window::new("작업을 마무리할까요?")
                .collapsible(false)
                .resizable(false)
                .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    if busy_export {
                        ui.label("파일을 만드는 중입니다. 저장이 끝나면 닫아주세요.");
                        if ui.button("계속 기다리기").clicked() {
                            self.close_dialog = false;
                        }
                    } else {
                        ui.label("녹음을 멈추고 받은 구간까지 저장한 뒤 닫습니다.");
                        ui.horizontal(|ui| {
                            if ui.button("계속 녹음").clicked() {
                                self.close_dialog = false;
                            }
                            if ui.button("저장하고 닫기").clicked() {
                                self.stop_record();
                                self.persist();
                                self.allow_close = true;
                                ctx.send_viewport_cmd(ViewportCommand::Close);
                            }
                        });
                    }
                });
        }
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.recorder.is_some() {
            self.stop_record();
        }
        self.persist();
        self.handoff(true);
    }
}

fn clip_drag_kind(clip: &Clip, rect: Rect, zoom: f32, pointer: Pos2) -> DragKind {
    let (a, b) = fade_handle_positions(clip, rect, zoom);
    let da = pointer.distance(a);
    let db = pointer.distance(b);
    if da.min(db) < 14.0 {
        return if da <= db {
            DragKind::FadeIn
        } else {
            DragKind::FadeOut
        };
    }
    if (pointer.x - rect.left()).abs() < 6.0 && pointer.y > rect.top() + 20.0 {
        DragKind::TrimLeft
    } else if (pointer.x - rect.right()).abs() < 6.0 && pointer.y > rect.top() + 20.0 {
        DragKind::TrimRight
    } else if (pointer.y - rect.top() - gain_line_y(clip.gain, rect.height())).abs() < 5.0
        && pointer.x > rect.left() + 8.0
        && pointer.x < rect.right() - 8.0
    {
        DragKind::Gain
    } else {
        DragKind::Move
    }
}

fn fade_handle_positions(clip: &Clip, rect: Rect, zoom: f32) -> (Pos2, Pos2) {
    let (fade_in, fade_out) = clip.visible_fades();
    let a = pos2(
        rect.left() + (fade_in * zoom as f64) as f32,
        rect.top() + 8.0,
    );
    let mut b = pos2(
        rect.right() - (fade_out * zoom as f64) as f32,
        rect.top() + 8.0,
    );
    if (a.x - b.x).abs() < 12.0 {
        b.y += 12.0;
    }
    (a, b)
}

fn gain_line_y(gain: f32, height: f32) -> f32 {
    let db = (20.0 * gain.max(0.000001).log10()).clamp(-48.0, 12.0);
    let mid = height * 0.53;
    let top = 26.0_f32.min(height * 0.4);
    let bottom = height - 7.0;
    if db >= 0.0 {
        mid - db / 12.0 * (mid - top)
    } else {
        mid - db / 48.0 * (bottom - mid)
    }
}
fn gain_from_y(y: f32, height: f32) -> f32 {
    let mid = height * 0.53;
    let top = 26.0_f32.min(height * 0.4);
    let bottom = height - 7.0;
    if y <= mid {
        ((mid - y) / (mid - top) * 12.0).clamp(0.0, 12.0)
    } else {
        (-(y - mid) / (bottom - mid) * 48.0).clamp(-48.0, 0.0)
    }
}

fn rate_label(rate: u32) -> String {
    if rate == 0 {
        "자동".into()
    } else {
        format!("{:.1} kHz", rate as f32 / 1000.)
    }
}
fn bits_label(bits: u16) -> &'static str {
    match bits {
        16 => "16 bit",
        32 => "32 bit float",
        _ => "24 bit",
    }
}
fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("explorer.exe");
        if path.is_file() {
            command.arg(format!("/select,{}", path.display()));
        } else {
            command.arg(path);
        }
        let _ = command.creation_flags(0x08000000).spawn();
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    use crate::model::Source;

    fn fixture() -> SoriApp {
        let mut project = Project::default();
        project.add_source(
            Source::new(1, vec![0.25; 10000], 1000, 1, String::new()),
            10,
            0,
            0.5,
        );
        project.clips[0].len = 2.0;
        project.clips[0].envelope_len = 2.0;
        project.clips[0].fade_in = 0.2;
        project.clips[0].fade_out = 0.2;
        let mut c = project.clips[0].clone();
        c.id = 12;
        c.start = 7.0;
        project.clips.push(c);
        project.add_track();
        let mut c = project.clips[0].clone();
        c.id = 11;
        c.start = 4.0;
        c.track = 1;
        project.clips.push(c);
        SoriApp {
            project,
            history: History::default(),
            record_settings: RecordSettings::default(),
            export_settings: ExportSettings::default(),
            devices: vec![],
            device: 0,
            recorder: None,
            playback: None,
            rec_start: 0.0,
            rec_track: 0,
            rec_id: 0,
            live_peaks: vec![],
            cursor: 0.0,
            active_track: 0,
            selected: None,
            clipboard: vec![],
            selection: Default::default(),
            drag: None,
            zoom: 64.0,
            last_fit_zoom: 0.0,
            view_start: 0.0,
            snap: false,
            level: 0.0,
            pending_edit: None,
            settings_open: false,
            export_open: false,
            export_job: None,
            message: String::new(),
            message_error: false,
            message_at: Instant::now(),
            last_export: None,
            host: None,
            host_export_dir: None,
            host_note: None,
            takes: vec![],
            exports: vec![],
            demo: true,
            close_dialog: false,
            allow_close: false,
            pan: None,
            scroll_y: 0.0,
            capture_path: None,
            capture_frames: 0,
            capture_sent: false,
            seeking: false,
            rename_target: None,
            rename_text: String::new(),
            rename_focus: false,
            marquee: None,
            snap_guide: None,
            snap_release: None,
            test_geometry: None,
            test_rects: vec![],
        }
    }

    struct Harness {
        ctx: Context,
        app: SoriApp,
        time: f64,
        size: Vec2,
        full: bool,
    }
    impl Harness {
        fn new() -> Self {
            let ctx = Context::default();
            theme::setup(&ctx);
            let mut h = Self {
                ctx,
                app: fixture(),
                time: 0.0,
                size: vec2(1180.0, 760.0),
                full: false,
            };
            h.frame(vec![]);
            h.frame(vec![]);
            h
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.03;
            self.app.test_rects.clear();
            let modifiers = events
                .iter()
                .filter_map(|e| match e {
                    Event::Key { modifiers, .. } | Event::PointerButton { modifiers, .. } => {
                        Some(*modifiers)
                    }
                    _ => None,
                })
                .last()
                .unwrap_or_default();
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.time),
                events,
                modifiers,
                focused: true,
                ..Default::default()
            };
            let _ = self.ctx.run(input, |ctx| {
                self.app.keyboard(ctx);
                if self.full {
                    self.app.footer(ctx);
                }
                CentralPanel::default()
                    .frame(egui::Frame::new().inner_margin(if self.full {
                        Margin::symmetric(28, 18)
                    } else {
                        Margin::ZERO
                    }))
                    .show(ctx, |ui| {
                        if self.full {
                            self.app.recorder_panel(ui);
                            ui.add_space(15.0);
                        }
                        self.app.timeline(ui);
                    });
                self.app.rename_window(ctx);
            });
        }
        fn g(&self) -> TestGeometry {
            self.app.test_geometry.unwrap()
        }
        fn at(&self, time: f64, track: f32, fraction: f32) -> Pos2 {
            let g = self.g();
            pos2(
                g.origin + ((time - self.app.view_start) * self.app.zoom as f64) as f32,
                g.lanes_top + (track + fraction) * g.row_h - self.app.scroll_y,
            )
        }
        fn clip(&self, id: u64) -> &Clip {
            self.app.project.clips.iter().find(|c| c.id == id).unwrap()
        }
        fn body(&self, id: u64) -> Pos2 {
            let c = self.clip(id);
            self.at(c.start + c.len * 0.35, c.track as f32, 0.80)
        }
        fn button(pos: Pos2, pressed: bool, button: PointerButton, modifiers: Modifiers) -> Event {
            Event::PointerButton {
                pos,
                pressed,
                button,
                modifiers,
            }
        }
        fn click(&mut self, pos: Pos2) {
            self.click_mod(pos, Modifiers::NONE);
        }
        fn click_mod(&mut self, pos: Pos2, modifiers: Modifiers) {
            self.frame(vec![Event::PointerMoved(pos)]);
            self.frame(vec![Self::button(
                pos,
                true,
                PointerButton::Primary,
                modifiers,
            )]);
            self.frame(vec![Self::button(
                pos,
                false,
                PointerButton::Primary,
                modifiers,
            )]);
        }
        fn drag(&mut self, from: Pos2, to: Pos2) {
            self.frame(vec![Event::PointerMoved(from)]);
            self.frame(vec![Self::button(
                from,
                true,
                PointerButton::Primary,
                Modifiers::NONE,
            )]);
            for f in [0.15, 0.4, 0.8] {
                self.frame(vec![Event::PointerMoved(from + (to - from) * f)]);
            }
            self.frame(vec![
                Event::PointerMoved(to),
                Self::button(to, false, PointerButton::Primary, Modifiers::NONE),
            ]);
            self.frame(vec![]);
        }
        fn seek(&mut self, time: f64) {
            let g = self.g();
            self.click(pos2(
                g.origin + ((time - self.app.view_start) * self.app.zoom as f64) as f32,
                g.lanes_top - 25.0,
            ));
        }
        fn key(&mut self, key: Key, modifiers: Modifiers) {
            self.frame(vec![Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }]);
        }
    }
    #[test]
    fn drag_distance_and_undo_are_exact() {
        let mut h = Harness::new();
        let from = h.body(10);
        let delta = h.app.zoom * 2.0;
        h.drag(from, from + vec2(delta, 0.0));
        assert!((h.clip(10).start - 2.5).abs() < 1e-6);
        h.app.undo(false);
        assert_eq!(h.clip(10).start, 0.5);
    }
    #[test]
    fn plain_empty_drag_selects_multiple_and_moves_them_together() {
        let mut h = Harness::new();
        let from = h.at(0.1, 0.0, 0.02);
        let to = h.at(6.5, 1.0, 0.96);
        h.drag(from, to);
        assert_eq!(h.app.selected_ids(), vec![10, 11]);
        assert!((h.app.cursor - 6.5).abs() < 1e-5);
        let from = h.body(10);
        let delta = h.app.zoom;
        h.drag(from, from + vec2(delta, 0.0));
        assert!((h.clip(10).start - 1.5).abs() < 1e-6);
        assert!((h.clip(11).start - 5.0).abs() < 1e-6);
        assert_eq!(h.clip(12).start, 7.0);
        h.app.undo(false);
        assert_eq!(h.clip(10).start, 0.5);
        assert_eq!(h.clip(11).start, 4.0);
    }
    #[test]
    fn ruler_drag_moves_playhead_without_moving_clips() {
        let mut h = Harness::new();
        let before = h.app.project.clips.clone();
        let g = h.g();
        h.drag(
            pos2(g.origin + h.app.zoom, g.lanes_top - 25.0),
            pos2(g.origin + h.app.zoom * 5.5, g.lanes_top - 25.0),
        );
        assert!((h.app.cursor - 5.5).abs() < 1e-5);
        assert_eq!(h.app.project.clips, before);
    }
    #[test]
    fn pointer_outside_window_keeps_playhead_scrolling() {
        let mut h = Harness::new();
        h.app.zoom *= 4.0;
        h.frame(vec![]);
        let g = h.g();
        let from = pos2(g.origin + 70.0, g.lanes_top - 25.0);
        h.frame(vec![
            Event::PointerMoved(from),
            Harness::button(from, true, PointerButton::Primary, Modifiers::NONE),
        ]);
        h.frame(vec![Event::PointerMoved(pos2(1270.0, from.y))]);
        let before = h.app.view_start;
        for _ in 0..12 {
            h.frame(vec![]);
        }
        assert!(h.app.view_start > before);
        assert!(h.app.cursor > 2.0);
        h.frame(vec![Harness::button(
            pos2(1270.0, from.y),
            false,
            PointerButton::Primary,
            Modifiers::NONE,
        )]);
    }
    #[test]
    fn track_creation_and_empty_track_removal_follow_drop() {
        let mut h = Harness::new();
        let from = h.body(10);
        let g = h.g();
        h.drag(from, pos2(from.x, g.lanes_top + 2.3 * g.row_h));
        assert_eq!(h.app.project.tracks.len(), 3);
        assert_eq!(h.clip(10).track, 2);
        let from = h.body(10);
        let g = h.g();
        h.drag(from, pos2(from.x, g.lanes_top - 7.0));
        assert_eq!(h.clip(10).track, 0);
        assert_eq!(h.app.project.tracks.len(), 3);
        let from = h.body(12);
        let g = h.g();
        h.drag(from, pos2(from.x, g.lanes_top + g.row_h * 0.8));
        assert_eq!(h.app.project.tracks.len(), 2);
        h.app.undo(false);
        assert_eq!(h.app.project.tracks.len(), 3);
    }
    #[test]
    fn wheel_zoom_is_anchored_and_middle_drag_pans() {
        let mut h = Harness::new();
        let p = h.at(2.0, 0.0, 0.5);
        let initial = h.app.zoom;
        h.frame(vec![
            Event::PointerMoved(p),
            Event::MouseWheel {
                unit: MouseWheelUnit::Point,
                delta: vec2(0.0, 180.0),
                modifiers: Modifiers::NONE,
            },
        ]);
        assert!(h.app.zoom > initial);
        let time = h.app.view_start + (p.x - h.g().origin) as f64 / h.app.zoom as f64;
        assert!((time - 2.0).abs() < 1e-5);
        let before = h.app.view_start;
        h.frame(vec![Harness::button(
            p,
            true,
            PointerButton::Middle,
            Modifiers::NONE,
        )]);
        h.frame(vec![Event::PointerMoved(p - vec2(70.0, 0.0))]);
        h.frame(vec![Harness::button(
            p - vec2(70.0, 0.0),
            false,
            PointerButton::Middle,
            Modifiers::NONE,
        )]);
        assert!(h.app.view_start > before);
    }
    #[test]
    fn five_rows_fill_minimum_window_and_no_tail_remains() {
        let mut h = Harness::new();
        h.full = true;
        h.size = vec2(940.0, 680.0);
        h.frame(vec![]);
        let g = h.g();
        assert!(g.row_h >= 63.0, "row height {}", g.row_h);
        let viewport = (h.size.x - 28.0 - 10.0 - g.origin) as f64 / h.app.zoom as f64;
        assert!((h.app.view_start + viewport - h.app.project.duration()).abs() < 0.01);
        h.app.project.clips.retain(|c| c.id != 12);
        h.frame(vec![]);
        let viewport = (h.size.x - 28.0 - 10.0 - h.g().origin) as f64 / h.app.zoom as f64;
        assert!((h.app.view_start + viewport - h.app.project.duration()).abs() < 0.01);
    }
    #[test]
    fn fades_and_source_trim_limits_use_total_drag() {
        let mut h = Harness::new();
        let g = h.g();
        let from = pos2(g.origin + 0.7 * h.app.zoom, g.lanes_top + 15.0);
        let delta = h.app.zoom;
        h.drag(from, from + vec2(delta, 0.0));
        assert!((h.clip(10).fade_in - 1.2).abs() < 0.002);
        let g = h.g();
        let right = pos2(
            g.origin + ((h.clip(10).end() - h.app.view_start) * h.app.zoom as f64) as f32,
            g.lanes_top + g.row_h - 18.0,
        );
        h.drag(right, right + vec2(h.app.zoom * 30.0, 0.0));
        let c = h.clip(10);
        assert!(c.offset + c.len <= h.app.project.sources[&c.source].duration() + 1e-9);
    }
    #[test]
    fn clip_gain_line_changes_only_gain_and_double_click_resets() {
        let mut h = Harness::new();
        let c = h.clip(10).clone();
        let g = h.g();
        let height = g.row_h - 14.0;
        let from = pos2(
            h.at(1.4, 0.0, 0.0).x,
            g.lanes_top + 7.0 + gain_line_y(c.gain, height),
        );
        h.drag(from, from - vec2(0.0, 10.0));
        assert!(h.clip(10).gain > 1.0);
        assert_eq!(h.clip(10).start, 0.5);
        let c = h.clip(10);
        let line = pos2(from.x, g.lanes_top + 7.0 + gain_line_y(c.gain, height));
        h.click(line);
        h.click(line);
        assert!((h.clip(10).gain - 1.0).abs() < 1e-6);
        assert!(h.app.rename_target.is_none());
    }
    #[test]
    fn n_toggles_object_snap_and_red_guide_targets_other_track() {
        let mut h = Harness::new();
        h.key(Key::N, Modifiers::NONE);
        assert!(h.app.snap);
        let from = h.body(10);
        let desired = (4.0 - 2.0 - 0.5) * h.app.zoom + 3.0;
        h.drag(from, from + vec2(desired, 0.0));
        assert!((h.clip(10).end() - 4.0).abs() < 1e-5);
        let guide = h.app.snap_guide.unwrap();
        assert!((guide.time - 4.0).abs() < 1e-5);
        assert_eq!(guide.target, Some(11));
        h.key(Key::N, Modifiers::NONE);
        assert!(!h.app.snap);
    }
    #[test]
    fn track_slider_number_double_click_resets_without_opening_name() {
        let mut h = Harness::new();
        h.app.project.tracks[0].gain = 1.65;
        h.frame(vec![]);
        let g = h.g();
        let p = pos2(
            108.0,
            g.lanes_top + 3.0 + (g.row_h - 37.0).min(36.0).max(28.0) + 14.0,
        );
        h.click(p);
        h.click(p);
        assert!((h.app.project.tracks[0].gain - 1.0).abs() < 1e-6);
        assert!(h.app.rename_target.is_none());
    }
    #[test]
    fn track_and_clip_double_click_names_survive_compaction() {
        let mut h = Harness::new();
        let p = h.body(10);
        h.click(p);
        h.click(p);
        assert!(matches!(h.app.rename_target, Some(RenameTarget::Clip(10))));
        h.app.rename_text = "첫 녹음".into();
        h.app.commit_rename();
        assert_eq!(h.clip(10).name, "첫 녹음");
        h.frame(vec![]);
        let g = h.g();
        let p = pos2(24.0, g.lanes_top + 17.0);
        h.click(p);
        h.click(p);
        assert!(matches!(h.app.rename_target, Some(RenameTarget::Track(0))));
        h.app.rename_text = "목소리".into();
        h.app.commit_rename();
        h.app.project.compact_tracks();
        assert_eq!(h.app.project.tracks[0].name, "목소리");
    }
    #[test]
    fn keyboard_split_copy_paste_undo_redo_reach_editor() {
        let mut h = Harness::new();
        let p = h.body(10);
        h.click(p);
        h.seek(1.2);
        h.key(Key::B, Modifiers::CTRL);
        assert_eq!(h.app.project.clips.len(), 4);
        h.key(Key::C, Modifiers::CTRL);
        assert!(!h.app.clipboard.is_empty());
        h.seek(3.0);
        h.key(Key::V, Modifiers::CTRL);
        assert_eq!(h.app.project.clips.len(), 5);
        h.key(Key::Z, Modifiers::CTRL);
        assert_eq!(h.app.project.clips.len(), 4);
        h.key(Key::Z, Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(h.app.project.clips.len(), 5);
    }
    #[test]
    fn controls_stay_separate_at_minimum_size_with_five_tracks_and_error() {
        let mut h = Harness::new();
        while h.app.project.tracks.len() < 5 {
            h.app.project.add_track();
            let mut c = h.app.project.clips[0].clone();
            c.id = 50 + h.app.project.tracks.len() as u64;
            c.track = h.app.project.tracks.len() - 1;
            h.app.project.clips.push(c);
        }
        h.full = true;
        h.size = vec2(940.0, 680.0);
        h.app.message_error = true;
        h.app.message = "test".into();
        h.app.last_export = Some(PathBuf::from("test.wav"));
        h.frame(vec![]);
        for (i, (a, ra)) in h.app.test_rects.iter().enumerate() {
            assert!(
                ra.min.x >= 0.0 && ra.max.x <= h.size.x && ra.min.y >= 0.0 && ra.max.y <= h.size.y,
                "outside: {a} {ra:?}"
            );
            for (b, rb) in h.app.test_rects.iter().skip(i + 1) {
                let intersection = ra.intersect(*rb);
                assert!(
                    intersection.width() <= 0.0 || intersection.height() <= 0.0,
                    "overlap: {a} {b} {ra:?} {rb:?}"
                );
            }
        }
    }
    #[test]
    fn deleting_all_and_pasting_restores_audio_without_empty_tracks() {
        let mut h = Harness::new();
        h.app.select_single(10);
        h.app.copy(&h.ctx);
        h.app.selection = [10, 11, 12].into_iter().collect();
        h.app.delete_clip();
        assert!(h.app.project.tracks.is_empty());
        assert!(h.app.project.sources.is_empty());
        h.frame(vec![]);
        h.app.paste();
        assert_eq!(h.app.project.tracks.len(), 1);
        assert_eq!(h.app.project.mix_frame(0.5), [0.25; 2]);
        h.app.undo(false);
        assert!(h.app.project.tracks.is_empty());
        h.frame(vec![]);
    }

    #[test]
    fn unlimited_tracks_scroll_vertically_with_middle_drag_and_remain_selectable() {
        let mut h = Harness::new();
        while h.app.project.tracks.len() < 14 {
            h.app.project.add_track();
            let i = h.app.project.tracks.len() - 1;
            let mut c = h.app.project.clips[0].clone();
            c.id = 100 + i as u64;
            c.track = i;
            h.app.project.clips.push(c);
        }
        h.frame(vec![]);
        let p = h.at(8.5, 3.0, 0.4);
        h.frame(vec![
            Event::PointerMoved(p),
            Harness::button(p, true, PointerButton::Middle, Modifiers::NONE),
        ]);
        h.frame(vec![Event::PointerMoved(p - vec2(0.0, 500.0))]);
        h.frame(vec![Harness::button(
            p - vec2(0.0, 500.0),
            false,
            PointerButton::Middle,
            Modifiers::NONE,
        )]);
        assert!(h.app.scroll_y > 400.0);
        assert_eq!(h.app.project.tracks.len(), 14);
        let p = h.body(106);
        h.click(p);
        assert_eq!(h.app.selected, Some(106));
        h.seek(1.0);
        assert!((h.app.cursor - 1.0).abs() < 1e-5);
    }

    #[test]
    fn meeting_fade_handles_remain_independently_draggable_and_push_each_other() {
        let mut h = Harness::new();
        let g = h.g();
        let c = h.clip(10).clone();
        let rect = Rect::from_min_max(
            pos2(h.at(c.start, 0.0, 0.0).x, g.lanes_top + 7.0),
            pos2(h.at(c.end(), 0.0, 0.0).x, g.lanes_top + g.row_h - 7.0),
        );
        let (a, _) = fade_handle_positions(&c, rect, h.app.zoom);
        h.drag(a, a + vec2(h.app.zoom * 1.6, 0.0));
        assert!((h.clip(10).fade_in - 1.8).abs() < 0.002);
        assert!(h.clip(10).fade_in + h.clip(10).fade_out <= 2.000001);
        let (_, b) = fade_handle_positions(h.clip(10), rect, h.app.zoom);
        h.drag(b, b - vec2(h.app.zoom * 1.5, 0.0));
        assert!((h.clip(10).fade_out - 1.7).abs() < 0.002);
        assert!((h.clip(10).fade_in - 0.3).abs() < 0.002);
    }
}
