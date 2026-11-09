// The app realm (docs/widget-apps.md §3, §5.2). Runs inside the QuickJS VM:
// no browser API exists here. The trusted Worker provides exactly three host
// functions — __emit(json), __send(text), __log(text), __fail(text) — and calls the
// __cowboy* entry points below under its CPU budget.
import "./vmTimers.js";
import * as React from "react";
import Reconciler from "react-reconciler";
import { transform } from "sucrase";

import { ELEMENT_NAMES as ELEMENTS } from "./limits.js";
const SYNC = 2;

let nextId = 1;
let ops = [];
const handlers = new Map();
// Host children per instance, so a removed subtree releases every handler.
const kids = new Map();
const attach = (parent, child) => {
  for (const set of kids.values()) set.delete(child);
  if (!kids.has(parent)) kids.set(parent, new Set());
  kids.get(parent).add(child);
};
const flush = () => {
  if (ops.length > 0) {
    __emit(JSON.stringify(ops));
    ops = [];
  }
};
const split = (id, props) => {
  const plain = {};
  const named = [];
  for (const [key, value] of Object.entries(props)) {
    if (key === "children" || key === "key" || key === "ref") continue;
    if (typeof value === "function") {
      named.push(key);
      handlers.set(`${id}:${key}`, value);
    } else if (value !== undefined) {
      plain[key] = value;
    }
  }
  if (named.length > 0) plain.__handlers = named;
  return plain;
};
const forget = (id) => {
  for (const key of handlers.keys()) if (key.startsWith(`${id}:`)) handlers.delete(key);
};
const forgetTree = (id) => {
  for (const child of kids.get(id) ?? []) forgetTree(child);
  kids.delete(id);
  for (const set of kids.values()) set.delete(id);
  forget(id);
};

const hostConfig = {
  supportsMutation: true,
  isPrimaryRenderer: false,
  noTimeout: -1,
  supportsMicrotasks: true,
  scheduleMicrotask: (fn) => void Promise.resolve().then(fn),
  scheduleTimeout: (fn) => setTimeout(fn),
  cancelTimeout: () => undefined,
  getRootHostContext: () => ({}),
  getChildHostContext: (context) => context,
  shouldSetTextContent: () => false,
  createInstance(type, props) {
    if (!ELEMENTS.includes(type)) throw new Error(`Unknown component <${type}>; use @cowboy/ui`);
    const id = nextId++;
    ops.push(["create", id, type, split(id, props)]);
    return { id };
  },
  createTextInstance(text) {
    const id = nextId++;
    ops.push(["text", id, String(text)]);
    return { id };
  },
  appendInitialChild: (parent, child) => {
    attach(parent.id, child.id);
    ops.push(["append", parent.id, child.id]);
  },
  appendChild: (parent, child) => {
    attach(parent.id, child.id);
    ops.push(["append", parent.id, child.id]);
  },
  appendChildToContainer: (_root, child) => {
    attach(0, child.id);
    ops.push(["append", 0, child.id]);
  },
  insertBefore: (parent, child, before) => {
    attach(parent.id, child.id);
    ops.push(["before", parent.id, child.id, before.id]);
  },
  insertInContainerBefore: (_root, child, before) => {
    attach(0, child.id);
    ops.push(["before", 0, child.id, before.id]);
  },
  removeChild: (_parent, child) => {
    forgetTree(child.id);
    ops.push(["remove", child.id]);
  },
  removeChildFromContainer: (_root, child) => {
    forgetTree(child.id);
    ops.push(["remove", child.id]);
  },
  commitTextUpdate: (instance, _old, next) => ops.push(["text", instance.id, String(next)]),
  commitUpdate(instance, _type, _old, next) {
    forget(instance.id);
    ops.push(["props", instance.id, split(instance.id, next)]);
  },
  finalizeInitialChildren: () => false,
  prepareForCommit: () => null,
  resetAfterCommit: flush,
  clearContainer: () => {
    for (const child of kids.get(0) ?? []) forgetTree(child);
    ops.push(["clear"]);
  },
  getPublicInstance: (instance) => instance,
  preparePortalMount: () => undefined,
  getCurrentUpdatePriority: () => SYNC,
  setCurrentUpdatePriority: () => undefined,
  resolveUpdatePriority: () => SYNC,
  getCurrentEventPriority: () => SYNC,
  getInstanceFromNode: () => null,
  beforeActiveInstanceBlur: () => undefined,
  afterActiveInstanceBlur: () => undefined,
  prepareScopeUpdate: () => undefined,
  getInstanceFromScope: () => null,
  detachDeletedInstance: () => undefined,
  shouldAttemptEagerTransition: () => false,
  trackSchedulerEvent: () => undefined,
  resolveEventType: () => null,
  resolveEventTimeStamp: () => -1.1,
  requestPostPaintCallback: () => undefined,
  maySuspendCommit: () => false,
  preloadInstance: () => true,
  startSuspendingCommit: () => undefined,
  suspendInstance: () => undefined,
  waitForCommitToBeReady: () => null,
  NotPendingTransition: null,
  HostTransitionContext: React.createContext(null),
  resetFormInstance: () => undefined,
  hideInstance: () => undefined,
  unhideInstance: () => undefined,
  hideTextInstance: () => undefined,
  unhideTextInstance: () => undefined,
};
const renderer = Reconciler(hostConfig);

