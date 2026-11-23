"use strict";

const bridge = window.OvertoneBridge;
let settings = bridge.settings();
const plugins = bridge.plugins();
let listening = false;
window.addEventListener("pagehide", () => {
    if (listening) bridge.listening(false);
});
let playback;
let playerTypes;
let tracksResource;
let remoteState;
let navSlot;
let settingsControl;
let lastPresence;
let presenceRevision = 0;
const lyricBodies = new Map();
const knownTracks = new Map();

function describe(track) {
    return {
        id: String(track.id ?? "").split(":")[0],
        albumId: String(track.albums?.[0]?.id ?? track.albumId ?? ""),
        title: track.title ?? "",
        artist: track.artists?.map(({name}) => name).filter(Boolean).join(", ") ?? "",
        album: track.albums?.[0]?.title ?? "",
        version: track.version ?? "",
        duration: (track.durationMs ?? 0) / 1000,
        covers: [track.coverUri, track.imageUrl, track.albums?.[0]?.coverUri, track.albums?.[0]?.cover?.uri,
            track.albums?.[0]?.imageUrl].filter((cover) => typeof cover === "string" && cover),
    };
}

function remember(track) {
    const info = describe(track);
    if (info.id && info.title && info.artist) {
        knownTracks.delete(info.id);
        knownTracks.set(info.id, info);
        if (knownTracks.size > 24) knownTracks.delete(knownTracks.keys().next().value);
    }
    return info;
}

function localTrack() {
    const state = playback?.state;
    const track = state?.queueState.currentEntity.value?.entity?.data?.meta;
    if (!track?.title) return null;
    const progress = state.playerState.progress.value;
    const position = state.playerState.status.value;
    const status = position === playerTypes.paused ? "paused" :
        [playerTypes.playing, playerTypes.buffering].includes(position) ? "playing" : null;
    if (!status) return null;
    const info = remember(track);
    const media = navigator.mediaSession?.metadata;
    const matches = media?.title === info.title && media.artist === info.artist
        && (!media.album || !info.album || media.album === info.album);
    const covers = matches ? [...info.covers, ...media.artwork.map(({src}) => src)] : info.covers;
    return {...info, covers, status, duration: progress?.duration || info.duration,
        position: progress?.position || 0, speed: state.playerState.speed?.value || 1, observedAt: Date.now()};
}

async function remoteTrack() {
    const {active_device_id_optional: device, player_state: player} = remoteState ?? {};
    const queue = player?.player_queue;
    const status = player?.status;
    const track = queue?.playable_list?.[queue.current_playable_index];
    if (!device || !track?.playable_id || !status || typeof status.paused !== "boolean") return null;
    const id = String(track.playable_id).split(":")[0];
    let info = knownTracks.get(id);
    if (!info && tracksResource) {
        let result = await tracksResource.getTracksMeta({trackIds: [id]});
        result = result?.result ?? result;
        if (result?.[0]) info = remember(result[0]);
    }
    if (!info) return null;
    const observedAt = Number(status.version?.timestamp_ms);
    return {...info, albumId: info.albumId || track.album_id_optional,
        status: status.paused ? "paused" : "playing", position: Number(status.progress_ms) / 1000,
        duration: Number(status.duration_ms) / 1000 || info.duration, speed: Number(status.playback_speed) || 1,
        observedAt: Number.isFinite(observedAt) && observedAt > 0 ? observedAt : Date.now()};
}

async function publish(force = false) {
    if (!settings.enabled && !plugins.time) return;
    const local = localTrack();
    const playing = Boolean(local && playback.state.playerState.status.value === playerTypes.playing);
    if (plugins.time && !addonHost.has("time") && playing !== listening) {
        listening = playing;
        bridge.listening(playing);
    }
    if (!settings.enabled) return;
    const revision = ++presenceRevision;
    const remote = settings.fromYnison ? await remoteTrack().catch(() => null) : null;
    if (revision !== presenceRevision || !settings.enabled) return;
    const value = remote || local;
    const key = JSON.stringify(value && [value.id, value.title, value.version, value.artist,
        value.status, value.album, value.covers, value.duration, value.speed]);
    if (force || key !== lastPresence) {
        lastPresence = key;
        bridge.presence(value);
    }
}

function removeOldLyrics() {
    if (lyricBodies.size > 16) lyricBodies.delete(lyricBodies.keys().next().value);
}

