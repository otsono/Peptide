// The entry the engine calls on a projectile's script. Emitted last, after the projectile's data.

function initialize() {
	PeptideRuntime.start(self, DATA, Projectiles);
	// its stats another object's code writes (a fighter written as Fraymakers code writes them through this)
	self.exports.__setStats = function(stats) { PeptideRuntime.run(function() { PeptideProjectile.updateProjectileStats(PeptideRuntime.object, stats); }); };
}
// what its declared stats have it do (`flyByStats`, runtime/projectile_stats.jsonc) runs after its
// own frame, while it flies: not frozen by a hit, nor playing its end. a projectile whose stats
// take none is written without that line
function update() {
	PeptideRuntime.update();
	if (!self.isDisposed() && self.getHitstop() == 0 && self.getState() != PState.DESTROYING) flyByStats();
}
// the engine's own end (its death bounds, or any kill) tells the source's end once
function onKill() { if (PeptideRuntime.object != null) Projectiles.killed(PeptideRuntime.object); }
function onTeardown() { PeptideRuntime.teardown(self); }
