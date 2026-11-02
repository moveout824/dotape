use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

pub const PEAK_BLOCK: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackDrop {
    Existing(usize),
    BeforeFirst,
    AfterLast,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Peak {
    pub min: f32,
    pub max: f32,
}

#[derive(Clone)]
pub struct Source {
    pub id: u64,
    pub samples: Vec<f32>,
    pub rate: u32,
    pub channels: usize,
    pub peaks: Vec<Vec<Peak>>,
    pub path: String,
}

impl Source {
    pub fn new(id: u64, samples: Vec<f32>, rate: u32, channels: usize, path: String) -> Self {
        let mut base = Vec::new();
        for block in samples.chunks(PEAK_BLOCK * channels) {
            let mut p = Peak::default();
            for &v in block {
                p.min = p.min.min(v);
                p.max = p.max.max(v);
            }
            base.push(p);
        }
        let mut peaks = vec![base];
        while peaks.last().is_some_and(|p| p.len() > 1) {
            let next = peaks
                .last()
                .unwrap()
                .chunks(4)
                .map(|c| {
                    c.iter().fold(Peak::default(), |p, q| Peak {
                        min: p.min.min(q.min),
                        max: p.max.max(q.max),
                    })
                })
                .collect();
            peaks.push(next);
        }
        Self {
            id,
            samples,
            rate,
            channels,
            peaks,
            path,
        }
    }
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }
    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.rate as f64
    }
    pub fn sample(&self, frame: f64, channel: usize) -> f32 {
        if frame < 0.0 || frame >= self.frames() as f64 {
            return 0.0;
        }
        let i = frame as usize;
        let f = (frame - i as f64) as f32;
        let c = channel.min(self.channels - 1);
        let a = self.samples[i * self.channels + c];
        let b = self.samples[i.saturating_add(1).min(self.frames() - 1) * self.channels + c];
        a + (b - a) * f
    }
    pub fn peak(&self, start: f64, end: f64) -> Peak {
        let blocks = ((end - start) * self.rate as f64 / PEAK_BLOCK as f64).max(1.0);
        let level = (blocks.log(4.0).floor() as usize).min(self.peaks.len() - 1);
        let scale = PEAK_BLOCK as f64 * 4.0_f64.powi(level as i32);
        let a = (start.max(0.0) * self.rate as f64 / scale) as usize;
        let b = ((end.max(0.0) * self.rate as f64 / scale).ceil() as usize).max(a + 1);
        self.peaks[level]
            .get(a..b.min(self.peaks[level].len()))
            .unwrap_or(&[])
            .iter()
            .fold(Peak::default(), |p, q| Peak {
                min: p.min.min(q.min),
                max: p.max.max(q.max),
            })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Track {
    pub name: String,
    pub gain: f32,
    pub muted: bool,
}

impl Track {
    pub fn new(_n: usize) -> Self {
        Self {
            name: String::new(),
            gain: 1.0,
            muted: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Clip {
    pub id: u64,
    pub source: u64,
    pub name: String,
    pub track: usize,
    pub start: f64,
    pub offset: f64,
    pub len: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub envelope_offset: f64,
    pub envelope_len: f64,
    pub gain: f32,
}

impl Clip {
    pub fn end(&self) -> f64 {
        self.start + self.len
    }
    pub fn envelope(&self, local: f64) -> f32 {
        let t = local + self.envelope_offset;
        let a = if self.fade_in > 0.0 {
            (t / self.fade_in).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let b = if self.fade_out > 0.0 {
            ((self.envelope_len - t) / self.fade_out).clamp(0.0, 1.0)
        } else {
            1.0
        };
        (a * b) as f32 * self.gain
    }
    pub fn set_fade(&mut self, is_in: bool, seconds: f64) {
        // A newly dragged handle describes the visible clip, including after a split.
        let fi = (self.fade_in - self.envelope_offset).clamp(0.0, self.len);
        let fo = (self.fade_out - (self.envelope_len - self.envelope_offset - self.len))
            .clamp(0.0, self.len);
        self.envelope_offset = 0.0;
        self.envelope_len = self.len;
        self.fade_in = fi;
        self.fade_out = fo;
        if is_in {
            self.fade_in = seconds.clamp(0.0, self.len);
            self.fade_out = self.fade_out.min((self.len - self.fade_in).max(0.0));
        } else {
            self.fade_out = seconds.clamp(0.0, self.len);
            self.fade_in = self.fade_in.min((self.len - self.fade_out).max(0.0));
        }
    }
    pub fn visible_fades(&self) -> (f64, f64) {
        (
            (self.fade_in - self.envelope_offset).clamp(0.0, self.len),
            (self.fade_out - (self.envelope_len - self.envelope_offset - self.len))
                .clamp(0.0, self.len),
        )
    }
}

#[derive(Clone)]
pub struct Project {
    pub name: String,
    pub tracks: Vec<Track>,
    pub clips: Vec<Clip>,
    pub sources: HashMap<u64, Arc<Source>>,
    pub master_db: f32,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            name: "이름 없는 녹음".into(),
            tracks: vec![],
            clips: vec![],
            sources: HashMap::new(),
            master_db: 0.0,
        }
    }
}

impl Project {
    /// Return the old-to-new track mapping. An empty project has no lanes.
    pub fn compact_tracks(&mut self) -> Vec<Option<usize>> {
        let mut used = vec![false; self.tracks.len()];
        for clip in &self.clips {
            if let Some(slot) = used.get_mut(clip.track) {
                *slot = true;
            }
        }
        let mut mapping = vec![None; self.tracks.len()];
        let mut tracks = Vec::new();
        for (old, track) in self.tracks.iter().enumerate() {
            if used[old] {
                mapping[old] = Some(tracks.len());
                tracks.push(track.clone());
            }
        }
        for clip in &mut self.clips {
            clip.track = mapping.get(clip.track).and_then(|v| *v).unwrap_or(0);
        }
        self.tracks = tracks;
        mapping
    }

    /// Move and compact in a single transaction, without an artificial track limit.
    #[cfg(test)]
    pub fn move_clip(&mut self, id: u64, start: f64, target: TrackDrop) -> bool {
        let Some(index) = self.clips.iter().position(|c| c.id == id) else {
            return false;
        };
        if !start.is_finite() {
            return false;
        }
        if let TrackDrop::Existing(track) = target {
            if track >= self.tracks.len() {
                return false;
            }
            self.clips[index].start = start.max(0.0);
            self.clips[index].track = track;
        } else {
            let old_track = self.clips[index].track;
            let is_only = self.clips.iter().filter(|c| c.track == old_track).count() == 1;
            let settings = if is_only {
                self.tracks[old_track].clone()
            } else {
                Track::new(1)
            };
            let mut clip = self.clips.remove(index);
            self.compact_tracks();
            if self.clips.is_empty() {
                self.tracks.clear();
            }
            clip.start = start.max(0.0);
            match target {
                TrackDrop::BeforeFirst => {
                    self.tracks.insert(0, settings);
                    for other in &mut self.clips {
                        other.track += 1;
                    }
                    clip.track = 0;
                }
                TrackDrop::AfterLast => {
                    clip.track = self.tracks.len();
                    self.tracks.push(settings);
                }
                TrackDrop::Existing(_) => unreachable!(),
            }
            self.clips.insert(index.min(self.clips.len()), clip);
        }
        self.compact_tracks();
        true
    }
    pub fn duration(&self) -> f64 {
        self.clips.iter().map(Clip::end).fold(0.0, f64::max)
    }
    pub fn add_track(&mut self) -> bool {
        self.tracks.push(Track::new(self.tracks.len() + 1));
        true
    }
    pub fn add_source(&mut self, source: Source, id: u64, track: usize, start: f64) {
        if self.tracks.is_empty() {
            self.tracks.push(Track::new(1));
        }
        let len = source.duration();
        self.clips.push(Clip {
            id,
            source: source.id,
            name: format!("녹음 {:02}", self.sources.len() + 1),
            track: track.min(self.tracks.len() - 1),
            start: start.max(0.0),
            offset: 0.0,
            len,
            fade_in: 0.0,
            fade_out: 0.0,
            envelope_offset: 0.0,
            envelope_len: len,
            gain: 1.0,
        });
        self.sources.insert(source.id, Arc::new(source));
    }
    pub fn split(&mut self, id: u64, at: f64, new_id: u64) -> bool {
        let Some(index) = self.clips.iter().position(|c| c.id == id) else {
            return false;
        };
        let clip = &self.clips[index];
        let cut = at - clip.start;
        if cut <= 0.000_01 || cut >= clip.len - 0.000_01 {
            return false;
        }
        let mut right = clip.clone();
        right.id = new_id;
        right.start = at;
        right.offset += cut;
        right.envelope_offset += cut;
        right.len -= cut;
        self.clips[index].len = cut;
        self.clips.insert(index + 1, right);
        true
    }
    pub fn constrain_clip(&mut self, id: u64) {
        if let Some(c) = self.clips.iter_mut().find(|c| c.id == id) {
            if let Some(source) = self.sources.get(&c.source) {
                c.offset = c.offset.clamp(0.0, source.duration());
                c.len = c.len.clamp(0.0, (source.duration() - c.offset).max(0.0));
                c.start = c.start.max(0.0);
            }
        }
    }

    pub fn move_group(&mut self, ids: &[u64], track_delta: isize) -> bool {
        let selected: std::collections::HashSet<_> = ids.iter().copied().collect();
        let occupied: std::collections::BTreeSet<isize> = self
            .clips
            .iter()
            .map(|c| {
                c.track as isize
                    + if selected.contains(&c.id) {
                        track_delta
                    } else {
                        0
                    }
            })
            .collect();
        let keys: Vec<_> = occupied.into_iter().collect();
        let old = self.tracks.clone();
        let tracks = keys
            .iter()
            .map(|key| {
                old.get(*key as usize)
                    .cloned()
                    .or_else(|| {
                        let from = *key - track_delta;
                        if from < 0 {
                            return None;
                        }
                        let all_selected = self
                            .clips
                            .iter()
                            .filter(|c| c.track == from as usize)
                            .all(|c| selected.contains(&c.id));
                        if all_selected {
                            old.get(from as usize).cloned()
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| Track::new(1))
            })
            .collect();
        for c in &mut self.clips {
            let key = c.track as isize
                + if selected.contains(&c.id) {
                    track_delta
                } else {
                    0
                };
            c.track = keys.binary_search(&key).unwrap();
        }
        self.tracks = tracks;
        true
    }

    /// The placed clip wins. Keep only the uncovered source ranges of clips below it.
    pub fn overwrite_clip(&mut self, id: u64) {
        self.constrain_clip(id);
        let Some(placed) = self.clips.iter().find(|c| c.id == id).cloned() else {
            return;
        };
        let mut next_id = self
            .clips
            .iter()
            .map(|c| c.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut result = Vec::with_capacity(self.clips.len() + 2);
        for other in self.clips.drain(..) {
            if other.id == id
                || other.track != placed.track
                || other.end() <= placed.start + 1e-9
                || other.start >= placed.end() - 1e-9
            {
                result.push(other);
                continue;
            }
            let left = other.start < placed.start - 1e-9;
            if left {
                let mut c = other.clone();
                c.len = placed.start - other.start;
                result.push(c);
            }
            if other.end() > placed.end() + 1e-9 {
                let cut = placed.end() - other.start;
                let mut c = other.clone();
                c.start = placed.end();
                c.offset += cut;
                c.envelope_offset += cut;
                c.len -= cut;
                if left {
                    c.id = next_id;
                    next_id = next_id.saturating_add(1);
                }
                result.push(c);
            }
        }
        self.clips = result;
    }

    pub fn resolve_overlaps(&mut self) {
        let original = std::mem::take(&mut self.clips);
        for clip in original {
            let id = clip.id;
            self.clips.push(clip);
            self.overwrite_clip(id);
        }
    }

    /// Rejoin contiguous timeline and source ranges. A split can be reversed without stretching.
    pub fn join_clip(&mut self, id: u64) -> bool {
        let mut joined = false;
        loop {
            let Some(selected) = self.clips.iter().find(|c| c.id == id).cloned() else {
                break;
            };
            let tolerance = self
                .sources
                .get(&selected.source)
                .map_or(1e-8, |s| 0.51 / s.rate as f64);
            let neighbor = self
                .clips
                .iter()
                .find(|c| {
                    if c.id == id
                        || c.track != selected.track
                        || c.source != selected.source
                        || (c.gain - selected.gain).abs() > 1e-6
                    {
                        return false;
                    }
                    ((c.end() - selected.start).abs() < tolerance
                        && (c.offset + c.len - selected.offset).abs() < tolerance)
                        || ((selected.end() - c.start).abs() < tolerance
                            && (selected.offset + selected.len - c.offset).abs() < tolerance)
                })
                .cloned();
            let Some(other) = neighbor else {
                break;
            };
            let (left, right) = if selected.start <= other.start {
                (&selected, &other)
            } else {
                (&other, &selected)
            };
            let mut combined = left.clone();
            combined.id = id;
            combined.len = right.end() - left.start;
            let same_envelope = (left.envelope_len - right.envelope_len).abs() < tolerance
                && (left.envelope_offset + left.len - right.envelope_offset).abs() < tolerance
                && (left.fade_in - right.fade_in).abs() < tolerance
                && (left.fade_out - right.fade_out).abs() < tolerance;
            if !same_envelope {
                combined.fade_in = (left.fade_in - left.envelope_offset).clamp(0.0, combined.len);
                combined.fade_out = (right.fade_out
                    - (right.envelope_len - right.envelope_offset - right.len))
                    .clamp(0.0, combined.len);
                combined.envelope_offset = 0.0;
                combined.envelope_len = combined.len;
            }
            self.clips.retain(|c| c.id != id && c.id != other.id);
            self.clips.push(combined);
            self.constrain_clip(id);
            joined = true;
        }
        joined
    }

    pub fn mix_frame(&self, seconds: f64) -> [f32; 2] {
        let mut out = [0.0; 2];
        for clip in &self.clips {
            let Some(track) = self.tracks.get(clip.track) else {
                continue;
            };
            if track.muted || seconds < clip.start || seconds >= clip.end() {
                continue;
            }
            let Some(source) = self.sources.get(&clip.source) else {
                continue;
            };
            let local = seconds - clip.start;
            let frame = (local + clip.offset) * source.rate as f64;
            let gain = clip.envelope(local) * track.gain;
            out[0] += source.sample(frame, 0) * gain;
            out[1] += source.sample(frame, 1) * gain;
        }
        let master = db_gain(self.master_db);
        [out[0] * master, out[1] * master]
    }
}

pub fn db_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

#[derive(Default)]
pub struct History {
    undo: Vec<Project>,
    redo: Vec<Project>,
}
impl History {
    pub fn push(&mut self, p: &Project) {
        self.undo.push(p.clone());
        if self.undo.len() > 80 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self, p: &mut Project) -> bool {
        if let Some(v) = self.undo.pop() {
            self.redo.push(std::mem::replace(p, v));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, p: &mut Project) -> bool {
        if let Some(v) = self.redo.pop() {
            self.undo.push(std::mem::replace(p, v));
            true
        } else {
            false
        }
    }
}

pub fn time_label(t: f64) -> String {
    let ms = (t.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}.{:03}",
        ms / 60_000,
        (ms / 1000) % 60,
        ms % 1000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> Project {
        let mut p = Project::default();
        p.add_source(
            Source::new(1, vec![0.5; 48000], 48000, 1, String::new()),
            2,
            0,
            0.0,
        );
        p
    }

    #[test]
    fn new_lanes_and_empty_lanes_form_one_undoable_edit() {
        let mut p = project();
        let mut second = p.clips[0].clone();
        second.id = 3;
        second.start = 2.0;
        p.clips.push(second);
        let before = p.clone();
        let mut h = History::default();
        h.push(&p);
        assert!(p.move_clip(3, 3.0, TrackDrop::AfterLast));
        assert_eq!(p.tracks.len(), 2);
        assert_eq!(p.clips.iter().find(|c| c.id == 3).unwrap().track, 1);
        assert!(h.undo(&mut p));
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.clips, before.clips);
        assert!(h.redo(&mut p));
        assert!(p.move_clip(2, 0.0, TrackDrop::Existing(1)));
        assert_eq!(p.tracks.len(), 1);
        assert!(p.clips.iter().all(|c| c.track == 0));
        assert!(p.move_clip(3, 1.0, TrackDrop::BeforeFirst));
        assert_eq!(p.tracks.len(), 2);
        assert_eq!(p.clips.iter().find(|c| c.id == 3).unwrap().track, 0);
    }

    #[test]
    fn occupied_lanes_can_be_reordered_and_expanded_beyond_five() {
        let mut p = project();
        for i in 1..5 {
            p.add_track();
            let mut c = p.clips[0].clone();
            c.id = 10 + i as u64;
            c.track = i;
            p.clips.push(c);
        }
        assert!(p.move_clip(2, 0.0, TrackDrop::AfterLast));
        assert_eq!(p.tracks.len(), 5);
        let mut c = p.clips[0].clone();
        c.id = 99;
        p.clips.push(c);
        assert!(p.move_clip(99, 4.0, TrackDrop::BeforeFirst));
        assert_eq!(p.tracks.len(), 6);
        p.clips.clear();
        p.compact_tracks();
        assert_eq!(p.tracks.len(), 0);
    }
    #[test]
    fn split_preserves_audio_and_fades() {
        let mut p = project();
        p.clips[0].fade_in = 0.6;
        p.clips[0].fade_out = 0.7;
        let reference = p.clone();
        assert!(p.split(2, 0.4, 3));
        for i in 0..48000 {
            let t = i as f64 / 48000.0;
            assert!((p.mix_frame(t)[0] - reference.mix_frame(t)[0]).abs() < 0.000001);
        }
        assert!(!p.split(2, 0.0, 4));
        assert!(!p.split(2, 0.4, 4));
    }
    #[test]
    fn overlap_mute_and_offset_are_sample_correct() {
        let mut p = project();
        p.add_track();
        let mut c = p.clips[0].clone();
        c.id = 3;
        c.track = 1;
        c.start = 0.5;
        c.offset = 0.5;
        c.len = 0.5;
        p.clips.push(c);
        assert_eq!(p.mix_frame(0.25), [0.5; 2]);
        assert_eq!(p.mix_frame(0.75), [1.0; 2]);
        p.tracks[1].muted = true;
        assert_eq!(p.mix_frame(0.75), [0.5; 2]);
        assert_eq!(p.mix_frame(1.0), [0.0; 2]);
    }
    #[test]
    fn history_shares_audio_and_redo_is_invalidated() {
        let mut p = project();
        let mut h = History::default();
        h.push(&p);
        let source = p.sources[&1].clone();
        p.clips[0].start = 2.0;
        assert!(h.undo(&mut p));
        assert_eq!(p.clips[0].start, 0.0);
        assert!(Arc::ptr_eq(&source, &p.sources[&1]));
        assert!(h.redo(&mut p));
        assert_eq!(p.clips[0].start, 2.0);
        h.undo(&mut p);
        h.push(&p);
        assert!(!h.can_redo());
    }
    #[test]
    fn track_growth_and_peak_queries() {
        let mut p = project();
        for _ in 0..24 {
            assert!(p.add_track());
        }
        assert_eq!(p.tracks.len(), 25);
        assert_eq!(p.sources[&1].peak(0.0, 0.8).max, 0.5);
        assert_eq!(p.sources[&1].peak(3.0, 4.0).max, 0.0);
    }

    #[test]
    fn overwrite_keeps_uncovered_audio_and_never_doubles_the_overlap() {
        let mut p = project();
        p.clips[0].fade_in = 0.3;
        p.clips[0].fade_out = 0.3;
        let before = p.clone();
        p.add_source(
            Source::new(9, vec![0.25; 9600], 48000, 1, String::new()),
            20,
            0,
            0.4,
        );
        p.overwrite_clip(20);
        assert_eq!(p.clips.len(), 3);
        assert!((p.mix_frame(0.5)[0] - 0.25).abs() < 1e-6);
        for t in [0.1, 0.2, 0.7, 0.8, 0.9] {
            assert!((p.mix_frame(t)[0] - before.mix_frame(t)[0]).abs() < 1e-6);
        }
        assert!(p.clips.iter().all(|a| p.clips.iter().all(|b| a.id == b.id
            || a.track != b.track
            || a.end() <= b.start + 1e-8
            || b.end() <= a.start + 1e-8)));
    }
    #[test]
    fn joining_a_split_restores_original_samples_and_source_bounds_hold() {
        let mut p = project();
        p.clips[0].fade_in = 0.6;
        p.clips[0].fade_out = 0.7;
        let before = p.clone();
        p.split(2, 0.4, 3);
        assert!(p.join_clip(3));
        assert_eq!(p.clips.len(), 1);
        for i in 0..48000 {
            let t = i as f64 / 48000.0;
            assert!((p.mix_frame(t)[0] - before.mix_frame(t)[0]).abs() < 1e-6);
        }
        p.clips[0].offset = 0.7;
        p.clips[0].len = 100.0;
        p.constrain_clip(3);
        assert!((p.clips[0].len - 0.3).abs() < 1e-8);
    }
    #[test]
    fn legacy_overlap_resolution_is_last_clip_wins() {
        let mut p = project();
        p.add_source(
            Source::new(9, vec![0.2; 48000], 48000, 1, String::new()),
            20,
            0,
            0.0,
        );
        p.resolve_overlaps();
        assert_eq!(p.clips.len(), 1);
        assert_eq!(p.clips[0].id, 20);
        assert_eq!(p.mix_frame(0.5), [0.2; 2]);
    }
    #[test]
    fn track_names_survive_compaction_and_group_move() {
        let mut p = project();
        p.tracks[0].name = "목소리".into();
        p.add_track();
        let mut c = p.clips[0].clone();
        c.id = 5;
        c.track = 1;
        p.clips.push(c);
        p.clips.retain(|c| c.id != 2);
        p.compact_tracks();
        p.tracks[0].name = "효과음".into();
        assert!(p.move_group(&[5], 1));
        assert_eq!(p.tracks[0].name, "효과음");
    }

    #[test]
    fn editing_one_fade_pushes_the_opposite_fade_without_overlap() {
        let mut p = project();
        let c = &mut p.clips[0];
        c.fade_in = 0.3;
        c.fade_out = 0.4;
        c.set_fade(true, 0.8);
        assert!((c.fade_in - 0.8).abs() < 1e-9);
        assert!((c.fade_out - 0.2).abs() < 1e-9);
        c.set_fade(false, 0.9);
        assert!((c.fade_out - 0.9).abs() < 1e-9);
        assert!((c.fade_in - 0.1).abs() < 1e-9);
        c.set_fade(true, 20.0);
        assert_eq!(c.fade_in, 1.0);
        assert_eq!(c.fade_out, 0.0);
    }
}
