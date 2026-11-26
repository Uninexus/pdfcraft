//! Preferences > Grid: the page grid View > Show Grid draws, and the unit its numbers are shown in.
//!
//! The grid is the session's grid preference (the one `doc_grid_snap` uses for what it is not given), in points; until
//! one is stored the grid is [`default_grid`]. Only the unit the Preferences page shows it in is the app's.

use pdfcraft_engine::Session;
use pdfcraft_engine::guides::Grid;

/// The spacing of the grid before one is chosen: an inch.
pub const DEFAULT_SPACING: f64 = 72.0;
/// The subdivisions of the grid before one is chosen: quarter inches.
pub const DEFAULT_SUBDIVISIONS: u32 = 4;

/// The grid shown when none was chosen: an inch in quarters, lines through the page's top left corner.
pub fn default_grid() -> Grid {
    Grid { spacing: DEFAULT_SPACING, subdivisions: DEFAULT_SUBDIVISIONS, offset_x: 0.0, offset_y: 0.0 }
}

/// The grid the session draws and snaps to: its preference, or the default.
pub fn effective(session: &Session) -> Grid {
    session.grid_preference().unwrap_or_else(default_grid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_grid_is_one_the_engine_accepts_and_a_stored_one_replaces_it() {
        assert!(default_grid().check().is_ok());
        let mut session = Session::new();
        assert_eq!(effective(&session), default_grid());
        let own = Grid { spacing: 36.0, subdivisions: 3, offset_x: 5.0, offset_y: 0.0 };
        assert!(session.set_grid_preference(Some(own)).is_ok());
        assert_eq!(effective(&session), own);
        assert!(session.set_grid_preference(Some(Grid::new(0.0))).is_err());
        assert_eq!(effective(&session), own, "a refused grid is not stored");
    }
}
