import {
  normalizeDomain,
  releaseDownload,
  recipes,
  previewBuilds,
} from "./model.js";

const $ = (selector) => document.querySelector(selector);
let selectedRecipe = "run";
let selectedPlatform = "mac";
let currentDomain = "example.com";
let releases = [];
let releaseCheckFailed = false;
let toastTimer;

function toast(message) {
  $("#toast").textContent = message;
  $("#toast").classList.add("visible");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => $("#toast").classList.remove("visible"), 3500);
}

function tabs(selector, onSelect) {
  const buttons = [...document.querySelectorAll(selector)];
  const select = (button) => {
    for (const item of buttons) {
      const active = item === button;
      item.setAttribute("aria-selected", String(active));
      item.tabIndex = active ? 0 : -1;
    }
    onSelect(button);
  };
  for (const [index, button] of buttons.entries()) {
    button.addEventListener("click", () => select(button));
    button.addEventListener("keydown", (event) => {
      const vertical =
        button.parentElement.getAttribute("aria-orientation") === "vertical";
      const next = vertical ? "ArrowDown" : "ArrowRight";
      const previous = vertical ? "ArrowUp" : "ArrowLeft";
      let target;
      if (event.key === next) target = buttons[(index + 1) % buttons.length];
      if (event.key === previous)
        target = buttons[(index - 1 + buttons.length) % buttons.length];
      if (event.key === "Home") target = buttons[0];
      if (event.key === "End") target = buttons.at(-1);
      if (target) {
        event.preventDefault();
        select(target);
        target.focus();
      }
    });
  }
  return select;
}

const selectDemo = tabs("[data-demo]", (button) => {
  for (const panel of document.querySelectorAll(".demo-panel"))
    panel.hidden = panel.id !== button.getAttribute("aria-controls");
});
$(".share-trigger").addEventListener("click", () => {
  const button = $("#demo-tab-shares");
  selectDemo(button);
  button.focus();
});

function renderRecipe() {
  const recipe = recipes(currentDomain)[selectedRecipe];
  $("#recipe-panel").setAttribute(
    "aria-labelledby",
    `recipe-tab-${selectedRecipe}`,
  );
  $("#recipe-comment").textContent = recipe.comment;
  $("#recipe-code").textContent = recipe.code;
  $("#recipe-result").textContent = recipe.result;
}
const recipeLayout = matchMedia("(max-width: 800px)");
function updateRecipeOrientation() {
  $(".recipe-tabs").setAttribute(
    "aria-orientation",
    recipeLayout.matches ? "horizontal" : "vertical",
  );
}
updateRecipeOrientation();
recipeLayout.addEventListener("change", updateRecipeOrientation);
tabs("[data-recipe]", (button) => {
  selectedRecipe = button.dataset.recipe;
  renderRecipe();
});
$("#copy-command").addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText($("#recipe-code").textContent);
    toast("Commands copied. Your terminal awaits.");
  } catch {
    const range = document.createRange();
    range.selectNodeContents($("#recipe-code"));
    const selection = window.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
    toast("Commands selected. Press ⌘C or Ctrl+C to copy.");
  }
});

$("#domain-form").addEventListener("submit", (event) => {
  event.preventDefault();
  const domain = normalizeDomain($("#base-domain").value);
  $("#base-domain").setAttribute("aria-invalid", String(!domain));
  $("#domain-error").textContent = domain
    ? ""
    : "Enter a domain such as example.com, without https:// or a path.";
  if (!domain) {
    $("#base-domain").focus();
    return;
  }
  currentDomain = domain;
  $("#base-domain").value = domain;
  $("#domain-preview").textContent = `preview.${domain}`;
  renderRecipe();
  toast(`Preview updated: preview.${domain}`);
});

