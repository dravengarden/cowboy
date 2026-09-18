/** Original-index reservations. No path lookup, serialized handle or Open here. */
import type { OwnedCodeBuffer } from "./owner.ts";
import {
  BufferClientError,
  requireValue,
  type ResourceId,
} from "./protocol.ts";
import type {
  NavigationDestination,
  NavigationLocation,
  NavigationSnapshot,
} from "./navigationProtocol.ts";

declare const navigationTarget: unique symbol;
export interface NavigationTarget {
  readonly [navigationTarget]: true;
  /** Historical evidence, not current coordinates or a native grant. */
  readonly location: NavigationLocation;
}

/** Core-only, created synchronously before dispatch and never returned publicly. */
export interface DestinationReservation {
  readonly owner: OwnedCodeBuffer;
  validate(id: ResourceId): void;
  adopt(id: ResourceId): void;
  abandon(): void;
}

export function createNavigationDestinations(
  reserve: (location: NavigationLocation) => DestinationReservation,
) {
  let targets: readonly NavigationTarget[] = Object.freeze([]);
  const indices = new WeakMap<NavigationTarget, number>();
  const entries = new Map<number, {
    reservation: DestinationReservation;
    observation: NavigationDestination | undefined;
  }>();
  const index = (target: NavigationTarget) => {
    const value = indices.get(target);
    if (value === undefined) throw new BufferClientError("state");
    return value;
  };
  return {
    targets: () => targets,
    hasUnrequested: () => entries.size < targets.length,
    requested: () => new Set(entries.keys()),
    destination(target: NavigationTarget) {
      return entries.get(index(target))?.reservation.owner;
    },
    reserve(target: NavigationTarget) {
      const destination = index(target);
      if (entries.has(destination)) throw new BufferClientError("state");
      const reservation = reserve(target.location);
      entries.set(destination, { reservation, observation: undefined });
      return {
        destination,
        content: target.location.content,
        owner: reservation.owner,
      };
    },
    accept(next: NavigationSnapshot) {
      const receipts = new Map(
        next.destinations.map((receipt) => [receipt.destination, receipt]),
      );
      // Preflight the complete receipt before adopting even the first owner.
      for (const [destination, entry] of entries) {
        const receipt = receipts.get(destination), previous = entry.observation;
        if (previous?.state === "prepared") {
          requireValue(
            receipt?.state === "prepared" &&
              receipt.resourceId === previous.resourceId,
          );
        } else if (previous?.state === "expired") {
          requireValue(receipt?.state === "expired");
        } else if (previous?.state === "unknown") {
          requireValue(receipt && receipt.state !== "pending");
        }
        // Pending can disappear only before Service dispatch. Absence never
        // rearms POST or releases local capacity without parent terminal proof.
        if (receipt?.state === "prepared") {
          entry.reservation.validate(receipt.resourceId);
        }
      }
      if (!targets.length && next.state === "retained") {
        targets = Object.freeze(next.locations.map((location, n) => {
          const target = Object.freeze({ location }) as NavigationTarget;
          indices.set(target, n);
          return target;
        }));
      }
      for (const [destination, entry] of entries) {
        const receipt = receipts.get(destination);
        if (receipt?.state === "prepared") {
          entry.reservation.adopt(receipt.resourceId);
        } else if (
          receipt?.state === "expired" || next.state === "released" ||
          next.state === "expired"
        ) entry.reservation.abandon();
        entry.observation = receipt;
      }
    },
  };
}
