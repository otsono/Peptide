// What the source's engine does around every fighter that this engine does differently. Each
// rule here was measured against the source (see docs/STATUS.md for the measurements).

var Fighters = {};
// the engine class a fighter is, whatever class its package declares for it
Fighters.api = function() { return PeptideCharacter; };
// the engine is entering an animation it started: an attack's rules start on its first frame
Fighters.entering = function(me, anim, ?again) {
	var p = PeptideRuntime.pieceOf(anim);
	if (p == null || p.label == "") return;
	// the same attack started over is a new one: it begins again, and hits what the first already
	// hit (luigi's up tilt pressed again as the first ended landed nothing)
	if (again == true && me.__rulesBase == p.label) {
		AttackRules.ended(me);
		me.__rulesBase = null;
		me.__usedLabel = null;
	}
	AttackRules.track(me, p.label);
	Fighters.attackStartSlow(me);
};
Fighters.recover = function(fm, n) { var d = fm.getDamage(); fm.setDamage(Math.max(0, d - AS3.num(n))); return d > 0; };
// the last projectile it made, ended or not: the source hands it back either way, and its code asks
// what it got
Fighters.lastProjectile = function(fm) { var p = fm.exports.__lastProj; return p == null ? null : Objects.view(p); };
Fighters.setCostume = function(me, i) { me.__fm.setCostumeIndex(AS3.int(i)); me.__palette = null; };
Fighters.hat = function(me) { if (me.__hat == null) me.__hat = MovieClip.create(); return me.__hat; };


// the end of a move: on the floor it stands; in the air an attack's animation ends (this engine
// then falls); anything else falls
Fighters.endAnimation = function(me) {
	var fm = me.__fm;
	if (fm.isOnFloor()) fm.toState(CState.STAND);
	else if (fm.inStateGroup(CStateGroup.ATTACK)) fm.endAnimation();
	else fm.toState(CState.FALL);
};

// a body status the source switches on and off: this engine grants one for a span and cannot
// end it early, so a status switched on is granted again every frame until it is switched off


// the speeds the fighter's code set this time: its walk answers an x speed
Fighters.speedsSet = function(me, x) {
	if (x) Fighters.xSet(me, me.__fm.getXSpeed() / PeptideRuntime.data.units.speed);
};
// an x speed the fighter's code set, on the floor. under a held direction the source's walk
// takes a zero to its starting speed (accel_start of norm_xSpeed, both declared) on its next step,
// so a fighter whose walk starts from nothing (sandbag) stops dead until its next write, and one
// whose walk starts moving (bowser writes 0 on his walk's first frame) walks on. a zero with no
// starting speed is held until the next write or the next animation, where this engine's walk
// would accelerate it. a nonzero one under a held direction carries one source frame of the walk's
// acceleration (the source accelerates a scripted walk speed before it moves on it: bowser's 2
// reads 2.5, its cap, on the very next frame)
Fighters.xSet = function(me, v) {
	var fm = me.__fm;
	var c = fm.isOnFloor() ? fm.getHeldControls() : null;
	var held = c != null && (c.LEFT || c.RIGHT);
	if (v == 0 && held && me == PeptideRuntime.object) {
		var st = PeptideRuntime.data.object.stats;
		var start = AS3.num(AS3.at(st, "accel_start")) * AS3.num(AS3.at(st, "norm_xSpeed"));
		if (start > 0) {
			me.__xHold = null;
			fm.setXSpeed(start * PeptideRuntime.data.units.speed);
			return;
		}
	}
	var hold = (v == 0 && fm.isOnFloor()) ? fm.getAnimation() : null;
	if (hold != null && me.__xHold == null) me.__xHoldAt = fm.getX();
	me.__xHold = hold;
	if (v != 0 && held) fm.setXSpeed(fm.getXSpeed() + (v > 0 ? 1 : -1) * fm.getCharacterStat("walkSpeedAcceleration") * 1.5);
};

// what holds on every one of this engine's frames
// the frames a hit held an attacker still: the source moves it none and pulls it down none, but its
// attack's slowdown runs on each of them (wario's side special writes -5 as it hits and is held 3
// frames: it slides off at 3.2 on the source, and kept the whole 5 here, 18 pixels back against 7).
// paid on its first frame out, a source frame's slowdown for each one held, the attack's own where it
// states one and the fighter's otherwise; the source skips it under a direction held where the
// attack gives control and lets it run (both declared, read as the source's defaults when not)
Fighters.freezeSlow = function(me) {
	var n = me.__frozenFrames == null ? 0 : me.__frozenFrames;
	var owed = n - (me.__frozenPaid == null ? 0 : me.__frozenPaid);
	me.__frozenPaid = n;
	var fm = me.__fm;
	if (owed <= 0 || me.__frozenUntil == null || !fm.inStateGroup(CStateGroup.ATTACK)) return;
	var air = !fm.isOnFloor();
	if (!Fighters.attackSlows(me, air)) return;
	// this engine's slowdown is per its own frame, two to the source's one: one per frame held
	fm.setXSpeed(Objects.slowed(fm.getXSpeed(), owed * Objects.slideStep(fm, air) / 1.5));
};
// whether the source slows a fighter in its attack this frame: always, but under a direction held
// where the attack gives control and lets it run (both declared, read as the source's defaults when
// not); an attack that keeps a share of its speed instead slows by that share (Objects.slideShare)
Fighters.attackSlows = function(me, air) {
	if ((air ? me.__decayMulAir : me.__decayMul) != null) return false;
	var c = me.__fm.getHeldControls();
	var dir = c != null && c.LEFT != c.RIGHT;
	var ctl = AS3.truthy(Attacks.stat(me, "allowControl")) && (air || Attacks.stat(me, "allowControlGround") != false);
	return (dir && ctl) || !AS3.truthy(Attacks.stat(me, "allowRun"));
};
// an attack begun on the floor: the source slows the fighter from the attack's first frame, before it
// moves it, where this engine carries the speed it came in with through two of its frames before it
// slows it, and the runtime hears of the attack only once the first of them has moved (falco's forward
// tilt out of a walk slid 27.6 pixels here and 15.5 there). three of this engine's slowdowns are paid
// as soon as it is heard of: the two it skips and the one the source takes first. not when the
// attack's own code wrote a speed that frame
Fighters.attackStartSlow = function(me) {
	var fm = me.__fm;
	var on = fm.inStateGroup(CStateGroup.ATTACK);
	var was = me.__wasAttacking;
	me.__wasAttacking = on;
	if (!on || was == true || !fm.isOnFloor()) return;
	if (me.__slid != null && me.__slid.at == PeptideRuntime.frame) return;
	if (!Fighters.attackSlows(me, false)) return;
	fm.setXSpeed(Objects.slowed(fm.getXSpeed(), 3 * Objects.slideStep(fm, false) / 1.5));
};

