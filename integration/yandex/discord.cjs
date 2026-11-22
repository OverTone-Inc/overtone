"use strict";

const { randomUUID } = require("node:crypto");
const { createConnection } = require("node:net");
const { tmpdir } = require("node:os");
const { join } = require("node:path");

const MAX_FRAME = 1024 * 1024;
const UPDATE_INTERVAL = 5000;

function frame(opcode, data) {
    const body = Buffer.isBuffer(data) ? data : Buffer.from(JSON.stringify(data));
    const header = Buffer.alloc(8);
    header.writeUInt32LE(opcode);
    header.writeUInt32LE(body.length, 4);
    return Buffer.concat([header, body]);
}

function paths() {
    const runtime = process.env.XDG_RUNTIME_DIR;
    const directories = new Set([runtime, runtime && join(runtime, "app/com.discordapp.Discord"),
        runtime && join(runtime, "app/com.discordapp.DiscordCanary"), tmpdir(), "/tmp"].filter(Boolean));
    return [...directories].flatMap((dir) => Array.from({length: 10}, (_, i) => join(dir, `discord-ipc-${i}`)));
}

function connect(path) {
    return new Promise((resolve) => {
        const socket = createConnection(path);
        socket.on("error", () => {});
        const deadline = setTimeout(() => socket.destroy(), 250);
        socket.once("connect", () => { clearTimeout(deadline); resolve(socket); });
        socket.once("close", () => { clearTimeout(deadline); resolve(null); });
    });
}

function limit(value) {
    let text = "";
    for (const character of value || "") {
        if (Buffer.byteLength(text + character) > 128) break;
        text += character;
    }
    return text.length === 1 ? text + " " : text;
}

function normalizeCover(value) {
    if (typeof value !== "string" || value.length > 1024) return "";
    const raw = value.trim().replace(/%%|%25%25/gi, "400x400");
    if (!raw) return "";
    try {
        const url = new URL(raw.startsWith("//") ? "https:" + raw :
            /^[a-z][a-z0-9+.-]*:/i.test(raw) ? raw : "https://" + raw);
        if (!["https:", "http:"].includes(url.protocol) || url.username || url.password || url.port
            || !/(^|\.)(yandex\.(net|ru|com|by|kz|uz)|yastatic\.net)$/i.test(url.hostname)
            || url.pathname === "/") return "";
        url.protocol = "https:";
        url.hash = "";
        return url.href.length <= 512 ? url.href : "";
    } catch { return ""; }
}

function activity(track, settings) {
    const russian = settings.locale === "ru";
    const suffix = settings.showVersion && track.version ? ` (${track.version})` : "";
    const url = track.id ? (track.albumId
        ? `https://music.yandex.ru/album/${track.albumId}/track/${track.id}`
        : `https://music.yandex.ru/track/${track.id}`) : "";
    const assets = {large_image: track.cover || "logo"};
    if (settings.showAlbum && track.album) assets.large_text = limit(track.album);
    else if (!settings.hideBranding) assets.large_text = "Overtone";
    if (settings.showSmallIcon) {
        assets.small_image = "logo";
        assets.small_text = track.status === "playing"
            ? (russian ? "Воспроизводится" : "Playing") : (russian ? "На паузе" : "Paused");
    }

    const result = {type: 2, status_display_type: settings.status, instance: false,
        details: limit(track.title + suffix), assets};
    if (track.artist) result.state = limit(track.artist);
    if (track.status === "playing" && track.duration && track.position < track.duration) {
        const speed = track.speed > 0 ? track.speed : 1;
        const start = Math.min(Date.now(), track.observedAt) - 1000 * track.position / speed;
        result.timestamps = {start: Math.floor(start / 1000),
            end: Math.ceil((start + 1000 * track.duration / speed) / 1000)};
    }
    if (settings.showButtons && url) {
        result.buttons = [{label: russian ? "Открыть в Яндекс Музыке" : "Open in Yandex Music", url}];
        if (!settings.mergeLinks && track.albumId) {
            result.buttons.unshift({label: russian ? "Открыть в приложении" : "Open in the app",
                url: `yandexmusic://album/${track.albumId}/track/${track.id}`});
        } else if (settings.showProjectButton && settings.projectUrl) {
            result.buttons.push({label: russian ? "Сайт проекта" : "Project website", url: settings.projectUrl});
        }
    }
    return result;
}

class Discord {
    constructor() {
        this.settings = {};
        this.generation = 0;
        this.status = "Выключено";
    }

    configure(settings) {
        const previous = this.settings;
        this.settings = settings;
        if (!settings.enabled || previous.applicationId !== settings.applicationId) {
            this.stop();
            this.status = settings.enabled ? "Подключение…" : "Выключено";
        }
        if (!settings.enabled) {
            this.current = undefined;
            this.pausedAt = undefined;
            return;
        }
        if (!previous.enabled || !this.socket) {
            clearTimeout(this.retry);
            this.retry = undefined;
            if (this.connecting !== this.generation) void this.open(this.generation);
        }
        this.update();
    }

