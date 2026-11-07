// What the source's engine does around every fighter that this engine does differently, written
// on the fighter itself (compile/native.rs, emit/native_fighter.rs). Each block is copied into a
// fighter written as Fraymakers code under its `// ---- name` line; the measurements behind each
// rule are in docs/STATUS.md. Capitalised words are filled in as the block's `//:` lines say.

// ---- attack
//: the attack the fighter is in, as the source counts one: its clip's label, while the engine has it
//: attacking (a throw too). `attackLabel` and `declared` are written for it from its declarations
// the source attack it is in (its clip's label), and how many it has begun
var attackIn = self.makeObject(null);
var attackUse = self.makeInt(0);
// the stats its code wrote for the attack since it began, over what the attack declares
var attackWritten = self.makeObject(new haxe.ds.StringMap());
// whom the attack threw (a box that passes over the one held passes over it too) and let go of
var thrownFoe = self.makeObject(null);
var releasedFoe = self.makeObject(null);
// who its code has listening for an attack to end (the source's `charAttackComplete`)
var attackDone = self.makeObject([]);
// the last sound and voice it played, for an attack that stops them as it ends
var lastSound = self.makeObject(null);
var lastVoice = self.makeObject(null);

function throwing() {
	return self.inState(CState.THROW_UP) || self.inState(CState.THROW_DOWN) || self.inState(CState.THROW_FORWARD) || self.inState(CState.THROW_BACK);
}

// a stat of the attack it is in: what its code wrote since it began, else what it declares
function attackStat(k) {
	var w = attackWritten.get();
	if (w.exists(k)) return w.get(k);
	return attackIn.get() == null ? null : declared(attackIn.get(), k);
}

// stats its code writes on the attack: kept for the rest of it
function attackWrite(k, v) {
	attackWritten.get().set(k, v);
}

// every frame: an attack's start and end, on the frame they happen
function attackFrame() {
	var label = (self.inStateGroup(CStateGroup.ATTACK) || throwing()) ? attackLabel(self.getAnimation()) : null;
	if (label == attackIn.get()) return;
	attackEnded();
	attackIn.set(label);
	if (label == null) return;
	attackUse.inc();
	attackWritten.set(new haxe.ds.StringMap());
	thrownFoe.set(null);
	releasedFoe.set(null);
}

// an attack ends: the sound or voice it said to stop goes
function attackEnded() {
	if (attackIn.get() == null) return;
	for (l in attackDone.get().copy()) l(null);
	if (attackStat("cancelSoundOnEnd") == true && lastSound.get() != null) lastSound.get().stop();
	if (attackStat("cancelVoiceOnEnd") == true && lastVoice.get() != null) lastVoice.get().stop();
	attackStopRefresh();
}

// ---- sounds
//: a sound or voice it plays, kept for an attack that stops it as it ends
function playSoundOf(id, voice) {
	if (id == null) return null;
	var clip = AudioClip.play(self.getResource().getContent(id));
	if (voice) lastVoice.set(clip); else lastSound.set(clip);
	return clip;
}

// ---- refresh
//: the attack hits again every `refreshRate` of the source's frames while it lasts, counted from
//: its first landed hit, only on frames the fighter is free (its own freeze pauses the count)
var refreshTimer = self.makeInt(-1);
var refreshRate = self.makeInt(0);

function attackRefreshes(r) { return r != null && r > 0 && r < 999; }

function attackLanded() {
	if (attackIn.get() == null || refreshTimer.get() >= 0) return;
	var r = attackStat("refreshRate");
	if (!attackRefreshes(r)) return;
	var n = Std.int(r);
	// (at a rate of 1 the source refreshes in the frame after its first hit, inside the freeze)
	if (n == 1) self.reactivateHitboxes();
	refreshRate.set(n);
	refreshTimer.set(self.addTimer(2 * (n > 1 ? n - 1 : 1), 1, refreshFirst, { persistent: true, pauseCondition: frozen }));
}
function frozen() { return self.getHitstop() > 0; }
function refreshFirst() {
	if (refreshTimer.get() < 0) return;
	self.reactivateHitboxes();
	refreshTimer.set(self.addTimer(2 * refreshRate.get(), -1, refreshAgain, { persistent: true, pauseCondition: frozen }));
}
function refreshAgain() {
	if (refreshTimer.get() >= 0) self.reactivateHitboxes();
}
function attackStopRefresh() {
	if (refreshTimer.get() >= 0) self.removeTimer(refreshTimer.get());
	refreshTimer.set(-1);
}
// a rate its code writes as the attack plays starts the count again
function attackSetRefresh(r) {
	attackWrite("refreshRate", r);
	attackStopRefresh();
	if (attackRefreshes(r) && attackIn.get() != null) {
		refreshRate.set(Std.int(r));
		refreshTimer.set(self.addTimer(2 * Std.int(r), -1, refreshAgain, { persistent: true, pauseCondition: frozen }));
	}
}

