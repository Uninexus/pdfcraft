use super::PdfCraftApp;

use craft::ui::runtime::frame::FrameInput;
use craft::ui::shell::embed;

impl PdfCraftApp {
    /// The app's part of [`eframe::App::raw_input_hook`], before egui reads the frame's input:
    /// the runtime hands over what it gathered since the last frame (the commands fired) and takes this frame's root values,
    /// command states and host. eframe calls the hook once per frame, before the first pass, so
    /// a second pass of the same frame runs on what this call set and fires nothing twice.
    pub(super) fn prepare_raw_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        for event in &raw.events {
            match event {
                egui::Event::Key { pressed: true, .. } => self.keyboard_last = true,
                egui::Event::PointerButton { pressed: true, .. } => self.keyboard_last = false,
                _ => {}
            }
        }
        self.return_focus_from_preferences(ctx);
        // A panel opened by keys takes the focus a couple of passes later, after the palette or menu has given its own back.
        if let Some((panel, passes)) = self.panel_focus {
            self.panel_focus = passes.checked_sub(1).map(|left| (panel, left));
            ctx.request_repaint();
        }
        // Asked before the framework moves the focus for this input: only a Tab pressed on the zoom field counts, not the
        // one that brings the focus to it. The press itself is read in the pass, where automation's input has arrived too.
        self.zoom_held = self.zoom_field.is_some_and(|id| craft::ui::runtime::focus::has(ctx, id));
        self.set_title_bar_choice(ctx, self.title_bar_choice);
        self.control_before_input(ctx);
        embed::before_input(ctx, |output| {
            for fired in &output.fired {
                self.run_fired(ctx, fired);
            }
            for declined in &output.declined {
                self.run_declined(declined);
            }
            let commands = crate::commands::catalog::command_input(self, ctx);
            FrameInput::new(self.root_env(ctx)).commands(commands).host(self.host.clone())
        });
        self.last_frame_active = self.active_ids().map(|(_, id)| id);
    }

    /// Keeps what had the focus while Preferences is closed, and puts the focus back there after it closes when nothing
    /// else took it: the button that opened it is off while the window is up, so the framework forgets it.
    fn return_focus_from_preferences(&mut self, ctx: &egui::Context) {
        if self.preferences.is_some() {
            self.prefs_return = Some(2);
            return;
        }
        let focused = ctx.memory(|memory| memory.focused());
        match self.prefs_return {
            None => self.focus_before_prefs = focused,
            Some(0) => {
                self.prefs_return = None;
                if focused.is_none()
                    && let Some(id) = self.focus_before_prefs
                {
                    craft::ui::runtime::focus::request(ctx, craft::ui::runtime::focus::FocusTarget::Id(id));
                }
            }
            Some(passes) => {
                self.prefs_return = Some(passes - 1);
                ctx.request_repaint();
            }
        }
    }
}
