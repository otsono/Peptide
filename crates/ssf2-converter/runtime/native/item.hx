// {title}: converted from SSF2 by Peptide.
//
// written as Fraymakers code: an item, a custom game object. its stats files hold what its package
// declares, and below is what its own code ({classes}) does, as this engine's calls, on top of
// the SSF2ItemHandler every item carries (the item's states, being picked up, held and thrown,
// and the events it raises). the handler comes from runtime/native/item.hx; edit that file, not
// this one.

// ---- its state, in the engine's containers (a rollback puts these back)
// how many engine frames it lives while nobody holds it (two to each of the source's), and how many it has
var life = self.makeInt(LIFE);
var age = self.makeInt(0);
// whether its first frame's code has run (the source runs it once its maker's code is done)
var started = self.makeBool(false);
// how many engine frames it has been out, held or not
var frames = self.makeInt(0);
// {fields}

// ---- SSF2ItemHandler: what every item does
// its state, by the source's numbers: free (0), held (1), thrown (2), gone (3)
var itemState = self.makeInt(0);
// who holds it, and whether that one picked it up here (then this handler throws it for them:
// a fighter that made it throws it itself)
var holder = self.makeObject(null);
var pickedUp = self.makeBool(false);
// whether it can be picked up now (its code turns this off: `canPickup`)
var pickupOn = self.makeBool(true);
// whether it was let go without a throw (its holder was hit or grabbed)
var zDropped = self.makeBool(false);
// what its holder's attack is put to first, while it is held (`setFrameInterrupt`)
var frameInterrupt = self.makeObject(null);
// the attack its holder was last seen starting, by the source's name for it
var holderAttack = self.makeObject(null);
// its own listeners for the events this handler raises: picked up, thrown
var pickupListeners = self.makeObject([]);
var tossListeners = self.makeObject([]);
// what it declares: thrown at this speed (source units), can be picked up, boxes to the size
var TOSS_SPEED = TOSS;
var CAN_PICKUP = PICKUP;
var HALF_WIDTH = WIDTH * SIZE / 2;
var HEIGHT_UP = HEIGHT * SIZE;

function inItemState(n) { return itemState.get() == n; }
// its state changed: it shows the clip that state has (held, thrown, lying free)
function itemTo(n) {
	if (itemState.get() == n) return;
	itemState.set(n);
	var a = n == 1 ? "projectileHeld" : n == 2 ? "projectileActive" : n == 0 ? IDLE_ANIM : null;
	if (a != null && self.hasAnimation(a)) self.playAnimation(a);
}
// a jump to a label: in the clip its state shows (held: `hold_`, thrown: `toss_`, else its own)
function playStance(label) {
	var a = (inItemState(1) ? "hold_" : inItemState(2) ? "toss_" : "") + label;
	if (self.hasAnimation(a)) self.playAnimation(a);
	else if (self.hasAnimation(label)) self.playAnimation(label);
}
// the label its clip is on: the animation it plays, as the label it was played by (`playStance`)
function stanceLabel() {
	var a = self.getAnimation();
	if (a == null) return null;
	if (a.indexOf("hold_") == 0) return a.substr(5);
	if (a.indexOf("toss_") == 0) return a.substr(5);
	return a;
}
function tell(listeners, e) { for (l in listeners.get().copy()) l(e); }

