// What the source's engine does with the stats an attack declares (aData, and what the fighter's
// code writes with updateAttackStats), each as it was measured against the source. The attack is
// the fighter clip's label; a piece cut from it is the same attack.

var AttackRules = {};

AttackRules.stat = Attacks.stat;

// every source frame, before the fighter's code
// which attack the fighter is in: its start and end take effect on the engine frame they happen
// (the source's engine starts an attack before it moves the fighter: a recovery's reset of its
// fall speed holds from its first frame, not a frame later)
AttackRules.track = function(me, ?entering) {
	var fm = me.__fm;
	var label = entering != null ? entering : (me.__mc == null ? null : me.__mc.currentLabel);
	// asked every frame: what it answers changes only with the engine's state or the clip's label
	// (an attack entered is always asked afresh)
	var state = fm.getState();
	if (entering == null && state == me.__trackState && label == me.__trackLabel) return;
	me.__trackState = state;
	me.__trackLabel = label;
	// an attack's stats hold while the fighter is attacking: its helpless fall after, shown with
	// the same clip, is not the attack
	// (a throw is an attack in the source; here its states are not in the attack group)
	var attacking = fm.inStateGroup(CStateGroup.ATTACK) || AttackRules.throwing(fm);
	var base = (attacking && label != null && AS3.at(PeptideRuntime.data.pkg.attacks, label) != null) ? label : null;
	if (base != me.__rulesBase) {
		AttackRules.ended(me);
		me.__rulesBase = base;
		if (base != null) AttackRules.began(me, entering != null);
	}
};
AttackRules.frame = function(me) {
	AttackRules.track(me);
	// a charge and a combo are an attack's: out of one, their stats read nothing and they do nothing
	if (me.__rulesBase != null) {
		AttackRules.charge(me);
		AttackRules.combo(me);
	} else me.__chargeT = 0;
	AttackRules.airDodge(me);
};

// the frame label the move's clip says it is in (its `xframe`), which the source's engine reads
AttackRules.xframe = function() {
	var st = PeptideRuntime.shownClip();
	return PeptideRuntime.xframeOf(st);
};

// a charge the source's engine runs: an attack that declares a charge time and does not say it
// charges itself (`useChargeBar: 0`) is charged while its clip is in "charging". held, it stays
// (the clip loops itself); let go, or in the air when it does not charge there, it plays "attack";
// charged to its time, it plays "finishedcharge", and from there a release plays "attack". how
// long it charged is the attack's `chargetime`, for the move's own code to read
AttackRules.charge = function(me) {
	var xf = AttackRules.xframe();
	var ct = AttackRules.stat(me, "chargetime_max");
	var bar = AttackRules.stat(me, "useChargeBar");
	var engine = ct != null && (bar == null || AS3.num(bar) != 0);
	if (!engine || (xf != "charging" && xf != "finishedcharge")) { me.__chargeT = 0; return; }
	var fm = me.__fm;
	var c = fm.getHeldControls();
	var held = c != null && AS3.truthy(c.SPECIAL);
	var go = null;
	if (xf == "charging") {
		me.__chargeT = (me.__chargeT == null ? 0 : me.__chargeT) + 0.5;
		var air = AttackRules.stat(me, "chargeInAir");
		if (!fm.isOnFloor() && air != null && AS3.num(air) == 0) go = "attack";
		else if (!held) go = "attack";
		else if (me.__chargeT >= AS3.num(ct)) go = "finishedcharge";
	} else if (!held) go = "attack";
	if (go == null) return;
	var st = PeptideRuntime.shownClip();
	Attacks.update(me, { chargetime: me.__chargeT });
	PeptideRuntime.setXframe(st, go);
	me.__chargeT = 0;
	st.gotoAndPlay(go);
};

