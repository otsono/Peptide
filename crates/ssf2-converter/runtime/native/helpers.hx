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

// ---- own_stat
//: STAT the stat, DECLARED what it states for it
// its STAT: what its code set it to since, else what it states
function NAME() {
	return source.__stats.exists("STAT") ? source.__stats.get("STAT") : DECLARED;
}

// ---- stance_jump
//: its cases are each clip's labels, as the animations that show each
// a jump by label on the clip it shows: the animation that shows the label in the clip of the one
// it plays (a label no animation here shows is a part of the clip this engine never plays)
function NAME(label) {
	switch (self.getAnimation()) {
		// {cases}
	}
}
// one of `anims` shows the label (`starts`: as its first frame): the one playing if it is one of them
function goToLabel(label, anims, starts) {
	var i = anims.indexOf(self.getAnimation());
	if (i < 0) i = 0;
	if (starts[i]) { playStart(anims[i]); return; }
	if (self.getAnimation() != anims[i]) self.playAnimation(anims[i]);
	self.playFrameLabel(label);
}

// ---- plays_on
//: its cases are each animation its clip plays on from, into the one that shows its next frame
// its clip plays on past an animation's last frame into the animation that shows its next frame.
// the engine holds a last frame here: an animation it ends itself goes by the projectile's state
// instead, back to its first or gone
function NAME() {
	if (self.getCurrentFrame() < self.getTotalFrames()) return;
	switch (self.getAnimation()) {
		// {cases}
	}
}

// ---- own_call
//: CALL the call it makes
// one of its own engine calls, as a timer of its code makes it
function NAME() {
	CALL;
}

// ---- play_start
// the first frame of animation `a`. one already on it stays: the source's jump to the frame it is
// on runs that frame's code no second time, and playing it again here would, from inside it
function NAME(a) {
	if (self.getAnimation() == a && self.getCurrentFrame() <= 2) return;
	self.playAnimation(a);
}

// ---- landing_lag
//: CONTINUE its listener that goes on from `continue`
// its landing: on touching down, the attack goes on from `continue` (`on`), or it lands
function NAME(on) {
	self.removeEventListener(EntityEvent.COLLIDE_FLOOR, CONTINUE);
	self.removeEventListener(EntityEvent.COLLIDE_FLOOR, landOnTouch);
	self.addEventListener(EntityEvent.COLLIDE_FLOOR, on ? CONTINUE : landOnTouch, { persistent: true });
	if (self.isOnFloor()) { if (on) CONTINUE(null); else landOnTouch(null); }
}
function landOnTouch(?e) {
	self.removeEventListener(EntityEvent.COLLIDE_FLOOR, landOnTouch);
	self.toState(CState.LAND);
}

// ---- jump_to_continue
// touched down in an attack that goes on there: from `continue`, with no steering, ending if it leaves the floor
function NAME(?e) {
	self.removeEventListener(EntityEvent.COLLIDE_FLOOR, jumpToContinue);
	self.updateAnimationStats({ allowMovement: false, leaveGroundCancel: true });
	stanceJump("continue");
}

// ---- clip_frame
//: its cases are each animation's first clip frame
// the frame its clip is on, as the source counts the clip's frames
function NAME() {
	var k = Std.int((self.getCurrentFrame() - 1) / 2);
	switch (self.getAnimation()) {
		// {cases}
	}
	return k + 1;
}

// ---- to_clip_frame
//: its cases are each clip's animations, the first clip frame each shows and how many
// its clip sent to a frame by number: the animation of its clip that shows it, at it
function NAME(n) {
	var f = Std.int(n);
	switch (self.getAnimation()) {
		// {cases}
	}
}
function goToFrame(f, anims, froms, lens) {
	var i = 0;
	while (i < anims.length) {
		if (f >= froms[i] && f < froms[i] + lens[i]) {
			if (self.getAnimation() != anims[i]) self.playAnimation(anims[i]);
			self.playFrame((f - froms[i]) * 2 + 1);
			return;
		}
		i++;
	}
}