    track(track) {
        const wasPaused = this.current?.status === "paused" && track?.status === "paused"
            && this.current.id === track.id && this.current.title === track.title;
        this.current = track;
        this.pausedAt = track?.status === "paused" ? (wasPaused ? this.pausedAt : Date.now()) : undefined;
        this.update();
    }

    stop() {
        this.generation++;
        clearTimeout(this.retry);
        clearTimeout(this.afk);
        clearTimeout(this.flush);
        this.retry = this.afk = this.flush = undefined;
        const socket = this.socket;
        this.socket = undefined;
        this.last = undefined;
        this.sentAt = undefined;
        const wasReady = this.ready;
        this.ready = false;
        if (socket) {
            if (wasReady) socket.end(this.command(null));
            else socket.destroy();
        }
    }

    schedule(generation) {
        if (generation !== this.generation || !this.settings.enabled || this.retry) return;
        if (this.settings.reconnect > 0) {
            this.retry = setTimeout(() => {
                this.retry = undefined;
                void this.open(generation);
            }, this.settings.reconnect * 1000);
            this.retry.unref();
        }
    }

    async open(generation) {
        this.connecting = generation;
        for (const path of paths()) {
            if (generation !== this.generation) return;
            const socket = await connect(path);
            if (!socket) continue;
            if (generation !== this.generation) {
                socket.destroy();
                return;
            }
            this.attach(socket, generation);
            return;
        }
        if (generation !== this.generation) return;
        this.connecting = undefined;
        this.status = "Discord не найден. Откройте настольное приложение.";
        this.schedule(generation);
    }

    attach(socket, generation) {
        this.connecting = undefined;
        this.socket = socket;
        let buffer = Buffer.alloc(0);
        let deadline = setTimeout(() => socket.destroy(), 5000);
        let failure = "Соединение с Discord закрыто";
        socket.on("close", () => {
            clearTimeout(deadline);
            if (socket !== this.socket) return;
            this.socket = undefined;
            this.ready = false;
            this.last = undefined;
            this.sentAt = undefined;
            this.status = failure;
            this.schedule(generation);
        });
        socket.on("data", (chunk) => {
            if (socket !== this.socket) return;
            buffer = Buffer.concat([buffer, chunk]);
            while (buffer.length >= 8) {
                const opcode = buffer.readUInt32LE(0);
                const length = buffer.readUInt32LE(4);
                if (length > MAX_FRAME) { socket.destroy(); return; }
                if (buffer.length < 8 + length) return;
                const body = buffer.subarray(8, 8 + length);
                buffer = buffer.subarray(8 + length);
                if (opcode === 3) { socket.write(frame(4, body)); continue; }
                if (opcode === 2) { failure = "Discord отклонил соединение. Проверьте Application ID."; socket.destroy(); return; }
                if (opcode !== 1) continue;
                let response;
                try { response = JSON.parse(body); } catch { socket.destroy(); return; }
                if (response.evt === "READY") {
                    clearTimeout(deadline);
                    this.ready = true;
                    this.status = "Discord подключён";
                    this.update();
                }
                if (response.evt === "ERROR") {
                    this.status = "Discord отклонил обновление статуса";
                }
                if (response.cmd === "SET_ACTIVITY" && !response.evt) this.status = "Discord подключён";
            }
        });
        socket.write(frame(0, {v: 1, client_id: this.settings.applicationId}));
    }

    command(value) {
        return frame(1, {cmd: "SET_ACTIVITY", nonce: randomUUID(),
            args: {pid: process.pid, activity: value}});
    }

    update() {
        clearTimeout(this.afk);
        this.afk = undefined;
        const track = this.current;
        if (!this.settings.enabled) return;
        const timeout = this.settings.afkTimeout * 60000;
        const remaining = this.pausedAt && timeout ? this.pausedAt + timeout - Date.now() : Infinity;
        if (Number.isFinite(remaining) && remaining > 0) {
            this.afk = setTimeout(() => this.update(), remaining);
            this.afk.unref();
        }
        if (!this.ready) return;
        const next = track && remaining > 0 ? activity(track, this.settings) : null;
        const key = JSON.stringify(next);
        if (key === this.last) return;
        const delay = this.sentAt ? UPDATE_INTERVAL - (Date.now() - this.sentAt) : 0;
        if (next && delay > 0) {
            if (!this.flush) {
                this.flush = setTimeout(() => { this.flush = undefined; this.update(); }, delay);
                this.flush.unref();
            }
            return;
        }
        this.socket.write(this.command(next));
        this.last = key;
        this.sentAt = Date.now();
    }
}

module.exports = {Discord, activity, frame, normalizeCover};