// ---- hits
//: how a hit lands and holds, the source's way. `declaredBox` is written from its declarations
// a box stat of the attack it is in, by the source's box number (1 its `attackBox`)
function boxStat(i, k) {
	return attackIn.get() == null ? null : declaredBox(attackIn.get(), i, k);
}

// the attack hits a fighter at most once a source frame (a source frame is two of the animation's
// own frames), the last nine attacks that hit stale what it deals, and a box says whether it hits
// the fighter held (`bypassGrabbed` passes over it, `bypassNonGrabbed` hits it alone)
var hitAt = self.makeObject([]);
var fresh = self.makeFloat(1.05);
var freshOf = self.makeInt(-1);

function freshness(key) {
	if (self.exports.__usedAttacks == null) self.exports.__usedAttacks = [];
	var q = self.exports.__usedAttacks;
	var cut = 0.0;
	var any = false;
	var i = 0;
	while (i < q.length) { if (key != null && q[i] == key) { cut += 0.10 - 0.01 * i; any = true; } i++; }
	if (freshOf.get() != attackUse.get()) {
		freshOf.set(attackUse.get());
		q.insert(0, key == null ? {} : key);
		if (q.length > 9) q.pop();
		fresh.set(any ? 1 - cut : 1.05);
	}
	return fresh.get();
}

function onBoxConnected(e) {
	var hs = e.data.hitboxStats;
	if (hs == null || hs.index == null || hs.index < 0) return;
	var foe = e.data.foe;
	var box = hs.index + 1;
	var held = self.getAllGrabbedFoes();
	var isHeld = held != null && held.indexOf(foe) >= 0 && foe != releasedFoe.get();
	if (isHeld && boxStat(box, "bypassGrabbed") == true) { hs.disabled = true; return; }
	if (!isHeld && foe == thrownFoe.get() && boxStat(box, "bypassGrabbed") == true) { hs.disabled = true; return; }
	if (isHeld) thrownFoe.set(foe);
	if (!isHeld && boxStat(box, "bypassNonGrabbed") == true) { hs.disabled = true; return; }
	// a box that turns its launch for a fighter behind the attacker judges where the attacker
	// stood before this frame's move
	if (hs.reversibleAngle == true) {
		var before = self.getX() - self.getXSpeed();
		if ((foe.getX() - before) * (self.isFacingRight() ? 1 : -1) > 0) hs.reversibleAngle = false;
	}
	var cf = self.getCurrentFrame();
	var at = (frames.get() - (cf - 1)) * 1000 + ((cf - 1) >> 1);
	var kept = [];
	for (h in hitAt.get()) {
		if (h.foe == foe && h.at == at) { hs.disabled = true; return; }
		if (h.foe != foe) kept.push(h);
	}
	kept.push({ foe: foe, at: at });
	hitAt.set(kept);
	hs.damage = hs.damage * freshness(attackIn.get());
	// poison: its damage every interval for its length, one at a time on a fighter
	var p = boxStat(box, "poison");
	if (p != null && p > 0) poison(foe, p, boxStat(box, "poisonInterval"), boxStat(box, "poisonLength"));
	// a box with no effect hits the one held and keeps hold of it, launching no one
	var effect = boxStat(box, "hasEffect");
	if (isHeld && (effect == null || effect != false)) releasedFoe.set(foe);
	else if (isHeld) { hs.baseKnockback = 0; hs.knockbackGrowth = 0; }
	// a box that reverses the one it hits turns it around, and its launch with it
	if (boxStat(box, "reverse_character") == true) {
		if (foe.isFacingRight()) foe.faceLeft(); else foe.faceRight();
		hs.angle = 180 - hs.angle;
	}
}

function poison(foe, dmg, every, length) {
	if (every == null || length == null || !(every > 0) || !(length > 0)) return;
	var token = {};
	foe.exports.__poison = token;
	self.addTimer(Std.int(every * 2), Std.int(length / every), function() {
		if (!foe.isDisposed() && foe.exports.__poison == token) foe.addDamage(dmg);
	}, { persistent: true });
}