window.OvertoneRuntime = Object.freeze({
    audio: (id) => plugins.fckcensor ? bridge.audio(String(id)).catch(() => null) : null,
    async metadata(tracks) {
        if (!Array.isArray(tracks) || !plugins.fckcensor) return tracks;
        const patches = await bridge.trackLabels(tracks.map(({id}) => String(id))).catch(() => ({}));
        return tracks.map((track) => {
            const alternative = Object.hasOwn(patches, String(track?.id))
                ? patches[String(track.id)] : null;
            if (!alternative) return track;
            return {...track, title: alternative.title ?? track.title,
                version: alternative.version ?? track.version, coverUri: alternative.coverUri ?? track.coverUri};
        });
    },
    navigation(element) {
        navSlot = element;
        if (settingsControl) insertControl();
        addonHost.move();
    },
    externalLyrics: () => settings.lyricsMode !== "yandex",
    async lyrics(request, original, getTrack) {
        const mode = settings.lyricsMode;
        if (mode === "yandex") return original();
        const attempt = async () => {
            const id = String(request.trackId).split(":")[0];
            let info = knownTracks.get(id);
            if (!info) {
                let response = await getTrack();
                response = response?.result ?? response;
                if (response?.[0]) info = remember(response[0]);
            }
            if (!info) return null;
            const content = await bridge.lyrics(info, request.format);
            if (!content) return null;
            const url = `overtone:lyrics/${crypto.randomUUID()}`;
            lyricBodies.set(url, content);
            removeOldLyrics();
            return {downloadUrl: url, writers: []};
        };
        const external = () => attempt().catch(() => null);
        const unavailable = () => {
            throw new Error("Overtone: в выбранном источнике текста нет");
        };
        if (mode === "external") return await external() ?? unavailable();
        if (mode === "prefer") return await external() ?? original();
        try {
            const result = await original();
            if (result?.downloadUrl) return result;
        } catch (error) {
            const result = await external();
            if (result) return result;
            throw error;
        }
        return await external() ?? unavailable();
    },
    text: (url) => lyricBodies.get(url),
    tracks: (resource) => { tracksResource = resource; },
    player(player, states) {
        if (playback === player) return;
        playback = player;
        playerTypes = states;
        const update = () => { addonHost.update(); void publish(); };
        player.state.queueState.currentEntity.onChange(update);
        player.state.playerState.status.onChange(update);
        player.state.playerState.event.onChange(() => {
            addonHost.update();
            if (player.state.playerState.event.value === states.seek) queueMicrotask(() => void publish(true));
            else void publish();
        });
        player.state.playerState.speed?.onChange(update);
        update();
    },
    ynison(snapshot) {
        remoteState = snapshot;
        if (settings.fromYnison) void publish();
    },
});

function insertControl() {
    const container = navSlot || document.body;
    if (!container || settingsControl.parentElement === container) return;
    settingsControl.toggleAttribute("data-fallback", !navSlot);
    container.append(settingsControl);
}

const icons = {
    sliders: '<path d="M4 6h5m4 0h7M4 12h9m4 0h3M4 18h2m4 0h10"/><circle cx="11" cy="6" r="2"/><circle cx="15" cy="12" r="2"/><circle cx="8" cy="18" r="2"/>',
    discord: '<path d="M9 7 8 5C6 5 4 6 3 7 1 10 1 15 2 17l4 2 1-2m8-10 1-2c2 0 4 1 5 2 2 3 2 8 1 10l-4 2-1-2M6 16c4 2 8 2 12 0M10 12h.01m4 0h.01"/>',
    lyrics: '<path d="M4 5h16M4 10h16M4 15h10M4 20h7"/>',
    close: '<path d="m6 6 12 12M18 6 6 18"/>',
};
const icon = (name) => `<svg aria-hidden="true" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${icons[name]}</svg>`;
const control = `<button type="button" class="trigger" aria-label="Настройки мода" title="Настройки мода" aria-haspopup="dialog">${icon("sliders")}<span>Настройки мода</span></button>`;