// a stood fighter pushed the way behind it: the source turns it and walks it off on the same frame,
// where this engine plays its turn first (falco, walked left 8 frames from a stand facing right, went
// 42 pixels there and 19 here)
Fighters.turnWalk = function(me) {
	var fm = me.__fm;
	if (!fm.inState(CState.STAND_TURN)) return;
	var c = fm.getHeldControls();
	if (c == null || c.LEFT == c.RIGHT) return;
	if (c.LEFT == true && fm.isFacingRight()) fm.faceLeft();
	else if (c.RIGHT == true && !fm.isFacingRight()) fm.faceRight();
	fm.toState(CState.WALK_IN);
};

// an attack in the air catches a ledge its grab box meets, as the source's does for any attack and
// whichever way the fighter faces (falco's side special, dashed off the stage facing away from it,
// hangs from the ledge there; it fell past it here): this engine tries a grab only from the states
// it falls in, and toward the way the fighter faces, so the runtime tries it for an attack, turned
// back if the grab does not take, and not while down is held
Fighters.attackLedge = function(me) {
	var fm = me.__fm;
	if (!fm.inStateGroup(CStateGroup.ATTACK) || fm.isOnFloor()) return;
	var c = fm.getHeldControls();
	if (c != null && c.DOWN == true) return;
	if (fm.attemptLedgeGrab()) return;
	var right = fm.isFacingRight();
	if (right) fm.faceLeft() else fm.faceRight();
	if (fm.attemptLedgeGrab()) return;
	if (right) fm.faceRight() else fm.faceLeft();
};

Fighters.engineFrame = function(me) {
	Fighters.freezeSlow(me);
	Fighters.attackLedge(me);
	Fighters.attackStartSlow(me);
	Fighters.turnWalk(me);
	// a jump's first frame off the floor: the source has the fighter a pixel higher than this engine
	// does from then on (measured with hop_trace.py, the same 1 source pixel for every fighter
	// whatever its gravity, so it is the take-off, not the arc)
	if (me.__stood == true && !me.__fm.isOnFloor() && me.__fm.inState(CState.JUMP_IN)) me.__fm.setY(me.__fm.getY() - PeptideRuntime.data.units.size);
	me.__stood = me.__fm.isOnFloor();
	Fighters.relaunch(me);
	Attacks.carry(me);
	if (me.__stunWant != null) {
		// put in once the hit's freeze is over (nothing runs here while it holds): the source's
		// hold counts down from its first frame out of the freeze (read on mario's tilts at 80%:
		// 19 and 25 held through the freeze, then one less each frame)
		var want = me.__stunWant - 1;
		// a hit that lands while the last one's hold still runs keeps the longer of the two, as the
		// source does (sora's fire, a string of weak hits, dropped its fighter a hit early)
		if (me.__stunEnd != null && me.__stunEnd - PeptideRuntime.frame > want) want = me.__stunEnd - PeptideRuntime.frame;
		me.__stunEnd = PeptideRuntime.frame + want;
		me.__stunWant = null;
		me.__stunLag = null;
		if (want > 0 && me.__fm.getHitstun() > 0) {
			// a hard enough hit sends the source's fighter flying, and it crashes to the floor when it
			// lands; a lighter one only hurts it, and it lands on its feet. this engine picks between
			// its heavy hurt and its light ones by its own measure (mario's forward tilt at 80% flew on the
			// source and stood here, held in its hurt on the floor 5 frames past the source's crash)
			if (Fighters.flies(me.__stunLaunch, want + 1) && !me.__fm.inState(CState.HURT_HEAVY)) me.__fm.toState(CState.HURT_HEAVY);
			me.__stunSet = true;
			me.__fm.forceStartHitstun(want);
		}
	}
	Objects.medium(me);
	AttackRules.engineFrame(me);
	for (st in me.__selfPlatforms) { st.setX(me.__fm.getX()); st.setY(me.__fm.getY()); }
	Fighters.rocketFrame(me);
	Fighters.launchFade(me);
	Fighters.pushFrame(me);
	if (me.__xHold != null) {
		var fm = me.__fm;
		if (fm.getAnimation() != me.__xHold) me.__xHold = null;
		else { fm.setXSpeed(0); fm.setX(me.__xHoldAt); }
	}
};

// a launch fades as the source's does: by the same amount every frame, 0.714 a source frame along
// its line (0.232 an engine frame here), split across and up by the line's direction, in the air
// and on the floor alike. this engine fades it 0.304 a frame along its line in the air (mario's up
// tilt at 80% rose 0.93 a source frame slower each frame against the source's 0.70, and topped
// out under the platform the source's landed on) and across at its whole rate on the floor (fox's
// forward tilt slid to a stop 15 short). each engine frame the fade is put back to the source's;
// a launch that is new, grew (a new hit) or turned takes its own line's split
Fighters.launchFadeRate = 0.232;
Fighters.launchFade = function(me) {
	var fm = me.__fm;
	var kx = fm.getXKnockback();
	var ky = fm.getYKnockback();
	if (me.__launchX != null && me.__fadeX != null) {
		var k = me.__bouncedLaunch == true ? Fighters.fadedShare(me.__launchX, kx) : Fighters.faded(me.__launchX, kx, me.__fadeX);
		if (k != kx) { fm.setXKnockback(k); kx = k; }
		k = me.__bouncedLaunch == true ? Fighters.fadedShare(me.__launchY, ky) : Fighters.faded(me.__launchY, ky, me.__fadeY);
		if (k != ky) { fm.setYKnockback(k); ky = k; }
	}
	if (Fighters.newLaunch(me.__launchX, kx) || Fighters.newLaunch(me.__launchY, ky)) {
		Fighters.fadeOf(me, kx, ky);
		// a launch off the floor (HIT_RECEIVED) is the runtime's own, its share of the source's
		// worked out with this engine's fade given back by its share (Fighters.fadedShare)
		me.__bouncedLaunch = me.__bounceHit == true;
		me.__bounceHit = false;
	}
	// a launch's first frame of gravity (Fighters.gravityOut), on its first frame out
	if (me.__gravNext == true && fm.getHitstop() <= 0) Fighters.gravityOut(me);
	me.__launchX = kx;
	me.__launchY = ky;
};
// a launch component `was` last frame and `now` after this engine's own fade, faded by `step` from
// `was` instead; one that is new, grew or turned, or that the engine did not touch, is left as it is
Fighters.faded = function(was, now, step) {
	// the engine's own fade past nothing ends it a frame or two before the source's would
	if (now == 0 && was != null && was != 0 && Math.abs(was) < 0.35) now = was > 0 ? 0.0001 : -0.0001;
	if (was == null || was == 0 || now == 0 || now == was || (was > 0) != (now > 0) || Math.abs(now) > Math.abs(was)) return now;
	var left = Math.abs(was) - step;
	if (left <= 0) return 0;
	return was > 0 ? left : -left;
};
// the fade a launch of (kx, ky) takes each engine frame, split along its line
Fighters.fadeOf = function(me, kx, ky) {
	var m = Math.sqrt(kx * kx + ky * ky);
	me.__fadeX = m > 0 ? Fighters.launchFadeRate * Math.abs(kx) / m : 0;
	me.__fadeY = m > 0 ? Fighters.launchFadeRate * Math.abs(ky) / m : 0;
};
// a launch component faded by the source's share of this engine's own fade, for a launch the
// runtime bounced off the floor (its 0.62 was measured against this fade: mario's down air struck
// a second time with the source's own)
Fighters.fadedShare = function(was, now) {
	if (was == null || was == 0 || now == 0 || (was > 0) != (now > 0) || Math.abs(now) >= Math.abs(was)) return now;
	return now + (was - now) * (1 - 0.232 / 0.304);
};
Fighters.newLaunch = function(was, now) {
	if (now == 0) return false;
	return was == null || was == 0 || (was > 0) != (now > 0) || Math.abs(now) > Math.abs(was);
};