// resetMovement: it starts from a standstill (mario's up special took seven frames of fall
// speed into its climb without it)
AttackRules.reset = function(me) {
	var fm = me.__fm;
	if (AS3.truthy(AttackRules.stat(me, "resetMovement"))) { fm.setXVelocity(0); fm.setYVelocity(0); }
};
// an attack begins: its declared motion, control, statuses and eases take hold
AttackRules.began = function(me, ?entering) {
	var fm = me.__fm;
	AttackRules.reset(me);
	// started as the engine entered it: the engine's own entry of the state comes after, keeps the
	// speed the fighter had and moves it once on that speed before the next update, so the reset is
	// made again there, and an attack that starts from a standstill is put back where it began
	me.__resetAgain = entering == true;
	me.__entryX = fm.getX();
	me.__entryY = fm.getY();
	// its running time starts here (Attacks.execTime)
	me.__execFrom = PeptideRuntime.frame;
	me.__execFrozen = me.__frozenFrames == null ? 0 : me.__frozenFrames;
	// a new attack has hit no one yet, and thrown no one
	me.__hitFoes = null;
	me.__useSerial = (me.__useSerial == null ? 0 : me.__useSerial) + 1;
	me.__thrownFoe = null;
	me.__releasedFoe = null;
	// an attack that says it flips turns its fighter around as it begins (marth's back air). this
	// engine turns no one in a throw: a throw that flips is drawn and launched already turned
	// (entity_gen.rs `turned_throws`), and the thrower turns once it is over
	// (read off the attack being begun: begun as the engine enters it, the clip still shows the
	// one before, and the fighter is still holding)
	var atk = AS3.at(PeptideRuntime.data.pkg.attacks, me.__rulesBase);
	if (atk != null && AS3.truthy(AS3.at(atk, "flip"))) {
		if (AttackRules.throwing(fm) || fm.inState(CState.GRAB_HOLD)) {
			me.__turnAtEnd = true;
		} else if (fm.isFacingRight()) fm.faceLeft(); else fm.faceRight();
	}
	AttackRules.motion(me);
	AttackRules.control(me);
	me.__endSound = AS3.truthy(AttackRules.stat(me, "cancelSoundOnEnd"));
	me.__endVoice = AS3.truthy(AttackRules.stat(me, "cancelVoiceOnEnd"));
};

// the attack's code rewrote some of its stats while it plays: the motion and the steering it
// declares take the new values at once, as the source's engine reads them every frame (peach's
// side special slows its flight by the decay a later frame writes)
AttackRules.changed = function(me, stats) {
	if (me.__rulesBase == null) return;
	var motion = false;
	var control = false;
	var ease = false;
	var ground = false;
	for (k in AS3.keysOf(stats)) {
		var n = AS3.str(k);
		if (n == "air_ease") ease = true;
		if (n == "cancelWhenAirborne" || n == "canFallOff") ground = true;
		if (n == "xSpeedDecay" || n == "xSpeedDecayAir" || n == "xSpeedCap" || n == "xSpeedAccel" || n == "xSpeedAccelAir") motion = true;
		if (n == "allowControl" || n == "allowControlGround") control = true;
	}
	if (motion) AttackRules.motion(me);
	if (control) AttackRules.control(me);
	// a new ease holds the speed the fighter has now (the source caps after its code has run)
	if (ease) AttackRules.ease(me);
	if (ground) AttackRules.ground(me);
};

// the floor's edge: an attack goes off it unless it says it cannot fall off, and ends once its
// fighter is airborne unless it says it carries on there (the conversion states both for how an
// attack starts; its code can change them as it plays: ganondorf's down special lets its slide
// go off the edge, and end there, once it is nearly done)
AttackRules.ground = function(me) {
	var c = AttackRules.stat(me, "cancelWhenAirborne");
	var f = AttackRules.stat(me, "canFallOff");
	me.__fm.updateAnimationStats({ leaveGroundCancel: c == null || AS3.truthy(c), slideOff: f == null || AS3.truthy(f) });
};