// ---- fighter_fire
//: PROJECTILE its name, CONTENT its content, DX DY its declared offset, XS YS its declared launch
//: (already in this engine's speed), SIZE the source's size here
// it fires PROJECTILE: off where it is (fired into the floor it stands on, on that floor), launched
// the way it faces. the launch is handed over first, as the source gives it before the
// projectile's own code runs: a projectile on the runtime takes it up as it starts
function NAME(?x, ?y, ?place) {
	if (x == null) x = 0;
	if (y == null) y = 0;
	var px = self.getX() + self.flipX((x + DX) * SIZE);
	var py = self.getY() + (y + DY) * SIZE;
	if (place == true && (x != 0 || y != 0)) { px = x * SIZE; py = y * SIZE; }
	else if (self.isOnFloor() && py > self.getY()) py = self.getY();
	var launch = { x: px, y: py, xs: null, ys: null, name: "PROJECTILE", decl: null, g: 0, max: null };
	if (XS != 0 || YS != 0) { launch.xs = self.flipX(XS); launch.ys = YS; }
	self.exports.__launch = launch;
	var p = match.createProjectile(self.getResource().getContent("CONTENT"), self);
	if (p == null) { self.exports.__launch = null; return null; }
	if (p.exports != null) p.exports.__thrownRight = p.isFacingRight();
	if (self.exports.__launch != null) {
		self.exports.__launch = null;
		p.exports.__launched = launch;
		p.setX(px);
		p.setY(py);
		if (launch.xs != null) { p.setXSpeed(p.flipX(launch.xs)); p.setYSpeed(launch.ys); }
	}
	self.exports.__lastProj = p;
	return p;
}

// ---- owner_field
// a field of one of its owner's clips ("self" its own class, "root" its clip, "stance" the clip its
// animation shows), through what the owner publishes
function NAME(o, clip, name) {
	var c = o == null || o.exports.__fieldOf == null ? null : o.exports.__fieldOf(clip, name);
	return c == null ? null : c.get();
}

// ---- owner_set_field
// a field of one of its owner's clips written, through what the owner publishes
function NAME(o, clip, name, v) {
	var c = o == null || o.exports.__fieldOf == null ? null : o.exports.__fieldOf(clip, name);
	if (c != null) c.set(v);
}

// ---- make_enemy
//: FIRE the function that fires it
// an enemy it makes, carried by a projectile here (not a projectile in the source, as an item is
// not either; the harness leaves both out of its count)
function NAME() {
	var o = FIRE(0, 0);
	if (o != null) { o.exports.__isEnemy = true; o.exports.__np = true; }
	return o;
}

// ---- controls_of
// another fighter's controls, by the source's names: its first button is special, its second attack
function NAME(o, pressed) {
	var c = o == null ? null : (pressed == true ? o.getPressedControls() : o.getHeldControls());
	if (c == null) return {};
	return { UP: c.UP, DOWN: c.DOWN, LEFT: c.LEFT, RIGHT: c.RIGHT, JUMP: c.JUMP, JUMP2: false,
	         BUTTON1: c.SPECIAL, BUTTON2: c.ATTACK, SPECIAL: c.SPECIAL, ATTACK: c.ATTACK,
	         SHIELD: c.SHIELD, SHIELD2: false, GRAB: c.GRAB, TAUNT: false, TAP_JUMP: c.TAP_JUMP };
}

// ---- move_if_clear
//: SIZE the source's size here
// an object moved as far as solid ground lets it, in the source's units: true when nothing was in the way
function NAME(o, dx, dy) {
	var x = o.getX();
	var y = o.getY();
	var a = Point.create(x, y - SIZE);
	var b = Point.create(x + dx * SIZE, y - SIZE + dy * SIZE);
	var hits = o.hitTestStructuresWithLineSegment(a, b, Point.create(b.x, b.y), {});
	if (hits != null && hits.length > 0) return false;
	o.setX(x + dx * SIZE);
	o.setY(y + dy * SIZE);
	return true;
}

// ---- source_rect
//: SIZE the source's size here
// a rectangle of the stage's, in the source's units
function NAME(r) {
	return r == null ? null : { x: r.x / SIZE, y: r.y / SIZE, width: r.width / SIZE, height: r.height / SIZE };
}