// a fighter riding a rocket moves by it alone while the attack it rides lasts: the speed it was
// given, then `decay` less each frame down to nothing (measured on samus's bomb lift: a pixel up,
// then 9, 9, 8, ... 1, then held still until her move ends)
Fighters.rocketFrame = function(me) {
	var r = me.__rocket;
	if (r == null) return;
	var fm = me.__fm;
	if (fm.getAnimation() != r.anim) { me.__rocket = null; return; }
	if (r.n >= 2 && r.n % 2 == 0) {
		r.vx = r.vx > 0 ? Math.max(0, r.vx - r.dx) : Math.min(0, r.vx + r.dx);
		r.vy = r.vy > 0 ? Math.max(0, r.vy - r.dy) : Math.min(0, r.vy + r.dy);
	}
	var lift = r.n < 2 ? r.lift : 0;
	r.n++;
	var u = PeptideRuntime.data.units.speed;
	fm.setXSpeed(r.vx * u);
	fm.setYSpeed((r.vy - lift) * u);
};

// an animation that runs out is the engine's to act on (its state machine ends it)
Fighters.playsOut = true;
Fighters.afterSwitch = function(me) { Attacks.carry(me); };
// the engine plays variants of its own on a fighter (a turn, an idle's fidget)
Fighters.variants = true;
// where it comes in the source engine's pass over its objects: first
Fighters.order = 0;
Fighters.waits = function(me) { return false; };
Fighters.attackChanged = function(me, stats) { AttackRules.changed(me, stats); };

// the source's engine plays a fighter's animation clip (its `stance`) itself, every frame, whatever
// the clip's own stop says: a move holds by looping two frames (kirby's stone freezes on
// `freeze`, whose next frame jumps back to it), and a jump by its code plays on from there
Fighters.beforeTick = function(me) {
	var st = PeptideRuntime.stanceOf(me.__mc);
	if (st != null) st.__playing = true;
};

// held still by a hit
Fighters.frozen = function(me) {
	// the frame after a hit that freezes it: held here (Fighters.begin, HIT_DEALT), then let go
	if (me.__holdUntil != null) {
		if (PeptideRuntime.frame <= me.__holdUntil) {
			if (me.__heldOn != PeptideRuntime.frame) { me.__heldOn = PeptideRuntime.frame; me.__heldCount = (me.__heldCount == null ? 0 : me.__heldCount) + 1; }
			return true;
		}
		me.__holdUntil = null;
		me.__fm.resume();
		if (me.__heldSpeed != null) { me.__fm.setXSpeed(me.__heldSpeed[0]); me.__fm.setYSpeed(me.__heldSpeed[1]); me.__heldSpeed = null; }
		if (me.__heldFrames != null) { var fs = me.__heldFrames; me.__heldFrames = null; for (f in fs) f(); }
	}
	return me.__fm.getHitstop() > 0;
};

// a launch's first frame of flight: the source pulls the fighter down by a frame of gravity before
// it moves it (its fall speed reads one frame of gravity on its first frame out), this engine a
// frame later, so the fighter rose a pixel a frame higher all through the flight (mario's up tilt at
// 80% topped out 9 high). over its two frames to the source's one, this engine's fall then trails
// the source's by three quarters of a source frame's gravity each source frame, which one and a
// half of its own frames' gravity given at the start makes up (two left mario's up tilt and bowser's
// down tilt rising a third of a frame's gravity short, and the down tilt's late box struck again). given on its first frame out of the freeze (the engine clears a fall speed
// given as the freeze ends). not for a hit bounced off the floor: that launch is the runtime's own
// (HIT_RECEIVED)
Fighters.gravityOut = function(me) {
	me.__gravNext = false;
	var frames = me.__gravLate == true ? 2.5 : 1.5;
	me.__gravLate = false;
	me.__fm.setYSpeed(me.__fm.getYSpeed() + frames * me.__fm.getGameObjectStat("gravity"));
};

