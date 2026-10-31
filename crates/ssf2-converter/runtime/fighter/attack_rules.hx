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
};

// resetMovement its code writes as it plays: it starts from a standstill (a declared one is the
// attack's own first-frame code, `runtime/attack_stats.jsonc`)
AttackRules.reset = function(me) {
	var fm = me.__fm;
	if (Attacks.overrides(me).exists("resetMovement") && AS3.truthy(AttackRules.stat(me, "resetMovement"))) { fm.setXVelocity(0); fm.setYVelocity(0); }
};
// an attack begins: its declared motion, control, statuses and eases take hold
AttackRules.began = function(me, ?entering) {
	var fm = me.__fm;
	AttackRules.reset(me);
	// started as the engine entered it: the engine's own entry of the state comes after and keeps the
	// speed the fighter had, so the reset is made again there
	me.__resetAgain = entering == true;
	// its running time starts here (Attacks.execTime)
	me.__execFrom = PeptideRuntime.frame;
	me.__execFrozen = me.__frozenFrames == null ? 0 : me.__frozenFrames;
	// a new attack has hit no one yet, and thrown no one
	me.__hitFoes = null;
	me.__useSerial = (me.__useSerial == null ? 0 : me.__useSerial) + 1;
	me.__thrownFoe = null;
	me.__releasedFoe = null;
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
// a share kept each frame (jigglypuff's side special keeps 0.8), run by the attack's own first-frame
// code (`attack_stats.jsonc`). not ported: a share its code works out or writes as it plays
AttackRules.motion = function(me) {
	var fm = me.__fm;
	var u = PeptideRuntime.data.units;
	var written = Attacks.overrides(me);
	var o = {};
	var any = false;
	var v = AttackRules.stat(me, "xSpeedDecay");
	if (v != null && v < 0 && written.exists("xSpeedDecay")) { o.friction = -v * u.accel; any = true; }
	v = AttackRules.stat(me, "xSpeedDecayAir");
	if (v != null && v < 0 && written.exists("xSpeedDecayAir")) { o.aerialFriction = -v * u.accel; any = true; }
	if (written.exists("xSpeedCap")) { o.groundSpeedCap = AS3.num(written.get("xSpeedCap")) * u.speed; any = true; }
	if (written.exists("xSpeedAccel")) { o.groundSpeedAcceleration = AS3.num(written.get("xSpeedAccel")) * u.accel; any = true; }
	if (written.exists("xSpeedAccelAir")) { o.aerialSpeedAcceleration = AS3.num(written.get("xSpeedAccelAir")) * u.accel; any = true; }
	if (any) fm.updateAnimationStats(o);
};

// steering. the conversion puts what an attack declares, and what its code writes as a plain
// value, on the animation's own stats (`runtime/attack_stats.jsonc`); what is left here is a value
// the code works out as it plays, held by a timer for as long as the attack lasts: in the air when
// the attack allows control, on the ground only when it also allows ground control (a landing's
// own animation states its steering itself, what the source's landing wrote)
AttackRules.control = function(me) {
	var fm = me.__fm;
	AttackRules.stopControl(me);
	var written = Attacks.overrides(me);
	if (!written.exists("allowControl") && !written.exists("allowControlGround")) return;
	var on = AttackRules.stat(me, "allowControl");
	var g = AttackRules.stat(me, "allowControlGround");
	var ground = g == null ? true : AS3.truthy(g);
	if (on == null && g != null) on = false;
	if (on == null) return;
	var base = me.__rulesBase;
	var s = { applied: null };
	var id = -1;
	id = fm.addTimer(1, -1, function(?t) {
		if (fm.isDisposed()) return;
		if (me.__rulesBase != base) { fm.removeTimer(id); return; }
		if (PeptideRuntime.data.isLanding != null && PeptideRuntime.data.isLanding(fm.getAnimation()) == true) return;
		var want = AS3.truthy(on) && (!fm.isOnFloor() || ground);
		if (want != s.applied) { s.applied = want; fm.updateAnimationStats({ allowMovement: want }); }
	}, { persistent: true });
	me.__ctlWatch = id;
};
AttackRules.stopControl = function(me) {
	if (me.__ctlWatch != null && !me.__fm.isDisposed()) me.__fm.removeTimer(me.__ctlWatch);
	me.__ctlWatch = null;
};

// the attack hits again every `refreshRate` of the source's frames while it lasts (the source's
// engine refreshes it; this engine hits each fighter once an attack otherwise): bowser's up throw
// hits seven times there. the source counts from the attack's first landed hit, only on frames
// the attacker is free, and refreshes at the rate's every multiple: n-1 free frames after that hit,
// then every n. here an engine timer counts it, waiting out the attacker's freezes
// (`pauseCondition`). at a rate of 1 the source also refreshes in the frame after its first hit,
// inside the freeze (meta knight's drill and jigglypuff's down air land a second hit the frame after their
// first): here that refresh comes as the hit lands. its code can change the rate as it plays.
// a rate the source never refreshes on: none stated, -1, or 999 and up (an attack that hits each
// fighter once)
AttackRules.refreshes = function(r) { var n = Std.int(AS3.num(r)); return n > 0 && n < 999; };
// every engine frame: a new attack starts with no refresh running; a rate its code changes restarts it
AttackRules.refresh = function(me) {
	if (me.__refreshOf != me.__rulesBase) { me.__refreshOf = me.__rulesBase; AttackRules.stopRefresh(me); return; }
	if (me.__refreshTimer == null) return;
	var r = Std.int(AS3.num(AttackRules.stat(me, "refreshRate")));
	if (r != me.__refreshRate) { AttackRules.stopRefresh(me); if (AttackRules.refreshes(r)) AttackRules.startRefresh(me, r, r); }
};
// a hit the attack landed (on a fighter, a shield, or one it holds)
AttackRules.landed = function(me) {
	if (me.__rulesBase == null) return;
	var r = AS3.num(AttackRules.stat(me, "refreshRate"));
	if (!AttackRules.refreshes(r)) return;
	if (me.__refreshTimer != null) return;
	var n = Std.int(r);
	if (n == 1) PeptideGameObject.reactivate(me);
	AttackRules.startRefresh(me, n, n > 1 ? n - 1 : 1);
};
AttackRules.startRefresh = function(me, n, firstIn) {
	var fm = me.__fm;
	// frozen: the engine's own freeze, the runtime's hold, or the source's freeze for the hit (its
	// box's stated self-hitstop, where the engine's is shorter or none)
	var paused = function() { return fm.getHitstop() > 0 || (me.__holdUntil != null && PeptideRuntime.frame <= me.__holdUntil) || (me.__frozenUntil != null && PeptideRuntime.frame <= me.__frozenUntil); };
	var go = function(?t) { if (!fm.isDisposed() && me.__refreshTimer != null) PeptideGameObject.reactivate(me); };
	me.__refreshRate = n;
	me.__refreshTimer = fm.addTimer(2 * firstIn, 1, function(?t) {
		go();
		if (me.__refreshTimer != null && !fm.isDisposed()) me.__refreshTimer = fm.addTimer(2 * n, -1, go, { pauseCondition: paused });
	}, { pauseCondition: paused });
};
AttackRules.stopRefresh = function(me) {
	if (me.__refreshTimer != null && !me.__fm.isDisposed()) me.__fm.removeTimer(me.__refreshTimer);
	me.__refreshTimer = null;
	me.__refreshRate = null;
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
	AttackRules.stopControl(me);
};

// air ease its code works out as it plays: the engine's own cap on the fall (a plain value is
// set at conversion)
AttackRules.ease = function(me) {
	var cap = AttackRules.easeCap(me);
	me.__fm.updateAnimationStats({ terminalVelocity: cap == null ? -1 : cap });
	// not ported: the source clamps the fall to it on the spot, where this engine only stops adding
	// gravity past it (a fall already faster than the cap keeps its speed a frame)
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
		// not ported: the engine has moved it one step on its old speed by now, and the source's never
		// took that step (falco's side special out of a walk slides 9 pixels further here)
		AttackRules.reset(me);
		// and the engine's entry put the animation's own stats back over the declared motion and
		// steering (ganondorf's down special slid on his own friction, not its declared 1 a frame)
		AttackRules.motion(me);
		AttackRules.control(me);
	}
	// an attack's `hit_ease` moves nothing the source shows: link's forward air (2) and pikachu's
	// (-3) fall on through their hit at the speed they had, land, and are cut to their landing,
	// where holding the fall here (or popping it up) kept them in the air for a second hit
};
