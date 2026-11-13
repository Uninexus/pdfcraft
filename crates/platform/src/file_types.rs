//! Per-user Windows registration as a PDF app, for the portable `pdfcraft.exe` (the MSI does the
//! same in `packaging/windows/pdfcraft.wxs`).
//!
//! Windows lists an app under Settings ▸ Default apps, and under "Open with", only when it has
//! registered a ProgID and Capabilities. [`register_current_user`] writes the keys the MSI writes
//! (ProgID with icon and open command, `OpenWithProgids`, Capabilities, RegisteredApplications)
//! under `HKEY_CURRENT_USER` (no administrator rights), pointing at the given executable;
//! [`unregister_current_user`] removes them again.
//!
//! The names differ from the MSI's (`PdfCraft.Portable`, `PdfCraft (portable)`, no
//! `Applications\pdfcraft.exe`), as in `packaging/windows/portable.txt`: an MSI installed for one
//! user writes the same hive, and a portable copy must neither overwrite its keys nor delete them
//! when it unregisters.
//!
//! **It never chooses the default.** Windows 10 and 11 protect the user's choice with a hash
//! (`UserChoice`), so an app can only be offered; the user confirms in Settings. This module
//! therefore never writes `.pdf`'s own default value and never touches `FileExts`/`UserChoice`
//! ([`check_plan`] refuses a plan that does, before anything is written).
//!
//! The work is split so it can be tested everywhere: [`registration_plan`] is a pure function of
//! the executable's path, and only the Windows executor runs `reg.exe` (by its full path under
//! `%SystemRoot%\System32`, with arguments rather than a shell string, as `owner_only` does for
//! the other tools Windows ships, without a console window; the registry API itself would need `unsafe`). It does not
//! broadcast `SHChangeNotify`, which needs the Windows API, so Explorer may show a new icon or an
//! "Open with" entry only after it is restarted or the user signs in again.

use std::fmt;
use std::path::Path;

/// The ProgID the files are opened with (the MSI's is `PdfCraft.Document`).
pub const PROG_ID: &str = "PdfCraft.Portable";

/// Where Windows' Default apps page is, for [`open_default_apps_settings`].
pub const DEFAULT_APPS_URI: &str = "ms-settings:defaultapps";

/// The same page, showing this app's own entry ([`APP_NAME`], percent-encoded).
pub const DEFAULT_APPS_APP_URI: &str = "ms-settings:defaultapps?registeredAppUser=PdfCraft%20(portable)";

const CLASSES: &str = r"Software\Classes";
/// Holds only the Capabilities key below; removal deletes it whole so no empty key stays.
const APP_KEY: &str = r"Software\PdfCraft.Portable";
const CAPABILITIES: &str = r"Software\PdfCraft.Portable\Capabilities";
/// The name under RegisteredApplications, and so in Settings ▸ Default apps.
const APP_NAME: &str = "PdfCraft (portable)";
const REGISTERED_APPLICATIONS: &str = r"Software\RegisteredApplications";

/// Why a registration did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTypeError {
    /// Not Windows: other systems have their own association mechanisms.
    OnlyOnWindows,
    /// The executable's path can't be written safely into a command line; the text says why.
    BadPath(String),
    /// `reg.exe` could not run or refused a change; the text names the key.
    Registry(String),
}

impl fmt::Display for FileTypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OnlyOnWindows => f.write_str("Registering PdfCraft as a PDF app is only available on Windows."),
            Self::BadPath(why) => write!(f, "PdfCraft can't be registered from this location: {why}"),
            Self::Registry(why) => write!(f, "The Windows registry could not be changed: {why}"),
        }
    }
}

impl std::error::Error for FileTypeError {}

/// One value to write under `HKEY_CURRENT_USER`. `name: None` is the key's (Default) value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryWrite {
    pub key: String,
    pub name: Option<String>,
    pub value: String,
}

/// What a registration writes and what removing it deletes, all under `HKEY_CURRENT_USER`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub writes: Vec<RegistryWrite>,
    /// Whole keys this app creates and nobody else uses.
    pub delete_keys: Vec<String>,
    /// Single values in keys that other programs share: (key, value name).
    pub delete_values: Vec<(String, String)>,
}