const content = `
<aside class="watermark" aria-label="Overtone ${overtoneVersion}">
 <b>Overtone</b><span>${overtoneVersion}</span>
</aside>
<dialog aria-labelledby="overtone-title">
 <header class="top">
  <div><span class="product">Overtone</span><h1 id="overtone-title">Настройки мода</h1><p class="network"></p></div>
  <button class="close" type="button" aria-label="Закрыть">${icon("close")}</button>
 </header>
 <div class="tabs" role="tablist" aria-label="Разделы настроек">
  <button type="button" role="tab" id="overtone-discord-tab" aria-controls="overtone-discord" aria-selected="true">${icon("discord")}Discord RPC</button>
  <button type="button" role="tab" id="overtone-lyrics-tab" aria-controls="overtone-lyrics" aria-selected="false" tabindex="-1">${icon("lyrics")}Тексты песен</button>
 </div>
 <form>
  <main>
   <section role="tabpanel" aria-labelledby="overtone-discord-tab" id="overtone-discord">
    <div class="section-head"><h2>Статус в Discord</h2><p>Название трека, исполнитель и обложка в вашем профиле.</p></div>
    <div class="card">
     <label class="row"><span>Показывать текущий трек</span><input type="checkbox" role="switch" name="enabled"></label>
     <p class="connection" aria-live="polite"></p>
    </div>
    <fieldset id="overtone-rpc-fields">
     <div class="card">
      <h3>Подключение</h3>
      <label class="row"><span>Данные с других устройств<small>Использовать Ynison, если он активен в Яндекс Музыке.</small></span><input type="checkbox" role="switch" name="fromYnison"></label>
      <label class="row"><span>Интервал переподключения<small>Секунды. 0 отключает повторы.</small></span><input type="number" name="reconnect" min="0" max="300"></label>
     </div>
     <div class="card">
      <h3>Внешний вид</h3>
     <label class="row"><span>Короткий статус</span><select name="status"><option value="0">Платформа</option><option value="1">Артист</option><option value="2">Трек</option></select></label>
     <label class="row"><span>Язык подписей в статусе</span><select name="locale"><option value="ru">Русский</option><option value="en">English</option></select></label>
     <label class="row"><span>Показывать название альбома</span><input type="checkbox" role="switch" name="showAlbum"></label>
     <label class="row"><span>Показывать версию трека</span><input type="checkbox" role="switch" name="showVersion"></label>
     <label class="row"><span>Скрывать подпись Overtone на обложке</span><input type="checkbox" role="switch" name="hideBranding"></label>
     <label class="row"><span>Маленькая иконка Overtone</span><input type="checkbox" role="switch" name="showSmallIcon"></label>
      <label class="row"><span>Скрывать статус после паузы<small>Минуты. 0 оставляет статус до смены трека.</small></span><input type="number" name="afkTimeout" min="0" max="30"></label>
     </div>
     <div class="card">
      <h3>Ссылки в профиле</h3>
     <label class="row"><span>Кнопки-ссылки</span><input type="checkbox" role="switch" name="showButtons"></label>
     <label class="row"><span>Одна ссылка для сайта и приложения</span><input type="checkbox" role="switch" name="mergeLinks"></label>
     <label class="row"><span>Кнопка сайта проекта</span><input type="checkbox" role="switch" name="showProjectButton"></label>
     <label class="row wrap"><span>Адрес сайта проекта</span><input type="url" name="projectUrl" placeholder="https://" autocomplete="off" maxlength="512"></label>
     </div>
    </fieldset>
   </section>
   <section role="tabpanel" aria-labelledby="overtone-lyrics-tab" id="overtone-lyrics" hidden>
    <div class="section-head"><h2>Источники текстов</h2><p>Текст появится в плеере Яндекс Музыки.</p></div>
    <div class="card">
    <label class="row"><span>Источник текста</span><select name="lyricsMode">
     <option value="yandex">Только Яндекс Музыка</option>
     <option value="external">Только LRCLIB</option>
     <option value="fallback">Яндекс Музыка → LRCLIB</option>
     <option value="prefer">LRCLIB → Яндекс Музыка</option>
    </select></label>
    <p class="source-note" aria-live="polite"></p>
    </div>
    <fieldset id="overtone-lyrics-fields" class="card">
     <h3>Поиск в LRCLIB</h3>
     <label class="row"><span>Режим поиска LRCLIB</span><select name="lookup"><option value="exact">Точный</option><option value="search">Расширенный</option></select></label>
     <label class="row"><span>Повторять поиск только по названию</span><input type="checkbox" role="switch" name="titleOnly"></label>
     <label class="row"><span>Учитывать версию трека</span><input type="checkbox" role="switch" name="trackVersion"></label>
    </fieldset>
   </section>
  </main>
  <footer>
   <span class="feedback" role="status"></span>
   <button type="button" class="reload" hidden>Обновить окно</button>
   <button type="submit" class="primary">Сохранить</button>
  </footer>
 </form>
</dialog>`;

