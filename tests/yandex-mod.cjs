"use strict";

const assert = require("node:assert/strict");
const {EventEmitter} = require("node:events");
const {mkdtempSync, readFileSync, rmSync} = require("node:fs");
const {tmpdir} = require("node:os");
const {join} = require("node:path");
const {Discord, activity, coverCandidates, frame, normalizeCover} = require("../integration/yandex/discord.cjs");
const {ListeningTime} = require("../integration/yandex/plugins.cjs");
const {Catalog, detectCodec} = require("../integration/yandex/fckcensor.cjs");

assert.equal(normalizeCover("//avatars.mds.yandex.net/get-music/a/%%"),
    "https://avatars.mds.yandex.net/get-music/a/400x400");
assert.equal(normalizeCover("avatars.mds.yandex.net/get-music/a/%25%25"),
    "https://avatars.mds.yandex.net/get-music/a/400x400");
assert.equal(normalizeCover("http://avatars.mds.yandex.net/get-music/a/400x400#foo"),
    "https://avatars.mds.yandex.net/get-music/a/400x400");
assert.equal(normalizeCover("https://notyandex.net/music"), "");
assert.equal(normalizeCover("https://yandex.net.attacker.invalid/music"), "");
assert.equal(normalizeCover("blob:https://music.yandex.ru/1234"), "");
assert.equal(activity({title:"Track", artist:"Artist", status:"paused"}, {}).assets.large_image, "logo");

function verifyArtworkChanges() {
    const first = coverCandidates(["//avatars.yandex.net/get-music-content/42/first/%%"]);
    const second = coverCandidates(["https://avatars.mds.yandex.net/get-music-content/42/second/%25%25"]);
    assert.deepEqual(first, [
        "https://avatars.mds.yandex.net/get-music-content/42/first/400x400",
        "https://avatars.yandex.net/get-music-content/42/first/400x400",
    ]);

    const socket = new EventEmitter();
    const writes = [];
    socket.write = (bytes) => writes.push(JSON.parse(bytes.subarray(8)));
    socket.end = socket.write;
    const discord = new Discord();
    discord.settings = {enabled: true, applicationId: "1"};
    discord.attach(socket, discord.generation);
    socket.emit("data", frame(1, {evt: "READY"}));

    const sendTrack = (covers) => {
        discord.sentAt = 0;
        discord.track({id: "42", title: "Test Track", status: "playing", covers});
        return writes.at(-1);
    };
    const reply = (sent, data, evt) => {
        discord.sentAt = 0;
        socket.emit("data", frame(1, {cmd: "SET_ACTIVITY", nonce: sent.nonce, data, evt}));
    };
    const image = (sent = writes.at(-1)) => sent.args.activity.assets.large_image;

    const previous = sendTrack(first);
    assert.equal(image(), first[0]);
    reply(previous, {details: "Test Track", assets: {large_image: "mp:external/old"}});
    const current = sendTrack(second);
    assert.equal(image(), second[0]);
    reply(previous, {details: "Test Track", assets: {}});
    assert.equal(writes.at(-1), current);
    reply(current, {details: "Test Track", assets: {large_image: "mp:external/old"}});
    assert.equal(image(), second[1]);
    reply(writes.at(-1), {message: "image unavailable"}, "ERROR");
    assert.equal(image(), "logo");
    discord.stop();
}

verifyArtworkChanges();

(async () => {
    const folder = mkdtempSync(join(tmpdir(), "overtone-time-test-"));
    try {
        const base = "raw.githubusercontent.com/Hazzz895/FckCensorData/main/tracks/";
        const data = {sources: [{tracks: {"42": {title: "Alternate track"}}, tracksStorages: [{
            urlTemplate: `${base}%%`, trackIds: [42], tracks: {"43": 42, "44": "https://example.com/audio"},
        }]}]};
        let probes = 0;
        const catalog = new Catalog(folder, async (url, options) => {
            if (url.endsWith("list_v2.json")) return Response.json(data);
            assert.equal(url, `https://${base}42`);
            assert.equal(options.headers.Range, "bytes=0-31");
            probes++;
            return new Response(Buffer.from("ID3\x04"), {status: 206});
        });
        await catalog.refresh();
        assert.equal(catalog.value.metadata["42"].title, "Alternate track");
        assert.equal(catalog.media("44"), null);
        assert.equal((await catalog.media("43")).codec, "mp3");
        assert.equal((await catalog.media("42")).url, `https://${base}42`);
        assert.equal(probes, 1);
        assert.equal(detectCodec(Buffer.from("fLaC")), "flac");
        assert.equal(detectCodec(Buffer.from("<!DO")), "");

        const file = join(folder, "time.json");
        const timer = new ListeningTime(file);
        timer.set(true);
        await new Promise((resolve) => setTimeout(resolve, 30));
        timer.set(false);
        const {days} = JSON.parse(readFileSync(file, "utf8"));
        assert.equal(Object.keys(days).length, 1);
        assert.ok(Object.values(days)[0] >= 20);
    } finally {
        rmSync(folder, {recursive:true, force:true});
    }
})().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
