//! "Set up PdfCraft as a PDF app…" (Windows): the portable `pdfcraft.exe` has no installer to
//! register it, so Windows doesn't list it under Settings ▸ Default apps. This registers it for
//! the current user (`pdfcraft_platform::file_types`), opens that Settings page and says what to
//! pick there. It never chooses the default itself: Windows doesn't let a program do that.

use crate::PdfCraftApp;

impl PdfCraftApp {
    pub(crate) fn set_up_default_app(&mut self) {
        use pdfcraft_platform::file_types::{self, FileTypeError};
        if !cfg!(windows) {
            self.notify_tr("Setting up PdfCraft as a PDF app is only available on Windows.");
            return;
        }
        let registered = std::env::current_exe()
            .map_err(|e| FileTypeError::BadPath(format!("the program's own path is unknown: {e}")))
            .and_then(|exe| file_types::register_current_user(&exe));
        match registered {
            Ok(()) => {
                // Settings opening is a convenience: the registration stands without it.
                if let Err(e) = file_types::open_default_apps_settings() {
                    log::warn!("{e}");
                }
                self.notify_tr("PdfCraft can now be chosen for PDF files. In Windows Settings > Default apps, pick PdfCraft for .pdf.");
            }
            Err(e) => self.notify_error(e),
        }
    }

    /// Undo [`Self::set_up_default_app`]: the user's own default is not changed, so they should
    /// pick another app for .pdf first (the notice says so).
    pub(crate) fn remove_default_app(&mut self) {
        if !cfg!(windows) {
            self.notify_tr("Setting up PdfCraft as a PDF app is only available on Windows.");
            return;
        }
        match pdfcraft_platform::file_types::unregister_current_user() {
            Ok(()) => self.notify_tr(
                "PdfCraft is no longer offered as a PDF app. If it is still the default for .pdf, choose another app in Windows Settings > Default apps.",
            ),
            Err(e) => self.notify_error(e),
        }
    }
}
