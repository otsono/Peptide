// The functions a projectile written as Fraymakers code gets when its code needs one
// (compile/native.rs). Each is one block under its `// ---- name` line; the writer copies the
// block with its capitalised words filled in (NAME the function's own name, the others as each
// block's `//:` lines say, which are not copied) and each `// {cases}` line replaced by the lines
// it works out. Edit them here.

// ---- owner_attack
//: STAT the stat; its cases are by the owner's animations
// what the attack its owner is in declares for its STAT, by the animation that shows it
function NAME(owner) {
	switch (owner.getAnimation()) {
		// {cases}
	}
	return null;
}

// ---- owner_label
// the label its owner's clip is on: the attack the animation it plays shows
function NAME(owner) {
	switch (owner.getAnimation()) {
		// {cases}
	}
	return null;
}

// ---- kind_name
// the source's name for an object's kind
function NAME(o) {
	switch (o.getType()) {
		// {cases}
	}
	return null;
}

// ---- owner_stat
//: STAT the stat, DECLARED what its owner states for it
// its owner's STAT: what its code set it to since, else what it states (its owner's script keeps
// what its code sets on the object it publishes, while it is on the runtime)
function NAME(owner) {
	var set = owner.exports.__source == null ? null : owner.exports.__source.__stats;
	return set != null && set.exists("STAT") ? set.get("STAT") : DECLARED;
}

// ---- fire
//: PROJECTILE its name, CONTENT its content, FROM who fires it (this one, or its owner when this
//: one's code asks the owner), DX DY its declared offset, XS YS its declared launch (already in
//: this engine's speed), SIZE the source's size here
// FROM fires PROJECTILE: off where it is, launched the way it faces
function NAME(?x, ?y, ?place) {
	if (x == null) x = 0;
	if (y == null) y = 0;
	var p = match.createProjectile(self.getResource().getContent("CONTENT"), self.getOwner());
	if (p == null) return null;
	if (place == true && (x != 0 || y != 0)) {
		p.setX(x * SIZE);
		p.setY(y * SIZE);
	} else {
		p.setX(FROM.getX() + FROM.flipX((x + DX) * SIZE));
		p.setY(FROM.getY() + (y + DY) * SIZE);
	}
	if (XS != 0 || YS != 0) {
		p.setXSpeed(p.flipX(FROM.flipX(XS)));
		p.setYSpeed(YS);
		// (handed over: one made on the floor is told of the touch with it, runtime/native/floor.hx)
		p.exports.__launched = { xs: FROM.flipX(XS), ys: YS };
	}
	// made in the projectiles' pass, it starts a source frame later, held by the engine's freeze
	p.forceStartHitstop(3, false);
	self.getOwner().exports.__lastProj = p;
	return p;
}

// ---- call_listener
//: CALL the call; its case is the call itself
// its own CALL, as a listener
function NAME(?e) {
	// {cases}
}

// ---- floor_listener
//: LISTENER its floor listener
// LISTENER, told of the floor the source's way (runtime/native/floor.hx)
function NAME(?e) {
	onFloor(LISTENER);
}

// ---- jump_to
//: its cases are each label its clips have, as the jump to it
// a jump by label another object's code asks of it (its owner's script, through `exports.__playFrame`)
function NAME(label) {
	switch (label) {
		// {cases}
	}
}