// its motion. the conversion puts what an attack declares, and what its code writes as a plain
// value, on the animation's own stats (`runtime/attack_stats.jsonc`); what is left here is what
// the engine has no stat for, and a value the code works out as it plays. a positive slowdown is
// a share kept each frame (jigglypuff's side special keeps 0.8), kept here as its square root
// each of this engine's frames, with the engine's friction off
AttackRules.motion = function(me) {
	var fm = me.__fm;
	var u = PeptideRuntime.data.units;
	var written = Attacks.overrides(me);
	var o = {};
	var any = false;
	me.__decayMul = null;
	me.__decayMulAir = null;
	var v = AttackRules.stat(me, "xSpeedDecay");
	if (v != null && v > 0) { me.__decayMul = Math.sqrt(v); o.friction = 0; any = true; }
	else if (v != null && v < 0 && written.exists("xSpeedDecay")) { o.friction = -v * u.accel; any = true; }
	v = AttackRules.stat(me, "xSpeedDecayAir");
	if (v != null && v > 0) { me.__decayMulAir = Math.sqrt(v); o.aerialFriction = 0; any = true; }
	else if (v != null && v < 0 && written.exists("xSpeedDecayAir")) { o.aerialFriction = -v * u.accel; any = true; }
	if (written.exists("xSpeedCap")) { o.groundSpeedCap = AS3.num(written.get("xSpeedCap")) * u.speed; any = true; }
	if (written.exists("xSpeedAccel")) { o.groundSpeedAcceleration = AS3.num(written.get("xSpeedAccel")) * u.accel; any = true; }
	if (written.exists("xSpeedAccelAir")) { o.aerialSpeedAcceleration = AS3.num(written.get("xSpeedAccelAir")) * u.accel; any = true; }
	if (any) fm.updateAnimationStats(o);
};

// steering. the conversion puts what an attack declares, and what its code writes as a plain
// value, on the animation's own stats (`runtime/attack_stats.jsonc`); what is left here is a value
// the code works out as it plays: in the air when the attack allows control, on the ground only
// when it also allows ground control
AttackRules.control = function(me) {
	var written = Attacks.overrides(me);
	me.__ctlApplied = null;
	if (!written.exists("allowControl") && !written.exists("allowControlGround")) { me.__ctlOn = null; return; }
	me.__ctlOn = AttackRules.stat(me, "allowControl");
	var g = AttackRules.stat(me, "allowControlGround");
	me.__ctlGround = g == null ? true : AS3.truthy(g);
	if (me.__ctlOn == null && g != null) me.__ctlOn = false;
};

// the attack hits again every `refreshRate` of the source's frames while it lasts (the source's
// engine refreshes it; this engine hits each fighter once an attack otherwise): bowser's up
// throw hits seven times there. its code can change the rate as it plays. the source counts the
// attack's frames only while the attacker is not frozen, refreshes once the attack has landed a
// hit, on its Nth frame and every N after, before that frame's hits, and refreshes again only
// after a frame the attacker is free and has not just refreshed: meta knight's drill and
// jigglypuff's down air, refreshed every frame, land a hit, another the next frame, then one every
// three as the freezes go (here, counting every frame, they landed one every two and held the
// fighter inside them). counted here a source frame at a time, on its first engine frame
// a rate the source never refreshes on: none stated, -1, or 999 and up (an attack that hits each
// fighter once): none of the refresh counting runs for it
AttackRules.refreshes = function(r) { var n = Std.int(AS3.num(r)); return n > 0 && n < 999; };
AttackRules.refresh = function(me) {
	var r = AS3.num(AttackRules.stat(me, "refreshRate"));
	if (me.__refreshOf != me.__rulesBase) {
		me.__refreshOf = me.__rulesBase;
		me.__refreshAt = me.__fm.getCurrentFrame() - 1;
		// the source's count takes a value only once the attack has landed a hit: an attack begins it
		// at 1 when the one before had landed, and leaves it where that one stopped when it had not;
		// it counts on from the hit (read frame by frame on the source: bandana dee's neutral special
		// sits at 1 through its first four frames, lands, and refreshes on its fourth free frame after)
		if (me.__refreshReady == true || me.__refreshCount == null) me.__refreshCount = 1;
		me.__refreshReady = false;
		me.__refreshDone = false;
	}
	me.__refreshAt++;
	if (!AttackRules.refreshes(r)) return;
	if (PeptideRuntime.frame % 2 != 1) return;
	var frozen = Fighters.frozen(me) || (me.__frozenUntil != null && PeptideRuntime.frame <= me.__frozenUntil);
	if (!frozen && me.__refreshReady == true) me.__refreshCount++;
	var n = Std.int(r);
	if (n > 0 && me.__refreshDone != true && me.__refreshReady == true && me.__refreshCount % n == 0) {
		me.__refreshDone = true;
		PeptideGameObject.reactivate(me);
	} else if (!frozen) me.__refreshDone = false;
};