// two fighters standing into each other part as the source parts them: each source frame a
// grounded fighter whose hurt boxes overlap another grounded one's moves that one a source pixel
// away from its own x, whatever the one pushing is doing, and only a fighter standing still is
// moved (not attacking, shielding, dodging, hurt, crouching, landing, grabbing or held). this
// engine's own push leaves a fighter standing in front of an attacking one where it is, and slides a
// hurt one through its freeze: bowser's down tilt began 51 from its fighter on both engines, the
// source's pushed it a pixel a frame through the move's start and the second kick missed it, this
// engine's struck it. the fighter is made not solid, as an object and in each animation it plays,
// and parts the others itself
Fighters.pushable = function(o) {
	if (!o.isOnFloor() || o.getXSpeed() != 0) return false;
	return o.inState(CState.STAND) || o.inState(CState.STAND_TURN) || o.inState(CState.JUMP_SQUAT) || o.inState(CState.WALK_IN) || o.inState(CState.WALK_LOOP) || o.inState(CState.WALK_OUT);
};
Fighters.pushFrame = function(me) {
	var fm = me.__fm;
	var anim = fm.getAnimation();
	if (me.__unsolidAnim != anim) { me.__unsolidAnim = anim; fm.updateAnimationStats({ solid: false }); }
	if (PeptideRuntime.frame % 2 != 1 || !fm.isOnFloor()) return;
	var step = PeptideRuntime.data.units.size;
	for (o in match.getCharacters()) {
		if (o == fm || !Fighters.pushable(o)) continue;
		var res = fm.hitTestEntity(o, CollisionBoxType.HURT, CollisionBoxType.HURT);
		if (res == null || res.length == 0) continue;
		// off a ledge it climbs or rolls from, the fighter pushes the way it faces
		var right = (fm.inState(CState.LEDGE_ROLL) || fm.inState(CState.LEDGE_CLIMB)) ? fm.isFacingRight() : fm.getX() < o.getX();
		o.setX(o.getX() + (right ? step : -step));
	}
};

// a FIXED hit (`weightKB`) launches by the weight of the fighter it hits on the source, its launch
// `(weightKB * 0.5 + 1) * 1.4 * 200 / (weight + 100) + 18`, times its growth, plus its power; this
// engine scales a fixed launch by nothing, and the conversion works it out for a fighter of 95
// (mappings: fixed_knockback). put to the weight of the one it hit as it lands (lloyd's down
// special, fixed at 150, sent a lloyd of 81 13 short of the source's peak)
Fighters.fixedForWeight = function(me, index, foe) {
	if (foe == null || index == null) return;
	var wkb = AS3.num(Attacks.boxStat(me, index + 1, "weightKB"));
	if (!(wkb > 0)) return;
	var w = AS3.num(foe.getCharacterStat("weight"));
	if (!(w > 0) || w == 95) return;
	var g = Attacks.boxStat(me, index + 1, "kbConstant");
	var p = Attacks.boxStat(me, index + 1, "power");
	var gr = g == null ? 1 : AS3.num(g) / 100;
	var pw = p == null ? 0 : AS3.num(p);
	var at = function(weight) { return ((wkb * 0.5 + 1) * 1.4 * 200 / (weight + 100) + 18) * gr + pw; };
	var r = at(w) / at(95);
	if (!(r > 0) || r == 1) return;
	foe.setXKnockback(foe.getXKnockback() * r);
	foe.setYKnockback(foe.getYKnockback() * r);
	var src = foe.exports == null ? null : foe.exports.__source;
	if (src != null && src.__stunLaunch != null) src.__stunLaunch = src.__stunLaunch * r;
};

// a thrown fighter takes back the launch the let-go cleared (HIT_DEALT), out of the hold, over the
// engine's own push for a let-go, and with no freeze of the engine's for it: the source's thrown
// fighter flies on the frame after the throw's hit. the freeze starting is the first the fighter
// hears of the let-go (nothing of its own runs while it holds), else its next frame
Fighters.relaunch = function(me) {
	if (me.__relaunch == null || me.__fm.inState(CState.HELD)) return;
	var rl = me.__relaunch;
	me.__relaunch = null;
	if (me.__fm.getHitstop() > 0) me.__fm.forceStartHitstop(0, false);
	me.__fm.setXKnockback(rl[0]);
	me.__fm.setYKnockback(rl[1]);
	me.__fm.setXSpeed(0);
	// a launch's first frame of gravity (Fighters.gravityOut), given with the launch: the let-go
	// moves the fighter before its next frame of its own (fox's back throw rose 1.4 a frame high
	// with it a frame later)
	me.__fm.setYSpeed(0);
	me.__gravNext = false;
	Fighters.gravityOut(me);
	me.__launchX = rl[0];
	me.__launchY = rl[1];
	Fighters.fadeOf(me, rl[0], rl[1]);
};

