// Container links in TestFlight must use the Rust host's accepted localhost form.
const R = [];
const t = (name, fn) => { try { fn(); R.push("PASS " + name); } catch (e) { R.push("FAIL " + name + ": " + e.message); } };
ui.status = { ...ui.status, sandboxed: true };
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
report(R);
