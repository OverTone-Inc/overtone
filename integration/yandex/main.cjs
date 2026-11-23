const { app, ipcMain } = require("electron");
const fs = require("node:fs");
const path = require("node:path");
const { homedir } = require("node:os");
const { spawn } = require("node:child_process");
const { Discord, coverCandidates } = require("./overtone-discord.cjs");

const configHome = path.isAbsolute(process.env.XDG_CONFIG_HOME || "")
    ? process.env.XDG_CONFIG_HOME : path.join(homedir(), ".config");
const directory = path.join(configHome, "overtone");
const config = path.join(directory, "yandex-mod.json");
const {load: loadPlugins, ListeningTime, write} = require("./overtone-plugins.cjs");
const {Catalog} = require("./overtone-fckcensor.cjs");
const plugins = loadPlugins(directory);
const timer = new ListeningTime(path.join(directory, "listening-time.json"));
const catalog = new Catalog(directory);
const fontFile = path.join(directory, "custom-font");
let catalogReady;
const discordAvailable = typeof applicationId === "string";
const readyFile = process.env.OVERTONE_READY_FILE;
delete process.env.OVERTONE_READY_FILE;
const discord = new Discord();
const defaults = {
    enabled: false, locale: "ru", fromYnison: false, status: 0,
    hideBranding: false, showAlbum: true, showVersion: true, showSmallIcon: true,
    showButtons: true, mergeLinks: true, showProjectButton: false, projectUrl: "",
    afkTimeout: 15, reconnect: 30, lyricsMode: "yandex", lookup: "exact",
    titleOnly: false, trackVersion: true,
};
const cache = new Map();
let nextRequest = 0;

const text = (value) => typeof value === "string" ? value.trim().slice(0, 512) : "";
const id = (value) => /^\d{1,30}$/.test(value) ? String(value) : "";
const number = (value, maximum) => Number.isFinite(value) ? Math.max(0, Math.min(value, maximum)) : 0;
function https(value) {
    try {
        const url = new URL(value);
        return url.protocol === "https:" && !url.username && !url.password && url.port === "" && url.href.length <= 512
            ? url.href : "";
    } catch { return ""; }
}

function sanitize(input) {
    const result = {...defaults};
    for (const [key, value] of Object.entries(defaults)) {
        if (typeof value === "boolean" && typeof input?.[key] === "boolean") result[key] = input[key];
    }
    for (const [key, values] of Object.entries({locale: ["ru", "en"], status: [0, 1, 2],
        lyricsMode: ["yandex", "external", "fallback", "prefer"], lookup: ["exact", "search"]})) {
        if (values.includes(input?.[key])) result[key] = input[key];
    }
    for (const [key, maximum] of [["afkTimeout", 30], ["reconnect", 300]]) {
        if (typeof input?.[key] === "number") result[key] = Math.round(number(input[key], maximum));
    }
    result.projectUrl = https(input?.projectUrl);
    if (!discordAvailable) result.enabled = false;
    return result;
}

function load() {
    try { return sanitize(JSON.parse(fs.readFileSync(config, "utf8"))); }
    catch (error) {
        if (error.code !== "ENOENT") console.warn("[Overtone] Настройки не прочитаны");
        return {...defaults};
    }
}

function save(update) {
    const updated = sanitize(update);
    fs.mkdirSync(path.dirname(config), {recursive: true, mode: 0o700});
    const temporary = `${config}.${process.pid}.tmp`;
    try {
        fs.writeFileSync(temporary, JSON.stringify(updated, null, 2) + "\n", {mode: 0o600});
        fs.renameSync(temporary, config);
    } finally {
        fs.rmSync(temporary, {force: true});
    }
    const resetSource = settings.fromYnison !== updated.fromYnison;
    settings = updated;
    if (resetSource) discord.track(null);
    discord.configure({...settings, applicationId});
    return publicSettings();
}

function trusted(event) {
    try {
        const frame = event.senderFrame;
        if (!frame || frame.parent) return false;
        const url = new URL(frame.url);
        return url.protocol === "music-application:" && url.hostname === "desktop";
    } catch { return false; }
}