const styles = `
:host {
    --ink: var(--ym-controls-color-primary-text-enabled_variant, #f6f6f6);
    --muted: color-mix(in srgb, var(--ink) 72%, var(--surface));
    --accent: var(--ym-controls-color-primary-default-enabled, #ffd633);
    --accent-ink: var(--ym-controls-color-primary-on_default-enabled, #1a180b);
    --surface: var(--ym-background-color-primary-enabled-content, #181818);
    --card: color-mix(in srgb, var(--surface) 94%, var(--ink));
    --line: color-mix(in srgb, var(--surface) 89%, var(--ink));
    --hover: color-mix(in srgb, var(--surface) 84%, var(--ink));
    --curve: cubic-bezier(.22,.78,.27,1);
    color: var(--ink);
    font: inherit;
}
:host([data-fallback]) { position: fixed; inset-inline-start: 16px; inset-block-end: 84px; z-index: 10; }
.watermark {
    position: fixed; inset-inline-end: 0; inset-block-end: 0;
    z-index: 100; display: flex; gap: 7px; align-items: center; pointer-events: none; user-select: none;
    padding: 6px 10px; border-radius: 10px 0 0 0; font-size: 13px; line-height: 1.1;
    color: var(--muted); background: color-mix(in srgb, var(--surface) 90%, transparent);
}
.watermark b { color: var(--ink); font-weight: 650; }
.watermark span { font-variant-numeric: tabular-nums; }
* { box-sizing: border-box; }
button, input, select { font: inherit; }
button { color: inherit; cursor: pointer; }
button:focus-visible, input:focus-visible, select:focus-visible { outline: 2px solid var(--accent); outline-offset: 3px; }
.trigger {
    --icon: 24px;
    display: flex; align-items: center; gap: 12px; width: calc(100% - 16px); min-height: 44px;
    margin: 3px 8px 8px; padding: 8px 12px; text-align: start; white-space: nowrap;
    background: transparent; border: 0; border-radius: 12px;
    color: var(--muted); font-size: 14px; font-weight: 600;
    -webkit-app-region: no-drag;
    transition: background 160ms ease, color 160ms ease;
}
.trigger svg { flex: 0 0 var(--icon); }
.trigger:hover, .trigger[aria-expanded=true] { color: var(--ink); background: var(--hover); }
:host-context([class*="Navbar_root_collapsed__"]) .trigger { justify-content: center; padding-inline: 0; }
:host-context([class*="Navbar_root_collapsed__"]) .trigger span { display: none; }
:host([data-fallback]) .trigger { width: auto; margin: 0; background: var(--card); }
dialog {
    color: var(--ink); background: var(--surface); padding: 0;
    width: min(640px, calc(100vw - 32px)); height: min(716px, calc(100dvh - 58px));
    max-height: none; max-width: none; overflow: hidden;
    border: 1px solid var(--line); border-radius: 23px;
    box-shadow: 0 24px 96px #0007; font-size: 14px; line-height: 1.45;
}
dialog[open] { display: flex; flex-direction: column; transition: height 200ms var(--curve); }
dialog[data-panel="lyrics"] { height: min(575px, calc(100dvh - 58px)); }
dialog::backdrop { background: #0009; transition: opacity 180ms ease; }
dialog[data-transition]::backdrop { opacity: 0; }
.top {
    display: flex; justify-content: space-between; align-items: start; gap: 16px;
    flex: none; padding: 23px 25px 16px;
    background: radial-gradient(ellipse 72% 140% at 77% -54%, color-mix(in srgb, var(--accent) 17%, transparent), transparent 78%);
}
.product { display: block; color: var(--muted); font-size: 12px; font-weight: 650; margin-bottom: 4px; }
.network { color: var(--muted); font-size: 12px; margin-top: 4px; }
h1, h2, h3, p { margin: 0; }
h1 { font-size: 30px; line-height: 1.07; letter-spacing: -.043em; }
.close, .reload { border: 0; background: var(--card); }
.close { display: grid; place-items: center; width: 36px; height: 36px; border-radius: 50%; }
.close svg { width: 18px; height: 18px; }
.close:hover, .reload:hover { background: var(--hover); }
.tabs {
    display: flex; gap: 5px; align-self: start;
    margin: 0 25px 13px; padding: 4px; border: 1px solid var(--line); border-radius: 14px;
    background: var(--card);
}
.tabs button {
    display: flex; align-items: center; gap: 8px; padding: 7px 11px;
    border: 0; border-radius: 10px; background: transparent; color: var(--muted); font-weight: 620;
    transition: background 160ms ease, color 160ms ease;
}
.tabs svg { width: 17px; height: 17px; }
.tabs button:hover { color: var(--ink); }
.tabs button[aria-selected=true] { color: var(--ink); background: var(--hover); }
form { display: flex; flex-direction: column; flex: 1; min-height: 0; }
main {
    flex: 1; min-height: 0; overflow: auto; overscroll-behavior: contain; scrollbar-width: thin;
    scrollbar-color: var(--hover) transparent; scroll-behavior: smooth; padding: 0 25px 16px;
}
section { animation: overtone-tab-in 200ms var(--curve) both; }
@keyframes overtone-tab-in { from { opacity: 0; transform: translateY(5px); } }
.section-head { padding: 6px 0 13px; }
h2 { font-size: 19px; line-height: 1.2; letter-spacing: -.025em; }
.section-head p { color: var(--muted); margin-top: 4px; }
.card { background: var(--card); border: 1px solid var(--line); border-radius: 15px; padding: 4px 13px; }
fieldset { border: 0; margin: 0; padding: 0; min-width: 0; }
fieldset .card, fieldset.card { margin-top: 10px; }
fieldset:disabled .row { color: var(--muted); }
h3 { font-size: 12px; color: var(--muted); padding: 9px 0 3px; font-weight: 650; }
.row { display: flex; align-items: center; justify-content: space-between; gap: 12px; min-height: 42px; padding: 7px 0; }
.row + .row { border-top: 1px solid var(--line); }
.row input:not([type=checkbox]), select {
    min-width: 0; max-width: 53%; color: var(--ink); border: 1px solid var(--line);
    border-radius: 9px; background: var(--surface); padding: 6px 9px;
}
input:disabled, select:disabled { opacity: .55; }
input[type=number] { width: 75px; }
input[type=checkbox] {
    appearance: none; flex: none; width: 36px; height: 21px; padding: 0; margin: 0;
    border: 1px solid var(--line); border-radius: 20px; background: var(--hover); cursor: pointer;
    transition: background 180ms ease;
}
input[type=checkbox]::before {
    content: ""; display: block; width: 15px; height: 15px; margin: 2px;
    border-radius: 50%; background: var(--muted); transition: transform 180ms var(--curve), background 180ms ease;
}
input[type=checkbox]:checked { background: var(--accent); }
input[type=checkbox]:checked::before { transform: translateX(15px); background: var(--accent-ink); }
.row small, .source-note, .connection { color: var(--muted); font-size: 12px; }
.row small { display: block; margin-top: 2px; }
.source-note, .connection { margin: 0 0 7px; }
.connection::before {
    display: inline-block; content: ""; width: 6px; height: 6px; margin-inline-end: 6px;
    border-radius: 50%; vertical-align: 2px; background: currentColor;
}
.connection[data-online]::before { background: #61be74; }
footer {
    display: flex; align-items: center; gap: 8px; flex: none; padding: 11px 25px;
    border-top: 1px solid var(--line);
}
.feedback { color: var(--muted); font-size: 12px; flex: 1; }
.primary, .reload { padding: 8px 15px; min-height: 38px; white-space: nowrap; border-radius: 20px; }
.primary { border: 0; color: var(--accent-ink); background: var(--accent); font-weight: 700; }
.primary:hover { background: var(--ym-controls-color-primary-default-hovered, #ffe066); }
.primary:disabled { opacity: .55; }
[hidden] { display: none; }
@media (max-width: 500px) {
    .top, main, footer { padding-inline: 14px; }
    .tabs { margin-inline: 14px; }
    .row.wrap { align-items: start; flex-direction: column; gap: 3px; }
    .row.wrap input { max-width: 100%; width: 100%; }
}
@media (prefers-reduced-motion: reduce) {
    *, *::before, *::after { animation-duration: 0s !important; transition-duration: 0s !important; scroll-behavior: auto !important; }
}
`;