const platforms = {
  mac: {
    title: "Skiff for macOS",
    description:
      "Apple’s Virtualization.framework, a native desktop window, and a home in your menu bar.",
    requirement: "macOS 14 or later · Apple Silicon and Intel",
    arches: [
      ["arm64", "Apple Silicon (M-series)"],
      ["x86_64", "Intel"],
    ],
    notice:
      "macOS releases are built with Developer ID signing and notarization. Check the release notes before installing.",
  },
  windows: {
    title: "Skiff for Windows",
    description:
      "A native Windows desktop app with a WSL2 engine and the tools you already know.",
    requirement: "Windows 10 or 11 · WSL2 required · x64 and ARM64",
    arches: [
      ["x86_64", "Intel / AMD (x64)"],
      ["arm64", "Windows on ARM"],
    ],
    notice:
      "Windows installers are currently unsigned. Windows may show a publisher warning. Review the release notes before installing.",
  },
  linux: {
    title: "Skiff for Linux",
    description:
      "Containers on your own kernel, with a GTK desktop app and a familiar command line.",
    requirement: "Linux · GTK 3 + WebKitGTK 4.1 · Debian package",
    arches: [
      ["x86_64", "Intel / AMD (x64)"],
      ["arm64", "ARM64"],
    ],
    notice:
      "This download is a Debian package. RPM packages and setup requirements are available in the release notes.",
  },
};
function renderDownload() {
  const platform = platforms[selectedPlatform];
  const asset = releaseDownload(
    releases,
    selectedPlatform,
    $("#architecture").value,
  );
  $("#release-status").textContent = asset
    ? `RELEASE v${asset.version}`
    : "DEVELOPMENT PREVIEW";
  $("#download-link").href = asset?.url || previewBuilds;
  $("#download-link").replaceChildren(
    document.createTextNode(
      asset
        ? `Download ${selectedPlatform === "linux" ? ".deb" : selectedPlatform === "mac" ? ".dmg" : ".msi"}`
        : "Browse preview builds",
    ),
  );
  $("#download-notice").textContent = asset
    ? platform.notice
    : releaseCheckFailed
      ? "We couldn’t check the latest release. View all releases on GitHub, or browse preview builds (GitHub login required). Preview builds are for evaluation."
      : "No published installer is available here yet for this selection. Browse preview builds through GitHub Actions (GitHub login required). Preview builds are for evaluation.";
}
function renderPlatform() {
  const platform = platforms[selectedPlatform];
  $("#platform-panel").setAttribute(
    "aria-labelledby",
    `platform-tab-${selectedPlatform}`,
  );
  $("#platform-title").textContent = platform.title;
  $("#platform-description").textContent = platform.description;
  $("#platform-requirement").textContent = platform.requirement;
  $("#architecture").replaceChildren(
    ...platform.arches.map(([value, label]) => new Option(label, value)),
  );
  renderDownload();
}
const selectPlatform = tabs("[data-platform]", (button) => {
  selectedPlatform = button.dataset.platform;
  renderPlatform();
});
$("#architecture").addEventListener("change", renderDownload);
const os = navigator.userAgentData?.platform || navigator.platform || "";
const detectedPlatform = /win/i.test(os)
  ? "windows"
  : /linux/i.test(os)
    ? "linux"
    : "mac";
selectPlatform($(`[data-platform="${detectedPlatform}"]`));

async function loadReleases() {
  try {
    const response = await fetch(
      "https://api.github.com/repos/shebashio/skiff/releases?per_page=5",
      {
        signal: AbortSignal.timeout(5000),
        credentials: "omit",
        referrerPolicy: "no-referrer",
      },
    );
    if (!response.ok) throw new Error("Release lookup unavailable");
    const data = await response.json();
    if (!Array.isArray(data)) throw new Error("Invalid release data");
    releases = data;
  } catch {
    releaseCheckFailed = true;
  }
  renderDownload();
}
// Defer the optional release lookup until the download section approaches the viewport.
const releaseObserver = new IntersectionObserver(
  (entries) => {
    if (entries.some((entry) => entry.isIntersecting)) {
      releaseObserver.disconnect();
      loadReleases();
    }
  },
  { rootMargin: "400px" },
);
releaseObserver.observe($("#download"));

const menu = $(".menu-toggle");
function closeMenu() {
  menu.setAttribute("aria-expanded", "false");
  menu.setAttribute("aria-label", "Open navigation");
  $("#nav-links").classList.remove("open");
}
menu.addEventListener("click", () => {
  const open = menu.getAttribute("aria-expanded") !== "true";
  menu.setAttribute("aria-expanded", String(open));
  menu.setAttribute(
    "aria-label",
    open ? "Close navigation" : "Open navigation",
  );
  $("#nav-links").classList.toggle("open", open);
});
$("#nav-links").addEventListener("click", (event) => {
  if (event.target.closest("a")) closeMenu();
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && menu.getAttribute("aria-expanded") === "true") {
    closeMenu();
    menu.focus();
  }
});
$("#privacy-button").addEventListener("click", () =>
  $("#privacy-dialog").showModal(),
);
$(".dialog-close").addEventListener("click", () =>
  $("#privacy-dialog").close(),
);