// as the fighter is made
Fighters.begin = function(me) {
	me.__fm.updateGameObjectStats({ solid: false });
	me.__fm.addEventListener(GameObjectEvent.ENTER_HITSTOP, function(?e) { Fighters.relaunch(me); }, { persistent: true });
	me.__used = new haxe.ds.StringMap();
	me.__usedAt = new haxe.ds.StringMap();
	me.__usedLabel = null;
	// a box says whether it hits the fighter held: `bypassGrabbed` passes over it (a throw's box
	// for whoever stands near), `bypassNonGrabbed` hits it alone (the throw itself). this engine
	// has no such condition of its own, and asks for the hit to be turned off as it connects
	// which box landed each hit, by the source's name for it, kept as the named value the
	// harness reads on either engine (`gvar pN __peptideHits`; a falsy value starts it afresh)
	me.__fm.addEventListener(GameObjectEvent.HIT_DEALT, function(?e) {
		var hs = e == null || e.data == null ? null : e.data.hitboxStats;
		if (hs == null || hs.index == null) return;
		// a hit lets the attack refresh (AttackRules.refresh)
		me.__refreshReady = true;
		Attacks.freshDealt(me, e.data.foe);
		// (before a throw lets go and keeps the launch the let-go clears)
		Fighters.fixedForWeight(me, hs.index, e.data.foe);
		// the hit that throws lets go of the one it throws as it lands, launch and all: this engine
		// held it on until the attack's own let-go (bowser's up throw carried its fighter up ten
		// frames with no gravity, then dropped it with its launch gone, 0.54 as far as the source's)
		var thrown = e.data.foe;
		if (thrown != null && thrown == me.__releasedFoe) {
			var grabbed = me.__fm.getAllGrabbedFoes();
			if (grabbed != null && grabbed.indexOf(thrown) >= 0) {
				var tkx = thrown.getXKnockback();
				var tky = thrown.getYKnockback();
				me.__fm.releaseCharacter(thrown);
				thrown.setXKnockback(tkx);
				thrown.setYKnockback(tky);
				// the engine clears it again as the let-go goes through and freezes the fighter: the
				// thrown one puts it back (Fighters.relaunch)
				var src = thrown.exports == null ? null : thrown.exports.__source;
				if (src != null) src.__relaunch = [tkx, tky];
			}
		}
		// the box's own hold for the fighter it hit (Fighters.stunOf), handed to that fighter
		var foe = e.data.foe == null || e.data.foe.exports == null ? null : e.data.foe.exports.__source;
		if (foe != null) {
			foe.__stunLag = Attacks.boxStat(me, hs.index + 1, "hitLag");
			if (foe.__stunWant != null && foe.__stunLaunch != null) foe.__stunWant = Fighters.stunWant(foe);
		}
		if (hs.selfHitstop != null && hs.selfHitstop >= 0 && me.__holdNext == true) {
			// the source freezes the attacker from the frame after its hit; this engine runs it one
			// more frame first (its clip went on into the next frame's script: mario's down air
			// refreshed there and hit twice). that frame is held here: the animation stopped and the
			// fighter kept in place. the engine's own freeze takes in that frame, so it keeps its whole
			// length (shortened by one, every hit froze its attacker half a source frame less than
			// the source's: mario's and pac-man's jabs and tilts, freeze_parity.py)
			me.__holdNext = false;
			me.__holdUntil = PeptideRuntime.frame + 1;
			me.__heldSpeed = [me.__fm.getXSpeed(), me.__fm.getYSpeed()];
			me.__fm.pause();
			me.__fm.setXSpeed(0);
			me.__fm.setYSpeed(0);
		}
		if (hs.selfHitstop != null && hs.selfHitstop >= 0 && me.__holdUntil != null) {
			// the source frames the hit holds the attacker for (its engine frames, two to one): a
			// refresh's count stands still through them (AttackRules.refresh), as the source's does
			me.__frozenUntil = PeptideRuntime.frame + hs.selfHitstop;
			AttackRules.hitFrozen(me);
		}
		var was = me.__vars.get("__peptideHits");
		var log = AS3.truthy(was) ? AS3.str(was) : "";
		me.__vars.set("__peptideHits", log + (hs.index == 0 ? "attackBox" : "attackBox" + (hs.index + 1)) + ",");
	}, { persistent: true });
	me.__fm.addEventListener(GameObjectEvent.SHIELD_HIT_DEALT, function(?e) { me.__refreshReady = true; me.__freshAdd = null; }, { persistent: true });
	me.__fm.addEventListener(GameObjectEvent.HITBOX_CONNECTED, function(?e) {
		var hs = e.data.hitboxStats;
		if (hs == null || hs.index == null || hs.index < 0) return;
		var held = me.__fm.getAllGrabbedFoes();
		var isHeld = false;
		if (held != null) for (h in held) if (h == e.data.foe) isHeld = true;
		// one a hit with its effect has already let go of is not held any more, though this engine
		// lets go a frame after the hit (chibi-robo's down throw, refreshed every two frames once its
		// finish has the effect, struck the one it threw a second time the frame after)
		if (isHeld && e.data.foe == me.__releasedFoe) isHeld = false;
		if (isHeld && AS3.truthy(Attacks.boxStat(me, hs.index + 1, "bypassGrabbed"))) { hs.disabled = true; return; }
		// one the attack has thrown is still the one held for the boxes that pass over it, as the
		// source's are (sheik's and lloyd's throws struck the fighter they had just thrown with the
		// box meant for those standing near, once a refresh had come)
		if (!isHeld && e.data.foe == me.__thrownFoe && AS3.truthy(Attacks.boxStat(me, hs.index + 1, "bypassGrabbed"))) { hs.disabled = true; return; }
		if (isHeld) me.__thrownFoe = e.data.foe;
		if (!isHeld && AS3.truthy(Attacks.boxStat(me, hs.index + 1, "bypassNonGrabbed"))) { hs.disabled = true; return; }
		// a box that turns its launch for a fighter behind the attacker judges where the attacker
		// stood before this frame's move, as the source checks its hits before it moves: one that
		// dashes through the fighter in a frame (fox's illusion) sends it the way it was going
		if (hs.reversibleAngle == true) {
			var f = me.__fm;
			var before = f.getX() - f.getXSpeed();
			var ahead = (e.data.foe.getX() - before) * (f.isFacingRight() ? 1 : -1);
			if (ahead > 0) hs.reversibleAngle = false;
		}
		// the attack hits a fighter at most once a source frame, refreshed or not: this engine runs
		// two frames to each of the source's, and a refresh in the first let the second land again
		// (mario's down air hit twice in one source frame; goku's neutral special, refreshed every
		// frame, lands on consecutive source frames on both)
		// (by the source frame the match is on: a frozen attacker's animation stands still, and its
		// next frame's hit is another frame's)
		// (a source frame is two of the animation's own frames, 1-2, 3-4..., as its boxes are laid
		// out: counted on the match's frames instead, the two halves of one box frame could fall in
		// two source frames, and meta knight's tornado, refreshed between them, hit twice in one)
		// (on the match's frames less those the runtime held still, Fighters.frozen: a held frame is
		// no new frame of the source's)
		var cf = me.__fm.getCurrentFrame();
		var eff = PeptideRuntime.frame - (me.__heldCount == null ? 0 : me.__heldCount);
		var at = (eff - (cf - 1)) * 1000 + ((cf - 1) >> 1);
		if (me.__hitAt == null) me.__hitAt = [];
		var again = false;
		for (h in me.__hitAt) if (h.foe == e.data.foe && h.at == at) again = true;
		if (again) { hs.disabled = true; return; }
		// an attack hits each one once until it is refreshed, whichever of its boxes reaches it
		// (here a second box of the same attack hit again: ganondorf's up throw twice)
		if (me.__hitFoes == null) me.__hitFoes = [];
		if (me.__hitFoes.indexOf(e.data.foe) >= 0) { hs.disabled = true; return; }
		me.__hitFoes.push(e.data.foe);
		var kept = [];
		for (h in me.__hitAt) if (h.foe != e.data.foe) kept.push(h);
		kept.push({ foe: e.data.foe, at: at });
		me.__hitAt = kept;
		// what this use of the move deals, by how fresh it is (Attacks.freshness), worked out at its
		// first hit and kept for every hit it lands after
		Attacks.useFresh(me);
		// dealt on top once the hit has landed (HIT_DEALT): the source works the launch out from the
		// move's own damage and deals the freshness on it after (mario's up tilt at 80% launched
		// 2.7% too hard with the freshness in the damage the engine launches from)
		me.__freshAdd = { foe: e.data.foe, add: hs.damage * (me.__fresh - 1) };
		// a hit that freezes the attacker: the frame after it is held by the runtime too (HIT_DEALT)
		if (hs.selfHitstop != null && hs.selfHitstop > 0 && hs.disabled != true) me.__holdNext = true;
		Hitboxes.poison(me, hs.index, e.data.foe);
		// a box with no effect deals its damage to the one held and keeps hold of it: a throw's
		// small hits (bowser's up throw hits six times, then its code gives the box its effect
		// and that hit throws). this engine lets go at any hit, so the damage is dealt here
		// (stated: a box that says nothing of it has its effect, and null reads as false here)
		var effect = Attacks.boxStat(me, hs.index + 1, "hasEffect");
		if (isHeld && (effect == null || AS3.truthy(effect))) me.__releasedFoe = e.data.foe;
		if (isHeld && effect != null && !AS3.truthy(effect)) {
			hs.disabled = true;
			e.data.foe.addDamage(hs.damage * me.__fresh);
			me.__freshAdd = null;
			// still a hit the attack landed: it refreshes after it (AttackRules.refresh), and it
			// freezes the attacker for its own count as any hit does. the engine, told the hit is
			// not one, freezes nothing, so the runtime holds it (pikachu's forward throw shocks the
			// one it holds twice before it throws, each holding pikachu a frame in the source)
			me.__refreshReady = true;
			if (hs.selfHitstop != null && hs.selfHitstop > 0) {
				me.__holdNext = false;
				// the frames it holds count from the one after the hit, and it is let go the frame
				// after the last of them (Fighters.frozen): one less is the box's whole count
				me.__holdUntil = PeptideRuntime.frame + hs.selfHitstop - 1;
				me.__heldSpeed = [me.__fm.getXSpeed(), me.__fm.getYSpeed()];
				me.__fm.pause();
				me.__fm.setXSpeed(0);
				me.__fm.setYSpeed(0);
				me.__frozenUntil = PeptideRuntime.frame + hs.selfHitstop;
				AttackRules.hitFrozen(me);
			}
		}
		// a box that says it reverses the one it hits turns that fighter around, and its launch
		// turns with it (mario's cape, stated at 180, sends the fighter away at 0 in the source)
		else if (AS3.truthy(Attacks.boxStat(me, hs.index + 1, "reverse_character"))) {
			var foe = e.data.foe;
			if (foe.isFacingRight()) foe.faceLeft(); else foe.faceRight();
			hs.angle = 180 - hs.angle;
		}
	}, { persistent: true });
	// how long a hit holds it: the source's hitstun is worked out from the launch, a fifth of it in
	// the source's knockback units (its speed a half frame over 0.1046), rounded down, times the
	// hold its box states and rounded up (Fighters.stunOf), in its frames, from the end of the freeze.
	// this engine holds near 12 for any launch (a jab held twice as long, a strong hit cut short,
	// lloyd's side special dropped its fighter); a straight-line fit to the weak hits held mario's
	// up tilt at 80% 5 frames past the source's
	me.__fm.addEventListener(GameObjectEvent.ENTER_HITSTUN, function(?e) {
		if (me.__stunSet == true) { me.__stunSet = false; return; }
		var f = me.__fm;
		var kx = f.getXKnockback();
		var ky = f.getYKnockback();
		var launch = Math.sqrt(kx * kx + ky * ky) / Fighters.launchScale;
		if (!(launch > 0)) return;
		// the engine sets its own once this has run: the source's is put in on its next frame
		me.__stunLaunch = launch;
		me.__stunWant = Fighters.stunWant(me);
	}, { persistent: true });
	// the source's hurt state is its hitstun: a fighter standing when it runs out is back to
	// standing, where this engine plays the rest of its hurt animation first (kirby's jab held its
	// fighter twice as long)
	me.__fm.addEventListener(GameObjectEvent.EXIT_HITSTUN, function(?e) {
		var f = me.__fm;
		if (me.__stunSet == true || !f.isOnFloor()) return;
		if (f.inState(CState.HURT_LIGHT) || f.inState(CState.HURT_MEDIUM) || f.inState(CState.HURT_HEAVY)) f.toState(CState.IDLE);
	}, { persistent: true });
	// a hit aimed down at it as it stands bounces it up off the floor on both engines, and the
	// source keeps less of the launch through the bounce: 0.62 of this engine's, measured on link's,
	// naruto's, peach's and simon's down tilts and samus's up tilt. this engine has bounced it by
	// the time it is told (its launch points up, and it has left the floor it stood on a frame ago)
	me.__fm.addEventListener(GameObjectEvent.HIT_RECEIVED, function(?e) {
		var f = me.__fm;
		// the freshness the hit's thrower deals on top (Attacks.freshDealt), taken here too: a
		// projectile gone as it hits tells its thrower nothing (ness's pk flash paid 0.947)
		var by = e == null || e.data == null || e.data.foe == null || e.data.foe.exports == null ? null : e.data.foe.exports.__source;
		if (by != null && by.__freshAdd != null) Attacks.freshDealt(by, f);
		if (me.__stood == true && !f.isOnFloor() && f.getYKnockback() < 0) {
			me.__bounced = true;
			f.setXKnockback(f.getXKnockback() * Fighters.groundBounce);
			f.setYKnockback(f.getYKnockback() * Fighters.groundBounce);
		}
		// one too weak for this engine to bounce keeps pointing into the floor here and slides
		// the fighter along it; the source sends it up at the mirrored angle, its whole launch
		// (lucario's jab, stated at 291 degrees, pops a standing fighter up at 69)
		else if (me.__stood == true && f.isOnFloor() && f.getYKnockback() > 0) {
			me.__bounced = true;
			f.setYKnockback(-f.getYKnockback());
		}
		// launched up off the floor it stands on, the source lifts it a pixel as the hit lands (a 55
		// degree launch read 133.25 then 132.25 through the freeze; a flat one stays put), as it does
		// a fighter leaving the floor on a jump
		else if (me.__stood == true && f.isOnFloor() && f.getYKnockback() < 0) f.setY(f.getY() - PeptideRuntime.data.units.size);
		// an upward launch takes a first frame of gravity as its freeze ends (Fighters.gravityOut)
		me.__gravNext = me.__bounced != true && f.getYKnockback() < 0;
		// a hit with no freeze moves the fighter once before its next frame of its own, a frame of
		// its gravity owed on top (the engine clears a fall speed given here as the launch starts)
		me.__gravLate = me.__gravNext && f.getHitstop() <= 0;
		me.__bounceHit = me.__bounced == true;
		me.__bounced = false;
	}, { persistent: true });
};
Fighters.groundBounce = 0.62;
// a launch here against the source's, a frame each (hit_parity.py's expected ratio), and the
// source's hitstun for a launch, in its frames
Fighters.launchScale = 0.65;
// the source's hold for a launch of `launch` source pixels a source frame, in its frames
Fighters.stunOf = function(launch, lag) {
	// a box states its hold as `hitLag`: a whole number of frames when it is not negative, else
	// the share of the launch's hold it takes (-1 the whole, -1.3 a third more: mario's up throw
	// held 39 frames where its launch alone holds 30); one stating none takes the whole
	var l = lag == null ? -1.0 : AS3.num(lag);
	if (Math.isNaN(l)) l = -1.0;
	if (l >= 0) return Math.round(l);
	return Math.ceil(Math.floor(launch / 2 / 0.10464 * 0.2) * -l);
};
// whether a launch of `launch` source pixels a source frame, held `frames` engine frames, sends the
// fighter flying on the source: 8.37 a half frame or more, held 16 of its frames or more
Fighters.flies = function(launch, frames) {
	return launch / 2 >= 8.37 && frames >= 32;
};
// the engine frames a hit holds `me` for, from the launch it took and the hold its box stated
Fighters.stunWant = function(me) {
	return Std.int(Math.round(Fighters.stunOf(me.__stunLaunch, me.__stunLag) * 2));
};

