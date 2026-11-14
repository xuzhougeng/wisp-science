//! The pet's animation: which row of the sprite sheet it plays for what
//! the conversation is doing, and which frame it is on.
//!
//! The sheet is 8 columns by 11 rows. A row is a mood; some play once and
//! give way to what the conversation is doing, the others loop.
use std::collections::BTreeMap;

pub const COLUMNS: u32 = 8;
pub const ROWS: u32 = 11;
/// One frame as it is drawn, in pixels: the sheet's cell at half size.
pub const CELL: (f32, f32) = (96., 104.);
/// How long a frame stays, in milliseconds.
pub const FRAME_MS: u64 = 140;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    Idle,
    Waving,
    Jumping,
    Failed,
    Waiting,
    Running,
    Review,
}

impl Mood {
    /// The row on the sheet, and the name its frame count is kept under.
    fn row(self) -> (u32, &'static str, u8) {
        match self {
            Mood::Idle => (0, "idle", 7),
            Mood::Waving => (3, "waving", 4),
            Mood::Jumping => (4, "jumping", 5),
            Mood::Failed => (5, "failed", 8),
            Mood::Waiting => (6, "waiting", 6),
            Mood::Running => (7, "running", 6),
            Mood::Review => (8, "review", 6),
        }
    }

    /// Played once, then back to what the conversation is doing.
    fn once(self) -> bool {
        matches!(self, Mood::Waving | Mood::Jumping | Mood::Failed)
    }

    /// What the pet is doing, for the tooltip.
    pub fn label(self) -> &'static str {
        match self {
            Mood::Idle | Mood::Waving => "Idle",
            Mood::Jumping => "Done",
            Mood::Failed => "Failed",
            Mood::Waiting => "Needs you",
            Mood::Running => "Working",
            Mood::Review => "Reviewing",
        }
    }
}

/// What the conversation on screen is doing, as the pet sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// A turn is running.
    pub running: bool,
    /// An approval or an answer is waited for.
    pub waiting: bool,
    /// The turn is being reviewed.
    pub reviewing: bool,
    /// The last turn ended in an error.
    pub failed: bool,
}

impl Activity {
    /// The mood that lasts while this goes on. Being needed comes first.
    pub fn mood(self) -> Mood {
        if self.waiting {
            Mood::Waiting
        } else if self.reviewing {
            Mood::Review
        } else if self.running {
            Mood::Running
        } else {
            Mood::Idle
        }
    }

    fn busy(self) -> bool {
        self.running || self.reviewing || self.waiting
    }
}

/// Which frame the pet is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    mood: Mood,
    frame: u8,
    seen: Activity,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            mood: Mood::Idle,
            frame: 0,
            seen: Activity::default(),
        }
    }
}

impl Player {
    pub fn mood(&self) -> Mood {
        self.mood
    }

    fn play(&mut self, mood: Mood) {
        if self.mood != mood {
            self.mood = mood;
            self.frame = 0;
        }
    }

    /// Takes in what the conversation is doing now. Work that ends makes
    /// the pet jump, or slump when it ended in an error.
    pub fn observe(&mut self, now: Activity) {
        let before = std::mem::replace(&mut self.seen, now);
        if now == before {
            return;
        }
        if before.busy() && !now.busy() {
            self.play(if now.failed {
                Mood::Failed
            } else {
                Mood::Jumping
            });
        } else if now.busy() || !self.mood.once() {
            self.play(now.mood());
        }
    }

    /// A click: an idle pet waves; a busy one is left to it.
    pub fn poke(&mut self) {
        if self.mood == Mood::Idle {
            self.play(Mood::Waving);
        }
    }

    /// Moves on one frame. Returns whether what is drawn changed.
    pub fn tick(&mut self, frames: &BTreeMap<String, u8>) -> bool {
        let before = self.cell();
        let count = self.count(frames);
        if self.frame + 1 < count {
            self.frame += 1;
        } else if self.mood.once() {
            self.mood = self.seen.mood();
            self.frame = 0;
        } else {
            self.frame = 0;
        }
        self.cell() != before
    }

    fn count(&self, frames: &BTreeMap<String, u8>) -> u8 {
        let (_, name, usual) = self.mood.row();
        frames.get(name).copied().unwrap_or(usual).clamp(1, 8)
    }