// a hit that freezes the attacker: the source still counts toward a refresh on the frame after, while
// it is frozen, with the count where the hit left it, and the attacker's boxes still hit then. this
// engine runs nothing of the fighter while it is frozen, so the refresh that frame comes as the hit
// lands (meta knight's drill and jigglypuff's down air land a second hit the frame after their
// first); the source frame the hit landed in is not hit again (Fighters, `__hitAt`)
AttackRules.hitFrozen = function(me) {
	if (me.__rulesBase == null || !AttackRules.refreshes(AttackRules.stat(me, "refreshRate"))) return;
	var n = Std.int(AS3.num(AttackRules.stat(me, "refreshRate")));
	if (n > 0 && me.__refreshDone != true && me.__refreshCount % n == 0) {
		me.__refreshDone = true;
		PeptideGameObject.reactivate(me);
	}
};

AttackRules.throwing = function(fm) {
	return fm.inState(CState.THROW_UP) || fm.inState(CState.THROW_DOWN) || fm.inState(CState.THROW_FORWARD) || fm.inState(CState.THROW_BACK);
};

// an attack ends: the sound or voice it said to stop goes
AttackRules.ended = function(me) {
	if (me.__rulesBase == null) return;
	if (me.__turnAtEnd == true) { me.__turnAtEnd = false; if (me.__fm.isFacingRight()) me.__fm.faceLeft(); else me.__fm.faceRight(); }
	if (me.__endSound == true) Sounds.stopLast(me, false);
	if (me.__endVoice == true) Sounds.stopLast(me, true);
	me.__endSound = false;
	me.__endVoice = false;
	me.__decayMul = null;
	me.__decayMulAir = null;
	me.__ctlOn = null;
};

