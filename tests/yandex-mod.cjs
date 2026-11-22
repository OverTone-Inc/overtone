"use strict";

const assert = require("node:assert/strict");
const {mkdtempSync, readFileSync, rmSync} = require("node:fs");
const {tmpdir} = require("node:os");
const {join} = require("node:path");
const {normalizeCover, activity} = require("../integration/yandex/discord.cjs");
const {ListeningTime} = require("../integration/yandex/plugins.cjs");

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

(async () => {
    const folder = mkdtempSync(join(tmpdir(), "overtone-time-test-"));
    try {
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
