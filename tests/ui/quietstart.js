// The Engine card shows the engine's real state (issue 188): Off with Start
// until an engine is up, Starting… while it boots, Running once it answers
// (setting up, BuildKit, asleep and waking included, since docker works then);
// Stop makes it Off. Skiff's builder (its BuildKit image and cache volume)
// isn't listed in Images or Volumes, and an empty Images list offers no
// download.
skiff.on({ type: "status", version: "0.1.14", dev: false, app: "Skiff", ours: "skiff", context: "skiff" });
skiff.on({ type: "prefs", prefs: {} });
const R = [];
const t = (name, fn) => { try { fn(); R.push("PASS " + name); } catch (e) { R.push("FAIL " + name + ": " + e.message); } };
const eq = (a, b, m) => { if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(m + ": got " + JSON.stringify(a) + " want " + JSON.stringify(b)); };
const vbtn = () => document.getElementById("vm-toggle");
const label = () => document.getElementById("vm-state").textContent;
const sbar = () => document.getElementById("statusbar").textContent;
const card = () => [label(), vbtn().hidden, vbtn().textContent, vbtn().classList.contains("primary"), document.body.dataset.vm];
const OFF = ["Off", false, "Start", true, "stopped"], ON = ["Running", false, "Stop", false, "running"];
const lists = (containers) => skiff.on({ type: "lists", containers, images: [], volumes: [], networks: [] });
const bk = { Id: "bk1", Names: ["/buildx_buildkit_skiff0"], Image: "moby/buildkit:buildx-stable-1", State: "running", Status: "Up 1 second", Labels: {} };
const web = (State) => ({ Id: "web1", Names: ["/web"], Image: "nginx", State, Status: State === "running" ? "Up 1 second" : "Exited (0)", Labels: {} });

// Fresh install: no engine yet, then Skiff sets up inside (boots, pulls
// BuildKit, sleeps). A fresh Skiff has no containers of its own.
lists([]);
skiff.on({ type: "state", state: "stopped" });
t("fresh Skiff with no engine yet reads Off with Start", () => eq(card(), OFF, "stopped"));
t("status bar says off", () => eq(sbar().includes(VM + " off"), true, "status: " + sbar()));
skiff.on({ type: "state", state: "booting" });
t("its engine booting reads Starting…", () => eq([label(), vbtn().disabled], ["Starting…", true], "booting"));
skiff.on({ type: "state", state: "running" });
lists([bk]);
t("the engine up with BuildKit inside reads Running", () => eq(card(), ON, "builder"));
t("status bar says running", () => eq(sbar().includes(VM + " running"), true, "status: " + sbar()));
skiff.on({ type: "state", state: "asleep" });
t("asleep without your Start reads Running: docker still works", () => eq(card(), ON, "asleep"));
skiff.on({ type: "state", state: "stopped" });

t("Start sends start and reads Starting… then Running", () => {
  vbtn().click();
  eq(sent.at(-1)?.type, "start", "sent");
  skiff.on({ type: "state", state: "booting" });
  eq([label(), vbtn().textContent, vbtn().disabled], ["Starting…", "Starting…", true], "starting");
  skiff.on({ type: "state", state: "running" });
  eq(card(), ON, "running");
});
t("asleep after your Start stays Running, with no Sleeping line", () => {
  skiff.on({ type: "state", state: "asleep" });
  eq([...card(), document.getElementById("vm-sub").hidden], [...ON, true], "asleep");
});
t("waking after your Start stays Running", () => {
  skiff.on({ type: "state", state: "booting" });
  eq(card(), ON, "waking");
  skiff.on({ type: "state", state: "running" });
});
t("Settings › Resources shows the same switch", () => {
  prefs.tab = "resources"; show("settings");
  const b = document.querySelector("[data-vmpower]");
  eq([b?.textContent, b?.dataset.vmpower], ["Turn off engine", "stop"], "button");
  show("containers");
});
t("Stop sends stop and reads Off", () => {
  vbtn().click();
  eq(sent.at(-1)?.type, "stop", "sent");
  skiff.on({ type: "state", state: "stopping" });
  eq(label(), "Stopping…", "stopping");
  skiff.on({ type: "state", state: "stopped" });
  eq(card(), OFF, "stopped");
});

// An engine started without the card (menu bar, tray, docker run) reads so too.
t("an engine started elsewhere reads Starting… then Running", () => {
  skiff.on({ type: "state", state: "booting" });
  eq(label(), "Starting…", "booting by itself");
  skiff.on({ type: "state", state: "running" });
  lists([web("running")]);
  eq(card(), ON, "yours");
  lists([web("exited")]);
  eq(card(), ON, "still on after it exits");
});
t("Settings › Resources Start engine starts it", () => {
  skiff.on({ type: "state", state: "stopped" });
  prefs.tab = "resources"; show("settings");
  const b = document.querySelector("[data-vmpower]");
  eq([b?.textContent, b?.dataset.vmpower], ["Start engine", "start"], "button");
  b.click();
  eq(sent.at(-1)?.type, "start", "sent");
  show("containers");
  skiff.on({ type: "state", state: "running" });
});
t("a stopped container of yours doesn't change it", () => {
  lists([web("exited")]);
  eq(card(), ON, "upgrade");
});

const img = (id, tags, size = 1e8) => ({ Id: `sha256:${id}`, RepoTags: tags, RepoDigests: [], Created: 1, Size: size, Containers: 0 });
skiff.on({ type: "lists", containers: [], networks: [],
  images: [img("bk", ["moby/buildkit:buildx-stable-1"], 112e6), img("ng", ["nginx:latest"]), img("bk2", ["moby/buildkit:v0.24.0"])],
  volumes: [{ Name: "buildx_buildkit_skiff-build0_state", Driver: "local" }, { Name: "pgdata", Driver: "local" }] });
t("Skiff's builder image isn't in Images; nginx and another BuildKit tag are", () => {
  eq(ui.images.map((i) => i.RepoTags[0]), ["nginx:latest", "moby/buildkit:v0.24.0"], "images");
});
t("its cache volume isn't in Volumes", () => eq(ui.volumes.map((v) => v.Name), ["pgdata"], "volumes"));
t("Settings → Builds shows the builder and Clear build cache", () => {
  const html = builderPref();
  if (!/Skiff's builder/.test(html) || !/data-buildprune/.test(html)) throw new Error(html);
});

skiff.on({ type: "lists", containers: [], networks: [], images: [img("bk", ["moby/buildkit:buildx-stable-1"])], volumes: [] });
show("images");
t("Images counts nothing and offers no download when only the builder is there", () => {
  eq([ui.images.length, !!document.getElementById("empty-pull"), /docker pull alpine/.test(document.getElementById("content").textContent)], [0, false, true], "empty");
});
skiff.on({ type: "lists", containers: [], networks: [], images: [], volumes: [] });
t("before any build, Settings → Builds says the builder isn't made yet", () => {
  if (!/Not set up yet/.test(builderPref())) throw new Error(builderPref());
});
report(R);
