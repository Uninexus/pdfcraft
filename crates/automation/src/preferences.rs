//! Preferences that need no document: Identity and Forms.

use pdfcraft_engine::js::MAX_IDENTITY_CHARS;
use serde_json::{Value, json};

use crate::comments::DEFAULT_AUTHOR;
use crate::{Args, Automation, Result, ToolError};

impl Automation {
    /// `identity`: set or read the name and email.
    pub(crate) fn identity(&mut self, a: &Args) -> Result<Value> {
        let name = a.opt_str("name")?;
        let email = a.opt_str("email")?;
        for (key, v) in [("name", name), ("email", email)] {
            if v.is_some_and(|v| v.trim().chars().count() > MAX_IDENTITY_CHARS) {
                return Err(ToolError::InvalidArgs(format!("{key} may be at most {MAX_IDENTITY_CHARS} characters")));
            }
        }
        if name.is_some() || email.is_some() {
            let now = self.session.identity().clone();
            self.session.set_identity(name.unwrap_or(&now.name), email.unwrap_or(&now.email));
        }
        let id = self.session.identity();
        let author = if id.name.is_empty() { DEFAULT_AUTHOR } else { id.name.as_str() };
        Ok(json!({ "name": id.name, "email": id.email, "author": author }))
    }

    /// `form_preferences`: Highlight fields.
    pub(crate) fn form_preferences(&mut self, a: &Args) -> Result<Value> {
        if let Some(on) = a.opt_bool("highlight_fields")? {
            self.session.set_field_highlight(on);
        }
        Ok(json!({ "highlight_fields": self.session.field_highlight() }))
    }
}