function metadata(value) {
    if (!value || typeof value !== "object") return null;
    const title = text(value.title);
    const artist = text(value.artist);
    return title && artist ? {
        title, artist, album: text(value.album), version: text(value.version),
        duration: number(value.duration, 86400),
    } : null;
}

const comparable = (value) => text(value).normalize("NFKC").toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, " ").trim();
function lyricsText(result, format) {
    if (!result || result.instrumental) return null;
    const content = format === "LRC" ? result.syncedLyrics :
        (result.plainLyrics || result.syncedLyrics?.replace(/^(?:\[[^\]]*\])+\s*/gm, ""));
    return typeof content === "string" && content.trim() && content.length <= 128 * 1024 ? content : null;
}

async function getLyrics(endpoint, params) {
    if (Date.now() < nextRequest) throw new Error("LRCLIB cooldown");
    const url = new URL(`https://lrclib.net/api/${endpoint}`);
    url.search = new URLSearchParams(params);
    const response = await fetch(url, {signal: AbortSignal.timeout(4500), redirect: "error",
        headers: {"User-Agent": `Overtone/${version} (desktop lyrics integration)`}});
    if (response.status === 429) {
        const wait = Number(response.headers.get("retry-after")) || 30;
        nextRequest = Date.now() + Math.min(Math.max(wait, 1), 300) * 1000;
    }
    if (response.status === 404) return null;
    if (!response.ok) throw new Error(`LRCLIB HTTP ${response.status}`);
    const reader = response.body.getReader();
    const chunks = [];
    let size = 0;
    for (;;) {
        const {done, value} = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > 1024 * 1024) { await reader.cancel(); throw new Error("LRCLIB response exceeds limit"); }
        chunks.push(value);
    }
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

async function lookup(track, format, policy) {
    const track_name = policy.trackVersion && track.version ? `${track.title} (${track.version})` : track.title;
    const artist_name = track.artist;
    if (policy.lookup === "exact" && track.album && track.duration) {
        return lyricsText(await getLyrics("get", {track_name, artist_name, album_name: track.album,
            duration: String(Math.round(track.duration))}), format);
    }
    const match = (results) => (Array.isArray(results) ? results : [])
        .filter((item) => comparable(item.trackName) === comparable(track_name)
            && comparable(item.artistName) === comparable(artist_name)
            && (!track.duration || Math.abs(Number(item.duration) - track.duration) <= 5))
        .sort((a, b) => (comparable(b.albumName) === comparable(track.album))
            - (comparable(a.albumName) === comparable(track.album)))
        .map((item) => lyricsText(item, format)).find(Boolean);
    const result = match(await getLyrics("search", {track_name, artist_name}));
    return result || (policy.titleOnly ? match(await getLyrics("search", {track_name})) : null);
}

function fetchLyrics(track, format) {
    if (settings.lyricsMode === "yandex" || !["LRC", "TEXT"].includes(format)) return null;
    const info = metadata(track);
    if (!info) return null;
    const policy = settings;
    const key = JSON.stringify([info, format, policy.lookup, policy.titleOnly, policy.trackVersion]);
    const now = Date.now();
    const existing = cache.get(key);
    if (existing?.expires > now) return existing.result;
    const entry = {expires: now + 60_000};
    entry.result = lookup(info, format, policy).then((result) => {
        if (result) entry.expires = Date.now() + 3600_000;
        return result;
    }).catch(() => null);
    cache.delete(key);
    cache.set(key, entry);
    if (cache.size > 48) cache.delete(cache.keys().next().value);
    return entry.result;
}

function presence(value) {
    if (!value || !["playing", "paused"].includes(value.status)) return null;
    const info = metadata(value);
    if (!info && !text(value.title)) return null;
    return {...info, title: text(value.title), artist: text(value.artist), album: text(value.album),
        version: text(value.version), id: id(value.id), albumId: id(value.albumId),
        status: value.status, covers: coverCandidates(Array.isArray(value.covers) ? value.covers.slice(0, 8) : [value.cover]),
        position: number(value.position, 86400),
        duration: number(value.duration, 86400), speed: number(value.speed, 5),
        observedAt: Math.abs(value.observedAt - Date.now()) < 86400_000 ? value.observedAt : Date.now()};
}

