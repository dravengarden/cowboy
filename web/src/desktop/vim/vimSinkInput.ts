/**
 * Whether a Vim Normal-mode command sink is waiting for more input
 * (an operator such as `d`, a prefix such as `g`, or the argument of
 * `f`/`t`/`r`/`m`/`'`). The leader must not take Space from such a command:
 * `f<Space>` finds a space and `r<Space>` replaces with one.
 */
const pendingProbes = new WeakMap<Element, () => boolean>();

export function registerVimSink(sink: Element, pending: () => boolean): void {
  pendingProbes.set(sink, pending);
}

export function vimSinkAwaitsInput(target: EventTarget | null): boolean {
  return target instanceof Element && (pendingProbes.get(target)?.() ?? false);
}