// a hit it dealt: the attack refreshes from it, the throw lets go of the one it throws as it lands
// (launch and all), and the box's own hold is handed to the fighter it hit
function onHitDealt(e) {
	var hs = e == null || e.data == null ? null : e.data.hitboxStats;
	if (hs == null || hs.index == null) return;
	attackLanded();
	var foe = e.data.foe;
	if (foe != null && foe == releasedFoe.get()) {
		var grabbed = self.getAllGrabbedFoes();
		if (grabbed != null && grabbed.indexOf(foe) >= 0) {
			var kx = foe.getXKnockback();
			var ky = foe.getYKnockback();
			self.releaseCharacter(foe);
			foe.setXKnockback(kx);
			foe.setYKnockback(ky);
			if (foe.exports.__source != null) foe.exports.__source.__relaunch = [kx, ky];
		}
	}
	var them = foe == null || foe.exports == null ? null : foe.exports.__source;
	if (them != null) {
		them.__stunLag = boxStat(hs.index + 1, "hitLag");
		if (them.__stunWant != null && them.__stunLaunch != null) them.__stunWant = stunWant(them.__stunLaunch, them.__stunLag);
	}
	// which box landed each hit, by the source's name for it, for the harness (`gvar pN __peptideHits`)
	var was = source.__vars.get("__peptideHits");
	var log = was == null || was == false || was == 0 ? "" : Std.string(was);
	source.__vars.set("__peptideHits", log + (hs.index == 0 ? "attackBox" : "attackBox" + (hs.index + 1)) + ",");
}
function onShieldHitDealt(e) { attackLanded(); }

// how long a hit holds it: the source's hitstun is a fifth of the launch in its units, times the
// hold its box states, rounded up, from the end of the freeze
var stunEnd = self.makeInt(-1);
var stunSet = self.makeObject(false);
var stood = self.makeObject(false);

function stunOf(launch, lag) {
	var l = lag == null ? -1.0 : lag;
	if (Math.isNaN(l)) l = -1.0;
	if (l >= 0) return Math.round(l);
	return Math.ceil(Math.floor(launch / 2 / 0.10464 * 0.2) * -l);
}
function stunWant(launch, lag) { return Std.int(Math.round(stunOf(launch, lag) * 2)); }

function onHitstun(e) {
	if (stunSet.get()) { stunSet.set(false); return; }
	var kx = self.getXKnockback();
	var ky = self.getYKnockback();
	var launch = Math.sqrt(kx * kx + ky * ky) / 0.7157;
	if (!(launch > 0)) return;
	source.__stunLaunch = launch;
	source.__stunWant = stunWant(launch, source.__stunLag);
}
function onHitstunOut(e) {
	if (stunSet.get() || !self.isOnFloor()) return;
	if (self.inState(CState.HURT_LIGHT) || self.inState(CState.HURT_MEDIUM) || self.inState(CState.HURT_HEAVY)) self.toState(CState.IDLE);
}
// a hit aimed down at a standing fighter too weak to bounce it sends it up at the mirrored angle
function onHitReceived(e) {
	if (stood.get() && self.isOnFloor() && self.getYKnockback() > 0) self.setYKnockback(-self.getYKnockback());
}
// a thrown fighter takes back the launch the let-go cleared, with no freeze of the engine's for it
function relaunch() {
	if (source.__relaunch == null || self.inState(CState.HELD)) return;
	var rl = source.__relaunch;
	source.__relaunch = null;
	if (self.getHitstop() > 0) self.forceStartHitstop(0, false);
	self.setXKnockback(rl[0]);
	self.setYKnockback(rl[1]);
	self.setXSpeed(0);
	self.setYSpeed(0);
}
function onHitstop(e) { relaunch(); }

// every frame: the hold a hit asked for, put in once its freeze is over
function hitFrame() {
	stood.set(self.isOnFloor());
	if (source.__relaunch != null) relaunch();
	if (source.__stunWant == null) return;
	var want = source.__stunWant - 1;
	if (stunEnd.get() >= 0 && stunEnd.get() - frames.get() > want) want = stunEnd.get() - frames.get();
	stunEnd.set(frames.get() + want);
	var launch = source.__stunLaunch;
	source.__stunWant = null;
	source.__stunLag = null;
	if (want > 0 && self.getHitstun() > 0) {
		// a hard enough hit sends it flying (8.37 a half frame, held 16 frames or more)
		if (launch / 2 >= 8.37 && want + 1 >= 32 && !self.inState(CState.HURT_HEAVY)) self.toState(CState.HURT_HEAVY);
		stunSet.set(true);
		self.forceStartHitstun(want);
	}
}

function listenForHits() {
	self.addEventListener(GameObjectEvent.HITBOX_CONNECTED, onBoxConnected, { persistent: true });
	self.addEventListener(GameObjectEvent.HIT_DEALT, onHitDealt, { persistent: true });
	self.addEventListener(GameObjectEvent.SHIELD_HIT_DEALT, onShieldHitDealt, { persistent: true });
	self.addEventListener(GameObjectEvent.ENTER_HITSTUN, onHitstun, { persistent: true });
	self.addEventListener(GameObjectEvent.EXIT_HITSTUN, onHitstunOut, { persistent: true });
	self.addEventListener(GameObjectEvent.HIT_RECEIVED, onHitReceived, { persistent: true });
	self.addEventListener(GameObjectEvent.ENTER_HITSTOP, onHitstop, { persistent: true });
}

