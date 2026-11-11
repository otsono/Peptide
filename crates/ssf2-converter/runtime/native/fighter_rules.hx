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
	// (an attack's first frame reads its own: its code runs before the frame's update would see it begin)
	attackFrame();
	var w = attackWritten.get();
	if (w.exists(k)) return w.get(k);
	return attackIn.get() == null ? null : declared(attackIn.get(), k);
}

// stats its code writes on the attack: kept for the rest of it
function attackWrite(k, v) {
	attackFrame();
	attackWritten.get().set(k, v);
}

// every frame: an attack's start and end, on the frame they happen
var attackTurning = false;
function attackFrame() {
	// (what runs as one ends reads that one's stats: it is still the attack it is in)
	if (attackTurning) return;
	var label = (self.inStateGroup(CStateGroup.ATTACK) || throwing()) ? attackLabel(self.getAnimation()) : null;
	if (label == attackIn.get()) return;
	attackTurning = true;
	attackEnded();
	attackTurning = false;
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
// an item it generates: a custom game object of its own riding its item box until it is thrown.
// the item's handler takes the hold and the throw as asks on its exports (`__holder`, `__ask`)
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
	if (it != null && it.exports != null && it.exports.__ask != null) it.exports.__ask(kind, v);
}
function generateItem(name, ?by) {
	if (name == "random" && by == null) name = anyItemName();
	var id = by != null ? null : itemContent(name);
	var content = id == null ? null : self.getResource().getContent(id);
	// (one of another fighter's, made by that one through the power it publishes)
	if (by != null) {
		var rec = by.exports == null ? null : by.exports.__kirbyPower;
		var made = rec == null || rec.item == null ? null : rec.item(name, self);
		return made == null ? null : holdMade(made, name);
	}
	// (a custom game object carrying the item handler, which keeps itself in the hand: its own, or
	// one the power it holds makes)
	var it = content != null ? match.createCustomGameObject(content, self) : (self.exports.__powerItem != null ? self.exports.__powerItem(name) : null);
	if (it == null) return null;
	return holdMade(it, name);
}
// an item another object of its asks it to make (one item opening into another), lying free
// where that one says
function makeItem(name, ?x, ?y) {
	if (name == "random") name = anyItemName();
	var id = itemContent(name);
	var content = id == null ? null : self.getResource().getContent(id);
	if (content == null) return null;
	var it = match.createCustomGameObject(content, self);
	it.exports.__isItem = true;
	it.exports.__np = true;
	it.exports.__itemName = name;
	if (x != null) { it.setX(x); it.setY(y); }
	return it;
}
// an item it has made, held from now on
function holdMade(it, name) {
	self.exports.__tossItem = tossItem;
	self.exports.__makeItem = makeItem;
	self.exports.__dropItem = dropItem;
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
// the nearest object of a kind ("character", "player", "item", "projectile", "enemy")
function nearest(type) {
	var list = (type == "character" || type == "player") ? match.getCharacters() : (type == "enemy" || type == "item" ? match.getCustomGameObjects() : match.getProjectiles());
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
//: `classOf` are written for it from its animations and its classes
// an attack another object's code starts on it, by the source's label, at a label of its clip
function forceAttackAsked(label, ?frame) {
	var anim = animOf(label);
	if (anim == null) {
		// {cases}
		return;
	}
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
// the clip it showed the update before: a clip's frame code declares its fields as the fighter comes
// to it from another (a loop or a jump inside it keeps the one it has)
var shownBefore = self.makeObject(null);
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

// ---- mid_jump
// a jump to a label inside another animation: the engine runs an animation's first frame as it
// starts it, and the source goes straight to the label's frame, so that frame's own code waits
// this out (`if (!midJump.get())`)
var midJump = self.makeObject(false);
function playLabelIn(anim, label) {
	if (self.getAnimation() != anim) {
		midJump.set(true);
		self.playAnimation(anim);
		midJump.set(false);
	}
	self.playFrameLabel(label);
}

// ---- tick_end
// what its code listens for at the end of every frame of the game: the end of its own update, in
// the order its code asked
var tickEnds = self.makeObject([]);
function listenTickEnd(fn) {
	if (tickEnds.get().indexOf(fn) < 0) tickEnds.get().push(fn);
}
function unlistenTickEnd(fn) {
	tickEnds.get().remove(fn);
}
function tickEndFrame() {
	for (fn in tickEnds.get().copy()) fn(null);
}

// ---- tint
// a colour transform its code sets on its clip: the fighter shows it as one colour shader, the
// transform's values copied in as it is set (changing the transform after shows nothing until it
// is set again, as the source's does)
var tintShown = self.makeObject(null);
function showTint(ct) {
	var sh = tintShown.get();
	if (sh == null) {
		sh = new RgbaColorShader();
		self.addShader(sh);
		tintShown.set(sh);
	}
	sh.redMultiplier = ct.redMultiplier;
	sh.greenMultiplier = ct.greenMultiplier;
	sh.blueMultiplier = ct.blueMultiplier;
	sh.alphaMultiplier = ct.alphaMultiplier;
	sh.redOffset = ct.redOffset;
	sh.greenOffset = ct.greenOffset;
	sh.blueOffset = ct.blueOffset;
	sh.alphaOffset = ct.alphaOffset;
}

// ---- frame_links
// a listener a clip's code adds lives one animation, as the engine has it, and the clip goes on
// when the engine links its animation to another at the same frame (the fighter lands or leaves
// the ground mid-move): the link drops it, so it is added again in the animation linked to. what
// the clip's code listens for, with the animation it lives in
var linkListeners = self.makeObject([]);
var linkShownBefore = self.makeObject(null);
function linkListen(ev, fn) {
	linkListeners.get().push({ a: self.getAnimation(), ev: ev, fn: fn });
}
function linkUnlisten(ev, fn) {
	var keep = [];
	for (l in linkListeners.get()) if (l.ev != ev || l.fn != fn) keep.push(l);
	linkListeners.set(keep);
}
// (the engine tells of the link once the fighter is in the animation linked to)
function framesLinked(e) {
	var now = self.getAnimation();
	for (l in linkListeners.get()) if (l.a != now) {
		self.addEventListener(l.ev, l.fn, { persistent: false });
		l.a = now;
	}
}
function listenForLinks() {
	self.addEventListener(GameObjectEvent.LINK_FRAMES, framesLinked, { persistent: true });
}
// an animation left any other way takes what it listened for with it
function linkFrame() {
	var now = self.getAnimation();
	if (now == linkShownBefore.get()) return;
	linkShownBefore.set(now);
	var keep = [];
	for (l in linkListeners.get()) if (l.a == now) keep.push(l);
	linkListeners.set(keep);
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
// the stage's ledge nearest it (the source's units), or null where the stage has none
function nearestLedge() {
	var best = null;
	var bd = 0.0;
	for (l in stageLedges()) {
		var dx = l.x - self.getX() / SIZE;
		var dy = l.y - self.getY() / SIZE;
		var d = dx * dx + dy * dy;
		if (best == null || d < bd) { best = l; bd = d; }
	}
	return best;
}

// ---- home
// it heads straight for the one given at a speed (the source's units), facing the way it goes
function homeTowards(speed, target) {
	if (target == null) return;
	var a = Math.atan2(target.getY() - self.getY(), target.getX() - self.getX());
	var xs = speed * Math.cos(a);
	if (xs <= 0) self.faceLeft(); else self.faceRight();
	self.setXSpeed(Math.abs(xs) * SPEED);
	self.setYSpeed(speed * Math.sin(a) * SPEED);
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

// ---- kirby_power
//: POWER the power it declares (`power`, in its stats); SIZE the source's size here. its cases are
//: each projectile it declares: its content, offset and launch (already in this engine's speed)
// the power it gives a fighter that swallows it, on its exports (`__kirbyPower`): the name it
// declares, and its own projectiles, fired for the one that has its power as if that one fired them
function powerShot(name) {
	switch (name) {
		// {cases}
	}
	return null;
}
function firePower(name, owner, ?x, ?y) {
	var d = powerShot(name);
	if (d == null || owner == null || owner.isDisposed()) return null;
	if (x == null) x = 0;
	if (y == null) y = 0;
	var px = owner.getX() + owner.flipX((x + d.dx) * SIZE);
	var py = owner.getY() + (y + d.dy) * SIZE;
	if (owner.isOnFloor() && py > owner.getY()) py = owner.getY();
	// (the launch handed over first, as its own throws hand theirs: one on the runtime takes it up)
	var launch = { x: px, y: py, xs: null, ys: null, name: name, decl: null, g: 0, max: null };
	if (d.xs != 0 || d.ys != 0) { launch.xs = owner.flipX(d.xs); launch.ys = d.ys; }
	owner.exports.__launch = launch;
	var p = match.createProjectile(self.getResource().getContent(d.content), owner);
	if (p == null) { owner.exports.__launch = null; return null; }
	if (p.exports != null) p.exports.__thrownRight = p.isFacingRight();
	if (owner.exports.__launch != null) {
		owner.exports.__launch = null;
		p.exports.__launched = launch;
		p.setX(px);
		p.setY(py);
		if (launch.xs != null) { p.setXSpeed(p.flipX(launch.xs)); p.setYSpeed(launch.ys); }
	}
	p.exports.__shotName = name;
	owner.exports.__lastProj = p;
	return p;
}

// ---- kirby_power_item
// an item of its own made for the one that has its power, which holds it as its own
function powerItemFor(name, holder) {
	var id = itemContent(name);
	var content = id == null ? null : self.getResource().getContent(id);
	if (content == null || holder == null) return null;
	return match.createCustomGameObject(content, holder);
}

// ---- kirby_power_inner
// a clip inside one of its effects made for the one that has its power, sent to a frame
function powerInnerFor(effect, clip, frame, hold) {
	var c = innerClip(effect, clip);
	if (c == null) return;
	if (frame != null) effectFrame(c, frame, hold); else if (hold) c.pause(); else c.resume();
}

// ---- act_as
// its own code run as another fighter: `actAs` points this script's `self` at that fighter while
// `fn` runs, so every function here the code calls acts on it (content is still found in this
// fighter's own files: its script reads them through `ownResource`). `moveCode` is the code each
// frame of its neutral special runs, by animation and frame: a fighter that copies the move runs it
// as itself, frame by frame, through these two
function actAs(me, fn) {
	var own = self;
	self = me;
	try { fn(); } catch (e:Dynamic) { self = own; throw e; }
	self = own;
}
function moveCode(anim, frame) {
	// {cases}
}

// ---- copy_power
//: a fighter whose code takes another's power (`KirbyPower`, `getCurrentKirbyPower`). the
//: power is the record the one it swallowed publishes (`exports.__kirbyPower`: `id`, and `fire`,
//: which fires that one's projectiles for this one); any fighter can publish one
// the power it holds, and how much damage it has left to take before it loses it
var power = self.makeObject(null);
var powerDamageLeft = self.makeObject(0);
var powerDamageWas = self.makeObject(0);
function powerName() {
	var p = power.get();
	return p == null ? null : p.id;
}
// a power taken: the record its move was handed (or a name its code gives), else none
function takePower(v) {
	var p = v == null ? null : (Std.isOfType(v, String) ? { id: v, fire: null } : v);
	if (p != null && (p.id == null || p.id == "")) p = null;
	power.set(p);
	// (the one swallowed is let out as the move takes its power)
	if (full.get() != null) letOut(full.get(), false);
	powerDamageLeft.set(45);
	powerDamageWas.set(self.getDamage());
}
// lost: a taunt, or enough damage taken while it holds it
function dropPower() {
	if (power.get() == null) return;
	power.set(null);
}
// a projectile it does not declare, fired from the power it holds as the one it copied fires it
function firePowerShot(name, ?x, ?y) {
	var p = power.get();
	if (p == null || p.fire == null) return null;
	return p.fire(name, self, x, y);
}
// an effect it has no art for, made by the one it copied (none where that one has no such effect)
function powerEffect(name, on, options) {
	var p = power.get();
	if (p == null || p.effect == null) return null;
	return p.effect(name, on, options);
}
// a clip inside an effect of the power it holds, sent to a frame by the one it copied
function powerInner(effect, clip, frame, hold) {
	var p = power.get();
	if (p != null && p.inner != null) p.inner(effect, clip, frame, hold);
}
// an item it does not declare, made by the one it copied (its own `generateItem` holds it)
function powerItem(name) {
	var p = power.get();
	if (p == null || p.item == null) return null;
	return p.item(name, self);
}
//: COPY_MOVES the source's names for its copy move (each animation showing the clip whose code
//: takes the power), SWALLOW and SPIT the labels in that clip where it swallows and spits, SUCKING
//: the `xframe` its first frame sets, POWER_FIELD the field it reads the power from: all read off
//: its clip (`native_fighter::copy_move`); STRIKE_FRAMES each of its animations' first hitbox frame,
//: read off its entity (`native_fighter::strike_frames`). the numbers are the source game's own
// full: one it grabbed in its copy move, holding no power, is held inside it, unseen, where it
// stands, struggling out over 60 of the source's frames (sooner as it mashes); an attack spits it out,
// a special or down swallows it: the move plays from its start and, once its clip says it is drawing
// one in, the one inside takes 6 damage, its power goes to the move's power field, and the move swallows
var full = self.makeObject(null);
var fullHold = self.makeInt(0);
var fullNext = self.makeObject(null);
function letOut(foe, spat) {
	full.set(null);
	if (foe == null || foe.isDisposed()) return;
	foe.setVisible(true);
	foe.setX(self.getX() + self.flipX(spat ? 20 : 0) * SIZE);
	foe.setY(self.getY());
	if (spat) { foe.setXSpeed(self.flipX(9) * SPEED); foe.setYSpeed(-4 * SPEED); }
	else { foe.setYSpeed(-8 * SPEED); self.setXSpeed(self.flipX(-9) * SPEED); }
}
function powerFrame() {
	borrowedFrame();
	if (power.get() != null) {
		if (self.inState(CState.TAUNT)) { dropPower(); return; }
		var d = self.getDamage();
		if (d > powerDamageWas.get()) powerDamageLeft.set(powerDamageLeft.get() - (d - powerDamageWas.get()));
		powerDamageWas.set(d);
		if (powerDamageLeft.get() <= 0) dropPower();
		return;
	}
	var foe = full.get();
	var label = rootLabel();
	if (foe == null) {
		var held = self.getAllGrabbedFoes();
		if (held == null || held.length == 0 || COPY_MOVES.indexOf(label) < 0) return;
		foe = held[0];
		self.releaseAllCharacters();
		full.set(foe);
		fullHold.set(120);
		fullNext.set(null);
		foe.setVisible(false);
		self.toState(self.isOnFloor() ? CState.STAND : CState.FALL);
		return;
	}
	if (foe.isDisposed()) { full.set(null); return; }
	// (the one inside goes with it, and does nothing)
	foe.setX(self.getX());
	foe.setY(self.getY());
	foe.setXSpeed(0);
	foe.setYSpeed(0);
	foe.forceStartHitstop(2, false);
	var asked = fullNext.get();
	if (asked != null) {
		// (the move it was asked for, once its clip says it is drawing one in: the `xframe` its code sets)
		var xf = fieldOf("stance", "xframe");
		if (xf == null || xf.get() != SUCKING) return;
		fullNext.set(null);
		if (asked == "swallow") {
			var f = fieldOf("stance", POWER_FIELD);
			if (f != null) f.set(powerOf(foe));
			foe.addDamage(6);
			stanceJump(SWALLOW);
		} else {
			if (SPIT != null) stanceJump(SPIT);
			letOut(foe, true);
		}
		return;
	}
	// (each engine frame half a source frame; each control the one inside presses, a frame more)
	var c = foe.getPressedControls();
	var struggle = c == null ? 0 : (c.ATTACK || c.SPECIAL || c.JUMP || c.SHIELD || c.UP || c.DOWN || c.LEFT || c.RIGHT ? 2 : 0);
	fullHold.set(fullHold.get() - 1 - struggle);
	if (fullHold.get() <= 0) { letOut(foe, false); return; }
	var mine = self.getPressedControls();
	if (mine == null) return;
	if (mine.ATTACK) { fullNext.set("spit"); forceAttackAsked(COPY_MOVES[0]); }
	else if (mine.SPECIAL || mine.DOWN) { fullNext.set("swallow"); forceAttackAsked(COPY_MOVES[0]); }
}

// the power a swallowed fighter gives: the record it publishes, if it publishes one, and in any case
// its neutral special as it plays it itself (`rehearse`), so a fighter this one has no art for, made
// anywhere, still lends it its move
function powerOf(foe) {
	var p = foe.exports == null ? null : foe.exports.__kirbyPower;
	var rec = { id: null, fire: null, effect: null, inner: null, item: null, body: null };
	if (p != null) { rec.id = p.id; rec.fire = p.fire; rec.effect = p.effect; rec.inner = p.inner; rec.item = p.item; }
	if (rec.id == null) rec.id = foe.getGameObjectStat("spriteContent");
	var ground = rehearse(foe, "special_neutral");
	var air = rehearse(foe, "special_neutral_air");
	// (its frame code too, where it lets another fighter run it as itself: `act_as`)
	var act = foe.exports == null ? null : foe.exports.__actAs;
	var code = foe.exports == null ? null : foe.exports.__moveCode;
	if (ground != null || air != null) rec.body = { ground: ground, air: air, act: act, code: code };
	return rec;
}

// one of its moves played through by the fighter itself, unseen while it is held, and written down
// frame by frame: the frame it shows, the hitboxes on it (and their stats), the projectiles its own
// code makes there (taken away again) and the speed it gives itself. it stops where its code ends the
// move, where the animation runs out, or where its code goes back to a frame it has shown already
function rehearse(foe, anim) {
	if (!foe.hasAnimation(anim)) return null;
	var was = { anim: foe.getAnimation(), frame: foe.getCurrentFrame(), state: foe.getState(), right: foe.isFacingRight() };
	var known = match.getProjectiles().copy();
	foe.faceRight();
	foe.setState(CState.SPECIAL_NEUTRAL);
	foe.setXSpeed(0);
	foe.setYSpeed(0);
	foe.playAnimation(anim);
	var mid = bodyMiddle(foe);
	var frames = [];
	var seen = [];
	while (frames.length < 300) {
		var at = foe.getAnimation() + "#" + foe.getCurrentFrame();
		if (seen.indexOf(at) >= 0) break;
		seen.push(at);
		frames.push({ anim: foe.getAnimation(), frame: foe.getCurrentFrame(), hits: rehearsedHits(foe), shots: rehearsedShots(foe, known), xs: foe.getXSpeed(), ys: foe.getYSpeed() });
		if (foe.getState() != CState.SPECIAL_NEUTRAL) break;
		if (foe.getCurrentFrame() >= foe.getTotalFrames()) break;
		foe.setXSpeed(0);
		foe.setYSpeed(0);
		foe.playFrame(foe.getCurrentFrame() + 1);
	}
	foe.setState(was.state);
	foe.playAnimation(was.anim);
	foe.playFrame(was.frame);
	if (was.right) foe.faceRight(); else foe.faceLeft();
	return frames.length == 0 ? null : { frames: frames, mid: mid };
}
// how high the middle of its body is over its feet, from its hurtboxes
function bodyMiddle(foe) {
	var hurt = foe.getCollisionBoxes(CollisionBoxType.HURT);
	if (hurt == null || hurt.length == 0) return 0;
	var top = 0.0; var bottom = 0.0;
	for (o in hurt) { top = Math.min(top, o.y); bottom = Math.max(bottom, o.y + o.height); }
	return (top + bottom) / 2;
}
// the hitboxes it shows now, each with its number and stats (its boxes tested against themselves
// are the one way to learn which hitbox each is)
function rehearsedHits(foe) {
	var out = [];
	var found = foe.hitTestEntity(foe, CollisionBoxType.HIT, CollisionBoxType.HIT);
	if (found == null) return out;
	var ids = [];
	for (c in found) {
		var b = c.boxA;
		if (ids.indexOf(b.index) >= 0) continue;
		ids.push(b.index);
		out.push({ id: b.index, x: b.x, y: b.y, w: b.width, h: b.height, stats: foe.getHitboxStats(b.index, foe.getAnimation()) });
	}
	return out;
}
// the projectiles its code made this frame, where they are and how they move from it; each is
// taken away at once (it is a rehearsal)
function rehearsedShots(foe, known) {
	var out = [];
	for (p in match.getProjectiles()) {
		if (known.indexOf(p) >= 0) continue;
		out.push({ content: p.getGameObjectStat("spriteContent"), x: p.getX() - foe.getX(), y: p.getY() - foe.getY(), xs: p.getXSpeed(), ys: p.getYSpeed() });
		p.kill();
	}
	return out;
}

// a move it asked for and has no animation for, while it holds a power that came with a move: that
// power's move, its hitboxes, projectiles and speeds as the fighter it came from played them, done in
// this fighter's own art. it stays in its neutral special on its own body (standing or falling, so it
// is hit where it is) and shows one of its own standard poses over the move's length: shooting when
// the move makes projectiles, else a tilt (an aerial in the air) toward where its hitboxes reach.
// the pose is drawn, not played, so the pose's own hitboxes and code stay out of it
var borrowed = self.makeObject(null);
var borrowedAt = self.makeInt(0);
var borrowedHit = self.makeObject(null);
// (the drawing is not game state: it is redrawn from `borrowedAt` every frame)
var borrowedArt = null;
function borrowedMove(label) {
	var p = power.get();
	if (p == null || p.body == null) return false;
	var floor = self.isOnFloor();
	var move = floor ? p.body.ground : p.body.air;
	if (move == null) move = floor ? p.body.air : p.body.ground;
	if (move == null) return false;
	var lands = -1;
	for (i in 0...move.frames.length) if (lands < 0 && (move.frames[i].hits.length > 0 || move.frames[i].shots.length > 0)) lands = i;
	borrowed.set({ frames: move.frames, pose: borrowedPose(move, floor), lands: lands, act: p.body.act, code: p.body.code });
	borrowedAt.set(0);
	borrowedHit.set([]);
	self.toState(CState.SPECIAL_NEUTRAL, floor ? "stand" : "fall_loop");
	return true;
}
// the pose of its own that fits the move: what the move does, read off its recording
function borrowedPose(move, floor) {
	var shoots = false; var dx = 0.0; var dy = 0.0; var n = 0;
	for (f in move.frames) {
		if (f.shots.length > 0) shoots = true;
		for (h in f.hits) { dx += h.x + h.w / 2; dy += h.y + h.h / 2 - move.mid; n++; }
	}
	var pose = null;
	if (shoots) pose = floor ? "item_shoot" : "item_throw_air_forward";
	else if (n > 0) {
		dx /= n; dy /= n;
		var way = dy < -Math.abs(dx) ? "up" : (dy > Math.abs(dx) ? "down" : "forward");
		pose = (floor ? "tilt_" : "aerial_") + way;
	}
	return pose != null && self.hasAnimation(pose) ? pose : null;
}
function borrowedFrame() {
	var b = borrowed.get();
	if (b == null) { if (borrowedArt != null) borrowedDone(); return; }
	if (!self.inState(CState.SPECIAL_NEUTRAL)) { borrowedDone(); return; }
	// (in hitstop it holds its frame)
	var t = borrowedAt.get();
	if (self.getHitstop() > 0 && t > 0) { borrowedDraw(b, t - 1); return; }
	if (t >= b.frames.length) {
		borrowedDone();
		self.toState(self.isOnFloor() ? CState.STAND : CState.FALL);
		return;
	}
	var f = b.frames[t];
	borrowedAt.set(t + 1);
	borrowedDraw(b, t);
	// (the fighter it came from runs that frame's own code as this one, where it lets it; else what
	// the rehearsal saw that code do is done again)
	if (b.act != null && b.code != null) {
		b.act(self, function() { b.code(f.anim, f.frame); });
		for (h in f.hits) borrowedStrike(h);
		return;
	}
	if (f.xs != 0 || f.ys != 0) { self.setXSpeed(self.flipX(f.xs)); self.setYSpeed(f.ys); }
	for (s in f.shots) {
		var q = match.createProjectile(s.content, self);
		if (q == null) continue;
		q.setX(self.getX() + self.flipX(s.x));
		q.setY(self.getY() + s.y);
		q.setXSpeed(self.flipX(s.xs));
		q.setYSpeed(s.ys);
		if (self.isFacingRight()) q.faceRight(); else q.faceLeft();
	}
	for (h in f.hits) borrowedStrike(h);
}
// a hitbox of the move on each other fighter it touches, once a hitbox a move, with that hitbox's stats
function borrowedStrike(h) {
	var done = borrowedHit.get();
	for (foe in match.getCharacters()) {
		if (foe == self || foe.isDisposed()) continue;
		var key = foe.getUid() + ":" + h.id;
		if (done.indexOf(key) >= 0 || !touches(h, foe)) continue;
		borrowedHit.set(done.concat([key]));
		done = borrowedHit.get();
		self.syncHitboxStats(h.stats);
		self.attemptHit(foe, h.stats, null);
	}
}
// whether a box of its move, where it stands and as it faces, overlaps one of the other's hurtboxes
function touches(h, foe) {
	var hurt = foe.getCollisionBoxes(CollisionBoxType.HURT);
	if (hurt == null) return false;
	var ax = self.getX() + (self.isFacingRight() ? h.x : -(h.x + h.w));
	var ay = self.getY() + h.y;
	for (o in hurt) {
		var bx = foe.getX() + (foe.isFacingRight() ? o.x : -(o.x + o.width));
		var by = foe.getY() + o.y;
		if (ax < bx + o.width && bx < ax + h.w && ay < by + o.height && by < ay + h.h) return true;
	}
	return false;
}
// its own pose drawn in its own colours over its hidden body, timed so the pose lands its blow on
// the frame the move first lands its own (STRIKE_FRAMES: where each of its animations first shows a
// hitbox, as [animation, frame] pairs), and stretched evenly where the pose has none
var strikeFrames = STRIKE_FRAMES;
function borrowedDraw(b, t) {
	if (b.pose == null) return;
	if (borrowedArt == null || borrowedArt.isDisposed()) {
		borrowedArt = Sprite.create(self.getGameObjectStat("spriteContent"));
		self.getTopLayer().addChild(borrowedArt);
	}
	// (its colours go on after its animation: showing another animation drops them)
	if (borrowedArt.currentAnimation != b.pose) {
		borrowedArt.currentAnimation = b.pose;
		var shader = self.getCostumeShader();
		if (shader != null) borrowedArt.setPalette(shader.paletteMap);
	}
	var all = borrowedArt.totalFrames;
	var strike = null;
	for (p in strikeFrames) if (p[0] == b.pose) strike = p[1];
	var frame = 1 + Math.floor(t * all / b.frames.length);
	if (strike != null && b.lands > 0 && t < b.lands) frame = 1 + Math.floor(t * (strike - 1) / b.lands);
	else if (strike != null && b.lands >= 0) frame = strike + Math.floor((t - b.lands) * (all - strike + 1) / (b.frames.length - b.lands));
	borrowedArt.currentFrame = Std.int(Math.min(frame, all));
	self.getSprite().visible = false;
}
function borrowedDone() {
	borrowed.set(null);
	if (borrowedArt != null) { borrowedArt.remove(); borrowedArt = null; }
	self.getSprite().visible = true;
}
