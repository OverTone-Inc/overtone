    fn run(&self, player: Player) {
        const UPDATE_INTERVAL: Duration = Duration::from_secs(5);
        type Artwork = (String, Vec<String>);

        let mut conn: Option<Conn> = None;
        let mut conn_app = String::new();
        let mut next_try = Instant::now();
        let mut next_update = Instant::now();
        let mut selected: Option<(Artwork, usize)> = None;
        let mut shown: Option<(Artwork, i64)> = None;
        let mut last_cover = None;
        let mut display_type_ok = true;
        loop {
            thread::sleep(Duration::from_secs(1));
            let cfg = self.cfg.lock().unwrap().clone();
            let app = match cfg.app_id.trim() {
                "" => APP_ID,
                own => own,
            };
            if !cfg.enabled || app != conn_app {
                conn = None;
                shown = None;
                last_cover = None;
                selected = None;
                display_type_ok = true;
                conn_app = app.into();
                next_try = Instant::now();
            }
            if !cfg.enabled {
                self.set_status("Выключено");
                continue;
            }
            if conn.is_none() {
                if Instant::now() < next_try {
                    continue;
                }
                match Conn::connect(app) {
                    Ok(client) => {
                        conn = Some(client);
                        shown = None;
                        last_cover = None;
                        next_update = Instant::now();
                        self.set_status("Discord подключён");
                    }
                    Err(error) => {
                        self.set_status(error);
                        next_try = Instant::now() + Duration::from_secs(15);
                        continue;
                    }
                }
            }
            let Some(client) = conn.as_mut() else { continue };
            let state = player.state();
            if (state.loading && state.playing) || Instant::now() < next_update {
                continue;
            }
            let track = state.track.filter(|_| state.status == Status::Ready && state.playing);
            let result = if let Some(track) = track {
                let images = covers::candidates(track.cover.iter().chain(track.album_cover.iter()));
                let key = (track.full_id(), images);
                let start = now_ms() - (state.position * 1000.0) as i64;
                if shown.as_ref().is_some_and(|(old, time)| *old == key && (time - start).abs() <= 3000) {
                    continue;
                }
                if selected.as_ref().is_none_or(|(old, _)| *old != key) {
                    selected = Some((key.clone(), 0));
                }
                let (artwork, image_index) = selected.as_mut().unwrap();
                let image = artwork.1.get(*image_index).map(String::as_str).unwrap_or("logo");
                let end = (state.duration > 1.0).then(|| start + (state.duration * 1000.0) as i64);
                next_update = Instant::now() + UPDATE_INTERVAL;
                client.set_activity(activity(&track, image, start, end, display_type_ok)).map(|reply| {
                    if reply["evt"].as_str() == Some("ERROR") {
                        let message = reply.pointer("/data/message").and_then(Value::as_str).unwrap_or("неизвестная ошибка");
                        let reason = message.to_lowercase();
                        log(format!("discord SET_ACTIVITY: {message}"));
                        if display_type_ok && (reason.contains("status_display_type") || reason.contains("status display type")) {
                            display_type_ok = false;
                        } else if reason.contains("rate limit") || reason.contains("too many") {
                            self.set_status("Discord ограничил частоту обновлений");
                        } else if image != "logo" {
                            *image_index += 1;
                        } else {
                            self.set_status("Discord отклонил обновление статуса");
                            // Keep the item pending after a rejected update, with a bounded retry cadence.
                            next_update = Instant::now() + Duration::from_secs(30);
                        }
                    } else if covers::rejected(&reply, image, last_cover.as_ref()) {
                        *image_index += 1;
                    } else {
                        if image != "logo" {
                            if let Some(resolved) = covers::resolved(&reply) {
                                last_cover = Some((image.to_owned(), resolved.to_owned()));
                            }
                        }
                        shown = Some((key, start));
                        self.set_status("Подключено, статус виден в профиле");
                    }
                })
            } else if selected.take().is_some() {
                shown = None;
                next_update = Instant::now() + UPDATE_INTERVAL;
                client.set_activity(Value::Null).map(|_| ())
            } else {
                Ok(())
            };
            if let Err(error) = result {
                log(format!("discord: {error}"));
                if let Some((artwork, index)) = selected.as_mut() {
                    *index = (*index + 1).min(artwork.1.len());
                }
                conn = None;
                shown = None;
                self.set_status("Связь с Discord потеряна, переподключение…");
                next_try = Instant::now() + Duration::from_secs(5);
            }
        }
    }