// Host context and props as external stores, so updates re-render in place.
const store = (initial) => {
  let value = initial;
  const listeners = new Set();
  return {
    get: () => value,
    set(next) {
      value = next;
      for (const listener of listeners) listener();
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
};
const hostContext = store({});
const appProps = store({});

const ui = Object.fromEntries(ELEMENTS.map((name) => [name, name]));
ui.useHostContext = () => React.useSyncExternalStore(hostContext.subscribe, hostContext.get);
ui.useSend = () => (text) => {
  if (typeof text !== "string") throw new TypeError("send(text) needs text");
  __send(text);
};
const MODULES = { react: React, "@cowboy/ui": ui };

let root = null;
function Shell({ App }) {
  const props = React.useSyncExternalStore(appProps.subscribe, appProps.get);
  return React.createElement(App, props);
}
// An error React caught still ends the app: a broken or exhausted app must not
// linger half-drawn. The Worker turns the message into a stop reason.
const report = (error) => __fail(String((error && error.message) || error).slice(0, 500));

let App = null;
/** Compiles and evaluates the app module (its own deadline). */
globalThis.__cowboyCompile = (source) => {
  const compiled = transform(source, {
    transforms: ["typescript", "jsx", "imports"],
    jsxRuntime: "classic",
    production: true,
  }).code;
  const module = { exports: {} };
  const require = (name) => {
    if (!Object.hasOwn(MODULES, name)) {
      throw new Error(`import "${name}" is not available; use ${Object.keys(MODULES).join(", ")}`);
    }
    return MODULES[name];
  };
  new Function("module", "exports", "require", "React", compiled)(
    module,
    module.exports,
    require,
    React,
  );
  App = module.exports.default;
  if (typeof App !== "function") throw new Error("the app must default-export a component");
};
/** The first render (its own deadline). */
globalThis.__cowboyRender = (propsJson, contextJson) => {
  appProps.set(JSON.parse(propsJson));
  hostContext.set(JSON.parse(contextJson));
  root = renderer.createContainer({}, 0, null, false, null, "", report, report, report, null);
  renderer.updateContainerSync(React.createElement(Shell, { App }), root, null, null);
  renderer.flushSyncWork();
};
globalThis.__cowboyProps = (propsJson) => {
  appProps.set(JSON.parse(propsJson));
  renderer.flushSyncWork();
};
globalThis.__cowboyContext = (contextJson) => {
  hostContext.set(JSON.parse(contextJson));
  renderer.flushSyncWork();
};
globalThis.__cowboyEvent = (id, name, payloadJson) => {
  const handler = handlers.get(`${id}:${name}`);
  if (!handler) return;
  renderer.batchedUpdates(() => handler(JSON.parse(payloadJson)));
  renderer.flushSyncWork();
};
/** Runs due timers; returns ms until the next one, or -1 when none. */
globalThis.__cowboyTimersRun = (max) => {
  const next = globalThis.__cowboyTimersDue(max);
  renderer.flushSyncWork();
  return next;
};
