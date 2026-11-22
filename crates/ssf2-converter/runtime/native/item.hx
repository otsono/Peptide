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
var started = self.makeObject(false);
// how many engine frames it has been out, held or not
var frames = self.makeInt(0);
// {fields}

// ---- SSF2ItemHandler: what every item does
// its state, by the source's numbers: free (0), held (1), thrown (2), gone (3)
var itemState = self.makeInt(0);
// who holds it, and whether that one picked it up here (then this handler throws it for them:
// a fighter that made it throws it itself)
var holder = self.makeObject(null);
var pickedUp = self.makeObject(false);
// whether it can be picked up now (its code turns this off: `canPickup`)
var pickupOn = self.makeObject(true);
// whether it was let go without a throw (its holder was hit or grabbed)
var zDropped = self.makeObject(false);
// what its holder's attack is put to first, while it is held (`setFrameInterrupt`)
var frameInterrupt = self.makeObject(null);
// the attack its maker last put to it (what a throw it then asks for is thrown off)
var lastAsked = self.makeObject(null);
// the attack its holder was last seen starting, by the source's name for it
var holderAttack = self.makeObject(null);
// the engine frames left of the pickup it was taken up in, through which its holder's state is
// the pickup's and no attack of its own
var takingUp = self.makeInt(0);
// its own listeners for the events this handler raises: picked up, thrown
var pickupListeners = self.makeObject([]);
var tossListeners = self.makeObject([]);
var touchListeners = self.makeObject([]);
// whether its own code has set its boxes' stats or started its attack anew since its state last
// changed (then its boxes hit whatever state it is in)
var armed = self.makeBool(false);
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
	armed.set(false);
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
	// (and the holder's own listeners for it, `itemTossed` heard on the fighter)
	if (who != null && who.exports != null && who.exports.__tossHeard != null) for (l in who.exports.__tossHeard.copy()) l({ holder: who, dropped: dropped });
}
// thrown by the holder this handler holds it for, the way they hold the stick: up, down, back or ahead
// thrown the way its holder holds the stick (a little up along its facing, as the source's forward
// throw goes), a smash harder by the holder's own multiplier (`__tossBy`: tilt, smash)
// it leaves the hand on the holder's throw stance's TOSS_FRAME (in engine frames), where the holder
// has one (`item_throw_<way>`, `item_throw_air_<way>`), else at once
var throwing = self.makeObject(false);
function throwFromHolder() {
	var who = holder.get();
	if (who == null || throwing.get()) return;
	var c = who.getHeldControls();
	var angle = TOSS_ANGLE;
	if (c != null && c.UP) angle = 90;
	else if (c != null && c.DOWN) angle = 270;
	else if (c != null && ((c.LEFT && who.isFacingRight()) || (c.RIGHT && !who.isFacingRight()))) angle = 180 - TOSS_ANGLE;
	var by = who.exports != null && who.exports.__tossBy != null ? who.exports.__tossBy : [1, 1.2];
	var attack = holderAttack.get();
	var speed = TOSS_SPEED * (attack == "a_forwardsmash" || attack == "a_up" || attack == "a_down" ? by[1] : by[0]);
	var way = angle == 90 ? "up" : angle == 270 ? "down" : angle == TOSS_ANGLE ? "forward" : "back";
	var stance = (who.isOnFloor() ? "item_throw_" : "item_throw_air_") + way;
	// (a fighter with no throw stance of its own, one of the engine's own: the press's attack is
	// the throw, so the item takes it over, standing the fighter or letting it fall, and leaves)
	if (!who.hasAnimation(stance)) { standDown(who); standing.set(STAND_FRAMES); standingWho.set(who); leaveHand(who, angle, speed); return; }
	throwing.set(true);
	who.playAnimation(stance);
	self.addTimer(TOSS_FRAME + 1, 1, function() {
		throwing.set(false);
		if (inItemState(1) && holder.get() == who) leaveHand(who, angle, speed);
	}, { persistent: true });
}
// (the frame an attack starts, the engine keeps it over a state another object puts its fighter
// in: the item stands its thrower down for this many frames after)
var STAND_FRAMES = 3;
var standing = self.makeInt(0);
var standingWho = self.makeObject(null);
// the attack a press started, taken over by the item: the fighter stands, or falls, as it was
function standDown(who) {
	if (who == null || who.isDisposed() || !(who.inStateGroup(CStateGroup.ATTACK) || who.inState(CState.GRAB))) return;
	who.toState(who.isOnFloor() ? CState.STAND : CState.FALL);
	// (and the press it took over is spent: the engine's buffer would start the attack again)
	who.clearInputBuffer();
}
function leaveHand(who, angle, speed) {
	var a = angle * Math.PI / 180;
	placeOn(who);
	letGo(false);
	// (its x speed is along the way it faces, which `placeOn` turned to its holder's)
	self.setXSpeed(Math.cos(a) * speed * SPEED);
	self.setYSpeed(-Math.sin(a) * speed * SPEED);
}
// what the API's item does itself where the item's own class calls it (`super.toToss()`)
function base_toToss() { if (!inItemState(2)) letGo(false); }
function base_toHeld() { if (holder.get() != null) holdBy(holder.get(), pickedUp.get()); }
// its own code has its holder throw it (`getOwner().toToss()`): a fighter that made it throws it
// its own way, where it publishes one; otherwise the handler throws it as the holder holds the stick
function tossByHolder() {
	var who = holder.get();
	if (who == null || !inItemState(1)) return;
	if (!pickedUp.get() && who.exports != null && who.exports.__tossItem != null) who.exports.__tossItem(null, lastAsked.get());
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
	// (sent back into its holder's hand by another object's code)
	else if (kind == "toHeld") base_toHeld();
	// (sent into its thrown state by another object's code: it goes up at its throw speed, as the
	// source's does with no direction given)
	// (sent into its thrown state by another object's code: its state alone, its hits armed as a
	// throw's, still where its holder holds it: sandbag's up special rides its bob-omb back down
	// on its item box until it goes off)
	else if (kind == "toToss" && inItemState(1)) itemTo(2);
	else if (kind == "interrupt") {
		lastAsked.set(v == null ? null : v.targetFrame);
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
	if ((inItemState(1) || inItemState(2)) && who != null) {
		if (who.isDisposed() || knocked(who)) {
			if (!who.isDisposed()) placeOn(who);
			letGo(true);
			return;
		}
		placeOn(who);
		if (takeUpFrame(who)) return;
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
			if (c.isDisposed() || knocked(c) || !canTake(c)) continue;
			var ctl = c.getPressedControls();
			if (ctl == null || (!ctl.GRAB && !ctl.ATTACK)) continue;
			if (!within(c)) continue;
			pickUpBy(c);
			return;
		}
	}
}
// the frames it is being taken up in: the press that took it up starts an attack on the engine's
// side as well, which the pickup stands in for (it is put back in its pickup stance), and nobody
// throws it off that press (`__takingUp`, which a holder's own hand reads too)
function takeUpFrame(who) {
	var left = takingUp.get();
	if (left > 0) takingUp.set(left - 1);
	if (left > 0 && who.hasAnimation("item_pickup") && who.getAnimation() != "item_pickup" && who.inStateGroup(CStateGroup.ATTACK)) {
		who.toState(CState.EMOTE);
		who.playAnimation("item_pickup");
	}
	// (one with no pickup stance of its own: the press's attack was the pickup, taken over)
	if (left > 0 && !who.hasAnimation("item_pickup")) standDown(who);
	var taking = left > 0 || who.getAnimation() == "item_pickup";
	self.exports.__takingUp = taking;
	if (taking) holderAttack.set(attackOf(who));
	return taking;
}
// whether a fighter can take it off the floor: standing on it, holding nothing, and not yet
// into anything but the attack the press itself started (the source picks an item up where
// that press would attack)
function canTake(c) {
	if (!c.isOnFloor()) return false;
	if (c.exports != null && c.exports.__heldItem != null && !c.exports.__heldItem.isDisposed()) return false;
	// (a grab press is the same press: the engine starts its grab before the item hears it)
	if (c.inStateGroup(CStateGroup.ATTACK) || c.inState(CState.GRAB)) return c.getCurrentFrame() <= 1;
	return !c.inState(CState.SHIELD_IN) && !c.inState(CState.SHIELD_LOOP) && !c.inState(CState.SHIELD_OUT)
		&& !c.inState(CState.GRAB) && !c.inState(CState.GRAB_HOLD) && !c.inState(CState.LEDGE_LOOP) && !c.inState(CState.LEDGE_IN);
}
// whether it lies where a fighter can reach it from where it stands
function within(c) {
	return Math.abs(c.getX() - self.getX()) <= HALF_WIDTH + 30 && self.getY() >= c.getY() - (c.getEcbFootY() - c.getEcbHeadY()) - HEIGHT_UP && self.getY() <= c.getY() + 20;
}
// taken up by a fighter's own code (`pickupItem`): lying free within its reach, it is picked up
// as a press would pick it up
function takenBy(c) {
	if (c == null || c.isDisposed() || !inItemState(0) || !canTake(c) || !within(c)) return false;
	pickUpBy(c);
	return true;
}
// picked up: the fighter plays its pickup stance where it has one, in place of the attack the
// press started. a fighter that makes items of its own takes it into its own hand (its code
// throws it, the item's own code sees its attacks first); any other is thrown by this handler
function pickUpBy(c) {
	var own = c.exports != null && c.exports.__tossItem != null;
	// (held in place as a taunt holds it, not an attack: one ended in an attack's state goes on
	// into the next of its moves, which a holder's hand throws the item off)
	if (c.hasAnimation("item_pickup")) {
		c.toState(CState.EMOTE);
		c.playAnimation("item_pickup");
	}
	if (own) self.exports.__holder = c;
	holdBy(c, !own);
	// (the stance it picks it up in is no attack of its own to throw it off: the engine shows the
	// state it was put in a frame on)
	holderAttack.set(attackOf(c));
	takingUp.set(3);
	self.exports.__takingUp = true;
	if (c.exports != null) c.exports.__heldItem = self;
	placeOn(c);
}
// lying free once a throw has come to rest on the floor (its own code bounces it till then)
function cameToRest() {
	if (inItemState(2) && self.isOnFloor() && Math.abs(self.getXSpeed()) < 0.5 && Math.abs(self.getYSpeed()) < 0.5) itemTo(0);
}
// its attack started anew by its own code
function armHits() {
	armAll();
	self.reactivateHitboxes();
}
// its attack set up by its own code: its boxes hit; lying free, they hit anyone, its maker too (a
// bob-omb's blast hurts the one who threw it), where a throw's never hits its thrower
function armAll() {
	armed.set(true);
	if (inItemState(0) && holder.get() == null && self.getOwner() != null) self.setOwner(null);
}
// its boxes hit only while it flies from a throw, or once its own code sets up its attack;
// meeting another's, they touch it either way
function hitOnlyThrown(e) {
	if (!inItemState(2) && !armed.get()) e.data.hitboxStats.disabled = true;
	tell(touchListeners, e);
}