function setFont(name) {
    if (!name) return;
    const custom = new CSSStyleSheet();
    const family = CSS.escape(name);
    custom.replaceSync(`body.ym-font-music {
        --ym-font-family-text: ${family}, sans-serif !important;
        --ym-font-text: ${family}, sans-serif !important;
        --ym-font-heading: ${family}, sans-serif !important;
        font-family: ${family}, sans-serif !important;
    } body.ym-font-music :where(button, input, textarea, select) {
        font-family: var(--ym-font-text) !important;
    }`);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, custom];
}

async function customFont() {
    try {
        const data = await bridge.fontFile();
        if (data) {
            const bytes = data instanceof Uint8Array ? data : new Uint8Array(data);
            const face = await new FontFace("OvertoneCustom", bytes).load();
            document.fonts.add(face);
            setFont("OvertoneCustom");
            return;
        }
    } catch {}
    setFont(plugins.font);
}

function mount() {
    if (plugins.fonts) void customFont();
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(styles);
    const pluginControls = new CSSStyleSheet();
    pluginControls.replaceSync(`.overtone-addon-link {
        display: block; margin: 3px 8px; padding: 8px 12px;
        color: var(--ym-controls-color-secondary-on_outline-enabled, #d7d7d7);
        background: transparent; border: 0; border-radius: 8px;
        font: inherit; font-size: 13px; cursor: pointer;
    }
    .overtone-addon-link:hover { background: var(--ym-controls-color-secondary-default-hovered, #fff2); }
    .overtone-addon-link:focus-visible { outline: 2px solid #ffd633; }`);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, pluginControls];
    const attach = (id, html) => {
        const host = document.createElement("div");
        host.id = id;
        const shadow = host.attachShadow({mode: "open"});
        shadow.adoptedStyleSheets = [sheet];
        const template = document.createElement("template");
        template.innerHTML = html;
        shadow.append(template.content);
        return host;
    };
    settingsControl = attach("overtone-navigation", control);
    const host = attach("overtone-dialog", content);
    const shadow = host.shadowRoot;
    const trigger = settingsControl.shadowRoot.querySelector(".trigger");
    const dialog = shadow.querySelector("dialog");
    const form = shadow.querySelector("form");
    const main = shadow.querySelector("main");
    const tabs = [...shadow.querySelectorAll('[role="tab"]')];
    const close = shadow.querySelector(".close");
    const feedback = shadow.querySelector(".feedback");
    const reload = shadow.querySelector(".reload");
    const connection = shadow.querySelector(".connection");
    const motion = matchMedia("(prefers-reduced-motion: reduce)");
    const get = (name) => form.elements.namedItem(name);
    const syncControls = () => {
        get("enabled").disabled = !settings.discordAvailable;
        shadow.querySelector("#overtone-rpc-fields").disabled = !get("enabled").checked || !settings.discordAvailable;
        shadow.querySelector("#overtone-lyrics-fields").disabled = get("lyricsMode").value === "yandex";
        get("mergeLinks").disabled = !get("showButtons").checked;
        get("showProjectButton").disabled = !get("showButtons").checked || !get("mergeLinks").checked;
        get("projectUrl").disabled = get("showProjectButton").disabled || !get("showProjectButton").checked;
        get("titleOnly").disabled = get("lookup").value !== "search";
        shadow.querySelector(".source-note").textContent = {
            yandex: "Используются тексты Яндекс Музыки.",
            external: "Поиск идёт только в LRCLIB.",
            fallback: "Если в Яндекс Музыке текста нет, Overtone ищет в LRCLIB.",
            prefer: "Если в LRCLIB текста нет, Overtone использует Яндекс Музыку.",
        }[get("lyricsMode").value];
    };
    const showConnection = async () => {
        const status = await bridge.connection();
        if (connection.textContent !== status) connection.textContent = status;
        connection.toggleAttribute("data-online", status === "Discord подключён");
    };

    let wheelTarget;
    let wheelTime = 0;
    main.addEventListener("wheel", (event) => {
        if (motion.matches || event.ctrlKey || event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)
            || (event.deltaMode === 0 && Math.abs(event.deltaY) < 48)
            || main.scrollHeight <= main.clientHeight) return;
        event.preventDefault();
        const start = wheelTime && event.timeStamp - wheelTime < 200 ? wheelTarget : main.scrollTop;
        const delta = event.deltaY * [1, 24, main.clientHeight][event.deltaMode];
        wheelTarget = Math.min(main.scrollHeight - main.clientHeight, Math.max(0, start + delta));
        wheelTime = event.timeStamp;
        main.scrollTo({top: wheelTarget, behavior: "smooth"});
    }, {passive: false});

    function select(tab, focus = false) {
        dialog.dataset.panel = tab === tabs[1] ? "lyrics" : "discord";
        for (const candidate of tabs) {
            const selected = candidate === tab;
            candidate.setAttribute("aria-selected", selected);
            candidate.tabIndex = selected ? 0 : -1;
            shadow.getElementById(candidate.getAttribute("aria-controls")).hidden = !selected;
        }
        main.scrollTo({top: 0, behavior: "instant"});
        wheelTime = 0;
        if (focus) tab.focus();
    }
    for (const [index, tab] of tabs.entries()) {
        tab.onclick = () => select(tab);
        tab.onkeydown = (event) => {
            const next = {Home: 0, End: tabs.length - 1, ArrowLeft: (index + tabs.length - 1) % tabs.length,
                ArrowRight: (index + 1) % tabs.length}[event.key];
            if (next === undefined) return;
            event.preventDefault();
            select(tabs[next], true);
        };
    }

    let refresh;
    let animation;
    let closing = false;
    const full = {opacity: 1, transform: "translateY(0) scale(1)"};
    const down = {opacity: 0, transform: "translateY(12px) scale(.985)"};
    function show() {
        if (dialog.open) return;
        for (const [key, value] of Object.entries(settings)) {
            const input = get(key);
            if (input?.type === "checkbox") input.checked = value;
            else if (input) input.value = value;
        }
        syncControls();
        select(tabs[0]);
        void showConnection();
        void bridge.network().then((status) => { shadow.querySelector(".network").textContent = status; });
        dialog.setAttribute("data-transition", "");
        trigger.setAttribute("aria-expanded", "true");
        dialog.showModal();
        close.focus({preventScroll: true});
        if (!motion.matches) animation = dialog.animate([down, full], {duration: 210, easing: "cubic-bezier(.22,.78,.27,1)"});
        requestAnimationFrame(() => { if (!closing) dialog.removeAttribute("data-transition"); });
        refresh = setInterval(showConnection, 1500);
    }
    function dismiss() {
        if (!dialog.open || closing) return;
        if (motion.matches) { dialog.close(); return; }
        closing = true;
        animation?.cancel();
        dialog.setAttribute("data-transition", "");
        animation = dialog.animate([full, down], {duration: 160, easing: "ease-in", fill: "forwards"});
        animation.onfinish = () => { dialog.close(); animation.cancel(); };
    }
    trigger.onclick = show;
    close.onclick = dismiss;
    dialog.oncancel = (event) => { event.preventDefault(); dismiss(); };
    dialog.onclick = (event) => { if (event.target === dialog) dismiss(); };
    dialog.onclose = () => {
        clearInterval(refresh);
        closing = false;
        trigger.setAttribute("aria-expanded", "false");
        trigger.focus({preventScroll: true});
    };
    reload.onclick = () => location.reload();
    form.oninput = syncControls;
    form.addEventListener("invalid", (event) => {
        const panel = event.target.closest("[role=tabpanel]");
        const tab = shadow.getElementById(panel?.getAttribute("aria-labelledby"));
        if (panel?.hidden && tab) select(tab);
    }, true);
    form.onsubmit = async (event) => {
        event.preventDefault();
        const update = {};
        const submit = shadow.querySelector(".primary");
        for (const [key, current] of Object.entries(settings)) {
            const field = get(key);
            if (field) update[key] = field.type === "checkbox" ? field.checked :
                typeof current === "number" ? Number(field.value) : field.value;
        }
        submit.disabled = true;
        try {
            const updated = await bridge.save(update);
            if (!updated) throw new Error("Настройки недоступны");
            reload.hidden = reload.hidden && ((settings.lyricsMode === "yandex") === (updated.lyricsMode === "yandex"));
            feedback.textContent = reload.hidden ? "Настройки сохранены." : "Обновите окно для новых кнопок текста.";
            settings = updated;
            void publish(true);
            await showConnection();
        } catch (error) {
            feedback.textContent = error.message.replace(/^Error invoking remote method.*?Error: /, "");
        } finally {
            submit.disabled = false;
        }
    };
    document.body.append(host);
    insertControl();
    bridge.ready();
    queueMicrotask(() => {
        for (const name of ["time", "spicy"]) {
            if (plugins[name]) void bridge.loadAddon(name).catch(() => {});
        }
    });
}

if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mount, {once: true});
else mount();