/// The exe path as it goes into a command line, or why it can't.
fn checked_path(exe: &Path) -> Result<&str, FileTypeError> {
    let text = exe.to_str().ok_or_else(|| FileTypeError::BadPath("the path is not valid Unicode".into()))?;
    let bytes = text.as_bytes();
    let drive = matches!(bytes, [d, b':', b'\\' | b'/', ..] if d.is_ascii_alphabetic());
    if !drive && !text.starts_with(r"\\") {
        return Err(FileTypeError::BadPath("the path is not absolute".into()));
    }
    if text.contains('"') {
        // A quote would end the quoted program name early and let the rest run as arguments.
        return Err(FileTypeError::BadPath("the path contains a double quote".into()));
    }
    if text.chars().any(char::is_control) {
        return Err(FileTypeError::BadPath("the path contains a control character".into()));
    }
    if text.contains('%') {
        // `%1` and friends are substituted in a registered command line.
        return Err(FileTypeError::BadPath("the path contains a percent sign".into()));
    }
    Ok(text)
}

fn write(key: &str, name: Option<&str>, value: &str) -> RegistryWrite {
    RegistryWrite { key: key.to_owned(), name: name.map(str::to_owned), value: value.to_owned() }
}

/// The registry writes that offer PdfCraft as a PDF app, for the executable at `exe`: the MSI's
/// keys (ProgID with icon and open command, `.pdf\OpenWithProgids`, Capabilities and
/// RegisteredApplications) under the portable names, without ever setting the default.
pub fn registration_plan(exe: &Path) -> Result<Plan, FileTypeError> {
    let exe = checked_path(exe)?;
    let command = format!("\"{exe}\" \"%1\"");
    // A negative index selects the icon resource by ID: 2 is the PDF document icon (build.rs).
    let icon = format!("\"{exe}\",-2");
    let prog = format!(r"{CLASSES}\{PROG_ID}");
    let plan = Plan {
        writes: vec![
            write(&prog, None, "PDF Document"),
            write(&format!(r"{prog}\DefaultIcon"), None, &icon),
            write(&format!(r"{prog}\shell\open\command"), None, &command),
            // No overlay on thumbnails: otherwise Windows paints the app icon over every PDF's.
            write(&prog, Some("TypeOverlay"), ""),
            write(&format!(r"{CLASSES}\.pdf\OpenWithProgids"), Some(PROG_ID), ""),
            write(CAPABILITIES, Some("ApplicationName"), APP_NAME),
            write(CAPABILITIES, Some("ApplicationDescription"), "Open-source PDF workbench"),
            write(CAPABILITIES, Some("ApplicationIcon"), &format!("{exe},0")),
            write(&format!(r"{CAPABILITIES}\FileAssociations"), Some(".pdf"), PROG_ID),
            write(REGISTERED_APPLICATIONS, Some(APP_NAME), CAPABILITIES),
        ],
        delete_keys: vec![prog, APP_KEY.to_owned()],
        delete_values: vec![
            (format!(r"{CLASSES}\.pdf\OpenWithProgids"), PROG_ID.to_owned()),
            (REGISTERED_APPLICATIONS.to_owned(), APP_NAME.to_owned()),
        ],
    };
    check_plan(&plan)?;
    Ok(plan)
}

/// Refuses a plan that would take the default away from the user: a write to `.pdf`'s own
/// default value, anything under `FileExts` or `UserChoice`, or a deletion of a key or value the
/// user or another program owns. The guard is what keeps registering an *offer* rather than a
/// takeover, so a plan is checked again before the executor runs it.
pub fn check_plan(plan: &Plan) -> Result<(), FileTypeError> {
    let refuse =
        |what: String| Err(FileTypeError::Registry(format!("refusing to change {what}: PdfCraft only offers itself, the user chooses the default")));
    let protected = |key: &str| {
        let lower = key.to_ascii_lowercase();
        lower.contains("userchoice") || lower.contains("fileexts")
    };
    for w in &plan.writes {
        let is_pdf = w.key.eq_ignore_ascii_case(&format!(r"{CLASSES}\.pdf"));
        if protected(&w.key) || is_pdf {
            return refuse(w.key.clone());
        }
    }
    for key in &plan.delete_keys {
        let owned = key.eq_ignore_ascii_case(&format!(r"{CLASSES}\{PROG_ID}")) || key.eq_ignore_ascii_case(APP_KEY);
        if protected(key) || !owned {
            return refuse(key.clone());
        }
    }
    for (key, name) in &plan.delete_values {
        let ours = (key.eq_ignore_ascii_case(&format!(r"{CLASSES}\.pdf\OpenWithProgids")) && name == PROG_ID)
            || (key.eq_ignore_ascii_case(REGISTERED_APPLICATIONS) && name == APP_NAME);
        if protected(key) || !ours {
            return refuse(format!("{key}\\{name}"));
        }
    }
    Ok(())
}