let settings = load();
let musicContents;
let relay;
let networkStatus = plugins.direct ? "Обычный маршрут" : "Выключено";

async function configureNetwork(session) {
    if (plugins.fckcensor && !catalogReady) catalogReady = catalog.refresh();
    if (process.platform !== "linux" || !plugins.direct || relay) return;
    const incompatible = ["proxy-server", "proxy-pac-url", "no-proxy-server", "proxy-auto-detect"];
    if (incompatible.some((flag) => app.commandLine.hasSwitch(flag))) return;
    try {
        const samples = ["https://music.yandex.ru/", "https://example.com/"];
        const system = await Promise.race([
            Promise.all(samples.map((url) => session.resolveProxy(url))),
            new Promise((resolve) => setTimeout(() => resolve([]), 2000).unref()),
        ]);
        if (!system.length || system.some((rule) => rule !== "DIRECT")) return;
        const worker = spawn(routeEntry, [], {stdio: ["pipe", "pipe", "ignore"]});
        worker.stdin.on("error", () => {});
        const result = await new Promise((resolve) => {
            let buffer = "";
            const deadline = setTimeout(() => done(null), 2500);
            function done(value) { clearTimeout(deadline); resolve(value); }
            worker.once("error", () => done(null));
            worker.once("exit", () => done(null));
            worker.stdout.on("data", (chunk) => {
                buffer += chunk;
                const first = buffer.indexOf("\n");
                if (first >= 0 || buffer.length > 8192) {
                    try { done(JSON.parse(buffer.slice(0, first))); } catch { done(null); }
                }
            });
        });
        if (!result || !Number.isInteger(result.port) || result.port < 1 || result.port > 65535) {
            worker.stdin.end();
            worker.kill();
            return;
        }
        relay = worker;
        worker.once("exit", () => {
            relay = undefined;
            networkStatus = "Обычный маршрут";
            void session.setProxy({mode: "system"}).catch(() => {});
        });
        // Avoid Chromium's PAC resolver. The loopback relay binds Music
        // sockets to Wi-Fi and uses the ordinary route for other HTTPS hosts.
        const local = `127.0.0.1:${result.port}`;
        await session.setProxy({mode: "fixed_servers", proxyRules: `http=direct://;https=${local},direct://`});
        const selected = await session.resolveProxy(samples[0]);
        if (selected.startsWith(`PROXY ${local}`)) {
            networkStatus = `Прямой интерфейс · ${result.interface}`;
        } else {
            await session.setProxy({mode: "system"});
            worker.stdin.end();
        }
    } catch {
        relay?.stdin.end();
    }
}