// how many of its frames ago the fighter began an attack before its latest start of it (the one
// named, else the one it is in; -1 out of an attack), as the source's engine counts: an attack is
// begun when its clip goes to the attack's label, and one never begun before counts from the
// start. measured on the source: goku's forward tilt reads its own on its first frame, and the
// first one played reads the frames since the match began, the second the frames since the first
// began (counting from this start, it read 0 and took its jab follow-up every time)
Fighters.lastUsed = function(me, n) {
	var label = n == null ? (me.__mc == null ? null : me.__mc.currentLabel) : AS3.str(n);
	if (n == null && (label == null || AS3.at(PeptideRuntime.data.pkg.attacks, label) == null)) return -1;
	var at = me.__used.exists(label) ? me.__used.get(label) : 0;
	// the attack it has just gone into, before its start is noted: its last start is the one before
	var cur = me.__mc == null ? null : me.__mc.currentLabel;
	if (label == cur && me.__usedLabel != cur && me.__usedAt.exists(label)) at = me.__usedAt.get(label);
	return PeptideRuntime.frame / 2 - at;
};
Fighters.setLastUsed = function(me, n, v) { me.__used.set(AS3.str(n), PeptideRuntime.frame / 2 - AS3.num(v)); };
Fighters.noteUsed = function(me) {
	var label = me.__mc == null ? null : me.__mc.currentLabel;
	if (label == me.__usedLabel) return;
	// the attack it was in is over
	if (me.__usedLabel != null && AS3.at(PeptideRuntime.data.pkg.attacks, me.__usedLabel) != null) Events.dispatch(me, "charAttackComplete");
	me.__usedLabel = label;
	if (label == null || AS3.at(PeptideRuntime.data.pkg.attacks, label) == null) return;
	// the start before this one is what is read from now on
	if (me.__usedAt.exists(label)) me.__used.set(label, me.__usedAt.get(label));
	me.__usedAt.set(label, PeptideRuntime.frame / 2);
};

