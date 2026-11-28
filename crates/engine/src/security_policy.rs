//! Security policies: named, reusable password-protection settings. A policy never holds a
//! password; one is given each time the policy is applied.

use serde_json::{Value, json};

use crate::{Changes, Printing, Protection};
use pdfcraft_cos::Algorithm;

/// Most policies kept.
pub const MAX_POLICIES: usize = 100;

/// Most characters in a policy's name.
pub const MAX_NAME_CHARS: usize = 80;

/// A named set of protection settings.
#[derive(Clone, Debug, PartialEq)]
pub struct SecurityPolicy {
    pub name: String,
    /// The password is needed to open the document; otherwise only to change its restrictions.
    pub viewing: bool,
    /// The settings, always without passwords.
    settings: Protection,
}

impl SecurityPolicy {
    /// A policy of `settings`; any password in them is left out. The name is trimmed, and must
    /// not be blank or longer than [`MAX_NAME_CHARS`].
    pub fn new(name: &str, viewing: bool, settings: &Protection) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("a security policy needs a name".into());
        }
        if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
            return Err(format!("a policy name is at most {MAX_NAME_CHARS} characters, without control characters"));
        }
        let settings = Protection { open_password: None, permissions_password: None, ..settings.clone() };
        Ok(Self { name: name.to_owned(), viewing, settings })
    }

    /// The settings, without passwords.
    pub fn settings(&self) -> &Protection {
        &self.settings
    }

    /// The protection to apply with `password`.
    pub fn protection(&self, password: String) -> Protection {
        let mut p = self.settings.clone();
        if self.viewing {
            p.open_password = Some(password);
        } else {
            p.permissions_password = Some(password);
        }
        p
    }

    /// The policy as stored and as tools list it.
    pub fn to_json(&self) -> Value {
        let s = &self.settings;
        let printing = match s.printing {
            Printing::None => "none",
            Printing::Low => "low",
            Printing::High => "high",
        };
        let changes = match s.changes {
            Changes::None => "none",
            Changes::Pages => "pages",
            Changes::FillSign => "fill-sign",
            Changes::CommentFillSign => "comment-fill-sign",
            Changes::AnyExceptExtract => "any-except-extract",
        };
        let compatibility = match s.algorithm {
            Algorithm::Aes256 => "aes-256",
            Algorithm::Aes128 => "aes-128",
            Algorithm::Rc4_128 => "rc4-128",
            Algorithm::Rc4_40 => "rc4-40",
        };
        json!({
            "name": self.name,
            "password_for": if self.viewing { "viewing" } else { "editing" },
            "printing": printing,
            "changes": changes,
            "copy": s.copy,
            "accessibility": s.accessibility,
            "compatibility": compatibility,
            "encrypt_metadata": s.encrypt_metadata,
        })
    }

    /// Read [`Self::to_json`]'s form; `None` for anything it would not write, so a damaged entry
    /// is never read as a looser policy.
    pub fn from_json(v: &Value) -> Option<Self> {
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        let on = |k: &str| v.get(k).and_then(Value::as_bool);
        let viewing = match text("password_for")? {
            "viewing" => true,
            "editing" => false,
            _ => return None,
        };
        let printing = match text("printing")? {
            "none" => Printing::None,
            "low" => Printing::Low,
            "high" => Printing::High,
            _ => return None,
        };
        let changes = match text("changes")? {
            "none" => Changes::None,
            "pages" => Changes::Pages,
            "fill-sign" => Changes::FillSign,
            "comment-fill-sign" => Changes::CommentFillSign,
            "any-except-extract" => Changes::AnyExceptExtract,
            _ => return None,
        };
        let algorithm = match text("compatibility")? {
            "aes-256" => Algorithm::Aes256,
            "aes-128" => Algorithm::Aes128,
            "rc4-128" => Algorithm::Rc4_128,
            "rc4-40" => Algorithm::Rc4_40,
            _ => return None,
        };
        let settings = Protection {
            printing,
            changes,
            copy: on("copy")?,
            accessibility: on("accessibility")?,
            algorithm,
            encrypt_metadata: on("encrypt_metadata")?,
            ..Protection::default()
        };
        Self::new(text("name")?, viewing, &settings).ok()
    }
}

/// Put `policy` into `list`: it replaces the one of the same name (letter case aside), else is
/// added in name order. Refused when the list is full.
pub fn save_policy(list: &mut Vec<SecurityPolicy>, policy: SecurityPolicy) -> Result<(), String> {
    let key = policy.name.to_lowercase();
    if let Some(old) = list.iter_mut().find(|p| p.name.to_lowercase() == key) {
        *old = policy;
        return Ok(());
    }
    if list.len() >= MAX_POLICIES {
        return Err(format!("at most {MAX_POLICIES} security policies are kept; delete one first"));
    }
    list.push(policy);
    list.sort_by_key(|p| p.name.to_lowercase());
    Ok(())
}

/// The policy called `name`, letter case aside.
pub fn find_policy<'a>(list: &'a [SecurityPolicy], name: &str) -> Option<&'a SecurityPolicy> {
    let key = name.trim().to_lowercase();
    list.iter().find(|p| p.name.to_lowercase() == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_policy_keeps_the_restrictions_and_never_a_password() {
        let typed = Protection {
            permissions_password: Some("hunter2".into()),
            printing: Printing::Low,
            changes: Changes::FillSign,
            copy: true,
            ..Protection::default()
        };
        let policy = SecurityPolicy::new("  Client copies ", false, &typed).unwrap();
        let stored = policy.to_json();
        assert!(!stored.to_string().contains("hunter2"));
        let read = SecurityPolicy::from_json(&stored).unwrap();
        assert_eq!(read, policy);
        assert_eq!(read.name, "Client copies");
        let applied = read.protection("new one".into());
        assert_eq!((applied.open_password, applied.permissions_password.as_deref()), (None, Some("new one")));
        assert_eq!((applied.printing, applied.changes, applied.copy), (Printing::Low, Changes::FillSign, true));

        // Damaged or unknown values are not guessed at.
        let mut bad = stored.clone();
        bad["changes"] = json!("everything");
        assert!(SecurityPolicy::from_json(&bad).is_none());
        assert!(SecurityPolicy::new(" ", true, &typed).is_err());

        let mut list = Vec::new();
        save_policy(&mut list, policy).unwrap();
        save_policy(&mut list, SecurityPolicy::new("client COPIES", true, &typed).unwrap()).unwrap();
        assert_eq!(list.len(), 1);
        assert!(find_policy(&list, "Client Copies").is_some_and(|p| p.viewing));
    }
}