// held: it rides its holder, touching nothing, until it is thrown or let go
function holdBy(who, byPickup) {
	holder.set(who);
	pickedUp.set(byPickup);
	zDropped.set(false);
	holderAttack.set(null);
	self.setOwner(who);
	self.updateGameObjectStats({ ghost: true, gravity: 0, deathBoundsDestroy: false });
	self.setXSpeed(0);
	self.setYSpeed(0);
	itemTo(1);
	tell(pickupListeners, { holder: who });
}
// thrown (or let go, `dropped`): it flies by its own stats again, and whoever listens is told
function letGo(dropped) {
	var who = holder.get();
	holder.set(null);
	// (a fighter that made it holds it no longer)
	if (who != null && who.exports != null && who.exports.__heldItem == self) who.exports.__heldItem = null;
	zDropped.set(dropped);
	// (nothing slows it in the air but what its own code does)
	self.updateGameObjectStats({ ghost: GHOST, gravity: GRAVITY, terminalVelocity: FALL_CAP, deathBoundsDestroy: true, aerialFriction: 0 });
	itemTo(2);
	tell(tossListeners, { holder: who, dropped: dropped });
}
// thrown by the holder this handler holds it for, the way they hold the stick: up, down, back or ahead
function throwFromHolder() {
	var who = holder.get();
	if (who == null) return;
	var c = who.getHeldControls();
	var angle = 12;
	if (c != null && c.UP) angle = 90;
	else if (c != null && c.DOWN) angle = 270;
	else if (c != null && ((c.LEFT && who.isFacingRight()) || (c.RIGHT && !who.isFacingRight()))) angle = 158;
	var a = angle * Math.PI / 180;
	placeOn(who);
	letGo(false);
	self.setXSpeed(who.flipX(Math.cos(a) * TOSS_SPEED * SPEED));
	self.setYSpeed(-Math.sin(a) * TOSS_SPEED * SPEED);
}
// what the API's item does itself where the item's own class calls it (`super.toToss()`)
function base_toToss() { if (!inItemState(2)) letGo(false); }
function base_toHeld() { if (holder.get() != null) holdBy(holder.get(), pickedUp.get()); }
// its own code has its holder throw it (`getOwner().toToss()`): a fighter that made it throws it
// its own way, where it publishes one; otherwise the handler throws it as the holder holds the stick
function tossByHolder() {
	var who = holder.get();
	if (who == null || !inItemState(1)) return;
	if (!pickedUp.get() && who.exports != null && who.exports.__tossItem != null) who.exports.__tossItem();
	else throwFromHolder();
}
// where its holder holds it: the holder's own item point where it publishes one, else ahead of its middle
function placeOn(who) {
	if (who.exports != null && who.exports.__ibx != null) {
		self.setX(who.getX() + who.flipX(who.exports.__ibx * SIZE));
		self.setY(who.getY() + who.exports.__iby * SIZE);
	} else {
		self.setX(who.getX() + who.flipX(HALF_WIDTH + 10));
		self.setY((who.getEcbHeadY() + who.getEcbFootY()) / 2 + who.getY() - who.getEcbFootY());
	}
	if (who.isFacingRight()) self.faceRight(); else self.faceLeft();
}

// what another script asks of it (`exports.__ask`): a fighter that made it holds it, throws it,
// and puts its attack to it first; anyone turns its pickup on or off
function asked(kind, ?v) {
	if (kind == "hold") holdBy(self.exports.__holder, false);
	else if (kind == "pickup") pickupOn.set(v == true);
	else if (kind == "toss") letGo(self.exports.__zDropped == true);
	// (sent into its thrown state by another object's code: it goes up at its throw speed, as the
	// source's does with no direction given)
	else if (kind == "toToss" && inItemState(1)) {
		letGo(false);
		self.setXSpeed(0);
		self.setYSpeed(-TOSS_SPEED * SPEED);
	}
	else if (kind == "interrupt") {
		var f = frameInterrupt.get();
		if (f != null) f(v);
	}
}

