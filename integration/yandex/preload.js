"use strict";

if (location.protocol !== "music-application:" || location.hostname !== "desktop" || !process.isMainFrame) return;
const { contextBridge: bridge, ipcRenderer: ipc } = require("electron");
bridge.exposeInMainWorld("OvertoneBridge", {
    settings: () => ipc.sendSync("overtone:settings"),
    plugins: () => ipc.sendSync("overtone:plugins"),
    fontFile: () => ipc.invoke("overtone:font"),
    trackLabels: (ids) => ipc.invoke("overtone:track-labels", ids),
    audio: (id) => ipc.invoke("overtone:audio", id),
    listening: (playing) => ipc.send("overtone:listening", playing),
    save: (settings) => ipc.invoke("overtone:save", settings),
    lyrics: (track, format) => ipc.invoke("overtone:lyrics", track, format),
    presence: (state) => ipc.send("overtone:presence", state),
    connection: () => ipc.invoke("overtone:connection"),
    network: () => ipc.invoke("overtone:network"),
    ready: () => ipc.send("overtone:ready"),
});
