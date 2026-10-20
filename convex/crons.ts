// Crons (ARCHITECTURE §4.5).
import { cronJobs } from "convex/server";
import { internal } from "./_generated/api";

const crons = cronJobs();

crons.interval("ops.reapLeases", { minutes: 1 }, internal.ops.reapLeases, {});
crons.interval("timers.sweep", { minutes: 1 }, internal.timers.sweep, {});
crons.interval("sync.watchdog", { minutes: 5 }, internal.sync.watchdog, {});
crons.interval("ops.purge", { hours: 1 }, internal.ops.purge, {});
crons.interval("tombstones.gc", { hours: 24 }, internal.sync.gcTombstones, {});
crons.interval("stripeEvents.gc", { hours: 24 }, internal.ee.service.gcStripeEvents, {});
crons.interval("sends.purgeTombstones", { hours: 1 }, internal.sends.purgeTombstones, {});
crons.interval("authLimits.gc", { minutes: 10 }, internal.authLimits.gc, {});
crons.interval("accounts.pauseLapsed", { hours: 24 }, internal.accounts.pauseLapsed, {});

export default crons;
