// Convex document ids on the wire (ARCHITECTURE §1.2). `Id<"messages">` here is exactly
// `convex/_generated/dataModel` `Id<"messages">` (both are `GenericId<"messages">`), so values pass between
// this package and Convex without casts, and the main entry needs no Convex sources.
import type { GenericId } from "convex/values";

/** A Convex document id of table `T`. Opaque: never parse it. */
export type Id<T extends string> = GenericId<T>;

/** Unix milliseconds, UTC (§1.3). */
export type UnixMs = number;
