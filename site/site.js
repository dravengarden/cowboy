const root = document.documentElement;
const header = document.querySelector(".site-header");
const themeColor = document.querySelector('meta[name="theme-color"]');
const navToggle = document.querySelector(".nav-toggle");
const navigation = document.querySelector(".site-navigation");
const preferencesControl = document.querySelector(".preferences-control");
const preferencesToggle = document.querySelector(".preferences-toggle");
const preferencesPopover = document.querySelector(".preferences-popover");
const backToTop = document.querySelector(".back-to-top");
const themeChoices = [...document.querySelectorAll("[data-theme-choice]")];
const languageChoices = [
  ...document.querySelectorAll("[data-language-choice]"),
];
const colorSchemeQuery = globalThis.matchMedia("(prefers-color-scheme: dark)");

let themeMode = ["system", "light", "dark"].includes(root.dataset.themeMode)
  ? root.dataset.themeMode
  : "system";

function resolvedTheme(mode) {
  if (mode === "system") return colorSchemeQuery.matches ? "dark" : "light";
  return mode;
}

function setThemeMode(mode, { persist = true } = {}) {
  if (!["system", "light", "dark"].includes(mode)) return;
  themeMode = mode;
  const theme = resolvedTheme(mode);
  const canvas = theme === "dark" ? "rgb(21, 17, 29)" : "rgb(246, 244, 251)";
  root.dataset.theme = theme;
  root.dataset.themeMode = mode;
  root.style.backgroundColor = canvas;
  document.body?.style.setProperty("background-color", canvas);
  themeColor?.setAttribute("content", canvas);
  themeChoices.forEach((choice) => {
    choice.setAttribute(
      "aria-pressed",
      String(choice.getAttribute("data-theme-choice") === mode),
    );
  });
  if (!persist) return;
  try {
    localStorage.setItem("cowboy-site-theme", mode);
  } catch (_) {
    // A theme choice can remain session-local when storage is unavailable.
  }
}

setThemeMode(themeMode, { persist: false });

themeChoices.forEach((choice) => {
  choice.addEventListener("click", () => {
    setThemeMode(choice.getAttribute("data-theme-choice") ?? "system");
  });
});

colorSchemeQuery.addEventListener?.("change", () => {
  if (themeMode === "system") setThemeMode("system", { persist: false });
});

