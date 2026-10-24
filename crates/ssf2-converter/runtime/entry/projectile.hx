// The entry the engine calls on a projectile's script. Emitted last, after the projectile's data.

function initialize() { PeptideRuntime.start(self, DATA, Projectiles); }
function update() { PeptideRuntime.update(); }
function onTeardown() { PeptideRuntime.teardown(self); }
