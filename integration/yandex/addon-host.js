// Compatibility layer for the pinned PulseSync web add-ons.
const addonHost = (() => {
    const listeners = new Set();
    const mounted = new Map();
    const scope = new AbortController();
    const caps = ["lifecycle-v1", "async-start-v1", "net-per-addon-v1", "resource-read-v1", "typed-settings-v1"];
    const spicyHosts = new Set(["lrclib.net", "avatars.yandex.net", "fonts.spikerko.org"]);

    function snapshot() {
        const state = playback?.state;
        const track = state?.queueState.currentEntity.value?.entity?.data?.meta;
        const status = state?.playerState.status.value;
        return {
            track, status: status === undefined ? "idle" : status === playerTypes?.playing ? "playing" :
                status === playerTypes?.paused ? "paused" : status === playerTypes?.buffering ? "buffering" : "idle",
            isPlaying: status !== undefined && status === playerTypes?.playing,
            progress: state?.playerState.progress.value,
            volume: state?.playerState.volume?.value,
        };
    }

    async function lyrics(id, format) {
        if (!tracksResource || !/^\d{1,24}(?::\d{1,24})?$/.test(id) || !["LRC", "TEXT"].includes(format)) return null;
        const endpoint = tracksResource.overtoneOriginalLyrics.bind(tracksResource);
        const result = await endpoint({trackId: id.split(":")[0], format}).catch(() => null);
        if (!result?.downloadUrl) return null;
        const response = await fetch(result.downloadUrl);
        return response.ok ? response.text() : null;
    }

    function readStats() {
        if (!mounted.has("time")) return;
        if (readStats.pending) return;
        readStats.pending = true;
        const req = indexedDB.open("YandexMusicStatsDB", 1);
        req.onupgradeneeded = () => req.transaction.abort();
        req.onerror = () => { readStats.pending = false; };
        req.onsuccess = () => {
            const db = req.result;
            const rows = db.transaction("sessions", "readonly").objectStore("sessions").openCursor(null, "prev");
            const days = {};
            rows.onerror = () => { readStats.pending = false; db.close(); };
            rows.onsuccess = () => {
                const row = rows.result;
                if (row) {
                    const {timestamp, duration} = row.value;
                    if (Number.isFinite(timestamp) && Number.isFinite(duration) && duration > 0) {
                        const date = new Date(timestamp);
                        const day = [date.getFullYear(), date.getMonth() + 1, date.getDate()]
                            .map(n => String(n).padStart(2, "0")).join("-");
                        days[day] = Math.min(86_400, (days[day] || 0) + duration);
                    }
                    row.continue();
                    return;
                }
                db.close();
                readStats.pending = false;
                let streak;
                try { streak = JSON.parse(localStorage.getItem("yandexMusicStreak.v1")); } catch {}
                bridge.addonStats(days, streak);
            };
        };
    }

    async function install(name, addon, react) {
        if (!addon || !react || mounted.has(name)) return;
        const cleanups = [];
        const key = (id) => `overtone-addon:${name}:${id}`;
        const values = name === "spicy" ? {useSpicy: false, replaceNativeLyricsButton: false, useReachVideoCover: false} : {};
        const api = {
            addonId: addon.id, logger: console, signal: scope.signal,
            onCleanup: (remove) => cleanups.push(remove),
            setTimeout: (run, delay) => {
                const timer = setTimeout(run, delay);
                cleanups.push(() => clearTimeout(timer));
                return timer;
            },
            settings: {getCurrent: () => values, onChange: () => () => {}},
            player: {
                getSnapshot: async () => snapshot(),
                onChange: (listener) => { listeners.add(listener); return () => listeners.delete(listener); },
            },
            user: {getLogin: async () => name === "time" ?
                await window.desktopEvents?.invoke("GET_PASSPORT_LOGIN") ?? "" : ""},
            router: {onChange: (listener) => {
                const target = window.navigation ?? window;
                const event = window.navigation ? "currententrychange" : "popstate";
                const changed = () => listener({pathname: location.pathname, search: location.search});
                target.addEventListener(event, changed);
                return () => target.removeEventListener(event, changed);
            }},
            notifications: {info: (message) => console.info("[Overtone]", message)},
            storage: {
                get: async (id) => { try { return JSON.parse(localStorage.getItem(key(id))); } catch { return null; } },
                set: async (id, value) => localStorage.setItem(key(id), JSON.stringify(value)),
                remove: async (id) => localStorage.removeItem(key(id)),
            },
            resources: {getLyrics: lyrics},
            net: {fetch: (input, init) => {
                const url = new URL(input);
                if (url.protocol !== "https:" || !spicyHosts.has(url.hostname) || url.port || url.username || url.password) {
                    return Promise.reject(new Error("Источник не включён в этой сборке Overtone"));
                }
                return fetch(url, {...init, credentials: "omit", redirect: "error"});
            }},
            client: {
                getSettings: async () => ({}),
                togglePlayPause: () => playback?.[snapshot().isPlaying ? "pause" : "resume"]?.(),
                previous: () => playback?.moveBackward?.(),
                next: () => playback?.moveForward?.(),
                setProgress: (sec) => playback?.setProgress?.(sec),
                setVolume: (volume) => playback?.setVolume?.(volume),
                refreshPlayerBar: () => {},
            },
        };
        let button;
        let dispose;
        try {
            dispose = await addon.activate(api);
            if (scope.signal.aborted) { dispose?.(); return; }
            const action = addon.playerBarButtons?.[0];
            if (action) {
                button = document.createElement("button");
                button.type = "button";
                button.className = "overtone-addon-link";
                button.textContent = action.label;
                button.addEventListener("click", action.onClick);
                (navSlot ?? document.body).append(button);
            }
            mounted.set(name, {button, cleanup: () => { dispose?.(); cleanups.forEach(fn => fn()); }});
            if (name === "time") {
                listening = false;
                bridge.listening(false);
                readStats();
                const timer = setInterval(readStats, 35_000);
                cleanups.push(() => clearInterval(timer));
            }
        } catch (error) {
            cleanups.forEach(fn => fn());
            console.warn("[Overtone] Не удалось запустить аддон", name, error);
            throw error;
        }
    }
    window.addEventListener("pagehide", () => {
        scope.abort();
        for (const {cleanup} of mounted.values()) cleanup();
    }, {once: true});
    window.OvertoneAddonRuntime = Object.freeze({install, caps});
    return {
        has: (name) => mounted.has(name),
        update() { const state = snapshot(); listeners.forEach(listener => listener(state)); },
        move() {
            for (const {button} of mounted.values()) if (button && navSlot && !navSlot.contains(button)) navSlot.append(button);
        },
    };
})();