const zhMessages = {
  "meta.title": "Cowboy — 远程 Agent IDE",
  "meta.description":
    "Cowboy 是自托管远程 Agent IDE，可从一个控制平面安全调度你的多台机器上的 Codex、Claude Code、Gemini、Grok 与 DeepSeek runtime。",
  "meta.socialDescription":
    "在一个自托管控制平面中管理你的 Agent 与 Machine，由独立版本、签名并固定的插件驱动。",
  "meta.twitterDescription":
    "自托管一个安全控制平面，调度你的多台 Machine 上的所有编码 Agent。",
  "skip.content": "跳到正文",
  "brand.home": "Cowboy 首页",
  "brand.github": "在 GitHub 上查看 Cowboy",
  "nav.primary": "主导航",
  "nav.product": "产品",
  "nav.safety": "安全",
  "nav.plugins": "插件",
  "nav.architecture": "架构",
  "nav.start": "开始使用",
  "controls.site": "网站控制",
  "preferences.title": "偏好设置",
  "preferences.subtitle": "主题与语言",
  "preferences.appearance": "外观",
  "preferences.system": "跟随系统",
  "preferences.light": "浅色",
  "preferences.dark": "深色",
  "preferences.language": "语言",
  "fleet.aria":
    "实时拓扑：Desktop 与 Mobile 通过一个 Cowboy Hub 连接三台 Machine 和六个 Agent；每台 Machine 同时运行两个 Agent",
  "fleet.live": "实时拓扑",
  "fleet.clients": "客户端",
  "fleet.machines": "Machines",
  "fleet.agents": "Agents",
  "fleet.controlPlane": "Cowboy Hub",
  "fleet.yourPlugin": "你的插件",
  "hero.eyebrow": "自托管远程 Agent IDE",
  "hero.agents": "你的 Agent。",
  "hero.machines": "你的机器。",
  "hero.workspace": "一个工作区。",
  "hero.hosted": "由你托管。",
  "hero.lede":
    "在你的基础设施上自托管 Cowboy，调度每台已加入 Machine 上的 Codex、Claude Code、Gemini、Grok 与 DeepSeek；稍后重新连接时，worker 仍持续运行在原 Machine 上。",
  "hero.ownership": "你的控制平面。你的数据。你的凭据。永远属于你。",
  "hero.run": "运行 Cowboy",
  "hero.explore": "探索插件",
  "hero.signals": "Cowboy 平台特性",
  "hero.signal.acp": "ACP 原生",
  "hero.signal.plugins": "已签名 + 已固定插件",
  "hero.signal.safety": "故障关闭安全",
  "hero.signal.platforms": "Linux + macOS Machines",
  "hero.imageAlt":
    "由插件 Session、已填写 Prompt 与 Agent 活动组成的 Cowboy Desktop 和 Phone 抽象界面",
  "facts.aria": "Cowboy 远程控制与安全数据",
  "facts.controlPlane": "自托管实例",
  "facts.fanout": "Machine 扇出",
  "facts.providers": "Agent 提供方",
  "facts.verification": "发布校验",
  "safety.eyebrow": "安全内建",
  "safety.title.remote": "远程控制。",
  "safety.title.local": "本地边界。",
  "safety.signed": "签名并固定",
  "safety.signedBody":
    "发布者、契约、插件包与 runtime 工件共同解析为同一个不可变发布。",
  "safety.scoped": "以 Machine 为边界",
  "safety.scopedBody":
    "在一台 Machine 上安装 Provider，不会静默更改其他 Machine。",
  "safety.closed": "故障关闭",
  "safety.closedBody":
    "不兼容或被篡改的字节永远不会替换当前启用的 generation。",
  "product.eyebrow": "一个控制平面，两种专用界面",
  "product.title.near": "工作运行在哪里，",
  "product.title.where": "你都在现场。",
  "product.intro":
    "即使浏览器关闭，Cowboy 仍让 Machine 上的 worker 持续运行。之后可从桌面或手机重新连接到同一 Session、状态与 worktree。",
  "desktop.label": "01 / Desktop",
  "desktop.title": "键盘优先，一切可见。",
  "desktop.body":
    "密集的 Session 导航、分屏工作区、Vim motion、可见命令和实时工具输出，让 Desktop 成为真正的 IDE，而不是拉伸后的移动布局。",
  "desktop.keyboardAria": "在 Cowboy Desktop 的每个界面中使用类 Vim 键盘控制",
  "desktop.keyboard": "键盘控制一切",
  "desktop.keyboardDetail": "类 Vim · 所有界面均可寻址",
  "desktop.imageAlt": "三栏 Cowboy Desktop 抽象界面",
  "mobile.label": "02 / Mobile",
  "mobile.title": "触控优先，同一份运行中的工作。",
  "mobile.body":
    "聚焦的 Agent 视图、原生选择与粘贴、大尺寸触控目标、安全的键盘行为，以及渐进展开的代码工具。",
  "mobile.imageAlt": "Cowboy Mobile 的 Sessions、Agent 对话与代码智能抽象界面",
  "capabilities.aria": "Cowboy 核心能力",
  "capabilities.machines": "Machines",
  "capabilities.machinesBody": "把工作放到合适的主机",
  "capabilities.agent": "Agent",
  "capabilities.agentBody": "计划、工具、Prompt、权限",
  "capabilities.code": "代码",
  "capabilities.codeBody": "Diff、符号、诊断、Git",
  "capabilities.sessions": "Sessions",
  "capabilities.sessionsBody": "暂停、恢复、重连、排序",
  "ecosystem.eyebrow": "插件生态",
  "ecosystem.title.main": "插件不是附加项。",
  "ecosystem.title.emphasis": "它们就是产品边界。",
  "ecosystem.intro":
    "每个集成都通过同一套通用生命周期独立版本化、打包、签名、发布、安装、升级、回滚与移除。Cowboy 从类型化契约中读取能力，而不是硬编码供应商界面。",
  "plugins.filterAria": "筛选插件",
  "plugins.filter.all": () =>
    `全部 ${String(document.querySelectorAll(".plugin-card").length)}`,
  "plugins.filter.agent": "Agent 提供方",
  "plugins.filter.code": "代码智能",
  "plugins.browse": "查看源码",
  "plugins.kind.agent": "Agent 提供方",
  "plugins.kind.code": "代码智能",
  "plugins.component": "组件",
  "plugins.summary.codex": "采用标准 OpenAI 账户与配置的 Codex",
  "plugins.summary.claude-code": "采用标准 Anthropic 账户与配置的 Claude Code",
  "plugins.summary.gemini": "支持 Google、API key 或 Vertex 认证的 Gemini CLI",
  "plugins.summary.grok": "使用标准 xAI 认证的官方 Grok Build Agent",
  "plugins.summary.codex-deepseek":
    "运行于隔离 DeepSeek runtime、支持 V4 Flash 与 V4 Pro 的 Codex",
  "plugins.summary.claude-deepseek":
    "运行于隔离 DeepSeek runtime、提供百万 token 通道的 Claude Code",
  "plugins.summary.zed":
    "为每个已连接 worktree 提供进程隔离的符号、悬停、定义与引用。",
  "lifecycle.eyebrow": "已签名、已固定、故障关闭",
  "lifecycle.title": "统一生命周期，没有移动目标。",
  "lifecycle.intro":
    "构建、Catalog 接入与目标 Machine 校验同一个不可变发布。不兼容的字节会故障关闭，当前 generation 继续运行。",
  "lifecycle.aria": "Cowboy 插件生命周期",
  "lifecycle.build": "构建",
  "lifecycle.buildBefore": "类型化 manifest 与 payload 组成一个纯数据",
  "lifecycle.period": "。",
  "lifecycle.sign": "签名",
  "lifecycle.signBody": "发布者身份、契约指纹与每个 runtime 工件被绑定在一起。",
  "lifecycle.publish": "发布",
  "lifecycle.publishBody": "Catalog 暴露不可变发布，而不会静默安装。",
  "lifecycle.install": "安装",
  "lifecycle.installBody": "选定的 Machine 以原子方式启用完整 generation。",
  "lifecycle.run": "运行",
  "lifecycle.runBody": "在明确变更前，Session 始终固定到确切的已安装发布。",
  "manifest.aria": "Cowboy 插件 manifest 示例",
  "contract.typed": "类型化能力",
  "contract.typedTitle": "让插件描述自己。",
  "contract.typedBody":
    "Provider 标识、设置、认证、模型控制、活动、Transcript 风格与主机行为全部通过已验证的数据契约传递。",
  "contract.shared": "共享组件",
  "contract.sharedTitle": "复用实现，保持独立发布。",
  "contract.sharedBody":
    "插件固定到精确的 SDK、UI、runtime 与代码智能组件。组件可以独立演进，而不会把整个生态变成一个单体。",
  "contract.isolation": "Machine 隔离",
  "contract.isolationTitle": "安装到工作真正运行的地方。",
  "contract.isolationBody":
    "Hub 把 Catalog 发布与每台 Machine 的平台和契约连接起来。凭据与 runtime 状态留在各自的所有权边界。",
  "topology.eyebrow": "一个远程控制平面",
  "topology.title.one": "一个地方调度",
  "topology.title.every": "所有 Machine。",
  "topology.intro":
    "Desktop 与 Mobile 汇入同一个自托管 Cowboy Hub。Hub 在已加入的 Machines 之间路由持久 Session，让你无需切换 IDE 或 SSH tab 即可切换主机，并重连同一个不会被移动或停止的 worker。",
  "topology.read": "阅读架构",
  "topology.aria": "Cowboy 系统拓扑",
  "topology.keyboard": "键盘",
  "topology.touch": "触控",
  "topology.controlPlane": "一个控制平面",
  "topology.connected": "已连接",
  "topology.yourPlugins": "Grok · 你的插件",
  "start.eyebrow": "开源，随时可构建",
  "start.title": "让每个 Agent 触手可及。",
  "start.intro":
    "Cowboy 在固定版本的 Nix shell 中开发，可使用内存、SQLite 或 PostgreSQL 存储。先在本地启动，再加入实际工作所在的 Machines。",
  "start.github": "在 GitHub 查看",
  "start.plugin": "构建插件",
  "start.quick": "cowboy — 快速开始",
  "start.copy": "复制",
  "footer.tagline": "为持续流动的工作而生的远程 Agent IDE。",
  "footer.product": "产品",
  "footer.desktop": "Desktop + Mobile",
  "footer.ecosystem": "插件生态",
  "footer.architecture": "架构",
  "footer.developers": "开发者",
  "footer.source": "源码",
  "footer.docs": "文档",
  "footer.contract": "平台契约",
  "footer.top": "返回顶部",
  "footer.license": "MIT 许可 · 基于 ACP 构建",
};

