//! The paper size the system's region prints on: the size of a new blank document (Create ▸
//! Blank page).

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Paper {
    #[default]
    Letter,
    A4,
}

impl Paper {
    /// Width and height in points, portrait.
    pub fn points(self) -> (f64, f64) {
        match self {
            Paper::Letter => (612.0, 792.0),
            Paper::A4 => (595.28, 841.89),
        }
    }

    /// The name saved in the settings and given to `ui.set blank-paper`.
    pub fn token(self) -> &'static str {
        match self {
            Paper::Letter => "letter",
            Paper::A4 => "a4",
        }
    }

    /// The paper a saved or typed name stands for (any case); `None` for anything else.
    pub fn from_token(name: &str) -> Option<Paper> {
        match name.trim().to_ascii_lowercase().as_str() {
            "letter" => Some(Paper::Letter),
            "a4" => Some(Paper::A4),
            _ => None,
        }
    }

    /// The system region's paper (craft-i18n reads it once per process).
    pub fn system() -> Paper {
        match craft::i18n::Region::system().paper {
            craft::i18n::Paper::A4 => Paper::A4,
            craft::i18n::Paper::Letter => Paper::Letter,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Paper;

    #[test]
    fn saved_names_round_trip_and_anything_else_is_refused() {
        for paper in [Paper::Letter, Paper::A4] {
            assert_eq!(Paper::from_token(paper.token()), Some(paper));
        }
        assert_eq!(Paper::from_token(" A4 "), Some(Paper::A4));
        assert_eq!(Paper::from_token("Letter"), Some(Paper::Letter));
        for bad in ["", "legal", "a3", "4", "letter\0"] {
            assert_eq!(Paper::from_token(bad), None, "{bad:?}");
        }
    }
}