// ---- walk_start
//: START the walk's own starting speed (its declared accel_start of norm_xSpeed), in this engine's units
// an x speed its code sets: zero, on the floor under a held direction, is the walk's own start on
// the source, which walks on from it
function setXSpeedFromCode(v) {
	self.setXSpeed(v);
	if (v != 0 || !self.isOnFloor()) return;
	var c = self.getHeldControls();
	if (c != null && (c.LEFT || c.RIGHT)) self.setXSpeed(START);
}

// ---- end_move
// the end of a move: on the floor it stands; in the air an attack's animation ends; else it falls
function endMove() {
	if (self.isOnFloor()) self.toState(CState.STAND);
	else if (self.inStateGroup(CStateGroup.ATTACK)) self.endAnimation();
	else self.toState(CState.FALL);
}

// ---- controls
// the source's controls, by its names: its first button is special, its second attack
function controls(pressed) {
	var c = pressed == true ? self.getPressedControls() : self.getHeldControls();
	if (c == null) return {};
	return { UP: c.UP, DOWN: c.DOWN, LEFT: c.LEFT, RIGHT: c.RIGHT, JUMP: c.JUMP, JUMP2: false,
	         BUTTON1: c.SPECIAL, BUTTON2: c.ATTACK, SPECIAL: c.SPECIAL, ATTACK: c.ATTACK,
	         SHIELD: c.SHIELD, SHIELD2: false, GRAB: c.GRAB, TAUNT: false, TAP_JUMP: c.TAP_JUMP };
}

// ---- start_attack
// an attack started by its code: the engine's attack state for it, by the name its animation
// starts with, on that animation (playing the animation alone leaves a standing fighter standing)
var attackStates = [
	["jab", CState.JAB], ["dash_attack", CState.DASH_ATTACK],
	["tilt_forward", CState.TILT_FORWARD], ["tilt_up", CState.TILT_UP], ["tilt_down", CState.TILT_DOWN],
	["strong_forward_in", CState.STRONG_FORWARD_IN], ["strong_forward_charge", CState.STRONG_FORWARD_CHARGE], ["strong_forward", CState.STRONG_FORWARD_ATTACK],
	["strong_up_in", CState.STRONG_UP_IN], ["strong_up_charge", CState.STRONG_UP_CHARGE], ["strong_up", CState.STRONG_UP_ATTACK],
	["strong_down_in", CState.STRONG_DOWN_IN], ["strong_down_charge", CState.STRONG_DOWN_CHARGE], ["strong_down", CState.STRONG_DOWN_ATTACK],
	["aerial_neutral", CState.AERIAL_NEUTRAL], ["aerial_forward", CState.AERIAL_FORWARD], ["aerial_back", CState.AERIAL_BACK],
	["aerial_up", CState.AERIAL_UP], ["aerial_down", CState.AERIAL_DOWN],
	["special_neutral", CState.SPECIAL_NEUTRAL], ["special_side", CState.SPECIAL_SIDE], ["special_up", CState.SPECIAL_UP], ["special_down", CState.SPECIAL_DOWN],
	["throw_forward", CState.THROW_FORWARD], ["throw_back", CState.THROW_BACK], ["throw_up", CState.THROW_UP], ["throw_down", CState.THROW_DOWN],
	["grab", CState.GRAB]
];
function startAttack(anim) {
	var best = null;
	for (row in attackStates) if (anim.indexOf(row[0]) == 0 && (best == null || row[0].length > best[0].length)) best = row;
	if (best == null) { self.playAnimation(anim); return; }
	if (self.getAnimation() != anim) self.toState(best[1], anim);
}

// ---- floor_hold
// held to the floor right under its feet
function holdToFloor() {
	var out = Point.create(self.getX(), self.getY() + 4);
	var hits = self.hitTestStructuresWithLineSegment(Point.create(self.getX(), self.getY() - 4), out, out, { structureType: StructureType.FLOOR });
	if (hits == null || hits.length == 0) return;
	self.setY(out.y);
	self.attachToFloor(hits[0]);
}

// ---- effects
// effects its moves made, taken off together
var effects = self.makeObject([]);
var effectsOnChange = self.makeObject(false);
function addEffectToList(fx) {
	if (fx != null) effects.get().push(fx);
	return fx;
}
function removeAllEffects(?e) {
	for (fx in effects.get()) if (fx != null && !fx.isDisposed()) fx.kill();
	effects.set([]);
	if (effectsOnChange.get()) { effectsOnChange.set(false); self.removeEventListener(EntityEvent.STATE_CHANGE, removeAllEffects); }
}
function clearEffectsOnStateChange(?once) {
	effectsOnChange.set(true);
	self.addEventListener(EntityEvent.STATE_CHANGE, removeAllEffects, { persistent: true });
}
function stopListening() {
	effectsOnChange.set(false);
	self.removeEventListener(EntityEvent.STATE_CHANGE, removeAllEffects);
}