const originalText = new WeakMap();
const originalAttributes = new WeakMap();

function translatedValue(key, language) {
  if (language !== "zh") return null;
  const value = zhMessages[key];
  return typeof value === "function" ? value() : value ?? null;
}

function applyTranslations(language) {
  document.querySelectorAll("[data-i18n]").forEach((node) => {
    if (!originalText.has(node)) {
      originalText.set(node, node.textContent.trim());
    }
    const translated = translatedValue(node.dataset.i18n, language);
    node.textContent = translated ?? originalText.get(node);
  });

  for (const attribute of ["aria-label", "alt", "content"]) {
    document.querySelectorAll(`[data-i18n-${attribute}]`).forEach((node) => {
      let originals = originalAttributes.get(node);
      if (!originals) {
        originals = {};
        originalAttributes.set(node, originals);
      }
      if (!(attribute in originals)) {
        originals[attribute] = node.getAttribute(attribute) ?? "";
      }
      const key = node.getAttribute(`data-i18n-${attribute}`);
      node.setAttribute(
        attribute,
        translatedValue(key, language) ?? originals[attribute],
      );
    });
  }
}

const runtimeMessages = {
  en: {
    openNavigation: "Open navigation",
    closeNavigation: "Close navigation",
    openPreferences: "Open preferences",
    closePreferences: "Close preferences",
    pluginsShown: (count) => `${String(count)} plugins shown`,
    copy: "Copy",
    copied: "Copied",
  },
  zh: {
    openNavigation: "打开导航",
    closeNavigation: "关闭导航",
    openPreferences: "打开偏好设置",
    closePreferences: "关闭偏好设置",
    pluginsShown: (count) => `显示 ${String(count)} 个插件`,
    copy: "复制",
    copied: "已复制",
  },
};

