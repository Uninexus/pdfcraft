//! Security policies through the tools: named password-protection settings without passwords,
//! kept for the tool session, and protecting a document with one.

use pdfcraft_engine::Edit;
use pdfcraft_engine::security_policy::{SecurityPolicy, find_policy, save_policy};

use super::error::{Result, ToolError, failed};
use super::json::Args;
use super::json::security;
use super::{Automation, Value, json};

/// The settings only a permissions password enforces.
const RESTRICTIONS: [&str; 4] = ["printing", "changes", "copy", "accessibility"];

impl Automation {
    pub(crate) fn security_policy_list(&self) -> Value {
        json!({ "policies": self.security_policies.iter().map(SecurityPolicy::to_json).collect::<Vec<_>>() })
    }

    pub(crate) fn security_policy_save(&mut self, a: &Args) -> Result<Value> {
        let password_for = a.str("password_for")?;
        // As doc_protect: a restriction that could not take effect is refused, not kept.
        if password_for == "viewing"
            && let Some(key) = RESTRICTIONS.into_iter().find(|k| a.get(k).is_some())
        {
            return Err(ToolError::InvalidArgs(format!("`{key}` needs password_for \"editing\": a password to open restricts nothing")));
        }
        let text = |key: &str, default: &str| -> Result<String> { Ok(a.opt_str(key)?.unwrap_or(default).to_owned()) };
        let stored = json!({
            "name": a.str("name")?,
            "password_for": password_for,
            "printing": text("printing", "high")?,
            "changes": text("changes", "none")?,
            "copy": a.opt_bool("copy")?.unwrap_or(false),
            "accessibility": a.opt_bool("accessibility")?.unwrap_or(true),
            "compatibility": text("compatibility", "aes-256")?,
            "encrypt_metadata": a.opt_bool("encrypt_metadata")?.unwrap_or(true),
        });
        let policy = SecurityPolicy::from_json(&stored)
            .ok_or_else(|| ToolError::InvalidArgs("a name of 1 to 80 characters and known values are needed".into()))?;
        save_policy(&mut self.security_policies, policy).map_err(failed)?;
        Ok(self.security_policy_list())
    }

    pub(crate) fn security_policy_delete(&mut self, a: &Args) -> Result<Value> {
        let name = a.str("name")?;
        let found = find_policy(&self.security_policies, name).map(|p| p.name.clone());
        let found = found.ok_or_else(|| failed(format!("there is no security policy named {name:?} (see security_policy_list)")))?;
        self.security_policies.retain(|p| p.name != found);
        Ok(self.security_policy_list())
    }

    pub(crate) fn doc_protect_with_policy(&mut self, a: &Args) -> Result<Value> {
        let name = a.str("policy")?;
        let policy = find_policy(&self.security_policies, name)
            .ok_or_else(|| failed(format!("there is no security policy named {name:?} (see security_policy_list)")))?;
        let protection = policy.protection(a.str("password")?.to_owned());
        let mut out = self.apply(a, Edit::Protect(protection))?;
        out["security"] = security(self.doc(a)?);
        Ok(out)
    }
}
