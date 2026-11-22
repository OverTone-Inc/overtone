// Catalog lookup adapted from Hazzz895/FckCensor, src/api/remote-api.ts.
// Copyright (c) 2026 Haz. Licensed under MIT, see licenses/FckCensor-MIT.txt.
const fs = require("node:fs/promises");
const path = require("node:path");

const SOURCE = "https://raw.githubusercontent.com/Hazzz895/FckCensorData/main/list_v2.json";
const MAX_CATALOG = 1024 * 1024;
const MAX_TRACKS = 10_000;
const sixHours = 6 * 3600_000;

function trackId(value) {
    return /^(?:[0-9]{1,24})$/.test(String(value)) ? String(value) : "";
}

function audioUrl(value) {
    if (typeof value !== "string") return "";
    const url = value.includes("://") ? value : "https://" + value;
    return /^https:\/\/raw\.githubusercontent\.com\/Hazzz895\/FckCensorData\/(?:refs\/heads\/)?main\/tracks\/[0-9]{1,24}$/.test(url) ? url : "";
}

function parseCatalog(data) {
    const metadata = {};
    const audio = {};
    if (!Array.isArray(data?.sources)) throw new Error("Некорректный каталог FckCensor");
    for (const source of [...data.sources].reverse()) {
        for (const [rawId, item] of Object.entries(source?.tracks ?? {})) {
            const id = trackId(rawId);
            if (!id || !item || typeof item !== "object") continue;
            const fields = {};
            for (const key of ["title", "version", "coverUri"]) {
                if (typeof item[key] === "string" && item[key].trim()) fields[key] = item[key].trim().slice(0, 260);
            }
            if (Object.keys(fields).length) metadata[id] = {...metadata[id], ...fields};
        }
        for (const storage of [...(source?.tracksStorages ?? [])].reverse()) {
            const template = typeof storage?.urlTemplate === "string" ? storage.urlTemplate : "";
            const add = (id, candidate, duration) => {
                const url = audioUrl(candidate);
                if (id && url) audio[id] = {url, durationMs: Number.isFinite(duration) ? duration : undefined};
            };
            for (const item of storage?.trackIds ?? []) {
                const id = trackId(item?.id ?? item);
                if (id && template.includes("%%")) add(id, template.replace("%%", id), item?.durationMs);
            }
            for (const [rawId, item] of Object.entries(storage?.tracks ?? {})) {
                const id = trackId(rawId);
                const value = item?.url ?? item;
                const reference = trackId(value);
                if (reference && template.includes("%%")) add(id, template.replace("%%", reference), item?.durationMs);
                else add(id, value, item?.durationMs);
            }
        }
    }
    if (!Object.keys(audio).length || Object.keys(audio).length > MAX_TRACKS || Object.keys(metadata).length > MAX_TRACKS) {
        throw new Error("Каталог FckCensor не содержит поддерживаемых записей");
    }
    return {metadata, audio};
}

function detectCodec(bytes) {
    const view = Buffer.from(bytes);
    if (view.subarray(0, 4).equals(Buffer.from("fLaC"))) return "flac";
    if (view.subarray(0, 3).equals(Buffer.from("ID3"))) return "mp3";
    if (view.length >= 4 && view[0] === 0xff && (view[1] & 0xe0) === 0xe0 && (view[1] & 0x06)) return "mp3";
    return "";
}

async function requestCatalog(fetcher) {
    const response = await fetcher(SOURCE, {signal: AbortSignal.timeout(7500), redirect: "error"});
    if (!response.ok) throw new Error("Сервер каталога недоступен");
    const reader = response.body.getReader();
    const chunks = [];
    let size = 0;
    try {
        for (;;) {
            const {done, value} = await reader.read();
            if (done) break;
            size += value.byteLength;
            if (size > MAX_CATALOG) throw new Error("Каталог FckCensor слишком большой");
            chunks.push(value);
        }
    } finally { await reader.cancel().catch(() => {}); }
    return Buffer.concat(chunks);
}

class Catalog {
    constructor(directory, fetcher = fetch) {
        this.file = path.join(directory, "fckcensor-catalog.json");
        this.fetch = fetcher;
        this.value = {metadata: {}, audio: {}};
        this.probes = new Map();
    }

    async refresh() {
        try {
            const file = await fs.stat(this.file);
            if (file.size <= MAX_CATALOG) {
                this.value = parseCatalog(JSON.parse(await fs.readFile(this.file, "utf8")));
                if (Date.now() - file.mtimeMs < sixHours) return;
            }
        } catch {}
        try {
            const data = await requestCatalog(this.fetch);
            this.value = parseCatalog(JSON.parse(data));
            await fs.mkdir(path.dirname(this.file), {recursive: true, mode: 0o700});
            const target = `${this.file}.${process.pid}.tmp`;
            try {
                await fs.writeFile(target, data, {mode: 0o600});
                await fs.rename(target, this.file);
            } finally { await fs.rm(target, {force: true}); }
        } catch (error) {
            console.warn("[Overtone] Каталог FckCensor недоступен", error.name);
        }
    }

    media(id) {
        const entry = this.value.audio[trackId(id)];
        if (!entry) return null;
        const cached = this.probes.get(entry.url);
        if (cached && cached.until > Date.now()) return cached.result;
        const result = this.probe(entry).catch(() => null);
        this.probes.delete(entry.url);
        this.probes.set(entry.url, {result, until: Date.now() + 120_000});
        if (this.probes.size > 64) this.probes.delete(this.probes.keys().next().value);
        return result;
    }

    async probe(entry) {
        const response = await this.fetch(entry.url, {signal: AbortSignal.timeout(3500), redirect: "error",
            headers: {Range: "bytes=0-31"}});
        if (!response.ok || !response.body) return null;
        const reader = response.body.getReader();
        try {
            const part = Buffer.alloc(4);
            let offset = 0;
            while (offset < part.length) {
                const {value, done} = await reader.read();
                if (done) break;
                const length = Math.min(value.length, part.length - offset);
                Buffer.from(value.buffer, value.byteOffset, length).copy(part, offset);
                offset += length;
            }
            const codec = detectCodec(part.subarray(0, offset));
            return codec ? {...entry, codec} : null;
        } finally { await reader.cancel().catch(() => {}); }
    }
}

module.exports = {Catalog, parseCatalog, detectCodec};
