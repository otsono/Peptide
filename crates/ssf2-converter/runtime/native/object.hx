// {title}: converted from SSF2 by Peptide.
//
// written as Fraymakers code: a stage's object (an enemy, a platform the tree places), its own code
// ({classes}) as this engine's calls. the parts every stage object has come from
// runtime/native/object.hx and runtime/native/object_rules.hx; edit those files, not this one.

// ---- its state, in the engine's containers (a rollback puts these back)
// how many engine frames it has run
var frames = self.makeInt(0);
// whether its first frame's code has run (the source runs it once its maker's code is done)
var started = self.makeObject(false);
// {fields}

// ---- its own code
// {functions}

// ---- what every stage object does
function initialize() {
	// {begin}
	// (its own start waits for the stage's, as the source starts the stage's code before its objects')
	if (stageStarted()) ownStart(); else waitingStart.set(true);
	// {publish}
}

// whether the stage's own code has started (a stage on the runtime, or none, has)
var waitingStart = self.makeObject(false);
function stageStarted() { return stage == null || stage.exports == null || stage.exports.__started != false; }
function ownStart() {
	// {own_start}
}

function update() {
	if (waitingStart.get()) {
		if (!stageStarted()) return;
		waitingStart.set(false);
		ownStart();
	}
	if (!started.get()) {
		started.set(true);
		// {first}
	}
	frames.inc();
	// no hit freezes it: it takes a hit its own way
	if (self.getHitstop() > 0) self.startHitstop(0);
	// {every_frame}
	// what its own class runs each of the source's frames (every second engine frame)
	if (frames.get() % 2 == 1) {
		// {own_tick}
	}
}

// its body stops standing as it goes
function onKill() {
	// {end}
}
function onTeardown() {
	// {end}
}
