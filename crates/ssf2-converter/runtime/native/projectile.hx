// {title}: converted from SSF2 by Peptide.
//
// written as Fraymakers code: its stats files hold what its package declares, and below is what
// its own code ({classes}) does, as this engine's calls. the parts every projectile has come from
// runtime/native/projectile.hx; edit that file, not this one.
//
// not carried over from the source's engine: a hit in the frame it is made, before its first
// frame's code listens for one; a projectile made on the floor hearing it touch down; a timer its
// frame code makes coming due a frame early.

// ---- its state, in the engine's containers (a rollback puts these back)
// how many engine frames it lives (two to each of the source's), and how many it has
var life = self.makeInt(LIFE);
var age = self.makeInt(0);
// whether its first frame's code has run (the source runs it once its maker's code is done)
var started = self.makeBool(false);
// {fields}

// ---- its own code
// {functions}

// ---- what every projectile does
function initialize() {
	// its owner's colours, and flying
	self.setCostumeIndex(self.getOwner().getCostumeIndex());
	// {reflect}
	self.setState(PState.ACTIVE);
	// {start}
	// {own_start}
	// {publish}
	// a projectile's hit is always fresh in the source: x1.05 on what each box deals
	self.addEventListener(GameObjectEvent.HITBOX_CONNECTED, freshHit, { persistent: true });
}

function freshHit(e) {
	e.data.hitboxStats.damage = e.data.hitboxStats.damage * 1.05;
}

function update() {
	if (!started.get()) {
		started.set(true);
		// {first}
	}
	// its lifetime runs out
	if (age.inc() >= life.get()) {
		self.kill();
		return;
	}
	// what its own class runs each of the source's frames (every second engine frame)
	if (age.get() % 2 == 1) {
		// {own_tick}
	}
	// {update}
}

// its end: whoever listens for it is told
function onKill() {
	// {ends}
}
