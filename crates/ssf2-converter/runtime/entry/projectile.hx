// The entry the engine calls on a projectile's script. Emitted last, after the projectile's data.

// its declared stats' own timers (`DATA.nativeStats`, runtime/projectile_stats.jsonc) start once it has
function initialize() { PeptideRuntime.start(self, DATA, Projectiles); if (!self.isDisposed()) DATA.nativeStats(); }
function update() { PeptideRuntime.update(); }
// the engine's own end (its death bounds, or any kill) tells the source's end once
function onKill() { if (PeptideRuntime.object != null) Projectiles.killed(PeptideRuntime.object); }
function onTeardown() { PeptideRuntime.teardown(self); }