let language = root.dataset.language === "zh" ? "zh" : "en";

function refreshControlLabels() {
  const messages = runtimeMessages[language];
  const navigationOpen = navToggle?.getAttribute("aria-expanded") === "true";
  const preferencesOpen =
    preferencesToggle?.getAttribute("aria-expanded") === "true";
  navToggle?.setAttribute(
    "aria-label",
    navigationOpen ? messages.closeNavigation : messages.openNavigation,
  );
  preferencesToggle?.setAttribute(
    "aria-label",
    preferencesOpen ? messages.closePreferences : messages.openPreferences,
  );
}

function setLanguage(nextLanguage, { persist = true } = {}) {
  language = nextLanguage === "zh" ? "zh" : "en";
  root.dataset.language = language;
  root.lang = language === "zh" ? "zh-CN" : "en";
  applyTranslations(language);
  languageChoices.forEach((choice) => {
    choice.setAttribute(
      "aria-pressed",
      String(choice.getAttribute("data-language-choice") === language),
    );
  });
  refreshControlLabels();
  const filterStatus = document.querySelector("#plugin-filter-status");
  const visibleCount = Number(filterStatus?.dataset.visibleCount);
  if (filterStatus && Number.isFinite(visibleCount)) {
    filterStatus.textContent = runtimeMessages[language].pluginsShown(
      visibleCount,
    );
  }
  if (!persist) return;
  try {
    localStorage.setItem("cowboy-site-language", language);
  } catch (_) {
    // A language choice can remain session-local when storage is unavailable.
  }
}

