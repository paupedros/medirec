const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const $ = (id) => document.getElementById(id);

const screens = ["s1", "sRec", "sReady", "s2", "s3"];
const stepOf = { s1: 1, sRec: 1, sReady: 1, s2: 2, s3: 3 };

let current = "s1";
function show(id) {
  // Spatial consistency: forward moves in from the right, backward from the left
  const back = screens.indexOf(id) < screens.indexOf(current);
  current = id;
  screens.forEach((s) => ($(s).hidden = s !== id));
  const el = $(id);
  el.classList.remove("enter", "back");
  void el.offsetWidth;
  el.classList.add("enter");
  if (back) el.classList.add("back");
  [1, 2, 3].forEach((n) => $("st" + n).classList.toggle("on", n === stepOf[id]));
  if (id !== "sReady" && id !== "s3") audio.pause();
  msg("");
}

let msgTimer;
function msg(text, error = false) {
  const m = $("msg");
  clearTimeout(msgTimer);
  m.hidden = !text;
  m.textContent = text;
  m.className = error ? "error" : "";
  if (text && !error) msgTimer = setTimeout(() => (m.hidden = true), 4000);
}

const fmt = (s) => {
  s = Math.max(0, Math.floor(s || 0));
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
};
const friendly = (e) => (typeof e === "string" ? e : "Ha ocurrido un problema. Inténtelo de nuevo.");

/* ---------- Microphone list ---------- */
async function loadMics() {
  const sel = $("micSelect");
  try {
    const [names, def] = await invoke("list_input_devices");
    let saved = null;
    try { saved = localStorage.getItem("mic"); } catch {}
    sel.innerHTML = "";
    if (!names.length) {
      sel.innerHTML = '<option value="">No se encontró ningún micrófono</option>';
      return;
    }
    names.forEach((n) => {
      const o = document.createElement("option");
      o.value = n;
      o.textContent = n === def ? `${n} (predeterminado)` : n;
      sel.appendChild(o);
    });
    sel.value = names.includes(saved) ? saved : names.includes(def) ? def : names[0];
  } catch {}
}
$("micSelect").onchange = () => {
  try { localStorage.setItem("mic", $("micSelect").value); } catch {}
};

/* ---------- Audio player (one <audio>, a UI in each screen) ---------- */
const audio = $("audio");
const SPEEDS = [1, 0.75, 0.5];
let speedIdx = 0;
const players = [...document.querySelectorAll("[data-player]")].map((slot) => {
  slot.appendChild($("tplPlayer").content.cloneNode(true));
  const p = slot.querySelector(".player");
  const seek = p.querySelector("[data-seek]");
  p.addEventListener("click", (e) => {
    const act = e.target.closest("[data-act]")?.dataset.act;
    if (act === "play") togglePlay();
    if (act === "back") audio.currentTime = Math.max(0, audio.currentTime - 5);
    if (act === "speed") {
      speedIdx = (speedIdx + 1) % SPEEDS.length;
      audio.playbackRate = SPEEDS[speedIdx];
      syncPlayers();
    }
  });
  seek.addEventListener("input", () => {
    if (audio.duration) audio.currentTime = (seek.value / 1000) * audio.duration;
  });
  return p;
});

function togglePlay() {
  if (!audio.src) return;
  audio.paused ? audio.play() : audio.pause();
}

function syncPlayers() {
  const d = audio.duration || 0;
  players.forEach((p) => {
    p.classList.toggle("playing", !audio.paused);
    p.querySelector(".lbl").textContent = audio.paused ? "Escuchar" : "Pausar";
    p.querySelector("[data-time]").textContent = `${fmt(audio.currentTime)} / ${fmt(d)}`;
    const seek = p.querySelector("[data-seek]");
    if (document.activeElement !== seek) seek.value = d ? (audio.currentTime / d) * 1000 : 0;
    seek.style.setProperty("--pos", (d ? (audio.currentTime / d) * 100 : 0) + "%");
    p.querySelector("[data-act=speed]").textContent =
      SPEEDS[speedIdx].toLocaleString("es-ES") + "×";
  });
}
["play", "pause", "timeupdate", "loadedmetadata", "ended", "seeked"].forEach((ev) =>
  audio.addEventListener(ev, syncPlayers)
);

let audioUrl = null;
async function loadPlayer() {
  const bytes = await invoke("get_audio_wav");
  if (audioUrl) URL.revokeObjectURL(audioUrl);
  audioUrl = URL.createObjectURL(new Blob([bytes], { type: "audio/wav" }));
  audio.src = audioUrl;
  audio.playbackRate = SPEEDS[speedIdx];
}

