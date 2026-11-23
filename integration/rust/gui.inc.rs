    fn sync_overtone(&self) {
        let ui = self.ui();
        let app = ui.global::<App>();
        let settings = crate::overtone::current();
        app.set_overtone_source(settings.lyrics as i32);
        app.set_overtone_fck(settings.fckcensor);
        app.set_overtone_time(settings.time);
        app.set_overtone_spicy(settings.spicy);
        app.set_overtone_fonts(settings.fonts);
        app.set_overtone_family(settings.font.into());
        app.set_overtone_display_font(crate::overtone::family().into());
        app.set_overtone_time_summary(crate::overtone::listening_summary().into());
        ui.set_overtone_font(app.get_overtone_display_font());
        let enabled = settings.discord.unwrap_or_else(|| self.st.borrow().cfg.discord_enabled);
        app.set_discord_on(enabled);
        self.discord.cfg.lock().unwrap().enabled = enabled;
        let track = self.player.state().track;
        app.set_has_lyrics(track.is_some_and(|track| track.lyrics || crate::overtone::lyrics_available()));
    }

    fn overtone_change(self: &Rc<Self>, key: &str, value: &str) {
        let ui = self.ui();
        let app = ui.global::<App>();
        match crate::overtone::options::change(key, value) {
            Ok(()) => {
                app.set_overtone_feedback("Сохранено".into());
                if key == "time" && value == "false" {
                    crate::overtone::observe(false);
                }
                self.sync_overtone();
                if matches!(key, "lyrics" | "spicy") {
                    self.reset_lyrics();
                    self.sync_lyrics();
                }
            }
            Err(error) => {
                self.sync_overtone();
                app.set_overtone_feedback(format!("Не удалось сохранить настройку: {error}").into());
            }
        }
    }
