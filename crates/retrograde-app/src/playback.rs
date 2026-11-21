use std::collections::{HashMap, HashSet};
use std::f64::consts::FRAC_PI_2;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use retrograde_core::{Album, PlayStats, SessionState, Track};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off = 0,
    All = 1,
    One = 2,
}

impl RepeatMode {
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Self::All,
            2 => Self::One,
            _ => Self::Off,
        }
    }

    pub fn as_i32(self) -> i32 {
        self as i32
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Repeat: Off",
            Self::All => "Repeat: All",
            Self::One => "Repeat: One",
        }
    }

    pub fn mpris_loop_status(self) -> &'static str {
        match self {
            Self::Off => "None",
            Self::All => "Playlist",
            Self::One => "Track",
        }
    }

    pub fn from_mpris_loop_status(status: &str) -> Self {
        match status {
            "Playlist" => Self::All,
            "Track" => Self::One,
            _ => Self::Off,
        }
    }
}

static SHUFFLE_NONCE: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

fn next_rand_u64() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x517c_c1b7_2722_0a95);
    let mut x = SHUFFLE_NONCE.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed) ^ nanos;
    if x == 0 {
        x = 0xdead_beef_cafe_babe;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

fn shuffle_slice<T>(slice: &mut [T]) {
    let n = slice.len();
    for i in (1..n).rev() {
        let j = (next_rand_u64() as usize) % (i + 1);
        slice.swap(i, j);
    }
}

/// The play queue, mirroring C# `PlayerViewModel` (+ `.Shuffle.cs`).
///
/// `tracks` is the play order: what follows the current track is what plays next, in shuffle too.
/// Shuffle plays the whole library: turning it on keeps the current track first and puts every other
/// library track behind it in random order; turning it off goes back to the queue from before
/// (usually the album), at the current track.
///
/// Tracks queued by hand form a line right after the current track: ADD_TO_QUEUE joins the end of
/// that line, PLAY_NEXT goes to its front. The line plays before the rest of the queue.
/// What [`PlaybackQueue::on_mpv_advanced`] found when mpv moved on into its queued file by itself.
#[derive(Debug, Clone)]
pub enum MpvAdvance {
    /// mpv entered the file it was handed: this track is now current and already playing.
    Followed(Track),
    /// The queue was edited right at the boundary and mpv entered a file that is no longer next:
    /// the intended track is now current and has to be loaded (C# `OnAdvanced` → `PlayIndexAsync`).
    Reload(Track),
    /// Nothing is meant to follow: stop.
    Stop,
}

#[derive(Debug, Default)]
pub struct PlaybackQueue {
    /// Shared with the UI ([`PlaybackQueue::shared_tracks`]): moving to another track only changes
    /// `index`, so sending the queue costs nothing; an edit copies it once (`Arc::make_mut`).
    tracks: Arc<Vec<Track>>,
    /// The queue from before shuffle (C# `_unshuffledQueue`); `None` while not shuffling, or after
    /// a restart that did not keep it.
    unshuffled: Option<Vec<Track>>,
    index: Option<usize>,
    shuffle: bool,
    repeat: RepeatMode,
    /// How many of the tracks right after the current one were queued by hand.
    hand: usize,
}

fn library_tracks(library: &[Album]) -> impl Iterator<Item = &Track> {
    library.iter().flat_map(|a| a.tracks.iter())
}

impl PlaybackQueue {
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// The play order, shared (no copy) for the UI.
    pub fn shared_tracks(&self) -> Arc<Vec<Track>> {
        Arc::clone(&self.tracks)
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn index(&self) -> Option<usize> {
        self.index
    }

    pub fn current(&self) -> Option<&Track> {
        self.index.and_then(|i| self.tracks.get(i))
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    /// How many upcoming tracks were queued by hand (they sit right after the current one).
    pub fn hand_len(&self) -> usize {
        self.hand
    }

    pub fn set_repeat(&mut self, repeat: RepeatMode) {
        self.repeat = repeat;
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat = self.repeat.cycle();
        self.repeat
    }

    pub fn can_go_next(&self) -> bool {
        match self.index {
            Some(i) => i + 1 < self.tracks.len() || (self.repeat == RepeatMode::All && !self.tracks.is_empty()),
            None => !self.tracks.is_empty(),
        }
    }

    pub fn can_go_previous(&self) -> bool {
        match self.index {
            Some(i) => i > 0 || (self.repeat == RepeatMode::All && self.tracks.len() > 1),
            None => false,
        }
    }

    /// A new current track (C# `NoteTrackChange`): moving on to the next track in order uses up one
    /// hand-queued track; a jump anywhere else ends the line (what it skipped stays as ordinary tracks).
    fn note_track_change(&mut self, new_index: usize) {
        let in_order = self.index.is_some_and(|i| i + 1 == new_index);
        self.hand = if in_order { self.hand.saturating_sub(1) } else { 0 };
    }

    fn set_index(&mut self, new_index: usize) {
        self.note_track_change(new_index);
        self.index = Some(new_index);
    }

    /// C# `PlayQueue`: in shuffle, the chosen track first and the rest of the library after it.
    pub fn set_queue(&mut self, tracks: Vec<Track>, start_index: usize, library: &[Album]) {
        if tracks.is_empty() {
            self.clear();
            return;
        }
        self.index = Some(start_index.min(tracks.len() - 1));
        self.tracks = Arc::new(tracks);
        self.unshuffled = None;
        self.hand = 0;
        if self.shuffle {
            self.unshuffled = Some(self.tracks.to_vec());
            self.spread_over_library(library);
            self.build_order();
        }
    }

    pub fn jump_to(&mut self, idx: usize) -> Option<Track> {
        if idx < self.tracks.len() {
            self.set_index(idx);
            self.tracks.get(idx).cloned()
        } else {
            None
        }
    }

    pub fn find_track_near(&self, hint: usize, track_id: &str) -> Option<usize> {
        let len = self.tracks.len();
        if len == 0 {
            return None;
        }
        if hint < len && self.tracks[hint].id == track_id {
            return Some(hint);
        }
        for dist in 1..=len {
            if let Some(left) = hint.checked_sub(dist)
                && left < len
                && self.tracks[left].id == track_id
            {
                return Some(left);
            }
            let right = hint.saturating_add(dist);
            if right < len && self.tracks[right].id == track_id {
                return Some(right);
            }
        }
        None
    }

    pub fn jump_to_track_near(&mut self, hint: usize, track_id: &str) -> Option<usize> {
        let idx = self.find_track_near(hint, track_id)?;
        self.set_index(idx);
        Some(idx)
    }

    pub fn toggle_shuffle(&mut self, library: &[Album]) -> bool {
        self.set_shuffle(!self.shuffle, library);
        self.shuffle
    }

    /// C# `OnIsShuffleChanged`.
    pub fn set_shuffle(&mut self, enabled: bool, library: &[Album]) {
        if self.shuffle == enabled {
            return;
        }
        self.shuffle = enabled;
        self.hand = 0; // the queue is rebuilt around the current track
        if enabled {
            if self.tracks.is_empty() {
                return;
            }
            self.unshuffled = Some(self.tracks.to_vec());
            self.spread_over_library(library);
            self.build_order();
        } else {
            self.return_from_library(library);
        }
    }

    /// C# `SpreadOverLibrary`: the current track first, then every other library track, then queued
    /// tracks the library lacks. Nothing playing or an empty library: the queue stays as it is.
    fn spread_over_library(&mut self, library: &[Album]) {
        let Some(current) = self.current().cloned() else {
            return;
        };
        if library_tracks(library).next().is_none() {
            return;
        }
        let mut seen: HashSet<String> = HashSet::new();
        seen.insert(current.id.clone());
        let mut list = vec![current];
        for t in library_tracks(library).chain(self.tracks.iter()) {
            if seen.insert(t.id.clone()) {
                list.push(t.clone());
            }
        }
        self.tracks = Arc::new(list);
        self.index = Some(0);
    }

    /// C# `BuildOrder`: a fresh round, the current track first and every other track once in
    /// random order.
    fn build_order(&mut self) {
        match self.index {
            Some(cur) if cur < self.tracks.len() => {
                let tracks = Arc::make_mut(&mut self.tracks);
                let current = tracks.remove(cur);
                shuffle_slice(tracks.as_mut_slice());
                tracks.insert(0, current);
                self.index = Some(0);
            }
            _ => shuffle_slice(Arc::make_mut(&mut self.tracks).as_mut_slice()),
        }
    }

    /// C# `ReturnFromLibrary`: back to the queue from before shuffle, at the track now playing.
    fn return_from_library(&mut self, library: &[Album]) {
        let back = self.unshuffled.take();
        let Some(current) = self.current().cloned() else {
            if let Some(back) = back {
                self.tracks = Arc::new(back);
                self.index = None;
            }
            return;
        };
        if back.is_none() {
            // After a restart the queue from before is not known. A queue as big as the library is
            // the shuffle one: go to the current track's album. A smaller one was the user's own: keep it.
            if self.tracks.len() < library_tracks(library).count() {
                return;
            }
        }
        let mut idx = back
            .as_ref()
            .and_then(|b| b.iter().position(|t| t.id == current.id));
        let mut back = back;
        if idx.is_none()
            && let Some(album) = library
                .iter()
                .find(|a| a.tracks.iter().any(|t| t.id == current.id))
        {
            idx = album.tracks.iter().position(|t| t.id == current.id);
            back = Some(album.tracks.clone());
        }
        match (back, idx) {
            (Some(back), Some(idx)) => {
                self.tracks = Arc::new(back);
                self.index = Some(idx);
            }
            _ => {
                self.tracks = Arc::new(vec![current]);
                self.index = Some(0);
            }
        }
    }

    /// Returns the track index that should play next automatically (gapless / EOF), the current
    /// one on `RepeatMode::One`.
    pub fn auto_next_index(&self) -> Option<usize> {
        if self.tracks.is_empty() {
            return None;
        }
        let cur = self.index?;
        if self.repeat == RepeatMode::One {
            return Some(cur);
        }
        if cur + 1 < self.tracks.len() {
            Some(cur + 1)
        } else if self.repeat == RepeatMode::All {
            Some(0)
        } else {
            None
        }
    }

    /// C# `CrossfadeApplies`: whether the change from the current track to `next` fades.
    pub fn crossfade_applies(&self, next: usize, crossfade_seconds: i32, is_direct: bool) -> bool {
        let Some(cur) = self.index else {
            return false;
        };
        if next == cur {
            return false;
        }
        let (Some(current), Some(following)) = (self.tracks.get(cur), self.tracks.get(next)) else {
            return false;
        };
        CrossfadeMath::should_crossfade(
            crossfade_seconds,
            self.repeat,
            is_direct,
            self.shuffle,
            current,
            following,
        )
    }

    pub fn step_next(&mut self, from_eof: bool) -> Option<Track> {
        let len = self.tracks.len();
        if len == 0 {
            return None;
        }
        let cur = match self.index {
            Some(i) => i,
            None => {
                self.hand = 0;
                self.index = Some(0);
                return self.tracks.first().cloned();
            }
        };
        if from_eof && self.repeat == RepeatMode::One {
            self.set_index(cur);
            return self.tracks.get(cur).cloned();
        }
        if cur + 1 < len {
            self.set_index(cur + 1);
            return self.tracks.get(cur + 1).cloned();
        }
        if self.repeat == RepeatMode::All {
            self.hand = 0;
            if self.shuffle && len > 1 {
                // Every track has had its turn: deal a new round, not opening with the one just played.
                let last_id = self.tracks[cur].id.clone();
                let tracks = Arc::make_mut(&mut self.tracks);
                shuffle_slice(tracks.as_mut_slice());
                if tracks[0].id == last_id
                    && let Some(other) = tracks.iter().position(|t| t.id != last_id)
                {
                    tracks.swap(0, other);
                }
            }
            self.index = Some(0);
            return self.tracks.first().cloned();
        }
        None
    }

    pub fn step_prev(&mut self) -> Option<Track> {
        let len = self.tracks.len();
        if len == 0 {
            return None;
        }
        let cur = self.index.unwrap_or(0);
        let target = if cur > 0 {
            cur - 1
        } else if self.repeat == RepeatMode::All {
            len - 1
        } else {
            0
        };
        self.set_index(target);
        self.tracks.get(target).cloned()
    }

    /// mpv ran from the current file into the one queued behind it (`path`); `expected` is the
    /// slot that was handed to it as next (C# `_queuedNextIndex`). When the queue changed at the
    /// boundary (a track queued by hand just as mpv moved on), mpv's file is not the intended one:
    /// the intended next track becomes current instead and must be loaded, so the hand-queued line
    /// is not skipped.
    pub fn on_mpv_advanced(&mut self, path: &str, expected: Option<usize>) -> MpvAdvance {
        if let Some(e) = expected
            && self.tracks.get(e).is_some_and(|t| t.file_path == path)
        {
            self.set_index(e);
            return MpvAdvance::Followed(self.tracks[e].clone());
        }
        match self.auto_next_index() {
            Some(want) => {
                self.set_index(want);
                MpvAdvance::Reload(self.tracks[want].clone())
            }
            None => MpvAdvance::Stop,
        }
    }

    /// PLAY_NEXT: right after the current track, ahead of the tracks queued before. Returns `true`
    /// when nothing was queued and this started a new queue (the caller starts playback).
    pub fn play_next(&mut self, tracks: Vec<Track>, library: &[Album]) -> bool {
        self.enqueue_by_hand(tracks, true, library)
    }

    /// ADD_TO_QUEUE: after the current track and the tracks queued by hand before it, without
    /// interrupting playback. Returns `true` when this started a new queue.
    pub fn enqueue(&mut self, tracks: Vec<Track>, library: &[Album]) -> bool {
        self.enqueue_by_hand(tracks, false, library)
    }

    fn enqueue_by_hand(&mut self, list: Vec<Track>, at_front: bool, library: &[Album]) -> bool {
        if list.is_empty() {
            return false;
        }
        let Some(cur) = self.index.filter(|&i| i < self.tracks.len()) else {
            // Nothing queued: start playing, as addToQueue does in the Electron build.
            self.set_queue(list, 0, library);
            return true;
        };
        let upcoming = self.tracks.len() - cur - 1;
        let offset = if at_front { 0 } else { self.hand.min(upcoming) };
        self.keep_unshuffled_insert(&list, offset);
        let n = list.len();
        let at = cur + 1 + offset;
        Arc::make_mut(&mut self.tracks).splice(at..at, list);
        self.hand = self.hand.min(upcoming) + n;
        false
    }

    /// Tracks queued by hand go to the same place in the queue from before: after the current track
    /// (and the line).
    fn keep_unshuffled_insert(&mut self, list: &[Track], offset: usize) {
        let Some(current_id) = self.current().map(|t| t.id.clone()) else {
            return;
        };
        let Some(back) = self.unshuffled.as_mut() else {
            return;
        };
        let pos = match back.iter().position(|t| t.id == current_id) {
            Some(at) => (at + 1 + offset).min(back.len()),
            None => back.len(),
        };
        back.splice(pos..pos, list.iter().cloned());
    }

    pub fn remove_at(&mut self, idx: usize) -> bool {
        if idx >= self.tracks.len() {
            return false;
        }
        if let Some(cur) = self.index {
            if idx == cur {
                // The track after it becomes current: one hand-queued track used up.
                self.hand = self.hand.saturating_sub(1);
            } else if idx > cur && idx - cur - 1 < self.hand {
                self.hand -= 1;
            }
        }
        let removed = Arc::make_mut(&mut self.tracks).remove(idx);
        if let Some(back) = self.unshuffled.as_mut() {
            // The queue from before keeps no more copies of this track than the play order still
            // has, so a duplicate id elsewhere is never dropped by mistake.
            let left = self.tracks.iter().filter(|t| t.id == removed.id).count();
            while back.iter().filter(|t| t.id == removed.id).count() > left {
                match back.iter().rposition(|t| t.id == removed.id) {
                    Some(at) => {
                        back.remove(at);
                    }
                    None => break,
                }
            }
        }
        self.index = match self.index {
            None => None,
            Some(_) if self.tracks.is_empty() => None,
            Some(cur) if idx < cur => Some(cur - 1),
            Some(cur) if idx == cur => Some(cur.min(self.tracks.len() - 1)),
            Some(cur) => Some(cur),
        };
        true
    }

    pub fn clear(&mut self) {
        self.unshuffled = None;
        self.tracks = Arc::default();
        self.index = None;
        self.hand = 0;
    }

    /// Drops all tracks whose id is in `ids`, keeping `index` and `hand` consistent when the current
    /// track survives, or clearing playback state (`current_was_removed = true`) like C# `RemoveTracksAsync`.
    /// Returns `(queue_changed, current_was_removed)`.
    pub fn remove_tracks_by_id(&mut self, ids: &HashSet<String>) -> (bool, bool) {
        if ids.is_empty() {
            return (false, false);
        }
        let had_in_tracks = self.tracks.iter().any(|t| ids.contains(&t.id));
        let had_in_back = self
            .unshuffled
            .as_ref()
            .is_some_and(|b| b.iter().any(|t| ids.contains(&t.id)));
        if !had_in_tracks && !had_in_back {
            return (false, false);
        }

        let cur_idx = self.index.filter(|&i| i < self.tracks.len());
        let current_removed = cur_idx
            .and_then(|i| self.tracks.get(i))
            .is_some_and(|t| ids.contains(&t.id));

        if let Some(back) = self.unshuffled.as_mut() {
            back.retain(|t| !ids.contains(&t.id));
        }

        if current_removed {
            Arc::make_mut(&mut self.tracks).retain(|t| !ids.contains(&t.id));
            self.index = None;
            self.hand = 0;
            return (true, true);
        }

        let mut removed_before_cur = 0usize;
        let mut removed_in_hand = 0usize;
        if let Some(ci) = cur_idx {
            for (i, t) in self.tracks.iter().enumerate() {
                if ids.contains(&t.id) {
                    if i < ci {
                        removed_before_cur += 1;
                    } else if i > ci && (i - ci - 1) < self.hand {
                        removed_in_hand += 1;
                    }
                }
            }
        }
        Arc::make_mut(&mut self.tracks).retain(|t| !ids.contains(&t.id));
        if self.tracks.is_empty() {
            self.index = None;
            self.hand = 0;
        } else if let Some(ci) = cur_idx {
            let nc = (ci - removed_before_cur).min(self.tracks.len() - 1);
            let upcoming = self.tracks.len() - nc - 1;
            self.index = Some(nc);
            self.hand = self.hand.saturating_sub(removed_in_hand).min(upcoming);
        }
        (true, false)
    }

    pub fn refresh_from_library(&mut self, track_index: &HashMap<String, (Track, Album)>) {
        let fresh = |t: &mut Track| {
            if let Some((f, _)) = track_index.get(&t.id) {
                *t = f.clone();
            }
        };
        Arc::make_mut(&mut self.tracks).iter_mut().for_each(fresh);
        if let Some(back) = self.unshuffled.as_mut() {
            back.iter_mut().for_each(fresh);
        }
    }

    /// Restores a saved queue. A shuffled one is the library-wide queue already, so it is not spread
    /// again; the queue from before comes from `original_queue_ids` when it was saved.
    pub fn restore_session(
        &mut self,
        track_index: &HashMap<String, (Track, Album)>,
        state: &SessionState,
        queue_ids: &[String],
        original_queue_ids: Option<&[String]>,
    ) -> Option<Track> {
        let resolve = |ids: &[String]| -> Vec<Track> {
            ids.iter()
                .filter_map(|id| track_index.get(id).map(|(t, _)| t.clone()))
                .collect()
        };
        self.shuffle = state.shuffle;
        self.repeat = RepeatMode::from_i32(state.repeat_mode);
        self.hand = 0;
        self.unshuffled = if state.shuffle {
            original_queue_ids
                .map(resolve)
                .filter(|orig| !orig.is_empty())
        } else {
            None
        };
        self.tracks = Arc::new(resolve(queue_ids));

        if self.tracks.is_empty() {
            self.index = None;
            return None;
        }

        let matched = state
            .current_id
            .as_deref()
            .and_then(|cid| self.tracks.iter().position(|t| t.id == cid))
            .or_else(|| {
                (state.index >= 0 && (state.index as usize) < self.tracks.len())
                    .then_some(state.index as usize)
            })
            .or(Some(0));
        self.index = matched;
        self.current().cloned()
    }

    /// The playback state alone (current track, position, shuffle, repeat), without the queue.
    pub fn session_state(&self, position_s: f64) -> SessionState {
        SessionState {
            current_id: self.current().map(|t| t.id.clone()),
            index: self.index.map(|i| i as i32).unwrap_or(-1),
            position: position_s.max(0.0),
            shuffle: self.shuffle,
            repeat_mode: self.repeat.as_i32(),
        }
    }

    /// `(state, play order ids, ids of the queue from before shuffle)`; the last is empty when
    /// there is none.
    pub fn to_session(&self, position_s: f64) -> (SessionState, Vec<String>, Vec<String>) {
        let state = self.session_state(position_s);
        let active_ids = self.tracks.iter().map(|t| t.id.clone()).collect();
        let original_ids = self
            .unshuffled
            .iter()
            .flatten()
            .map(|t| t.id.clone())
            .collect();
        (state, active_ids, original_ids)
    }
}

#[derive(Debug, Default)]
pub struct PlayTracker {
    track_id: Option<String>,
    heard_seconds: f64,
    last_pos: Option<f64>,
    recorded: bool,
}

impl PlayTracker {
    pub fn reset(&mut self, track_id: Option<String>) {
        self.track_id = track_id;
        self.heard_seconds = 0.0;
        // A track starts at 0: the time before the first poll counts. A start further in (restored
        // session, resume) is a jump and is ignored like a seek.
        self.last_pos = Some(0.0);
        self.recorded = false;
    }

    /// Listening time needed: the C# threshold less one poll gap, since the polls never land exactly
    /// on the end (short tracks have to be heard in full, which the summed poll deltas never reach).
    fn needed(duration: f64) -> f64 {
        let full = PlayStats::threshold(duration);
        (full - Self::POLL_SLACK_S).max(full * 0.5)
    }

    /// The longest position poll interval (power-saver tier).
    const POLL_SLACK_S: f64 = 1.0;

    /// Accumulates active listening time without letting seeks trigger spurious play counts.
    pub fn on_time_pos(&mut self, pos: f64, duration: f64, is_playing: bool) -> Option<String> {
        let id = self.track_id.as_ref()?;
        if !is_playing || self.recorded {
            self.last_pos = Some(pos);
            return None;
        }
        if let Some(prev) = self.last_pos {
            let delta = pos - prev;
            if delta > 0.0 && delta <= 2.5 {
                self.heard_seconds += delta;
            }
        }
        self.last_pos = Some(pos);
        if self.heard_seconds >= Self::needed(duration) {
            self.recorded = true;
            Some(id.clone())
        } else {
            None
        }
    }
}

pub struct CrossfadeMath;

impl CrossfadeMath {
    /// C# `CrossfadeApplies` for two different queue slots.
    pub fn should_crossfade(
        crossfade_seconds: i32,
        repeat: RepeatMode,
        is_direct: bool,
        shuffle: bool,
        current: &Track,
        next: &Track,
    ) -> bool {
        // Direct output: one stream on the card.
        if crossfade_seconds <= 0 || repeat == RepeatMode::One || is_direct {
            return false;
        }
        // An album heard in order stays gapless.
        if !shuffle && !current.album.is_empty() && current.album == next.album {
            return false;
        }
        // Too short to overlap.
        current.duration <= 0.0 || current.duration >= f64::from(crossfade_seconds) * 2.0 + 2.0
    }

    pub fn equal_power_gains(t: f64, base_vol: f64) -> (f64, f64) {
        let t = t.clamp(0.0, 1.0);
        let base = base_vol.clamp(0.0, 1.0);
        let theta = t * FRAC_PI_2;
        let fade_out = base * theta.cos().max(0.0).cbrt();
        let fade_in = base * theta.sin().max(0.0).cbrt();
        (fade_out, fade_in)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mk_track(id: &str, album: &str, duration: f64) -> Track {
        Track {
            id: id.to_owned(),
            title: id.to_owned(),
            artist: "Artist".to_owned(),
            album: album.to_owned(),
            duration,
            file_path: id.to_owned(),
            track_number: 1,
            disc_number: 1,
            codec: "FLAC".to_owned(),
            sample_rate: 44100,
            bits_per_sample: 16,
            channels: 2,
            cover_thumb_path: Some(PathBuf::from("/tmp/a.jpg")),
            cover_full_path: None,
        }
    }

    fn mk_album(name: &str, ids: &[&str]) -> Album {
        Album {
            name: name.to_owned(),
            artist: "Artist".to_owned(),
            cover_thumb_path: None,
            cover_full_path: None,
            year: 2024,
            added_at: 1,
            tracks: ids.iter().map(|id| mk_track(id, name, 200.0)).collect(),
        }
    }

    /// Two albums: A (a1..a4) and B (b1..b3).
    fn library() -> Vec<Album> {
        vec![
            mk_album("A", &["/a/1", "/a/2", "/a/3", "/a/4"]),
            mk_album("B", &["/b/1", "/b/2", "/b/3"]),
        ]
    }

    fn ids(q: &PlaybackQueue) -> Vec<&str> {
        q.tracks().iter().map(|t| t.id.as_str()).collect()
    }

    fn upcoming(q: &PlaybackQueue) -> Vec<&str> {
        let cur = q.index().unwrap();
        q.tracks()[cur + 1..].iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn shuffle_without_library_keeps_queue_and_restores_order() {
        let mut q = PlaybackQueue::default();
        let tracks = (1..=8)
            .map(|i| mk_track(&format!("/m/{i}.flac"), "Album", 200.0))
            .collect();
        q.set_queue(tracks, 3, &[]);
        assert_eq!(q.current().unwrap().id, "/m/4.flac");

        q.set_shuffle(true, &[]);
        assert_eq!(q.index(), Some(0));
        assert_eq!(q.current().unwrap().id, "/m/4.flac");
        assert_eq!(q.tracks().len(), 8);

        q.set_shuffle(false, &[]);
        assert_eq!(q.index(), Some(3));
        assert_eq!(q.current().unwrap().id, "/m/4.flac");
        let expected: Vec<String> = (1..=8).map(|i| format!("/m/{i}.flac")).collect();
        assert_eq!(ids(&q), expected.iter().map(String::as_str).collect::<Vec<_>>());
    }

    #[test]
    fn shuffle_spreads_over_whole_library_and_off_returns_to_album() {
        let lib = library();
        let mut q = PlaybackQueue::default();
        q.set_queue(lib[0].tracks.clone(), 1, &lib);
        assert_eq!(q.current().unwrap().id, "/a/2");

        q.set_shuffle(true, &lib);
        assert_eq!(q.index(), Some(0));
        assert_eq!(q.current().unwrap().id, "/a/2");
        // Every library track exactly once, the current one first.
        let mut all = ids(&q);
        assert_eq!(all.len(), 7);
        all.sort();
        assert_eq!(all, vec!["/a/1", "/a/2", "/a/3", "/a/4", "/b/1", "/b/2", "/b/3"]);

        // Turning shuffle off goes back to the queue from before (album A), at the current track.
        q.set_shuffle(false, &lib);
        assert_eq!(ids(&q), vec!["/a/1", "/a/2", "/a/3", "/a/4"]);
        assert_eq!(q.index(), Some(1));
    }

    #[test]
    fn shuffle_off_on_a_track_outside_the_old_queue_goes_to_its_album() {
        let lib = library();
        let mut q = PlaybackQueue::default();
        q.set_queue(lib[0].tracks.clone(), 0, &lib);
        q.set_shuffle(true, &lib);
        let b2 = q.tracks().iter().position(|t| t.id == "/b/2").unwrap();
        q.jump_to(b2);
        q.set_shuffle(false, &lib);
        assert_eq!(ids(&q), vec!["/b/1", "/b/2", "/b/3"]);
        assert_eq!(q.current().unwrap().id, "/b/2");
    }

    #[test]
    fn play_in_shuffle_puts_chosen_track_first_then_the_library() {
        let lib = library();
        let mut q = PlaybackQueue::default();
        q.set_shuffle(true, &lib);
        q.set_queue(lib[1].tracks.clone(), 2, &lib);
        assert_eq!(q.current().unwrap().id, "/b/3");
        assert_eq!(q.tracks().len(), 7);
        q.set_shuffle(false, &lib);
        assert_eq!(ids(&q), vec!["/b/1", "/b/2", "/b/3"]);
        assert_eq!(q.index(), Some(2));
    }

    #[test]
    fn restored_library_queue_turns_into_album_without_saved_original() {
        let lib = library();
        let index: HashMap<String, (Track, Album)> = lib
            .iter()
            .flat_map(|a| a.tracks.iter().map(move |t| (t.id.clone(), (t.clone(), a.clone()))))
            .collect();
        let state = SessionState {
            current_id: Some("/a/3".to_owned()),
            index: 0,
            position: 0.0,
            shuffle: true,
            repeat_mode: 0,
        };
        let saved: Vec<String> = ["/a/3", "/b/1", "/a/1", "/b/3", "/a/2", "/b/2", "/a/4"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let mut q = PlaybackQueue::default();
        q.restore_session(&index, &state, &saved, None);
        // Restoring does not spread again.
        assert_eq!(ids(&q), saved.iter().map(String::as_str).collect::<Vec<_>>());
        q.set_shuffle(false, &lib);
        assert_eq!(ids(&q), vec!["/a/1", "/a/2", "/a/3", "/a/4"]);
        assert_eq!(q.index(), Some(2));

        // A smaller restored queue was the user's own: it is kept.
        let mut q = PlaybackQueue::default();
        q.restore_session(&index, &state, &saved[..3], None);
        q.set_shuffle(false, &lib);
        assert_eq!(ids(&q), vec!["/a/3", "/b/1", "/a/1"]);
    }

    #[test]
    fn hand_queue_add_appends_to_line_and_play_next_goes_first() {
        let lib = library();
        let mut q = PlaybackQueue::default();
        q.set_queue(lib[0].tracks.clone(), 0, &lib);
        assert!(!q.enqueue(vec![mk_track("/x/1", "X", 200.0)], &lib));
        assert!(!q.enqueue(vec![mk_track("/x/2", "X", 200.0)], &lib));
        // ADD_TO_QUEUE lands right after the current track, not after the album.
        assert_eq!(upcoming(&q), vec!["/x/1", "/x/2", "/a/2", "/a/3", "/a/4"]);
        assert_eq!(q.hand_len(), 2);

        assert!(!q.play_next(vec![mk_track("/y/1", "Y", 200.0)], &lib));
        assert_eq!(upcoming(&q), vec!["/y/1", "/x/1", "/x/2", "/a/2", "/a/3", "/a/4"]);
        assert_eq!(q.hand_len(), 3);

        // Playing on in order uses the line up one track at a time.
        q.step_next(false);
        assert_eq!(q.current().unwrap().id, "/y/1");
        assert_eq!(q.hand_len(), 2);
        q.enqueue(vec![mk_track("/x/3", "X", 200.0)], &lib);
        assert_eq!(upcoming(&q), vec!["/x/1", "/x/2", "/x/3", "/a/2", "/a/3", "/a/4"]);

        // A jump out of order ends the line.
        let a3 = q.tracks().iter().position(|t| t.id == "/a/3").unwrap();
        q.jump_to(a3);
        assert_eq!(q.hand_len(), 0);
        q.enqueue(vec![mk_track("/z/1", "Z", 200.0)], &lib);
        assert_eq!(upcoming(&q), vec!["/z/1", "/a/4"]);
    }

    #[test]
    fn hand_queue_in_shuffle_also_lands_in_the_queue_from_before() {
        let lib = library();
        let mut q = PlaybackQueue::default();
        q.set_queue(lib[0].tracks.clone(), 0, &lib);
        q.set_shuffle(true, &lib);
        q.enqueue(vec![mk_track("/x/1", "X", 200.0)], &lib);
        q.play_next(vec![mk_track("/y/1", "Y", 200.0)], &lib);
        assert_eq!(&upcoming(&q)[..2], &["/y/1", "/x/1"]);
        q.set_shuffle(false, &lib);
        assert_eq!(ids(&q), vec!["/a/1", "/y/1", "/x/1", "/a/2", "/a/3", "/a/4"]);
    }

    #[test]
    fn enqueue_into_empty_queue_starts_a_new_one() {
        let lib = library();
        let mut q = PlaybackQueue::default();
        assert!(q.enqueue(vec![mk_track("/x/1", "X", 200.0)], &lib));
        assert_eq!(q.current().unwrap().id, "/x/1");
        assert_eq!(q.hand_len(), 0);
    }

    #[test]
    fn remove_at_handles_duplicate_ids_and_hand_line() {
        let lib = library();
        let mut q = PlaybackQueue::default();
        let dup = mk_track("/d/1", "D", 200.0);
        q.set_queue(
            vec![mk_track("/a/1", "A", 200.0), dup.clone(), mk_track("/a/2", "A", 200.0), dup.clone()],
            0,
            &[],
        );
        // Removing the second copy keeps the first one where it is.
        assert!(q.remove_at(3));
        assert_eq!(ids(&q), vec!["/a/1", "/d/1", "/a/2"]);

        // In shuffle, the queue from before keeps as many copies as the play order still has.
        let mut q = PlaybackQueue::default();
        q.set_queue(
            vec![mk_track("/a/1", "A", 200.0), dup.clone(), mk_track("/a/2", "A", 200.0), dup.clone()],
            0,
            &[],
        );
        q.set_shuffle(true, &[]);
        let first_dup = q.tracks().iter().position(|t| t.id == "/d/1").unwrap();
        assert!(q.remove_at(first_dup));
        q.set_shuffle(false, &[]);
        assert_eq!(ids(&q), vec!["/a/1", "/d/1", "/a/2"]);

        // Removing a hand-queued track shortens the line.
        let mut q = PlaybackQueue::default();
        q.set_queue(lib[0].tracks.clone(), 0, &lib);
        q.enqueue(vec![mk_track("/x/1", "X", 200.0), mk_track("/x/2", "X", 200.0)], &lib);
        assert!(q.remove_at(1));
        assert_eq!(q.hand_len(), 1);
        q.enqueue(vec![mk_track("/x/3", "X", 200.0)], &lib);
        assert_eq!(upcoming(&q), vec!["/x/2", "/x/3", "/a/2", "/a/3", "/a/4"]);
        assert!(!q.remove_at(99));
    }

    #[test]
    fn repeat_modes_handle_wrap_and_single_loop() {
        let mut q = PlaybackQueue::default();
        let tracks = vec![
            mk_track("/m/1.flac", "A", 100.0),
            mk_track("/m/2.flac", "B", 100.0),
        ];
        q.set_queue(tracks, 1, &[]);
        assert_eq!(q.auto_next_index(), None);
        assert!(q.step_next(true).is_none());

        q.set_repeat(RepeatMode::All);
        assert_eq!(q.auto_next_index(), Some(0));
        assert_eq!(q.step_next(true).unwrap().id, "/m/1.flac");

        // Repeat one: the same track follows itself (preloaded for gapless).
        q.set_repeat(RepeatMode::One);
        assert_eq!(q.auto_next_index(), Some(0));
        assert_eq!(q.step_next(true).unwrap().id, "/m/1.flac");
        assert!(matches!(
            q.on_mpv_advanced("/m/1.flac", Some(0)),
            MpvAdvance::Followed(t) if t.id == "/m/1.flac"
        ));
        assert_eq!(q.index(), Some(0));
        assert!(!q.crossfade_applies(0, 5, false));
    }

    #[test]
    fn play_tracker_requires_real_playback_and_ignores_seeks() {
        let mut tracker = PlayTracker::default();
        tracker.reset(Some("/m/song.flac".to_owned()));
        // Jump from 0s -> 150s via seek must NOT trigger play count immediately.
        assert_eq!(tracker.on_time_pos(0.0, 180.0, true), None);
        assert_eq!(tracker.on_time_pos(150.0, 180.0, true), None);

        // Threshold for a 180 s track is 90 s; feed one-second ticks.
        let mut fired = None;
        for sec in 1..=92 {
            if let Some(id) = tracker.on_time_pos(150.0 + sec as f64, 180.0, true) {
                fired = Some(id);
            }
        }
        assert_eq!(fired.as_deref(), Some("/m/song.flac"));
        // Subsequent ticks on the same play must not double count.
        assert_eq!(tracker.on_time_pos(245.0, 180.0, true), None);
    }

    #[test]
    fn play_tracker_counts_short_track_heard_in_full() {
        // A 10 s track has to be heard in full (C# threshold 10 s); polls never land on the end.
        for step in [0.2, 0.5, 1.0] {
            let mut tracker = PlayTracker::default();
            tracker.reset(Some("/m/short.flac".to_owned()));
            let mut fired = false;
            let mut t = 0.3_f64.min(step);
            while t < 10.0 {
                fired |= tracker.on_time_pos(t, 10.0, true).is_some();
                t += step;
            }
            assert!(fired, "poll step {step}");
        }
        // Half of it is not enough.
        let mut tracker = PlayTracker::default();
        tracker.reset(Some("/m/short.flac".to_owned()));
        let mut t = 0.0;
        while t < 5.0 {
            assert!(tracker.on_time_pos(t, 10.0, true).is_none());
            t += 0.5;
        }
    }

    #[test]
    fn crossfade_rule_matches_csharp() {
        let a1 = mk_track("/m/1.flac", "Same Album", 180.0);
        let a2 = mk_track("/m/2.flac", "Same Album", 180.0);
        let a2_case = mk_track("/m/2b.flac", "same album", 180.0);
        let b1 = mk_track("/m/3.flac", "Other Album", 180.0);
        let no_album = mk_track("/m/4.flac", "", 180.0);
        let short = mk_track("/m/5.flac", "Short", 11.0);
        let edge = mk_track("/m/6.flac", "Edge", 12.0);
        let unknown = mk_track("/m/7.flac", "Unknown", 0.0);
        let f = CrossfadeMath::should_crossfade;

        // Same album in order stays gapless; in shuffle it fades.
        assert!(!f(5, RepeatMode::Off, false, false, &a1, &a2));
        assert!(f(5, RepeatMode::Off, false, true, &a1, &a2));
        // The album compare is exact, like C# string ==.
        assert!(f(5, RepeatMode::Off, false, false, &a1, &a2_case));
        // An empty album never counts as "same album".
        assert!(f(5, RepeatMode::Off, false, false, &no_album, &no_album));
        assert!(f(5, RepeatMode::Off, false, false, &a1, &b1));
        // Off, repeat one, direct output.
        assert!(!f(0, RepeatMode::Off, false, false, &a1, &b1));
        assert!(!f(5, RepeatMode::One, false, false, &a1, &b1));
        assert!(!f(5, RepeatMode::Off, true, false, &a1, &b1));
        // The current track must last at least 2 * fade + 2 s; unknown length fades.
        assert!(!f(5, RepeatMode::Off, false, false, &short, &b1));
        assert!(f(5, RepeatMode::Off, false, false, &edge, &b1));
        assert!(f(5, RepeatMode::Off, false, false, &unknown, &b1));

        let mut q = PlaybackQueue::default();
        q.set_queue(vec![a1.clone(), b1.clone()], 0, &[]);
        assert!(q.crossfade_applies(1, 5, false));
        assert!(!q.crossfade_applies(0, 5, false));
        assert!(!q.crossfade_applies(7, 5, false));
    }

    #[test]
    fn equal_power_gains_endpoints() {
        let (out0, in0) = CrossfadeMath::equal_power_gains(0.0, 0.8);
        assert!((out0 - 0.8).abs() < 1e-6);
        assert!(in0.abs() < 1e-6);

        let (out1, in1) = CrossfadeMath::equal_power_gains(1.0, 0.8);
        assert!(out1.abs() < 1e-4);
        assert!((in1 - 0.8).abs() < 1e-6);
    }

    #[test]
    fn jump_to_track_near_tracks_id_across_play_next_and_remove() {
        let mut q = PlaybackQueue::default();
        let tracks = (1..=4)
            .map(|i| mk_track(&format!("/m/{i}.flac"), "Album", 200.0))
            .collect();
        q.set_queue(tracks, 0, &[]);

        // Pretend fade targeted index 1 ("/m/2.flac"), then PlayNext inserts at index 1.
        q.play_next(vec![mk_track("/m/inserted.flac", "Album", 200.0)], &[]);
        assert_eq!(q.find_track_near(1, "/m/2.flac"), Some(2));
        assert_eq!(q.jump_to_track_near(1, "/m/2.flac"), Some(2));
        assert_eq!(q.current().unwrap().id, "/m/2.flac");

        // Removing the target track makes find_track_near return None.
        assert!(q.remove_at(2));
        assert_eq!(q.find_track_near(1, "/m/2.flac"), None);
    }

    #[test]
    fn restore_shuffled_session_keeps_original_order() {
        let t1 = mk_track("/m/1.flac", "A", 180.0);
        let t2 = mk_track("/m/2.flac", "A", 180.0);
        let t3 = mk_track("/m/3.flac", "A", 180.0);
        let alb = Album {
            name: "A".to_owned(),
            artist: "Artist".to_owned(),
            cover_thumb_path: None,
            cover_full_path: None,
            year: 2024,
            added_at: 1,
            tracks: vec![t1.clone(), t2.clone(), t3.clone()],
        };
        let mut map = HashMap::new();
        for t in [&t1, &t2, &t3] {
            map.insert(t.id.clone(), (t.clone(), alb.clone()));
        }

        let state = SessionState {
            current_id: Some("/m/3.flac".to_owned()),
            index: 0,
            position: 12.0,
            shuffle: true,
            repeat_mode: 0,
        };
        let shuffled_ids = vec!["/m/3.flac".to_owned(), "/m/1.flac".to_owned(), "/m/2.flac".to_owned()];
        let orig_ids = vec!["/m/1.flac".to_owned(), "/m/2.flac".to_owned(), "/m/3.flac".to_owned()];

        let mut q = PlaybackQueue::default();
        q.restore_session(&map, &state, &shuffled_ids, Some(&orig_ids));
        assert_eq!(q.current().unwrap().id, "/m/3.flac");
        let (_, active, original) = q.to_session(12.0);
        assert_eq!(active, shuffled_ids);
        assert_eq!(original, orig_ids);

        q.set_shuffle(false, std::slice::from_ref(&alb));
        let restored_ids: Vec<&str> = q.tracks().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(restored_ids, vec!["/m/1.flac", "/m/2.flac", "/m/3.flac"]);
        assert_eq!(q.index(), Some(2));
        // Not shuffling: no queue from before is saved.
        assert!(q.to_session(0.0).2.is_empty());
    }

    #[test]
    fn enqueue_racing_mpv_advance_reloads_the_hand_queued_track() {
        // D-25: A plays, B was handed to mpv as next. C is queued by hand just as mpv moves into B.
        let mut q = PlaybackQueue::default();
        let tracks = vec![mk_track("A", "X", 100.0), mk_track("B", "X", 100.0)];
        q.set_queue(tracks, 0, &[]);
        let handed = q.auto_next_index();
        assert_eq!(handed, Some(1));
        q.enqueue(vec![mk_track("C", "Y", 100.0)], &[]);
        // The controller hands C over now, but mpv already entered B (handed before the edit).
        let handed_after = q.auto_next_index();
        assert_eq!(handed_after, Some(1));
        match q.on_mpv_advanced("B", handed_after) {
            MpvAdvance::Reload(t) => assert_eq!(t.id, "C"),
            other => panic!("expected a reload of C, got {other:?}"),
        }
        // C is current and its hand slot used up; B still follows.
        assert_eq!(q.current().unwrap().id, "C");
        assert_eq!(q.hand_len(), 0);
        assert_eq!(q.tracks()[q.auto_next_index().unwrap()].id, "B");

        // The normal case: mpv entered exactly what it was handed.
        match q.on_mpv_advanced("B", q.auto_next_index()) {
            MpvAdvance::Followed(t) => assert_eq!(t.id, "B"),
            other => panic!("expected B to follow, got {other:?}"),
        }
        // Last track, nothing handed, repeat off: mpv moving on means stop.
        assert!(matches!(q.on_mpv_advanced("Z", None), MpvAdvance::Stop));
    }

    #[test]
    fn track_changes_share_the_queue_and_edits_copy_it_once() {
        // D-26: the UI's copy is the same allocation until the queue itself changes.
        let mut q = PlaybackQueue::default();
        let tracks: Vec<Track> = (0..5).map(|i| mk_track(&format!("t{i}"), "X", 100.0)).collect();
        q.set_queue(tracks, 0, &[]);
        let ui = q.shared_tracks();
        q.step_next(false);
        q.jump_to(3);
        assert!(Arc::ptr_eq(&ui, &q.shared_tracks()));
        q.enqueue(vec![mk_track("h", "Y", 100.0)], &[]);
        assert!(!Arc::ptr_eq(&ui, &q.shared_tracks()));
        // The UI's old list is untouched by the edit.
        assert_eq!(ui.len(), 5);
        assert_eq!(q.tracks().len(), 6);
        assert_eq!(q.tracks()[4].id, "h");
    }

    #[test]
    fn remove_tracks_by_id_updates_indices_and_stops_when_current_deleted() {
        let mut q = PlaybackQueue::default();
        let tracks: Vec<Track> = (0..5).map(|i| mk_track(&format!("t{i}"), "X", 100.0)).collect();
        q.set_queue(tracks, 1, &[]);
        q.enqueue(vec![mk_track("t3", "Y", 100.0), mk_track("h2", "Y", 100.0)], &[]);
        // Remove t0 (before current t1) and t3 (in hand queue) -> t1 stays current at index 0.
        let mut drop = HashSet::new();
        drop.insert("t0".to_owned());
        drop.insert("t3".to_owned());
        let (changed, cur_removed) = q.remove_tracks_by_id(&drop);
        assert!(changed);
        assert!(!cur_removed);
        assert_eq!(q.index(), Some(0));
        assert_eq!(q.current().unwrap().id, "t1");
        assert_eq!(q.hand_len(), 1);
        assert_eq!(q.tracks()[1].id, "h2");

        let mut drop_cur = HashSet::new();
        drop_cur.insert("t1".to_owned());
        let (changed2, cur_removed2) = q.remove_tracks_by_id(&drop_cur);
        assert!(changed2);
        assert!(cur_removed2);
        assert_eq!(q.index(), None);
    }
}