// F4 toggles playback anywhere, also while typing in the text
document.addEventListener("keydown", (e) => {
  if (e.key === "F4" && (current === "sReady" || current === "s3")) {
    e.preventDefault();
    togglePlay();
  }
});

/* ---------- Recording ---------- */
const rec = $("sRec");
let poll = null;
const setRecState = (s) => (rec.dataset.state = s);

function startPolling() {
  stopPolling();
  poll = setInterval(async () => {
    const [lvl, secs] = await invoke("rec_status");
    $("timer").textContent = fmt(secs);
    // Decibel scale (-50 dB .. 0 dB) so quiet voices still move the bar
    const v = lvl / 1000;
    const db = v > 0 ? 20 * Math.log10(v) : -50;
    $("meterBar").style.width = Math.max(0, Math.min(100, ((db + 50) / 50) * 100)) + "%";
  }, 100);
}
function stopPolling() {
  clearInterval(poll);
  poll = null;
  $("meterBar").style.width = "0%";
}

$("btnRec").onclick = async () => {
  setRecState("idle");
  $("timer").textContent = "00:00";
  show("sRec");
  await loadMics();
  $("btnStart").focus();
};

$("btnBackRec").onclick = () => show("s1");

$("btnStart").onclick = async () => {
  try {
    await invoke("start_recording", { device: $("micSelect").value || null });
  } catch (e) {
    return msg(friendly(e), true);
  }
  $("recMic").textContent = "Micrófono: " + ($("micSelect").selectedOptions[0]?.textContent || "predeterminado");
  setRecState("rec");
  startPolling();
};

$("btnPause").onclick = async () => {
  await invoke("pause_recording", { paused: true });
  setRecState("paused");
};

$("btnResume").onclick = async () => {
  await invoke("pause_recording", { paused: false });
  setRecState("rec");
};

async function finishRecording() {
  stopPolling();
  try {
    const [d, quiet] = await invoke("stop_recording");
    $("audioInfo").textContent = `Grabación de ${fmt(d)}`;
    await loadPlayer();
    show("sReady");
    if (quiet)
      msg("⚠️ Casi no se ha oído nada. Escuche la grabación; si está en silencio, elija otro micrófono y vuelva a grabar.", true);
  } catch (e) {
    setRecState("idle");
    $("timer").textContent = "00:00";
    msg(friendly(e), true);
  }
}
$("btnStop").onclick = finishRecording;
$("btnStop2").onclick = finishRecording;

// Discard needs two presses so nothing is lost by accident
let discardArmed = null;
$("btnDiscard").onclick = async () => {
  const label = $("btnDiscard").querySelector("span");
  if (!discardArmed) {
    label.textContent = "¿Seguro? Pulse otra vez para borrar";
    $("btnDiscard").classList.add("armed");
    discardArmed = setTimeout(() => {
      discardArmed = null;
      label.textContent = "Borrar y empezar de nuevo";
      $("btnDiscard").classList.remove("armed");
    }, 4000);
    return;
  }
  clearTimeout(discardArmed);
  discardArmed = null;
  label.textContent = "Borrar y empezar de nuevo";
  $("btnDiscard").classList.remove("armed");
  stopPolling();
  await invoke("cancel_recording");
  setRecState("idle");
  $("timer").textContent = "00:00";
};

/* ---------- File ---------- */
$("btnFile").onclick = async () => {
  try {
    msg("Leyendo el archivo…");
    const r = await invoke("pick_audio");
    if (!r) return msg("");
    $("audioInfo").textContent = `${r[0]} (${fmt(r[1])})`;
    await loadPlayer();
    show("sReady");
  } catch (e) {
    msg(friendly(e), true);
  }
};

$("btnBack1").onclick = () => show("s1");

/* ---------- Transcription ---------- */
listen("progress", (ev) => {
  $("prog").value = ev.payload;
  $("progText").textContent = ev.payload + " %";
});

$("btnTranscribe").onclick = async () => {
  $("prog").value = 0;
  $("progText").textContent = "0 %";
  show("s2");
  try {
    const text = await invoke("transcribe");
    $("text").value = text || "(No se ha entendido ninguna palabra)";
    audio.currentTime = 0;
    show("s3");
  } catch (e) {
    show("sReady");
    msg(friendly(e), true);
  }
};

/* ---------- PDF ---------- */
$("btnPdf").onclick = async () => {
  try {
    const date = new Date().toLocaleString("es-ES", { dateStyle: "long", timeStyle: "short" });
    const ok = await invoke("export_pdf", {
      title: $("title").value || "Transcripción",
      date,
      text: $("text").value,
    });
    if (ok) msg("✅ PDF guardado correctamente.");
  } catch (e) {
    msg(friendly(e), true);
  }
};

$("btnNew").onclick = () => show("s1");

syncPlayers();