// the source's names for this engine's attacks, for the one its holder starts
var attackNames = [
	[CState.JAB, "a"], [CState.DASH_ATTACK, "a_forward"], [CState.TILT_FORWARD, "a_forward_tilt"], [CState.TILT_UP, "a_up_tilt"],
	[CState.TILT_DOWN, "crouch_attack"], [CState.STRONG_FORWARD_IN, "a_forwardsmash"], [CState.STRONG_UP_IN, "a_up"],
	[CState.STRONG_DOWN_IN, "a_down"], [CState.AERIAL_NEUTRAL, "a_air"], [CState.AERIAL_FORWARD, "a_air_forward"],
	[CState.AERIAL_BACK, "a_air_backward"], [CState.AERIAL_UP, "a_air_up"], [CState.AERIAL_DOWN, "a_air_down"],
	[CState.SPECIAL_NEUTRAL, "b"], [CState.SPECIAL_SIDE, "b_forward"], [CState.SPECIAL_UP, "b_up"], [CState.SPECIAL_DOWN, "b_down"],
	[CState.GRAB, "grab"]
];
// whether a fighter is in no shape to hold it: hit, grabbed, knocked out
function knocked(who) {
	return who.inStateGroup(CStateGroup.HURT_HEAVY) || who.inState(CState.HURT_LIGHT) || who.inState(CState.HURT_MEDIUM)
		|| who.inState(CState.TUMBLE) || who.inState(CState.HELD) || who.inState(CState.FROZEN) || who.inStateGroup(CStateGroup.KO);
}
function attackOf(who) {
	for (row in attackNames) if (who.inState(row[0])) return row[1];
	return null;
}

// each frame: held, it rides its holder and is let go when the holder is hit or gone; picked up
// here, the holder's next attack is put to it first and throws it unless its own code takes the
// move; lying free, anyone touching it who presses grab picks it up
function handleItem() {
	var who = holder.get();
	if (inItemState(1) && who != null) {
		if (who.isDisposed() || knocked(who)) {
			if (!who.isDisposed()) placeOn(who);
			letGo(true);
			return;
		}
		placeOn(who);
		if (pickedUp.get()) {
			var attack = attackOf(who);
			if (attack != holderAttack.get()) {
				holderAttack.set(attack);
				if (attack != null) {
					var f = frameInterrupt.get();
					var taken = f != null && f({ targetFrame: attack, character: who, cStick: false, zair: false }) == true;
					if (!taken && inItemState(1)) throwFromHolder();
				}
			}
		}
		return;
	}
	if (CAN_PICKUP && pickupOn.get() && inItemState(0)) {
		for (c in match.getCharacters()) {
			if (c.isDisposed() || knocked(c)) continue;
			var ctl = c.getPressedControls();
			if (ctl == null || !ctl.GRAB) continue;
			if (Math.abs(c.getX() - self.getX()) > HALF_WIDTH + 30 || self.getY() < c.getY() - (c.getEcbFootY() - c.getEcbHeadY()) - HEIGHT_UP || self.getY() > c.getY() + 20) continue;
			holdBy(c, true);
			placeOn(c);
			return;
		}
	}
}
// lying free once a throw has come to rest on the floor (its own code bounces it till then)
function cameToRest() {
	if (inItemState(2) && self.isOnFloor() && Math.abs(self.getXSpeed()) < 0.5 && Math.abs(self.getYSpeed()) < 0.5) itemTo(0);
}
// its boxes hit only while it flies from a throw
function hitOnlyThrown(e) {
	if (!inItemState(2)) e.data.hitboxStats.disabled = true;
}

// ---- its own code
// {functions}

function initialize() {
	// its maker's colours
	if (self.getOwner() != null) self.setCostumeIndex(self.getOwner().getCostumeIndex());
	self.setState(PState.ACTIVE);
	self.exports.__ask = asked;
	self.exports.__isItem = true;
	self.exports.__inItemState = inItemState;
	self.addEventListener(GameObjectEvent.HITBOX_CONNECTED, hitOnlyThrown, { persistent: true });
	// {start}
	// {own_start}
	// {publish}
}

function update() {
	if (!started.get()) {
		started.set(true);
		// {first}
	}
	handleItem();
	cameToRest();
	// its lifetime runs out while nobody holds it
	if (!inItemState(1) && age.inc() >= life.get()) {
		itemTo(3);
		self.kill();
		return;
	}
	// what its own class runs each of the source's frames (every second engine frame)
	if (frames.inc() % 2 == 1) {
		// {own_tick}
	}
	// {update}
}

// its end: whoever listens for it is told
function onKill() {
	itemTo(3);
	// {ends}
}