// air ease its code works out as it plays: the engine's own cap on the fall (a plain value is
// set at conversion)
AttackRules.ease = function(me) {
	var cap = AttackRules.easeCap(me);
	me.__fm.updateAnimationStats({ terminalVelocity: cap == null ? -1 : cap });
	// the source clamps the fall to it on the spot; this engine only stops gravity past it
	if (cap != null && me.__fm.getYVelocity() > cap) me.__fm.setYVelocity(cap);
};
// the fall its attack's ease holds it to, in this engine's units (null when none is in force)
AttackRules.easeCap = function(me) {
	var ease = me.__rulesBase == null ? null : AttackRules.stat(me, "air_ease");
	return ease == null || AS3.num(ease) < 0 ? null : AS3.num(ease) * PeptideRuntime.data.units.speed;
};
// every engine frame
AttackRules.engineFrame = function(me) {
	var fm = me.__fm;
	AttackRules.track(me);
	if (me.__rulesBase == null) return;
	AttackRules.refresh(me);
	if (me.__resetAgain == true) {
		me.__resetAgain = false;
		// along the floor too: the step it took on its old speed is not the source's (falco's side
		// special out of a walk slid 9 pixels into it, and off the ledge the source stops short of)
		if (AS3.truthy(AttackRules.stat(me, "resetMovement"))) {
			fm.setX(me.__entryX);
			if (!fm.isOnFloor()) fm.setY(me.__entryY);
		}
		AttackRules.reset(me);
		// and the engine's entry put the animation's own stats back over the declared motion and
		// steering (ganondorf's down special slid on his own friction, not its declared 1 a frame)
		AttackRules.motion(me);
		AttackRules.control(me);
	}
	// an attack's `hit_ease` moves nothing the source shows: link's forward air (2) and pikachu's
	// (-3) fall on through their hit at the speed they had, land, and are cut to their landing,
	// where holding the fall here (or popping it up) kept them in the air for a second hit
	var m = fm.isOnFloor() ? me.__decayMul : me.__decayMulAir;
	if (m != null) fm.setXVelocity(fm.getXVelocity() * m);
	// invincibility and super armor it declares (or its writes switch), held for the attack
	var status = null;
	if (AS3.truthy(AttackRules.stat(me, "invincible"))) status = AS3.truthy(AttackRules.stat(me, "forceGrabbable")) ? BodyStatus.INVINCIBLE_GRABBABLE : BodyStatus.INVINCIBLE;
	else if (AS3.truthy(AttackRules.stat(me, "superArmor"))) status = BodyStatus.SUPER_ARMOR;
	if (status != null) fm.applyGlobalBodyStatus(status, 2);
	// a landing's own animation states its steering itself (what the source's landing wrote)
	var landing = PeptideRuntime.data.isLanding != null && PeptideRuntime.data.isLanding(fm.getAnimation()) == true;
	if (me.__ctlOn != null && !landing) {
		var want = AS3.truthy(me.__ctlOn) && (!fm.isOnFloor() || me.__ctlGround);
		if (want != me.__ctlApplied) { me.__ctlApplied = want; fm.updateAnimationStats({ allowMovement: want }); }
	}
};

// the source's combo protocol, which its engine runs and the clip only answers: while an attack
// that declares combo_max plays, its button pressed (or held, for one that repeats on a hold)
// tells the clip to go on, through the two fields the fighter keeps (handled, continuePlaying);
// one that also charges goes on only once it is attacking (its `xframe` at "attack")
AttackRules.combo = function(me) {
	var cm = AttackRules.stat(me, "combo_max");
	if (cm == null || AS3.num(cm) < 1) return;
	var ct = AttackRules.stat(me, "chargetime_max");
	var xf = AttackRules.xframe();
	if (ct != null && AS3.num(ct) > 0 && xf != null && xf != "attack") return;
	if (me.handled != false) return;
	var fm = me.__fm;
	var special = fm.inState(CState.SPECIAL_NEUTRAL) || fm.inState(CState.SPECIAL_SIDE) || fm.inState(CState.SPECIAL_UP) || fm.inState(CState.SPECIAL_DOWN);
	var held = fm.getHeldControls();
	var pressed = fm.getPressedControls();
	var p = special ? pressed.SPECIAL : (pressed.ATTACK || pressed.GRAB);
	var h = special ? held.SPECIAL : (held.ATTACK || held.GRAB);
	if (AS3.truthy(p) || (AS3.truthy(AttackRules.stat(me, "holdRepeat")) && AS3.truthy(h))) { me.continuePlaying = true; me.handled = true; }
};

// the source's air dodge: on a fresh shield press in the air it plays its own clip where it is,
// keeping its momentum (measured: the source moves no distance on a neutral dodge); this
// engine's airdash, which blinks across the stage, is switched off in the fighter's stats
AttackRules.airDodge = function(me) {
	var fm = me.__fm;
	var c = fm.getHeldControls();
	var held = c != null && c.SHIELD == true;
	var fresh = held && me.__dodgeHeld != true;
	me.__dodgeHeld = held;
	if (!fresh || fm.isOnFloor() || PeptideRuntime.pieceOf("airdodge") == null) return;
	if (!(fm.inState(CState.JUMP_IN) || fm.inState(CState.JUMP_LOOP) || fm.inState(CState.JUMP_OUT) || fm.inState(CState.JUMP_MIDAIR) || fm.inState(CState.FALL))) return;
	fm.toState(CState.FALL_SPECIAL);
	fm.playAnimation("airdodge");
};
