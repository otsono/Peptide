// {title}: converted from SSF2 by Peptide.
//
// written as Fraymakers code: its stats files hold what its package declares, and below is what
// its own code ({classes}) does, as this engine's calls. the parts every fighter has come from
// runtime/native/fighter.hx and the rules the source's engine plays a fighter by from
// runtime/native/fighter_rules.hx; edit those files, not this one.

// ---- its state, in the engine's containers (a rollback puts these back)
// how many engine frames it has run
var frames = self.makeInt(0);
// what its code keeps by name (`setGlobalVariable`) and the stats its code set, on the object it
// publishes for other objects' scripts (its projectiles) and the debugger (`gvar`)
var source = { __vars: new haxe.ds.StringMap(), __stats: new haxe.ds.StringMap() };
// {fields}

// ---- its own code
// {functions}

// ---- what every fighter does
function initialize() {
	self.exports.__source = source;
	// how many frames it has run, for the verify gates
	self.exports.__verifyPulse = function() { return frames.get(); };
	// {publish}
	// {start}
	// {own_start}
}

function update() {
	frames.inc();
	// {update}
	// what its own class runs each of the source's frames (every second engine frame)
	if (frames.get() % 2 == 1) {
		// {own_tick}
	}
}

function onTeardown() {
	// {teardown}
}

// the animations its tables name, checked against its entity by the verify gates
function __selfCheck() {
	var bad = [];
	var names = [/* {anims} */];
	for (n in names) if (!self.hasAnimation(n)) bad.push(n);
	return bad.length == 0 ? "ok " + names.length : bad.length + " " + bad.slice(0, 4).join(",");
}