    /// The row and column drawn now.
    pub fn cell(&self) -> (u32, u32) {
        (self.mood.row().0, u32::from(self.frame))
    }

    /// Where the whole sheet is put, in pixels, so that the frame fills a
    /// box of `CELL`: always left of and above the box's corner.
    pub fn offset(&self) -> (f32, f32) {
        let (row, column) = self.cell();
        (-(column as f32) * CELL.0, -(row as f32) * CELL.1)
    }
}

/// The folder field holds this choice already, so saving would change
/// nothing.
pub fn unchanged(typed: &str, saved: &str) -> bool {
    typed.trim() == saved.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames() -> BTreeMap<String, u8> {
        BTreeMap::from([("idle".to_owned(), 3), ("jumping".to_owned(), 2)])
    }

    fn running() -> Activity {
        Activity {
            running: true,
            ..Activity::default()
        }
    }

    #[test]
    fn being_needed_outranks_reviewing_and_working() {
        let mut now = Activity {
            running: true,
            waiting: true,
            reviewing: true,
            failed: false,
        };
        assert_eq!(now.mood(), Mood::Waiting);
        now.waiting = false;
        assert_eq!(now.mood(), Mood::Review);
        now.reviewing = false;
        assert_eq!(now.mood(), Mood::Running);
        assert_eq!(Activity::default().mood(), Mood::Idle);
    }

    #[test]
    fn a_loop_goes_round_and_uses_the_folders_frame_counts() {
        let mut pet = Player::default();
        let frames = frames();
        let mut columns = vec![pet.cell().1];
        for _ in 0..4 {
            assert!(pet.tick(&frames));
            columns.push(pet.cell().1);
        }
        assert_eq!(columns, [0, 1, 2, 0, 1]);
        assert_eq!(pet.cell().0, 0);
        // A row the folder says nothing of has its usual length.
        pet.observe(running());
        assert_eq!(pet.cell(), (7, 0));
        for _ in 0..5 {
            pet.tick(&frames);
        }
        assert_eq!(pet.cell(), (7, 5));
        pet.tick(&frames);
        assert_eq!(pet.cell(), (7, 0));
    }

    #[test]
    fn finished_work_is_a_jump_and_failed_work_a_slump_played_once() {
        let frames = frames();
        let mut pet = Player::default();
        pet.observe(running());
        pet.observe(Activity::default());
        assert_eq!(pet.mood(), Mood::Jumping);
        pet.tick(&frames);
        assert_eq!(pet.cell(), (4, 1));
        pet.tick(&frames);
        assert_eq!(pet.mood(), Mood::Idle);

        pet.observe(running());
        pet.observe(Activity {
            failed: true,
            ..Activity::default()
        });
        assert_eq!(pet.mood(), Mood::Failed);
        // New work cuts the slump short.
        pet.observe(running());
        assert_eq!(pet.mood(), Mood::Running);
        // A failure noted while nothing ran does not replay it.
        let mut quiet = Player::default();
        quiet.observe(Activity {
            failed: true,
            ..Activity::default()
        });
        assert_eq!(quiet.mood(), Mood::Idle);
    }

    #[test]
    fn a_click_waves_only_an_idle_pet() {
        let frames = frames();
        let mut pet = Player::default();
        pet.poke();
        assert_eq!(pet.mood(), Mood::Waving);
        // Work that starts during the wave takes over at once.
        pet.observe(running());
        assert_eq!(pet.mood(), Mood::Running);
        pet.poke();
        assert_eq!(pet.mood(), Mood::Running);
        // A wave that is left alone ends in what is going on.
        let mut idle = Player::default();
        idle.poke();
        for _ in 0..4 {
            idle.tick(&frames);
        }
        assert_eq!(idle.mood(), Mood::Idle);
    }

    #[test]
    fn the_sheet_is_moved_so_the_frame_fills_the_box() {
        let mut pet = Player::default();
        assert_eq!(pet.offset(), (0., 0.));
        pet.observe(running());
        pet.tick(&BTreeMap::new());
        assert_eq!(pet.offset(), (-CELL.0, -7. * CELL.1));
        assert!(unchanged(" /pets/mochi ", "/pets/mochi"));
        assert!(!unchanged("/pets/other", "/pets/mochi"));
    }
}
