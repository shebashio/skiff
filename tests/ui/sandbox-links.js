// Container links in TestFlight must use the Rust host's accepted localhost form.
const R = [];
const t = (name, fn) => { try { fn(); R.push("PASS " + name); } catch (e) { R.push("FAIL " + name + ": " + e.message); } };
skiff.on({ type: "status", ...ui.status, sandboxed: true });
t("sandbox links select the published TCP port", () => {
  const url = localUrl({ Ports: [{ Type: "udp", PublicPort: 53 }, { Type: "tcp", PublicPort: 8080 }] }, "web");
  if (url !== "http://localhost:8080") throw new Error(url);
  document.body.insertAdjacentHTML("beforeend", localChip(url));
  sent.length = 0;
  document.querySelector('[data-open="http://localhost:8080"]').click();
  if (!sent.some(m => m.type === "open" && m.name === url)) throw new Error(JSON.stringify(sent));
});
t("UDP-only or unpublished containers have no browser link", () => {
  for (const c of [{}, { Ports: [{ Type: "udp", PublicPort: 53 }] }, { Ports: [{ Type: "tcp", PrivatePort: 80 }] }]) {
    if (localUrl(c, "web") !== "") throw new Error("unexpected link");
  }
});
t("sandbox hides managed Kubernetes and does not send cluster requests", () => {
  if (!$("k8s-label").hidden || !$("k8s-nav").hidden) throw new Error("Kubernetes navigation is visible");
  sent.length = 0;
  show("pods");
  if (ui.view !== "containers") throw new Error("entered unavailable Pods view");
  prefs.tab = "kubernetes";
  show("settings");
  if (document.querySelector('[data-stab="kubernetes"]')) throw new Error("Kubernetes settings tab is visible");
  k8sAsk(true);
  if (sent.some(m => m.type === "k8s")) throw new Error("sent Kubernetes request");
});
t("keyboard copy still sends selected log text to the clipboard handler", () => {
  const text = document.createElement("pre");
  text.textContent = "selected container log";
  text.style.cssText = "position:fixed;top:0;left:0;user-select:text";
  document.body.append(text);
  const range = document.createRange(); range.selectNodeContents(text);
  getSelection().removeAllRanges(); getSelection().addRange(range);
  sent.length = 0;
  text.dispatchEvent(new KeyboardEvent("keydown", { key: "c", metaKey: !WIN && !LINUX, ctrlKey: WIN || LINUX, bubbles: true }));
  if (!sent.some(m => m.type === "term" && m.name === "copy" && m.value === text.textContent)) throw new Error(JSON.stringify({ selection: String(getSelection()), sent }));
  getSelection().removeAllRanges(); text.remove();
});
report(R);