module.exports = {configureNetwork};
const publicSettings = () => ({...settings, discordAvailable});
discord.configure({...settings, applicationId});
app.on("before-quit", () => {
    timer.set(false);
    relay?.stdin.end();
    discord.stop();
});
ipcMain.on("overtone:plugins", (event) => {
    event.returnValue = trusted(event) ? {time: plugins.time, font: plugins.font, fonts: plugins.fonts,
        fckcensor: plugins.fckcensor, spicy: plugins.spicy} : {};
});
ipcMain.handle("overtone:addon", async (event, name) => {
    if (!trusted(event) || !["time", "spicy"].includes(name) || !plugins[name]) return false;
    const root = path.join(directory, "addons", name);
    try {
        const read = async (file) => {
            const target = path.join(root, file);
            if ((await fs.promises.stat(target)).size > 512_000) throw new Error("Addon asset too large");
            return fs.promises.readFile(target, "utf8");
        };
        const [source, css, react] = await Promise.all(["script.js", "script.css", "react.js"].map(read));
        const frame = event.senderFrame;
        if (!trusted(event)) return false;
        await event.sender.insertCSS(css);
        const code = `(async () => {
            const host = window.OvertoneAddonRuntime;
            const library = (() => {const exports = {};
${react}
return exports;})();
            const previous = window.__PULSESYNC_WEB_HOST__;
            let addon;
            window.__PULSESYNC_WEB_HOST__ = {apiVersion: 1, capabilities: host.caps, React: library};
            try {
                addon = (() => {
${source}
return PulseSyncAddon;})();
            } finally {
                if (previous) window.__PULSESYNC_WEB_HOST__ = previous;
                else delete window.__PULSESYNC_WEB_HOST__;
            }
            await host.install(${JSON.stringify(name)}, addon, library);
            return true;
        })()`;
        return await frame.executeJavaScript(code);
    } catch (error) {
        if (error.code !== "ENOENT") console.warn("[Overtone] Ошибка загрузки аддона", name, String(error));
        return false;
    }
});
ipcMain.on("overtone:addon-stats", (event, days, streak) => {
    if (!trusted(event) || !plugins.time || !days || typeof days !== "object") return;
    const clean = Object.fromEntries(Object.entries(days).slice(0, 15_000).filter(([key, value]) =>
        /^\d{4}-\d{2}-\d{2}$/.test(key) && Number.isFinite(value) && value >= 0 && value <= 86_400));
    const count = (number) => Number.isInteger(number) ? Math.min(40_000, Math.max(0, number)) : 0;
    const series = /^\d{4}-\d{2}-\d{2}$/.test(streak?.lastDate) ?
        {count: count(streak.count), best: count(streak.best), lastDate: streak.lastDate} : null;
    try { write(path.join(directory, "time-addon.json"), {days: clean, streak: series}); }
    catch (error) { console.warn("[Overtone] Не удалось сохранить статистику аддона", error.code); }
});
ipcMain.handle("overtone:font", async (event) => {
    if (!trusted(event) || !plugins.fonts) return null;
    let file;
    try {
        file = await fs.promises.open(fontFile, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
        const info = await file.stat();
        if (!info.isFile() || info.size <= 4 || info.size > 10 * 1024 * 1024) return null;
        return await file.readFile();
    } catch { return null; }
    finally { await file?.close(); }
});
ipcMain.handle("overtone:track-labels", (event, ids) => {
    if (!trusted(event) || !plugins.fckcensor || !Array.isArray(ids)) return {};
    const entries = ids.slice(0, 500).filter((item) => typeof item === "string" && /^\d{1,24}$/.test(item));
    return Object.fromEntries(entries.flatMap((item) => {
        const patch = catalog.value.metadata[item];
        return patch ? [[item, patch]] : [];
    }));
});
ipcMain.handle("overtone:audio", async (event, id) => {
    if (!trusted(event) || !plugins.fckcensor || typeof id !== "string" || !/^\d{1,24}$/.test(id)) return null;
    if (!Object.keys(catalog.value.audio).length && catalogReady) {
        await Promise.race([catalogReady, new Promise((resolve) => setTimeout(resolve, 1200).unref())]);
    }
    return catalog.media(id);
});
ipcMain.on("overtone:listening", (event, playing) => {
    if (trusted(event) && plugins.time) timer.set(playing === true);
});
ipcMain.on("overtone:settings", (event) => { event.returnValue = trusted(event) ? publicSettings() : defaults; });
ipcMain.handle("overtone:save", (event, update) => trusted(event) ? save(update) : null);
ipcMain.handle("overtone:network", (event) => trusted(event) ? networkStatus : "");
ipcMain.handle("overtone:connection", (event) => trusted(event) ? (discordAvailable ? discord.status : "Discord RPC ещё не подключён в этой сборке") : "");
ipcMain.handle("overtone:lyrics", (event, track, format) => trusted(event) ? fetchLyrics(track, format) : null);
ipcMain.on("overtone:presence", (event, value) => {
    if (trusted(event) && settings.enabled) discord.track(presence(value));
});
ipcMain.on("overtone:ready", (event) => {
    if (!trusted(event)) return;
    if (readyFile) {
        try { fs.writeFileSync(readyFile, "ready", {mode: 0o600}); } catch { /* launcher may be closed */ }
    }
    if (musicContents !== event.sender) {
        musicContents = event.sender;
        event.sender.once("destroyed", () => {
            if (musicContents === event.sender) {
                musicContents = undefined;
                discord.track(null);
            }
        });
    }
});