// ---- items
//: TOSS an item's throw speed where it states none, in this engine's units. `itemBox`,
//: `itemContent` and `itemDeclared` are written for it from its animations and declarations
// an item it generates: a projectile of its own riding its item box until it is thrown. the item's
// own script takes the hold and the throw as asks on its exports (`__holder`, `__ask`)
function heldItem() {
	var it = self.exports.__heldItem;
	if (it != null && it.isDisposed()) { self.exports.__heldItem = null; return null; }
	return it;
}
function placeItem(it) {
	var b = itemBox();
	it.setX(self.getX() + self.flipX(b.x * SIZE));
	it.setY(self.getY() + b.y * SIZE);
	self.exports.__ibx = b.x;
	self.exports.__iby = b.y;
}
function askItem(it, kind, ?v) {
	if (it == null || it.exports == null) return;
	if (it.exports.__ask != null) { it.exports.__ask(kind, v); return; }
	if (kind == "attack") it.exports.__askAttack = v; else if (kind == "interrupt") it.exports.__askInterrupt = v; else it.exports.__askState = kind;
}
function generateItem(name) {
	var id = itemContent(name);
	var content = id == null ? null : self.getResource().getContent(id);
	if (content == null) return null;
	var it = match.createProjectile(content, self);
	if (it == null) return null;
	// in the hand from its first frame, touching nothing until thrown
	it.exports.__ghostDeclared = it.getProjectileStat("ghost") == true;
	it.updateProjectileStats({ ghost: true });
	placeItem(it);
	it.exports.__holder = self;
	it.exports.__isItem = true;
	it.exports.__np = true;
	it.exports.__itemName = name;
	askItem(it, "hold");
	self.exports.__heldItem = it;
	return it;
}
// thrown the way it holds the stick: up, down, or along its facing
function itemDirection() {
	var c = self.getHeldControls();
	if (c != null && c.UP) return 90;
	if (c != null && c.DOWN) return 270;
	if (c != null && ((c.LEFT && self.isFacingRight()) || (c.RIGHT && !self.isFacingRight()))) return 158;
	return 12;
}
function tossItem(?angle) {
	var it = heldItem();
	if (it == null) return;
	if (angle == null) angle = itemDirection();
	var a = angle * Math.PI / 180;
	var speed = it.exports.__tossSpeed != null ? it.exports.__tossSpeed : TOSS;
	placeItem(it);
	it.exports.__holder = null;
	askItem(it, "toss");
	it.setXSpeed(self.flipX(Math.cos(a) * speed * SPEED));
	it.setYSpeed(-Math.sin(a) * speed * SPEED);
	self.exports.__heldItem = null;
}
function dropItem(?z) {
	var it = heldItem();
	if (it == null) return;
	placeItem(it);
	it.exports.__holder = null;
	askItem(it, "toss");
	it.exports.__zDropped = z == true;
	it.setXSpeed(0);
	it.setYSpeed(0);
	self.exports.__heldItem = null;
}
function removeItem() {
	var it = heldItem();
	if (it != null) it.kill();
	self.exports.__heldItem = null;
}
// what an item it made declares
function itemStat(it, k) {
	if (it == null || it.exports == null) return null;
	var name = it.exports.__itemName;
	if (k == "linkage_id") return name;
	return itemDeclared(name, k);
}
// every frame it holds one, the item rides the box; an attack begun while holding it is put to the item first
var itemAttack = self.makeObject(null);
function itemFrame() {
	var it = heldItem();
	if (it == null) { itemAttack.set(null); return; }
	placeItem(it);
	var label = attackLabel(self.getAnimation());
	if (label != itemAttack.get()) {
		itemAttack.set(label);
		if (label != null) askItem(it, "interrupt", { targetFrame: label, character: self, cStick: false, zair: false });
	}
}