// ---- its own code
// {functions}

function initialize() {
	// its maker's colours
	if (self.getOwner() != null) self.setCostumeIndex(self.getOwner().getCostumeIndex());
	self.setState(PState.ACTIVE);
	self.exports.__ask = asked;
	self.exports.__isItem = true;
	self.exports.__takeBy = takenBy;
	// (the game's listeners for an item made, `itemCreatedListen`: told of this one)
	var told = match.globals.__itemCreatedHeard;
	if (told != null) for (h in told.copy()) h({ data: { item: self, auto: false } });
	self.exports.__inItemState = inItemState;
	// (the one that made it, kept for its code's asks of its maker once its blast is anyone's)
	self.exports.__maker = self.getOwner();
	// (the speed it is thrown at, by whoever throws it)
	self.exports.__tossSpeed = TOSS_SPEED;
	self.addEventListener(GameObjectEvent.HITBOX_CONNECTED, hitOnlyThrown, { persistent: true });
	// how it slows along the floor is its own (`slideDecay`), not the engine's friction
	self.updateGameObjectStats({ friction: SLIDE_FRICTION });
	// {start}
	// {own_start}
	// {publish}
}

function update() {
	if (!started.get()) {
		started.set(true);
		// {first}
	}
	// (one its first frame's code ended runs nothing more: a capsule made inside another ends
	// itself there, and its floor touch would open it)
	if (inItemState(3)) return;
	if (standing.get() > 0) { standing.set(standing.get() - 1); standDown(standingWho.get()); }
	handleItem();
	if (!inItemState(1) && self.isOnFloor()) self.setXSpeed(self.getXSpeed() * SLIDE_KEEP);
	cameToRest();
	// its lifetime runs out while nobody holds it (one that declares none lasts until its own code
	// ends it)
	if (!inItemState(1) && age.inc() >= life.get() && life.get() >= 0) {
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
