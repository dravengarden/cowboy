const root = document.documentElement;
const header = document.querySelector(".site-header");
const themeToggle = document.querySelector(".theme-toggle");
const themeColor = document.querySelector('meta[name="theme-color"]');
const navToggle = document.querySelector(".nav-toggle");
const navigation = document.querySelector(".site-navigation");

function setTheme(theme) {
  root.dataset.theme = theme;
  themeToggle?.setAttribute(
    "aria-label",
    theme === "dark" ? "Use light theme" : "Use dark theme",
  );
  themeColor?.setAttribute("content", theme === "dark" ? "#15111d" : "#f6f4fb");
  try {
    localStorage.setItem("cowboy-site-theme", theme);
  } catch (_) {
    // A theme choice can remain session-local when storage is unavailable.
  }
}

setTheme(root.dataset.theme === "dark" ? "dark" : "light");

themeToggle?.addEventListener("click", () => {
  setTheme(root.dataset.theme === "dark" ? "light" : "dark");
});

function closeNavigation() {
  navToggle?.setAttribute("aria-expanded", "false");
  navToggle?.setAttribute("aria-label", "Open navigation");
  navigation?.classList.remove("is-open");
}

navToggle?.addEventListener("click", () => {
  const opening = navToggle.getAttribute("aria-expanded") !== "true";
  navToggle.setAttribute("aria-expanded", String(opening));
  navToggle.setAttribute(
    "aria-label",
    opening ? "Close navigation" : "Open navigation",
  );
  navigation?.classList.toggle("is-open", opening);
});

navigation?.querySelectorAll("a").forEach((link) => {
  link.addEventListener("click", closeNavigation);
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") closeNavigation();
});

document.addEventListener("click", (event) => {
  const target = event.target;
  if (!(target instanceof Node)) return;
  if (!navigation?.contains(target) && !navToggle?.contains(target)) {
    closeNavigation();
  }
});

function updateHeader() {
  header?.classList.toggle("is-scrolled", globalThis.scrollY > 8);
}

updateHeader();
globalThis.addEventListener("scroll", updateHeader, { passive: true });

const filterButtons = [...document.querySelectorAll("[data-filter]")];
const pluginCards = [...document.querySelectorAll(".plugin-card")];
const filterStatus = document.querySelector("#plugin-filter-status");

filterButtons.forEach((button) => {
  button.addEventListener("click", () => {
    const filter = button.getAttribute("data-filter") ?? "all";
    let visible = 0;

    filterButtons.forEach((candidate) => {
      candidate.setAttribute("aria-pressed", String(candidate === button));
    });
    pluginCards.forEach((card) => {
      const show = filter === "all" ||
        card.getAttribute("data-kind") === filter;
      card.toggleAttribute("hidden", !show);
      if (show) visible += 1;
    });
    if (filterStatus) {
      filterStatus.textContent = `${String(visible)} plugins shown`;
    }
  });
});

const copyButton = document.querySelector("[data-copy-target]");

async function copyQuickStart() {
  const targetId = copyButton?.getAttribute("data-copy-target");
  const source = targetId ? document.getElementById(targetId) : null;
  const text = source?.innerText ?? "";
  if (!text) return;

  try {
    await navigator.clipboard.writeText(text);
  } catch (_) {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.append(textarea);
    textarea.select();
    document.execCommand("copy");
    textarea.remove();
  }

  if (copyButton) {
    copyButton.textContent = "Copied";
    globalThis.setTimeout(() => {
      copyButton.textContent = "Copy";
    }, 1600);
  }
}

copyButton?.addEventListener("click", () => void copyQuickStart());

document.querySelectorAll("[data-current-year]").forEach((node) => {
  node.textContent = String(new Date().getFullYear());
});

const revealItems = [...document.querySelectorAll(".reveal")];
const reducedMotion = globalThis.matchMedia?.(
  "(prefers-reduced-motion: reduce)",
).matches;

if (!("IntersectionObserver" in globalThis) || reducedMotion) {
  revealItems.forEach((item) => item.classList.add("is-visible"));
} else {
  const observer = new IntersectionObserver(
    (entries) => {
      entries.forEach((entry) => {
        if (!entry.isIntersecting) return;
        entry.target.classList.add("is-visible");
        observer.unobserve(entry.target);
      });
    },
    { rootMargin: "0px 0px -7%", threshold: 0.08 },
  );
  revealItems.forEach((item) => observer.observe(item));
}