/// Offer PdfCraft (the executable at `exe`) as a PDF app for the current user. The user still has
/// to choose it in Settings ▸ Default apps ([`open_default_apps_settings`]).
pub fn register_current_user(exe: &Path) -> Result<(), FileTypeError> {
    let plan = registration_plan(exe)?;
    imp::apply(&plan)
}

/// Remove what [`register_current_user`] wrote. Keys that are already gone are fine.
pub fn unregister_current_user() -> Result<(), FileTypeError> {
    let plan = registration_plan(Path::new(r"C:\pdfcraft.exe"))?;
    imp::remove(&plan)
}

/// Open Windows Settings ▸ Default apps.
pub fn open_default_apps_settings() -> Result<(), FileTypeError> {
    imp::open_settings()
}

#[cfg(not(windows))]
mod imp {
    use super::{FileTypeError, Plan};

    pub fn apply(_: &Plan) -> Result<(), FileTypeError> {
        Err(FileTypeError::OnlyOnWindows)
    }
    pub fn remove(_: &Plan) -> Result<(), FileTypeError> {
        Err(FileTypeError::OnlyOnWindows)
    }
    pub fn open_settings() -> Result<(), FileTypeError> {
        Err(FileTypeError::OnlyOnWindows)
    }
}

/// Whether a deletion counts as done: it succeeded, or it failed because there was nothing to
/// delete (`still_present` is the answer of a query for that very key or value afterwards). A
/// failure while the thing is still there is a real one (access denied, a policy lock).
#[cfg_attr(not(windows), allow(dead_code))]
fn deletion_settled(deleted: bool, still_present: bool) -> bool {
    deleted || !still_present
}

#[cfg(windows)]
mod imp {
    use super::{FileTypeError, Plan, check_plan, deletion_settled};
    use std::process::Stdio;

    /// Runs `reg.exe` with `args`, without a console window and by its full path, and says
    /// whether it succeeded with the start of its error text (at most 300 characters). Standard
    /// output is discarded: nothing here needs it.
    fn reg(args: &[&str]) -> Result<(bool, String), FileTypeError> {
        let output = crate::owner_only::system_tool("reg.exe")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .output()
            .map_err(|e| FileTypeError::Registry(format!("reg.exe could not be started: {e}")))?;
        let text: String = String::from_utf8_lossy(&output.stderr).trim().chars().take(300).collect();
        Ok((output.status.success(), text))
    }

    fn hkcu(key: &str) -> String {
        format!(r"HKCU\{key}")
    }

    /// Writes the plan. Failing part way removes the whole registration again, so nothing is left
    /// half done; that includes a registration made earlier from the same or another folder,
    /// which the user then makes again.
    pub fn apply(plan: &Plan) -> Result<(), FileTypeError> {
        check_plan(plan)?;
        for w in &plan.writes {
            let key = hkcu(&w.key);
            let mut args = vec!["add", key.as_str()];
            match &w.name {
                Some(name) => args.extend(["/v", name.as_str()]),
                None => args.push("/ve"),
            }
            args.extend(["/t", "REG_SZ", "/d", w.value.as_str(), "/f"]);
            let (ok, why) = reg(&args)?;
            if !ok {
                let _ = remove(plan);
                return Err(FileTypeError::Registry(format!("{key}: {why}")));
            }
        }
        Ok(())
    }