// ---- actions
//: an attack the source turns off or back on by its name (`setAttackEnabled(false, "b_down")`):
//: this engine turns off the ACTIONS its animations start. `labelAnims` is written from its animations
// the actions it has off, and the status effect holding each
var actionsOff = self.makeObject([]);
var actionEffects = self.makeObject([]);
var actionsWatched = self.makeObject(false);
var actionTable = [
	["jab", CharacterActions.JAB], ["dash_attack", CharacterActions.DASH_ATTACK],
	["tilt_forward", CharacterActions.TILT_FORWARD], ["tilt_up", CharacterActions.TILT_UP], ["tilt_down", CharacterActions.TILT_DOWN],
	["strong_forward", CharacterActions.STRONG_FORWARD], ["strong_up", CharacterActions.STRONG_UP], ["strong_down", CharacterActions.STRONG_DOWN],
	["aerial_neutral", CharacterActions.AERIAL_NEUTRAL], ["aerial_forward", CharacterActions.AERIAL_FORWARD], ["aerial_back", CharacterActions.AERIAL_BACK],
	["aerial_up", CharacterActions.AERIAL_UP], ["aerial_down", CharacterActions.AERIAL_DOWN],
	["special_neutral", CharacterActions.SPECIAL_NEUTRAL], ["special_side", CharacterActions.SPECIAL_SIDE], ["special_up", CharacterActions.SPECIAL_UP], ["special_down", CharacterActions.SPECIAL_DOWN],
	["grab", CharacterActions.GRAB]
];
// the actions the animations of the source's label start (the longest name that begins one)
function actionsOf(label) {
	var out = [];
	for (anim in labelAnims(label)) {
		var best = null;
		for (row in actionTable) if (anim.indexOf(row[0]) == 0 && (best == null || row[0].length > best[0].length)) best = row;
		if (best != null && out.indexOf(best[1]) < 0) out.push(best[1]);
	}
	return out;
}
// turned off with no count of frames, an attack comes back after the count it declares
// (`reenableTimer`, source frames); 0 keeps it off. every one turned off is back on when it lands
function allowAttack(on, label, ?frames) {
	if (!on && (frames == null || frames < 0)) {
		var d = declared(label, "reenableTimer");
		frames = d != null && d > 0 ? d : null;
	}
	if (!on && frames != null && frames > 0) self.addTimer(Std.int(frames * 2), 1, function() { allowAttack(true, label, null); }, { persistent: true });
	var off = actionsOff.get();
	var effects = actionEffects.get();
	for (a in actionsOf(label)) {
		var k = off.indexOf(a);
		if (on && k >= 0) {
			self.removeStatusEffect(StatusEffectType.DISABLE_ACTION, effects[k]);
			off.splice(k, 1);
			effects.splice(k, 1);
		} else if (!on && k < 0) {
			var e = self.addStatusEffect(StatusEffectType.DISABLE_ACTION, a);
			off.push(a);
			effects.push(e == null ? null : e.id);
		}
	}
	if (off.length > 0 && !actionsWatched.get()) {
		actionsWatched.set(true);
		self.addEventListener(GameObjectEvent.LAND, actionsBack, { persistent: true });
	}
}
function attackAllowed(label) {
	for (a in actionsOf(label)) if (actionsOff.get().indexOf(a) >= 0) return false;
	return true;
}
function actionsBack(?e) {
	var off = actionsOff.get();
	var effects = actionEffects.get();
	while (off.length > 0) {
		self.removeStatusEffect(StatusEffectType.DISABLE_ACTION, effects[0]);
		off.splice(0, 1);
		effects.splice(0, 1);
	}
}

// ---- place_effect
// an effect its code keeps, placed off where it is (and turned, where it asks)
function placeEffect(e, x, y, ?rot) {
	if (e == null) return null;
	e.setX(self.getX() + x);
	e.setY(self.getY() + y);
	if (rot != null) e.setRotation(rot);
	return e;
}

// ---- nearest
// the nearest object of a kind ("character", "player", "item", "projectile", "enemy"), and that one
// as the source's path to it
function nearest(type) {
	var list = (type == "character" || type == "player") ? match.getCharacters() : (type == "enemy" ? match.getCustomGameObjects() : match.getProjectiles());
	var best = null;
	var bd = 1.0e12;
	for (o in list) {
		if (o == self || o.isDisposed()) continue;
		if (type == "item" && (o.exports == null || o.exports.__isItem != true)) continue;
		var dx = o.getX() - self.getX();
		var dy = o.getY() - self.getY();
		var d = dx * dx + dy * dy;
		if (d < bd) { bd = d; best = o; }
	}
	return best;
}
function nearestPath(type) {
	var t = nearest(type);
	return t == null ? [] : [t];
}

// ---- last_used
// how many of its frames ago it began an attack before its latest start of it (the one named,
// else the one it is in; -1 out of one), as the source's engine counts: one never begun counts
// from the start of the match
var usedAt = self.makeObject(new haxe.ds.StringMap());
var usedBefore = self.makeObject(new haxe.ds.StringMap());
var usedLabel = self.makeObject(null);
function usedFrame() {
	var label = attackLabel(self.getAnimation());
	if (label == usedLabel.get()) return;
	usedLabel.set(label);
	if (label == null) return;
	if (usedAt.get().exists(label)) usedBefore.get().set(label, usedAt.get().get(label));
	usedAt.get().set(label, frames.get() / 2);
}
function lastUsed(?n) {
	var cur = attackLabel(self.getAnimation());
	var label = n == null ? cur : n;
	if (label == null) return -1;
	var at = usedBefore.get().exists(label) ? usedBefore.get().get(label) : 0;
	if (label == cur && usedLabel.get() != cur && usedAt.get().exists(label)) at = usedAt.get().get(label);
	return frames.get() / 2 - at;
}
function setLastUsed(n, v) { usedBefore.get().set(n, frames.get() / 2 - v); }

