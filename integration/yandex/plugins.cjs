const fs = require("node:fs");
const path = require("node:path");
const {performance} = require("node:perf_hooks");

function read(file) {
    try { return JSON.parse(fs.readFileSync(file, "utf8")); } catch { return {}; }
}

function write(file, data) {
    fs.mkdirSync(path.dirname(file), {recursive: true, mode: 0o700});
    const temporary = `${file}.${process.pid}.tmp`;
    try {
        fs.writeFileSync(temporary, JSON.stringify(data) + "\n", {mode: 0o600});
        fs.renameSync(temporary, file);
    } finally { fs.rmSync(temporary, {force: true}); }
}

function load(directory) {
    const settings = read(path.join(directory, "plugins.json"));
    const number = /^[0-9]{1,24}$/;
    const replacements = {};
    if (settings.fckcensor) {
        for (const [from, to] of Object.entries(settings.replacements ?? {}).slice(0, 5000)) {
            if (number.test(from) && typeof to === "string" && number.test(to) && from !== to) replacements[from] = to;
        }
    }
    return {
        replacements,
        metadata: settings.fckcensor ? read(path.join(directory, "fckcensor-metadata.json")) : {},
        time: settings.time !== false,
        font: settings.fonts && typeof settings.font === "string" ? settings.font.slice(0, 100) : "",
        direct: settings.direct !== false,
    };
}

function day(date) {
    return [date.getFullYear(), date.getMonth() + 1, date.getDate()]
        .map((part) => String(part).padStart(2, "0")).join("-");
}

class ListeningTime {
    constructor(file) {
        this.file = file;
        const days = read(file).days;
        this.days = days && typeof days === "object" && !Array.isArray(days) ? days : {};
    }

    set(playing) {
        if (playing && !this.start) {
            this.start = {clock: performance.now(), wall: Date.now()};
            this.timer = setInterval(() => this.flush(), 30_000);
            this.timer.unref();
        } else if (!playing && this.start) {
            clearInterval(this.timer);
            this.flush();
            this.start = undefined;
        }
    }

    flush() {
        if (!this.start) return;
        const clock = performance.now();
        const wall = Date.now();
        // Suspend and clock adjustments must not turn idle time into playback.
        let amount = Math.max(0, Math.min(clock - this.start.clock, 35_000));
        let position = wall - amount;
        this.start = {clock, wall};
        while (amount >= 1) {
            const date = new Date(position);
            const nextDay = new Date(date.getFullYear(), date.getMonth(), date.getDate() + 1);
            const segment = Math.min(amount, nextDay.getTime() - position);
            const key = day(date);
            this.days[key] = (Number(this.days[key]) || 0) + segment;
            amount -= segment;
            position += segment;
        }
        try { write(this.file, {days: this.days}); } catch (error) {
            console.warn("[Overtone] Не удалось сохранить время прослушивания", error.code);
        }
    }
}

module.exports = {load, ListeningTime};