    /// Deletes everything in the plan, trying every item and reporting all failures at the end.
    pub fn remove(plan: &Plan) -> Result<(), FileTypeError> {
        check_plan(plan)?;
        let mut failures: Vec<String> = Vec::new();
        for (key, name) in &plan.delete_values {
            let key = hkcu(key);
            let (deleted, why) = reg(&["delete", key.as_str(), "/v", name.as_str(), "/f"])?;
            let present = !deleted && reg(&["query", key.as_str(), "/v", name.as_str()])?.0;
            if !deletion_settled(deleted, present) {
                failures.push(format!("{key}\\{name}: {why}"));
            }
        }
        for key in &plan.delete_keys {
            let key = hkcu(key);
            let (deleted, why) = reg(&["delete", key.as_str(), "/f"])?;
            let present = !deleted && reg(&["query", key.as_str()])?.0;
            if !deletion_settled(deleted, present) {
                failures.push(format!("{key}: {why}"));
            }
        }
        if failures.is_empty() { Ok(()) } else { Err(FileTypeError::Registry(failures.join("; "))) }
    }

    pub fn open_settings() -> Result<(), FileTypeError> {
        // Explorer starts a URI and needs no console; the exit code of the short-lived launcher
        // means nothing. The page that lists this app, else the plain Default apps page.
        let root = std::env::var_os("SystemRoot").map_or_else(|| std::path::PathBuf::from(r"C:\Windows"), std::path::PathBuf::from);
        let start = |uri: &str| {
            std::process::Command::new(root.join("explorer.exe"))
                .arg(uri)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(drop)
        };
        start(super::DEFAULT_APPS_APP_URI)
            .or_else(|_| start(super::DEFAULT_APPS_URI))
            .map_err(|e| FileTypeError::Registry(format!("Settings could not be opened: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXE: &str = r"C:\Tools\PdfCraft\pdfcraft.exe";

    fn plan() -> Plan {
        registration_plan(Path::new(EXE)).expect("a plain path")
    }

    fn value<'a>(plan: &'a Plan, key: &str, name: Option<&str>) -> Option<&'a str> {
        plan.writes.iter().find(|w| w.key == key && w.name.as_deref() == name).map(|w| w.value.as_str())
    }

    #[test]
    fn the_plan_offers_the_app_the_way_the_installer_does() {
        let plan = plan();
        let command = "\"C:\\Tools\\PdfCraft\\pdfcraft.exe\" \"%1\"";
        assert_eq!(value(&plan, r"Software\Classes\PdfCraft.Portable\shell\open\command", None), Some(command));
        assert_eq!(value(&plan, r"Software\Classes\PdfCraft.Portable\DefaultIcon", None), Some("\"C:\\Tools\\PdfCraft\\pdfcraft.exe\",-2"));
        assert_eq!(value(&plan, r"Software\Classes\PdfCraft.Portable", Some("TypeOverlay")), Some(""));
        assert_eq!(value(&plan, r"Software\Classes\.pdf\OpenWithProgids", Some("PdfCraft.Portable")), Some(""));
        assert_eq!(value(&plan, r"Software\PdfCraft.Portable\Capabilities\FileAssociations", Some(".pdf")), Some("PdfCraft.Portable"));
        assert_eq!(value(&plan, r"Software\PdfCraft.Portable\Capabilities", Some("ApplicationName")), Some("PdfCraft (portable)"));
        assert_eq!(value(&plan, r"Software\RegisteredApplications", Some("PdfCraft (portable)")), Some(r"Software\PdfCraft.Portable\Capabilities"));
    }

    #[test]
    fn nothing_the_installer_may_have_written_for_the_same_user_is_touched() {
        // An MSI installed for one user writes PdfCraft.Document, Applications\pdfcraft.exe,
        // Software\PdfCraft and the RegisteredApplications value "PdfCraft" in this hive.
        let plan = plan();
        for w in &plan.writes {
            assert!(
                !w.key.contains("PdfCraft.Document") && !w.key.contains("Applications\\") && !w.key.starts_with(r"Software\PdfCraft\"),
                "{}",
                w.key
            );
            assert_ne!(w.name.as_deref(), Some("PdfCraft"));
        }
        assert!(plan.delete_keys.iter().all(|k| !k.contains("PdfCraft.Document") && !k.contains("Applications")));
    }

    #[test]
    fn the_plan_never_takes_the_default_from_the_user() {
        let plan = plan();
        for w in &plan.writes {
            let key = w.key.to_ascii_lowercase();
            assert!(!key.contains("userchoice") && !key.contains("fileexts"), "{} would touch the user's choice", w.key);
            assert!(key != r"software\classes\.pdf", "{} would set the .pdf default value", w.key);
        }
        assert_eq!(check_plan(&plan), Ok(()));
    }

    #[test]
    fn a_plan_that_would_take_the_default_is_refused() {
        let takeover = |key: &str, name: Option<&str>| {
            let mut plan = plan();
            plan.writes.push(write(key, name, PROG_ID));
            check_plan(&plan)
        };
        assert!(takeover(r"Software\Classes\.pdf", None).is_err(), "the .pdf default value");
        assert!(takeover(r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.pdf\UserChoice", Some("ProgId")).is_err());
        let mut deletes = plan();
        deletes.delete_keys.push(r"Software\Classes\.pdf".into());
        assert!(check_plan(&deletes).is_err(), "removing the whole .pdf key would take other apps' entries too");
        let mut msi = plan();
        msi.delete_keys.push(r"Software\PdfCraft".into());
        assert!(check_plan(&msi).is_err(), "the installer's own key is not ours to delete");
        let mut values = plan();
        values.delete_values.push((r"Software\Classes\.pdf\OpenWithProgids".into(), "AcroExch.Document".into()));
        assert!(check_plan(&values).is_err(), "another app's entry stays");
    }

    #[test]
    fn removal_deletes_only_what_the_registration_created() {
        let plan = plan();
        assert_eq!(plan.delete_keys, [r"Software\Classes\PdfCraft.Portable", r"Software\PdfCraft.Portable"]);
        assert_eq!(
            plan.delete_values,
            [
                (r"Software\Classes\.pdf\OpenWithProgids".to_owned(), "PdfCraft.Portable".to_owned()),
                (r"Software\RegisteredApplications".to_owned(), "PdfCraft (portable)".to_owned())
            ]
        );
        // Every key a write lands in is inside what removal deletes, or is a shared key whose one value is removed.
        for w in &plan.writes {
            let covered = plan.delete_keys.iter().any(|k| w.key == *k || w.key.starts_with(&format!("{k}\\")))
                || plan.delete_values.iter().any(|(k, n)| *k == w.key && Some(n) == w.name.as_ref());
            assert!(covered, "{} would be left behind by an unregister", w.key);
        }
    }

    #[test]
    fn unsafe_executable_paths_are_refused() {
        for bad in [r#"C:\a"b\pdfcraft.exe"#, r"relative\pdfcraft.exe", "pdfcraft.exe", "C:\\a\nb\\pdfcraft.exe", r"C:\100%\pdfcraft.exe", ""] {
            assert!(matches!(registration_plan(Path::new(bad)), Err(FileTypeError::BadPath(_))), "{bad:?}");
        }
        for good in [EXE, r"\\server\share\pdfcraft.exe", r"D:\My Apps\PdfCraft (portable)\pdfcraft.exe"] {
            assert!(registration_plan(Path::new(good)).is_ok(), "{good:?}");
        }
        let spaced = registration_plan(Path::new(r"D:\My Apps\pdfcraft.exe")).expect("spaces are quoted");
        assert_eq!(value(&spaced, r"Software\Classes\PdfCraft.Portable\shell\open\command", None), Some("\"D:\\My Apps\\pdfcraft.exe\" \"%1\""));
    }

    #[test]
    fn a_deletion_of_something_already_gone_is_settled_but_a_refused_one_is_not() {
        assert!(deletion_settled(true, false), "deleted");
        assert!(deletion_settled(false, false), "value or key already gone");
        assert!(!deletion_settled(false, true), "refused while it is still there");
    }

    #[cfg(not(windows))]
    #[test]
    fn other_systems_get_a_clear_error() {
        let message = register_current_user(Path::new(EXE)).expect_err("not Windows").to_string();
        assert!(message.contains("only available on Windows"), "{message}");
        assert_eq!(unregister_current_user(), Err(FileTypeError::OnlyOnWindows));
        assert_eq!(open_default_apps_settings(), Err(FileTypeError::OnlyOnWindows));
        // A bad path is still reported as one, wherever it is tried.
        assert!(matches!(register_current_user(Path::new(r#"C:\a"b.exe"#)), Err(FileTypeError::BadPath(_))));
    }
}