// ---- char_stats
// the engine's own value of a stat before its code first changed it (a write moves it by the share
// the source's declared one moves)
var statBase = self.makeObject(new haxe.ds.StringMap());
function engineStat(name) {
	var b = statBase.get();
	if (!b.exists(name)) b.set(name, self.getCharacterStat(name));
	return b.get(name);
}

// ---- fired_stats
// a stat of a projectile it fired, as its declaration states it (by the name it was fired by)
function firedStat(p, k) {
	if (p == null || p.exports == null || p.exports.__launched == null) return null;
	return firedDeclared(p.exports.__launched.name, k);
}

// ---- asked
//: what another object's code asks of it, published as it starts (`self.exports`): an attack
//: started at a frame of its clip, and the fields of its clips. `animOf`, `fieldOf` and
//: `stanceClass` are written for it from its animations and its classes
// an attack another object's code starts on it, by the source's label, at a label of its clip
function forceAttackAsked(label, ?frame) {
	var anim = animOf(label);
	if (anim == null) return;
	startAttack(anim);
	if (frame != null && frame != true) stanceJump(frame);
}

// ---- rocket
//: ATTACK the attack its rocket rides in, else null; BLOCKED, ATTACKING and STOPS the tests of
//: the source's states it is not set off in, its attacking, and those it stands from first
// a rocket another object sets off on it (samus's bomb's lift): into the attack it rides, at its
// `rocket` frame, carried by the speed and angle given, losing `decay` of it a source frame, while
// that attack lasts; not on or climbing a ledge, grabbing, hurt or launched, and in an attack
// only in a down special
var rocketAnim = self.makeObject(null);
var rocketSpeed = self.makeObject(null);
var rocketTimer = self.makeInt(-1);
function rocketAsked(speed, angle, decay, ?rotate) {
	var attack = ATTACK;
	if (attack == null) return;
	if (BLOCKED) return;
	var label = attackLabel(self.getAnimation());
	if ((ATTACKING) && (label == null || label.indexOf("b_down") < 0)) return;
	if (STOPS) self.toState(CState.STAND);
	var a = angle * Math.PI / 180;
	var d = Math.abs(decay);
	forceAttackAsked(attack, "rocket");
	rocketAnim.set(self.getAnimation());
	rocketSpeed.set({ vx: speed * Math.cos(a), vy: -speed * Math.sin(a), dx: d * Math.abs(Math.cos(a)), dy: d * Math.abs(Math.sin(a)), n: 0 });
	self.removeTimer(rocketTimer.get());
	rocketTimer.set(self.addTimer(1, -1, rideRocket, { persistent: true, pauseCondition: frozen }));
}
// a speed given, then `decay` less each source frame down to nothing, until its attack is over
function rideRocket() {
	if (self.getAnimation() != rocketAnim.get()) { self.removeTimer(rocketTimer.get()); rocketTimer.set(-1); return; }
	var r = rocketSpeed.get();
	if (r.n >= 2 && r.n % 2 == 0) {
		r.vx = r.vx > 0 ? Math.max(0, r.vx - r.dx) : Math.min(0, r.vx + r.dx);
		r.vy = r.vy > 0 ? Math.max(0, r.vy - r.dy) : Math.min(0, r.vy + r.dy);
	}
	r.n++;
	self.setXSpeed(r.vx * SPEED);
	self.setYSpeed(r.vy * SPEED);
}

// ---- fresh_clip
//: its cases are each clip class's fields, back to how they start
// a clip made anew: the source makes a fresh one each time the fighter comes to it from another,
// its fields as they start (a loop or a jump inside it keeps the one it has). this runs first
// thing on the clip's first frame, by what was shown the update before
var shownBefore = self.makeObject(null);
function freshClip(c) {
	if (classOf(shownBefore.get()) == c) return;
	switch (c) {
		// {cases}
	}
}
function shownFrame() {
	var was = shownBefore.get();
	var now = self.getAnimation();
	// (the clip it showed is gone: what it listened for on the fighter goes with it)
	if (was != null && classOf(was) != classOf(now)) clipLeft(classOf(was));
	shownBefore.set(now);
}

// what a clip of its animations listens for on the fighter belongs to that clip, and goes when the
// fighter leaves it for another clip (the source's clip and its code are gone by then)
var clipListeners = self.makeObject([]);
function clipListen(c, ev, fn) {
	clipListeners.get().push({ c: c, ev: ev, fn: fn });
}
function clipLeft(c) {
	var keep = [];
	for (l in clipListeners.get()) if (l.c == c) self.removeEventListener(l.ev, l.fn); else keep.push(l);
	clipListeners.set(keep);
}