setLanguage(language, { persist: false });

languageChoices.forEach((choice) => {
  choice.addEventListener("click", () => {
    setLanguage(choice.getAttribute("data-language-choice") ?? "en");
  });
});

function closeNavigation() {
  navToggle?.setAttribute("aria-expanded", "false");
  navigation?.classList.remove("is-open");
  refreshControlLabels();
  updateBackToTop();
}

function closePreferences({ restoreFocus = false } = {}) {
  preferencesToggle?.setAttribute("aria-expanded", "false");
  preferencesPopover?.setAttribute("hidden", "");
  refreshControlLabels();
  updateBackToTop();
  if (restoreFocus) preferencesToggle?.focus();
}

navToggle?.addEventListener("click", () => {
  const opening = navToggle.getAttribute("aria-expanded") !== "true";
  if (opening) closePreferences();
  navToggle.setAttribute("aria-expanded", String(opening));
  navigation?.classList.toggle("is-open", opening);
  refreshControlLabels();
  updateBackToTop();
});

preferencesToggle?.addEventListener("click", () => {
  const opening = preferencesToggle.getAttribute("aria-expanded") !== "true";
  if (opening) closeNavigation();
  preferencesToggle.setAttribute("aria-expanded", String(opening));
  preferencesPopover?.toggleAttribute("hidden", !opening);
  refreshControlLabels();
  updateBackToTop();
});

navigation?.querySelectorAll("a").forEach((link) => {
  link.addEventListener("click", closeNavigation);
});

document.addEventListener("keydown", (event) => {
  if (event.key !== "Escape") return;
  const preferencesOpen =
    preferencesToggle?.getAttribute("aria-expanded") === "true";
  closeNavigation();
  closePreferences({ restoreFocus: preferencesOpen });
});

document.addEventListener("click", (event) => {
  const target = event.target;
  if (!(target instanceof Node)) return;
  if (!navigation?.contains(target) && !navToggle?.contains(target)) {
    closeNavigation();
  }
  if (!preferencesControl?.contains(target)) closePreferences();
});

function updateHeader() {
  header?.classList.toggle("is-scrolled", globalThis.scrollY > 8);
}

function updateBackToTop() {
  if (!backToTop) return;
  const controlsOpen = navToggle?.getAttribute("aria-expanded") === "true" ||
    preferencesToggle?.getAttribute("aria-expanded") === "true";
  const threshold = Math.max(520, globalThis.innerHeight * 0.65);
  const visible = globalThis.scrollY > threshold && !controlsOpen;
  backToTop.classList.toggle("is-visible", visible);
  backToTop.setAttribute("aria-hidden", String(!visible));
  backToTop.tabIndex = visible ? 0 : -1;
}

function updateViewportChrome() {
  updateHeader();
  updateBackToTop();
}

updateViewportChrome();
globalThis.addEventListener("scroll", updateViewportChrome, { passive: true });
globalThis.addEventListener("resize", updateBackToTop, { passive: true });

backToTop?.addEventListener("click", (event) => {
  event.preventDefault();
  const reducedMotion = globalThis.matchMedia?.(
    "(prefers-reduced-motion: reduce)",
  ).matches ?? false;
  globalThis.scrollTo({
    top: 0,
    left: 0,
    behavior: reducedMotion ? "auto" : "smooth",
  });
});

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
      filterStatus.dataset.visibleCount = String(visible);
      filterStatus.textContent = runtimeMessages[language].pluginsShown(
        visible,
      );
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
    copyButton.textContent = runtimeMessages[language].copied;
    globalThis.setTimeout(() => {
      copyButton.textContent = runtimeMessages[language].copy;
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