// an attack started from outside the fighter's own code, at its clip's frame `at`: the engine is
// put in that attack's state on the animation the conversion made of its label (playing the
// animation alone left a standing fighter standing, and a lift its code gave it fell as a plain
// jump), and the clip goes to its label
Fighters.startAsked = function(me, n, ?at) {
	var fm = me.__fm;
	var anim = PeptideRuntime.data.animOf == null ? null : PeptideRuntime.data.animOf(n);
	var st = anim == null ? null : Fighters.stateOf(anim);
	if (st != null && fm.getAnimation() != anim) {
		fm.toState(st, anim);
		// the runtime shows it from here: the clip's jump below lands on this animation's frames
		PeptideRuntime.shown = anim;
		PeptideRuntime.shownBy = anim;
	}
	PeptideGameObject.startAttack(me, n, at);
};
// this engine's attack state for one of its animations, by the name the animation starts with
Fighters.attackStates = [
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
Fighters.stateOf = function(anim) {
	var best = null;
	for (row in Fighters.attackStates) if (anim.indexOf(row[0]) == 0 && (best == null || row[0].length > best[0].length)) best = row;
	return best == null ? null : best[1];
};

// what the source's engine does around the fighter every one of its frames, before its code
Fighters.frame = function(me) {
	Events.stateMoved(me);
	Fighters.noteUsed(me);
	// a throw its item asked for, from the item's own script
	if (me.__fm.exports.__askToss == true) { me.__fm.exports.__askToss = null; Items.throwAsked(me); }
	// a rocket another object's script set off on it (its bomb's lift): into the attack it rides
	var r = me.__fm.exports.__askRocket;
	if (r != null) {
		me.__fm.exports.__askRocket = null;
		var stood = me.__fm.isOnFloor();
		Fighters.startAsked(me, r.attack, "rocket");
		r.anim = me.__fm.getAnimation();
		me.__rocket = r;
		// lifted a pixel off the floor it stands on as it goes: the source moves it the frame the
		// rocket starts, which here is the fighter's next update, so it goes with the first step
		r.lift = r.vy < 0 && stood ? 1 : 0;
	}
	Fighters.squatFrame(me);
	Fighters.smashInput(me);
	AttackRules.frame(me);
	Actions.frame(me);
	Items.frame(me);
	Fighters.smashFrame(me);
};

// attack pressed with a direction pressed just before it is a smash, as the source reads it: the
// controls held no more than 3 of the source's frames before the press (forward) or 2 (up, down)
// play the smash, longer the tilt. the count is of any control held, back to 0 only on a frame
// none is: a way let go and pressed again is fresh, and up after a crouch's down is still a tilt
// (measured on mario, bowser and kirby, from standing, walking and crouching). counted here in this
// engine's frames, two a source frame
Fighters.smashInput = function(me) {
	var fm = me.__fm;
	var h = fm.getHeldControls();
	var p = fm.getPressedControls();
	if (h == null || p == null) return;
	// (rounded up: a state change can take the first engine frame of a hold before it is counted,
	// kirby's up and mario's down read a frame short)
	var before = me.__heldFor == null ? 0 : Std.int((me.__heldFor + 1) / 2);
	var any = h.UP == true || h.DOWN == true || h.LEFT == true || h.RIGHT == true || h.ATTACK == true || h.SPECIAL == true;
	me.__heldFor = any ? (me.__heldFor == null ? 0 : me.__heldFor) + 1 : 0;
	if (p.ATTACK != true || h.SPECIAL == true || !fm.isOnFloor()) return;
	var neutral = fm.inState(CState.STAND) || fm.inState(CState.STAND_TURN) || fm.inState(CState.WALK_IN) || fm.inState(CState.WALK_LOOP)
		|| fm.inState(CState.WALK_OUT) || fm.inState(CState.CROUCH_IN) || fm.inState(CState.CROUCH_LOOP) || fm.inState(CState.CROUCH_OUT);
	// (or the tilt the engine itself just started off this same press)
	var tilt = fm.getCurrentFrame() <= 1 && (fm.inState(CState.TILT_FORWARD) || fm.inState(CState.TILT_UP) || fm.inState(CState.TILT_DOWN));
	if (!neutral && !tilt) return;
	var smash = null;
	var right = fm.isFacingRight();
	if (h.UP == true && before <= 2) smash = CState.STRONG_UP_IN;
	else if (h.DOWN == true && before <= 2) smash = CState.STRONG_DOWN_IN;
	else if ((right ? h.RIGHT == true : h.LEFT == true) && before <= 3) smash = CState.STRONG_FORWARD_IN;
	// the way behind, as fresh, turns the fighter and smashes forward (measured: fox, marth and
	// sheik turned into their forward smash for left and attack pressed together, facing right)
	else if ((right ? h.LEFT == true : h.RIGHT == true) && before <= 3) {
		if (right) fm.faceLeft(); else fm.faceRight();
		smash = CState.STRONG_FORWARD_IN;
	}
	if (smash != null) fm.toState(smash);
};

// up and attack pressed while the fighter crouches to jump is its up smash, as the source plays it
// (measured: mario and bowser play their up smash for up and attack pressed before the jump's last
// grounded frame, which is where this engine's squat ends, and their up air from that frame on; any
// other attack there still plays its aerial)
Fighters.squatFrame = function(me) {
	var fm = me.__fm;
	if (!fm.inState(CState.JUMP_SQUAT) || !fm.isOnFloor()) return;
	var p = fm.getPressedControls();
	var h = fm.getHeldControls();
	if (p == null || h == null || p.ATTACK != true || h.UP != true || h.SPECIAL == true) return;
	fm.toState(CState.STRONG_UP_IN);
};

// how many frames the fighter's controls have been held, as the source counts it for telling a
// flick from a hold (samus's side special fires its power missile off a fresh press, under 4):
// back to 0 on a frame nothing is held but while landing or skidding, one more on any other, and
// out of reach (99) once an attack ends or the fighter lands. kept on its exports, so code of
// any object can read it (Objects.smashTimer)
Fighters.smashFrame = function(me) {
	var fm = me.__fm;
	// run every source frame on every fighter: the engine's own held controls are read as they are
	// (the source's BUTTON1 is special, BUTTON2 attack), and whether the animation is a landing is
	// asked again only when the animation changes
	var c = fm.getHeldControls();
	var anim = fm.getAnimation();
	if (anim != me.__smashAnim) {
		me.__smashAnim = anim;
		me.__smashLanding = PeptideRuntime.data.isLanding != null && PeptideRuntime.data.isLanding(anim) == true;
	}
	var landing = me.__smashLanding == true;
	var atk = me.__rulesBase != null;
	var t = fm.exports.__smashT == null ? 0 : fm.exports.__smashT;
	if (me.__smashAtk == true && !atk) t = 99;
	me.__smashAtk = atk;
	var held = c != null && (c.UP == true || c.DOWN == true || c.LEFT == true || c.RIGHT == true || c.SPECIAL == true || c.ATTACK == true);
	t = (!held && !landing && anim != "skid") ? 0 : t + 1;
	if (landing) t = 99;
	fm.exports.__smashT = t;
};

// the fighter's code changes one of its stats: kept for it to read back, and put on the engine.
// the engine's stats were worked out from the source's by the conversion, so a new value moves
// the engine's by the same share it moves the source's declared one (a gravity written at 2.25
// times its own is 2.25 times the engine's); a stat the source never declared converts by its unit
Fighters.updateStats = function(me, o) {
	var fm = me.__fm;
	var u = PeptideRuntime.data.units;
	var t = PeptideRuntime.data.stats;
	for (k in AS3.keysOf(o)) {
		var v = AS3.at(o, k);
		me.__stats.set(k, v);
		var name = AS3.at(t.names, k);
		if (name == null) name = k;
		// a stat of the source's own that this engine does not keep is only the fighter's
		if (t.record(name, 0) == null) continue;
		if (me.__engineBase == null) me.__engineBase = new haxe.ds.StringMap();
		if (!me.__engineBase.exists(name)) me.__engineBase.set(name, { eng: fm.getCharacterStat(name), src: AS3.at(PeptideRuntime.data.object.stats, k) });
		var base = me.__engineBase.get(name);
		var value = null;
		if (Std.isOfType(v, Float) && base.src != null && AS3.num(base.src) != 0 && base.eng != null) value = AS3.num(base.eng) * AS3.num(v) / AS3.num(base.src);
		else if (Std.isOfType(v, Float)) {
			var f = 1.0;
			if (t.units.size.indexOf(name) >= 0) f = u.size;
			else if (t.units.accel.indexOf(name) >= 0) f = u.accel;
			else if (t.units.speed.indexOf(name) >= 0 || t.units.launch.indexOf(name) >= 0) f = u.speed;
			value = AS3.num(v) * f;
		} else value = v;
		var record = t.record(name, value);
		if (record != null) fm.updateCharacterStats(record);
	}
};

// the source's controls, by its names: its first button is special, its second attack
Fighters.controls = function(me, pressed) {
	var fm = me.__fm;
	var c = AS3.truthy(pressed) ? fm.getPressedControls() : fm.getHeldControls();
	if (c == null) return {};
	return { UP: c.UP, DOWN: c.DOWN, LEFT: c.LEFT, RIGHT: c.RIGHT, JUMP: c.JUMP, JUMP2: false,
	         BUTTON1: c.SPECIAL, BUTTON2: c.ATTACK, SPECIAL: c.SPECIAL, ATTACK: c.ATTACK,
	         SHIELD: c.SHIELD, SHIELD2: false, GRAB: c.GRAB, TAUNT: false, TAP_JUMP: c.TAP_JUMP };
};