// ---- timers
// a timer its code makes (`createTimer(frames, repeats, fn)`): two engine frames to a source frame,
// forever for none. it drops with the state it was made in unless asked to stay, waits out the
// fighter's freezes, and one made in a frame's code comes due two engine frames sooner the first
// time (the source ticks the frame it is made in too). the record is what its code stops it by
function startTimer(n, times, fn, keep, inFrame) {
	var t = { id: -1, left: times, dead: false, fn: fn };
	var opts = { persistent: keep, pauseCondition: frozen };
	var first = inFrame ? 2 * n - 2 : 2 * n;
	if (first == 2 * n) { t.id = self.addTimer(2 * n, times, function() { tickTimer(t); }, opts); return t; }
	t.id = self.addTimer(first < 1 ? 1 : first, 1, function() {
		// (its first run is that timer's last: its id is the engine's to give out again)
		t.id = -1;
		tickTimer(t);
		if (!t.dead && t.left != 0) t.id = self.addTimer(2 * n, t.left, function() { tickTimer(t); }, opts);
	}, opts);
	return t;
}
function tickTimer(t) {
	if (t.dead) return;
	// (one that has run its last forgets the engine's id, so stopping it later stops nothing else)
	if (t.left > 0) { t.left--; if (t.left == 0) { t.dead = true; t.id = -1; } }
	t.fn();
}
function stopTimer(t) {
	if (t == null) return;
	t.dead = true;
	// (the engine gives ids out again, one object's timers counting from one: a stale one would
	// stop another of its timers)
	if (t.id >= 0) self.removeTimer(t.id);
	t.id = -1;
}

// ---- ledges
// the stage's ledges, where each is in the source's units
function stageLedges() {
	var out = [];
	for (st in match.getStructures()) {
		if (st.getAllPoints == null) continue;
		var pts = st.getAllPoints();
		if (pts == null || pts.length < 2) continue;
		if (st.getStructureStat("leftLedge") == true) out.push({ x: pts[0].x / SIZE, y: pts[0].y / SIZE });
		if (st.getStructureStat("rightLedge") == true) { var p = pts[pts.length - 1]; out.push({ x: p.x / SIZE, y: p.y / SIZE }); }
	}
	return out;
}

// ---- own_floor
// a floor of its own, the size given (the source's units, from where it is): one at a time, a
// platform it stands on and drops through unless it is asked for as terrain, carried with it
var ownFloors = self.makeObject([]);
function makeOwnFloor(x, y, w, h, ?terrain) {
	dropOwnFloor();
	var x0 = Math.min(x, x + w) * SIZE;
	var x1 = Math.max(x, x + w) * SIZE;
	var y0 = y * SIZE;
	var st = match.createLineSegmentStructure([x0, y0, x1, y0], null);
	if (st == null) return null;
	var solid = terrain == true;
	st.updateStructureStats({ structureType: StructureType.FLOOR, dropThrough: !solid, leftLedge: solid, rightLedge: solid });
	st.addToBlacklist(self);
	st.setX(self.getX());
	st.setY(self.getY());
	ownFloors.get().push(st);
	return st;
}
function dropOwnFloor() {
	for (st in ownFloors.get()) { st.updateStructureStats({ disabled: true }); st.setX(-2000); st.setY(-3000); }
	ownFloors.set([]);
}
function ownFloorFrame() {
	for (st in ownFloors.get()) { st.setX(self.getX()); st.setY(self.getY()); }
}

// ---- ground_test
// the nearest surface on the line between two spots (the source's units): terrain, platforms or
// both as the options ask (`terrains`, `platforms`), none in `ignoreList`
function groundBetween(ax, ay, bx, by, ?o) {
	var terrains = o == null || (o.terrains != false && o.terrain != false);
	var platforms = o == null || o.platforms != false;
	var ignore = o == null ? null : o.ignoreList;
	var p1 = Point.create(ax * SIZE, ay * SIZE);
	var p2 = Point.create(bx * SIZE, by * SIZE);
	var best = null;
	var bestD = 0.0;
	for (k in [StructureType.FLOOR, StructureType.CEILING, StructureType.LEFT_WALL, StructureType.RIGHT_WALL]) {
		var out = Point.create(p2.x, p2.y);
		var hits = self.hitTestStructuresWithLineSegment(p1, p2, out, { structureType: k });
		if (hits == null) continue;
		for (st in hits) {
			var drop = st.getStructureStat("dropThrough") == true;
			if (drop ? !platforms : !terrains) continue;
			if (ignore != null && ignore.indexOf(st) >= 0) continue;
			var d = (out.x - p1.x) * (out.x - p1.x) + (out.y - p1.y) * (out.y - p1.y);
			if (best == null || d < bestD) { best = st; bestD = d; }
		}
	}
	return best;
}

// ---- recover
// damage it heals, down to none: whether it had any
function recover(n) {
	var d = self.getDamage();
	self.setDamage(Math.max(0, d - n));
	return d > 0;
}
